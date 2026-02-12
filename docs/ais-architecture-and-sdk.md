# AIS 架构与 TS SDK 实现总览（用于设计与实现 Review）

面向读者：需要快速理解 AIS 的整体架构、关键设计点、模块边界，并能对照 TS SDK 的实现做 review 的工程同学。

本仓库的 AIS 主要由两部分组成：
- **规范（Spec）**：`specs/` 下的分拆文档（AIS-0/1/2/3/4）
- **实现（TS SDK）**：`ts-sdk/src/`，提供 schema/解析/加载/校验/执行计划/引擎参考实现等

本文尽量用“分层架构 + 入口文件”把链路串起来，避免陷入单个字段细节。字段级细节请直接参考 `specs/` 与 `ts-sdk/src/schema/*`。

---

## 1. 一页速览（AIS 是什么，解决什么问题）

AIS（Agent Interaction Spec）要解决的是：让 AI/Agent 能以**可验证、可约束、跨链**的方式与链上协议交互，避免“把执行指令写死在代码里”或“从不可信 URL 拉脚本”带来的风险。

AIS 的核心思想是把系统拆成三层角色：
- **协议作者（Protocol Spec）**：描述“能做什么、怎么做、风险是什么”
- **部署/风控方（Pack）**：选择允许的协议版本，并定义“强约束与审批策略”
- **编排方（Workflow）**：把多个协议动作/查询按 DAG 编排成可执行流程

典型端到端链路（从 YAML 到执行）在 SDK 内可以映射为：

```text
YAML 文件(Protocol/Pack/Workflow)
  -> parser/schema 校验
  -> workspace 关系校验 (workflow -> pack -> protocol)
  -> resolver context (runtime root + 已加载 protocols)
  -> ExecutionPlan (ais-plan/0.0.3)
  -> engine.runPlan() 事件流
  -> executor 执行链上 IO (EVM/Solana/...)
```

把 “安全” 放到哪里：
- **静态层面**：schema 严格校验（strict + extensions）、workspace 引用一致性校验
- **策略层面**：pack 的 allowlist 与 policy gate（滑点、无限授权、token allowlist、风险审批阈值等）
- **运行层面**：ExecutionPlan readiness、solver/executor 分离、checkpoint/trace（可恢复、可审计）

---

## 2. 规范分层与文档地图（specs/）

AIS 0.0.2 的规范索引：`specs/index.md`

建议按下面顺序阅读（从“概念”到“可执行”）：

1. **AIS-0 总览**：`specs/ais-0-overview.md`
2. **AIS-1 核心三文档**
   - Protocol Spec：`specs/ais-1-protocol.md`（`schema: ais/0.0.2`）
   - Pack：`specs/ais-1-pack.md`（`schema: ais-pack/0.0.2`）
   - Workflow：`specs/ais-1-workflow.md`（文档写 `ais-flow/0.0.2`）
3. **AIS-1 关键底座**
   - Types & Numeric Model：`specs/ais-1-types.md`
   - ValueRef：`specs/ais-1-expressions.md`
   - Capabilities：`specs/ais-1-capabilities.md`
4. **AIS-2 执行层**：`specs/ais-2-evm.md`、`specs/ais-2-solana.md`、`specs/ais-2-composite.md`
5. **AIS-3 分发/发现/验证**：`specs/ais-3-registry.md` 等
6. **AIS-4 一致性向量**：`specs/ais-4-conformance.md`（配套向量 `specs/conformance/vectors/*.json`）

重要“规范原则”（会影响实现与 review 重点）：
- **strict schemas**：未知字段必须拒绝，扩展只能放 `extensions`
- **ValueRef 强制结构化**：动态值必须显式 `{lit|ref|cel|detect|object|array}`
- **数值模型明确**：执行关键路径禁止 IEEE754，建议用 BigInt + 精确十进制模型

---

## 3. AIS 的核心对象与职责边界

### 3.1 Protocol Spec（`ais/0.0.2`）

它描述单一协议的“接口与执行配方”：
- `meta`：协议 id/version/name/tags 等
- `deployments[]`：不同链上的合约地址绑定（`contracts: {name->address}`）
- `actions{}` / `queries{}`：操作与查询
- `execution{}`：链特定执行 spec（AIS-2）
- `capabilities_required[]`：协议或 action 的能力要求（引擎是否支持）
- 风险字段：`risk_level` / `risk_tags` / `risks`（供 pack/引擎做 policy gate）

实现要点（review 关注）：
- 协议作者是“描述者”，不应把不安全的执行能力藏进可变字符串里（ValueRef 必须结构化）
- 风险/约束的结构需要能被 pack/engine 消费（否则只是注释）

对应 SDK 入口：
- schema：`ts-sdk/src/schema/protocol.ts`
- parser：`ts-sdk/src/parser.ts`（`parseProtocolSpec` 额外做 plugin execution type 校验与语义断言）

### 3.2 Pack（`ais-pack/0.0.2`）

Pack 是 **策略边界** 与 **allowlist 边界**：
- `includes[]`：允许使用哪些协议版本（protocol@version），可选 `chain_scope`
- `policy`：审批阈值、hard constraints 默认值等
- `token_policy`：token allowlist 与解析策略
- `providers`：quote/detect providers allowlist（capabilities 边界的一部分）
- `plugins`：execution plugins allowlist（非 core execution types）
- `overrides`：对特定 action 的覆盖（当前 SDK 侧实现支持有限，见后文）

对应 SDK 入口：
- schema：`ts-sdk/src/schema/pack.ts`
- parser：`ts-sdk/src/parser.ts`（`parsePack`）
- policy gate：`ts-sdk/src/validator/constraint.ts`（`validateConstraints`）
- workspace allowlist 校验：`ts-sdk/src/validator/workspace.ts`（detect/providers/plugins 与 includes 的一致性）

Pack 的更细使用说明见：`docs/ais-pack.md`

### 3.3 Workflow（`ais-flow/*`）

Workflow 描述跨协议编排（DAG）：
- `nodes[]`：`action_ref` / `query_ref` 两种引用节点
- `deps`：显式依赖
- `args`：节点参数，必须是 ValueRef
- `default_chain` + `nodes[].chain`：链选择（每个 node 必须落到具体 CAIP-2 chain id）
- `until/retry/timeout_ms`：引擎驱动的轮询语义
- `assert/assert_message`：引擎驱动的后置断言
- `requires_pack`：声明需要某个 pack（策略与 allowlist 边界）

对应 SDK 入口：
- schema：`ts-sdk/src/schema/workflow.ts`（当前为 `ais-flow/0.0.3`）
- workflow validator：`ts-sdk/src/validator/workflow.ts`
- DAG 构建：`ts-sdk/src/workflow/dag.ts`
- ExecutionPlan 构建：`ts-sdk/src/execution/plan.ts`（`buildWorkflowExecutionPlan`）

一致性提醒（review 必看）：
- spec 文档中 workflow schema 写作 `ais-flow/0.0.2`（`specs/ais-0-overview.md`、`specs/ais-1-workflow.md`）
- TS SDK 的 `WorkflowSchema`/parser/loader 使用的是 `ais-flow/0.0.3`（`ts-sdk/src/schema/workflow.ts`、`ts-sdk/src/parser.ts`）
- `ts-sdk/src/workflow/README.md` 与 `ts-sdk/src/scripts/build-schema.ts` 仍提到 `ais-flow/0.0.2`

这类“版本漂移”会直接影响：
- 文件识别（loader/CLI）
- schema discriminated union
- conformance 测试与外部实现对齐

---

## 4. TS SDK 模块地图（ts-sdk/src/）

建议把 SDK 理解成“从静态校验，到可执行计划，再到参考引擎”的一条流水线。

### 4.1 schema：Zod 运行时校验 + TS 类型推断

目录：`ts-sdk/src/schema/`
- `common.ts`：CAIP-2、Address、Asset、TokenAmount、ValueRef、extensions 等基础类型
- `protocol.ts` / `pack.ts` / `workflow.ts`：三文档 schema
- `execution.ts`：AIS-2 执行 spec 的 schema（EVM/Solana/BTC/Composite + plugin）
- `conformance.ts`：conformance 向量文件 schema

核心设计点：
- 所有对象 `.strict()`，未知字段直接报错
- `extensions` 是唯一的扩展槽（`z.record(z.unknown()).optional()`）

### 4.2 parser：YAML 解析 + schema 校验 +（部分）后置语义校验

入口：`ts-sdk/src/parser.ts`
- `parseAIS`：按 `schema` discriminator 自动解析；对 protocol 额外做执行插件校验与 `assertProtocolSemantics`
- `parseProtocolSpec` / `parsePack` / `parseWorkflow`
- YAML 解析开启 `uniqueKeys: true`（重复 key 会报错）

设计关注点：
- parse 阶段只做“单文档级”校验，跨文档引用一致性在 workspace validator 做
- plugin execution types 需要预注册，否则 parser 会报错（见 plugins 模块）

### 4.3 loader：文件系统加载 + bundle/目录扫描

入口：`ts-sdk/src/loader.ts`
- `loadDirectory()`：识别 `.ais.yaml` / `.ais-pack.yaml` / `.ais-flow.yaml` 并分类加载
- `loadDirectoryAsContext()`：加载协议并注册到 resolver context（source: workspace）
- `loadWorkflowBundle()`：按 workflow.imports.protocols 把 protocol 导入到一个新 context，并可选 `validateWorkflow`

注意：
- loader 负责 IO 与路径解析；validator/workspace 则是 filesystem-agnostic

### 4.4 resolver：引用解析 + ValueRef 求值 + CEL + detect

目录：`ts-sdk/src/resolver/`
- `context.ts`：`ResolverContext`（runtime root + 已加载 protocols + protocol_sources）
- `reference.ts`：协议/action/query 引用解析、pack expand、contracts 选择等
- `expression.ts`：`${...}` 模板解析（可选糖）
- `value-ref.ts`：ValueRef（lit/ref/cel/detect/object/array）求值，同步/异步

配套：
- CEL：`ts-sdk/src/cel/`（自研 parser/evaluator + 精确数值模型）
- detect：`ts-sdk/src/detect/`（provider registry + `createDetectResolver`）

review 重点：
- ValueRef 是整个体系“语义唯一性”的基石，必须避免“裸字符串同时表示 literal/ref/cel”
- 数值模型是否在执行关键路径真正禁用 `number`（尤其金额/滑点/minOut）

### 4.5 validator：静态约束与一致性校验

目录：`ts-sdk/src/validator/`
- `workflow.ts`：workflow 节点引用、deps、ValueRef ref/cel 的基本绑定检查 + DAG cycle
- `workspace.ts`：跨文件校验（workflow.requires_pack -> pack -> protocol 的闭环）
- `constraint.ts`：pack policy gate（token allowlist、滑点上限、无限授权、风险审批阈值等）
- `lint.ts`：最佳实践 lint（可扩展）
- `plugins.ts`：validator 插件机制

review 重点：
- workspace 校验是否真正把 pack 的 allowlist 变成“强约束”，防止 workflow/protocol 绕过
- constraint.ts 当前覆盖面是否与 pack schema 字段一致（存在字段支持但未 enforce 的情况，后文列出）

### 4.6 execution：ExecutionPlan IR + 编译/编码工具

目录：`ts-sdk/src/execution/`
- `plan.ts`：`ExecutionPlan (ais-plan/0.0.3)`，workflow -> plan，readiness 检查
- `evm/`：JSON ABI 编码/解码、keccak、EVM execution 编译（sync/async）
- `solana/`：Solana instruction 规划辅助
- `builder.ts`：legacy builder（README 提示不推荐，推荐 plan）

关键设计点：
- ExecutionPlan 是 JSON 可序列化的 DAG IR，便于 checkpoint/trace/跨进程协作
- readiness 把“缺上下文输入/需要 detect”显式化，交给 solver/外部系统处理

### 4.7 engine：参考执行引擎（planner/solver/executor 解耦）

目录：`ts-sdk/src/engine/`
- `runner.ts`：`runPlan()`，调度、并发控制、polling、checkpoint、trace
- `solvers/solver.ts`：最小内置 solver（自动填 contracts + 缺输入时 need_user_confirm）
- `executors/*`：参考 executor（EVM JSON-RPC、Solana RPC）
- `patch.ts`：runtime patch（set/merge）与 undo 记录

重要边界：
- engine 本身不做网络 IO，网络 IO 由 executor 插件执行
- engine 事件流（`EngineEvent`）适合作为 UI/agent 的进度与交互接口

### 4.8 cli：validate/lint/check 的命令行工具

目录：`ts-sdk/src/cli/`
- `ais validate`：schema 校验
- `ais lint`：最佳实践
- `ais check`：组合校验（含 workspace/workflow 引用）

---

## 5. “从目录到执行”的推荐使用方式（SDK 组合姿势）

下面给出一个“团队 review 时可对齐的”推荐流水线。它的目标是明确每一步的输入/输出与失败点。

### 5.1 静态加载与校验（CI/开发期）

```ts
import {
  loadDirectory,
  validateWorkspaceReferences,
  validateWorkflow,
  loadDirectoryAsContext,
} from '@owliabot/ais-ts-sdk';

const dir = await loadDirectory('./examples', { recursive: true });
if (dir.errors.length > 0) throw new Error(JSON.stringify(dir.errors));

const issues = validateWorkspaceReferences({
  protocols: dir.protocols,
  packs: dir.packs,
  workflows: dir.workflows,
});
if (issues.some((i) => i.severity === 'error')) throw new Error(JSON.stringify(issues));

const { context } = await loadDirectoryAsContext('./examples');
for (const wf of dir.workflows) {
  const r = validateWorkflow(wf.document, context);
  if (!r.valid) throw new Error(JSON.stringify(r.issues));
}
```

这里的分工是：
- `loadDirectory`/parser：单文件 schema 正确
- `validateWorkspaceReferences`：跨文件引用正确 + pack allowlist 逻辑一致
- `validateWorkflow`：workflow 在给定 resolver context 下可解析（imports、action/query 存在、ref/cel 绑定等）

### 5.2 构建执行计划（Plan）并运行引擎（运行期）

```ts
import {
  buildWorkflowExecutionPlan,
  runPlan,
  createContext,
  solver,
  EvmJsonRpcExecutor,
} from '@owliabot/ais-ts-sdk';

// 1) 准备 resolver context：注册 protocols，写入 runtime.inputs/ctx/...
const ctx = createContext();
// registerProtocol(ctx, ...); setRef(ctx, 'inputs.xxx', ...); setRef(ctx, 'ctx.wallet_address', ...);

// 2) 生成 plan
const plan = buildWorkflowExecutionPlan(workflow, ctx);

// 3) 执行：solver 解决缺输入/自动填 contracts；executor 负责链上 IO
const executor = new EvmJsonRpcExecutor({ /* transport */ } as any);
for await (const ev of runPlan(plan, ctx, { solver, executors: [executor] })) {
  // 监听 need_user_confirm / node_blocked / error 等事件
}
```

注意：ExecutionPlan readiness 只负责“这个 node 现在能不能跑”，并不会自动做 pack policy gate。pack 的执行期 gate 一般在：
- solver（决定是否 need_user_confirm）
- executor（决定是否拒绝广播）
- 或者更上层 runner wrapper（执行 write 前调用 `validateConstraints`）

---

## 6. 当前 SDK 覆盖范围（支持什么，缺什么）

这一节用于同事快速判断“你看到的字段到底有没有 enforce”。

### 6.1 已覆盖且可用（相对闭环）

- strict schema + extensions 槽：`ts-sdk/src/schema/*`
- YAML 解析与错误聚合：`ts-sdk/src/parser.ts`
- 目录加载/按后缀识别文件类型：`ts-sdk/src/loader.ts`
- workflow 静态校验（refs/deps/DAG/import enforce）：`ts-sdk/src/validator/workflow.ts`
- workspace 跨文件校验（workflow->pack->protocol）：`ts-sdk/src/validator/workspace.ts`
- pack policy gate 的一部分（token allowlist、滑点、无限授权、风险审批阈值）：`ts-sdk/src/validator/constraint.ts`
- ValueRef/CEL/detect 体系（含 async detect）：`ts-sdk/src/resolver/*`、`ts-sdk/src/cel/*`、`ts-sdk/src/detect/*`
- ExecutionPlan IR + readiness：`ts-sdk/src/execution/plan.ts`
- 引擎参考实现（事件流、checkpoint、trace、executor/solver 插件接口）：`ts-sdk/src/engine/*`

### 6.2 schema 有字段，但 enforce/使用不完整（review 时要点名）

以下属于“看起来能配置，但不一定真的生效”的典型点（以当前实现为准）：

- pack `hard_constraints_defaults.max_spend` / `max_approval` / `max_approval_multiplier`
  - schema 存在：`ts-sdk/src/schema/pack.ts`
  - 约束校验未实现数值/单位解析与比较：`ts-sdk/src/validator/constraint.ts`（`spend_amount`/`approval_amount` 输入目前未消费）

- pack `providers.quote.enabled`
  - schema 存在：`ts-sdk/src/schema/pack.ts`
  - workspace 校验主要覆盖 detect providers 与 execution plugins，对 quote providers 未做 enforce：`ts-sdk/src/validator/workspace.ts`

- pack `providers.detect.enabled[].chains/priority`
  - schema 存在
  - workspace 校验目前主要按 `(kind, provider)` 做 allowlist，不做 chain/priority 的选择逻辑校验

- pack `plugins.execution.enabled[].chains`
  - schema 存在
  - workspace 校验目前按 `type` allowlist，不做 chain 维度 enforce

- pack `overrides.actions.*`
  - schema 存在：`ts-sdk/src/schema/pack.ts`
  - 当前未看到 validator/plan/engine 将 overrides 合并进执行期 gate（需要单独设计落地）

- Workflow policy（`workflow.policy.*`）
  - schema 存在：`ts-sdk/src/schema/workflow.ts`
  - 当前 engine/plan readiness 不直接消费该 policy（更多是留给上层 runner/executor）

### 6.3 规范与实现的“有意偏离/漂移点”（需要对齐决策）

这些不是单纯 bug，往往是“规范仍在 draft、实现先行”的产物，但必须在 review 时明确：

- Workflow schema 版本（spec 0.0.2 vs SDK 0.0.3），见第 3.3 节
- `TokenAmountSchema` 支持 `"max"` sentinel：`ts-sdk/src/schema/common.ts`
  - spec 的 numeric model 把 token_amount 定义为 DecimalString（sentinel 必须由协议逻辑处理）
  - 如果 `"max"` 是扩展语义，需要在 spec 或实现文档中明确约束与落地方式（例如：哪些 execution types 支持）

---

## 7. Review 清单（按层分工，避免“全都看”）

### 7.1 规范与示例一致性（Spec <-> examples <-> SDK）

- `schema` 版本号是否一致（尤其 workflow）
- examples 是否只使用了 schema/validator 真正支持的字段与语义
- README/模块 README 是否跟当前代码一致（避免误导外部使用者）

### 7.2 Schema 层（Zod）

- strict 是否覆盖所有核心对象，extensions 是否为唯一扩展槽
- CAIP-2/地址/数值字符串等基础约束是否够严格
- discriminated union 是否能无歧义地区分文档类型

### 7.3 Parser/Loader 层

- YAML `uniqueKeys` 是否足够防御“同 key 重复覆盖”
- 错误聚合是否保留足够的 path 信息，方便定位
- loader 的文件后缀识别与 schema discriminator 是否存在矛盾

### 7.4 Resolver/ValueRef/CEL/Detect 层（语义核心）

- ValueRef 的求值是否严格区分 `{lit/ref/cel/detect/object/array}`
- missing ref 的错误是否能精确定位（`ValueRefEvalError.refPath`）
- CEL 数值模型是否在关键路径拒绝 `number`（金额计算、编码输入）
- detect 的 provider 选择逻辑（显式 provider vs implicit provider）是否符合 pack allowlist 预期

### 7.5 Validator/Workspace/Policy Gate 层（安全边界）

- `validateWorkspaceReferences` 是否能阻断 workflow 绕过 pack allowlist
- `validateConstraints` 是否能覆盖 pack policy 字段，并且在引擎里有明确集成点
- “软门槛”与“硬违规”的区分是否明确（例如 allowlist 未命中是 violation 还是 require approval）

### 7.6 Plan/Engine 层（可运行性与可恢复性）

- ExecutionPlan 是否严格可序列化并可稳定恢复（checkpoint）
- readiness 的定义是否足够：缺 ref、需要 detect、condition=false 的 skip
- 并发/每链 read/write concurrency、polling/until/assert 的语义是否符合 workflow spec
- executor 的能力边界是否清晰（不应在 executor 隐式改写语义）

---

## 8. 建议的下一步（让 review 更快闭环）

如果你希望同事 review 完能给出可执行的改动建议，建议把讨论聚焦到以下输出：

1. **版本对齐决策**：workflow schema 到底是 `ais-flow/0.0.2` 还是 `0.0.3`，spec/examples/SDK/CLI 全部统一
2. **pack overrides 的落地路径**：是进入 plan（静态重写），还是进入 engine（执行前 gate），还是进入 executor（广播前 gate）
3. **max_spend/max_approval 的数值语义**：字符串格式、单位、与 token_amount/atomic 的关系，以及 conformance 向量
4. **quote providers 的 enforce 点**：属于 detect 层的 provider 选择，还是属于 solver/executor 的能力边界
5. **`"max"` token_amount 的语义**：规范化或明确为实现扩展（并写明禁止/允许的执行类型）

---

## 9. 参考与进一步阅读

- 规范索引：`specs/index.md`
- Pack 使用与约束：`docs/ais-pack.md`
- 现有评审记录（偏问题清单与建议）：`AIS_SPEC_TS_SDK_REVIEW.md`
- 引擎路线图与设计草稿：`docs/TODO.md`、`docs/design-ts-sdk-internal-runner-main.md`

