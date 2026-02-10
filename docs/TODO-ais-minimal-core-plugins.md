# AIS 0.0.2 Minimal Core（EVM+Solana）+ 插件化生态：可追踪重构 TODO

日期：2026-02-06  
适用范围：`specs/`、`schemas/`、`ts-sdk/`、`docs/`、`examples/`  
前提：**不考虑历史兼容**（可删改现有字段/类型/文件）；目标是“精准、表达力强、可扩展、可实现、可审计”，避免 spec 膨胀。

关联资料：
- 多链 workflow + 引擎语义最小增量方案：`docs/design-multichain-workflow-engine.md`
- Pi 对 AIS 的启示（小核心/事件流/资源加载/会话树）：`docs/analysis-pi-lessons-for-ais.md`
- 权威 TODO（替代所有旧 TODO）：`docs/TODO.md`

> 本文件是下一阶段 Roadmap：在保持 AIS 核心极简的前提下，把“多链 + 轮询 + 可恢复 + AI 介入点”做成**可嵌入组件**的规范化闭环，并把非 EVM/Solana 链/协议能力放到插件体系里。

**重要定位（避免跑偏）：**
- AIS **不是**一个 agent 产品/对话系统/工具调用平台；AIS 是 **agent 可接入的组件化协议与 SDK**（可被任意上层 Agent/Executor/UI 集成）。
- 本 TODO 的所有工作都必须落在“库能力/规范能力”上：schema/validate/plan/readiness/patch/executor/engine runner。
- **不做**：会话聊天管理、UI、skills/prompt 模板体系、通用工具调用框架（这些应由上层 agent 应用实现）。

---

## 0) 你已确认的总原则（本 TODO 的硬约束）

- AIS 核心内置支持最多到 **EVM + Solana + Bitcoin(PSBT)**（以及通用 `ValueRef/CEL` 与 DAG 语义）。  
- 其它链/协议扩展通过 **插件**挂载（自定义 schema/validate/compile/execute 等组件）。
- spec 不应该不断新增 execution types；优先用“少量通用语义”覆盖复杂场景（例如 `retry/until/timeout`）。

---

## 1) 需要你确认的决策（如果你不回复，我将按推荐默认继续）

### D13（推荐 A）插件 execution type 的命名与发现机制
- A（推荐）**插件注册 execution schema**：execution 的 `type` 仍是字符串；TS SDK 内置 union 只覆盖核心类型；未知 `type` 交给已注册插件 schema 解析/校验/编译/执行。
- B `execution: { type:"plugin", plugin:"x", spec:{...} }`（更显式，但更冗余）。
状态：已采用 A（已实现，T440）。

### D14（推荐 A）插件分发与加载方式（最小可用）
- A（推荐）先做 **SDK 级插件注册 API**（程序调用注册），把“包管理/安装”作为后续工具链（类似 Pi packages）。
- B 直接做 CLI 安装/启用插件（npm/git），一次到位但工程面更大。
状态：已采用 A（已实现，T440）。

### D15（推荐 A）跨链“等待/轮询”的语义放在哪里
- A（推荐）放在 **workflow node 通用字段**：`retry/until/timeout_ms`（引擎实现，spec 极简）。
- B 新增 execution type：`wait_until` / `poll_query`（表达更集中，但 execution types 变多）。
状态：已采用 A（已实现，T401/T411）。

---

## 2) P0 里程碑：让“多链 Bridge 闭环”先可表达、可执行、可恢复

### Spec / Schema（P0）

- [x] T400 Workflow node 支持多链：新增 `nodes[].chain?` + `workflow.default_chain?` `P0` (Deps: REFACTOR_TODO#T011,T012)
  - AC: 一个 workflow 内可同时存在 `eip155:*` 与 `solana:*` 节点；planner/validator 明确继承规则（node.chain → workflow.default_chain → runner default）。
  - AC: `deps` 与隐式 `nodes.*` 引用依赖仍然成立（跨链也成立）。

- [x] T401 Workflow node 支持轮询/直到满足：新增 `retry? / until? / timeout_ms?` `P0` (Deps: T400, REFACTOR_TODO#T011)
  - AC: `until` 是 ValueRef（CEL/refs 均可）；仅在执行后判定是否“完成”；不污染 readiness。
  - AC: `retry` 定义最小集合：`interval_ms/max_attempts/backoff?`（默认 fixed）。

- [x] T402 收缩 AIS 核心执行类型：Protocol `execution` 核心只保证 EVM/Solana/Composite/Bitcoin `P0` (Deps: D13)
  - AC: spec/SDK 核心移除/降级 `cosmos_message/move_entry` 等非 EVM/Solana/BTC 的“核心定义”，改为插件扩展示例/附录或完全移出核心 spec。
  - AC: TS SDK 核心 schema 不再内置这些 placeholder union（避免锁死生态）；通过插件注册提供。

### TS SDK（P0）

- [x] T410 Planner 按 node.chain 选择 execution：`buildWorkflowExecutionPlan()` 多链化 `P0` (Deps: T400)
  - AC: 每个 `ExecutionPlanNode.chain` 来自 node.chain 继承规则；`selectExecutionSpec()` 以该 chain 选 spec。

- [x] T411 Engine 支持 `until/retry/timeout`（query 节点轮询）`P0` (Deps: T401, REFACTOR_TODO#T011)
  - AC: 对带 `until` 的 query 节点：executor 每次执行后写回 outputs，再评估 until；false → 等待 interval_ms 继续；true → completed。
  - AC: 超时/超次数输出结构化 `error` 事件（含 attempts/elapsed/last outputs 摘要）。
  - AC: 轮询期间不阻塞其它可运行分支（DAG 并行推进）。

- [x] T412 Engine “blocked/need_user_confirm” 不应默认终止全局：改为“挂起节点 + 继续推进其它分支” `P0` (Deps: T411)
  - AC: 当某节点 `need_user_confirm` 时，引擎继续执行所有不依赖该节点的可运行节点；当全图无可推进节点时再暂停并汇总待确认项。

- [x] T413 ExecutionTraceSink（执行审计/恢复树）最小实现：JSONL + (id,parentId) 记录事件与分叉 `P0` (Deps: T412, D14)
  - AC: SDK 提供可选注入接口 `traceSink.append(event)`（或同等能力），默认不开启（不影响嵌入方）。
  - AC: Trace 的目的仅是**审计、调试、恢复与分叉对比**（不是聊天会话）；并保证不进入 LLM context（由上层 agent 决定何时摘要给 AI）。

- [x] T414 Checkpoint 序列化标准化：BigInt/Uint8Array 的 replacer/reviver 进入 SDK 工具 `P0` (Deps: T413)
  - AC: SDK 提供 `serializeCheckpoint()/deserializeCheckpoint()`（或 store wrapper）；demo 不再手写 replacer。

---

## 3) P1 里程碑：EVM/Solana 执行闭环补齐（让桥接 + 目的链操作可跑）

### Executors（P1）

- [x] T420a Solana 指令编译插件点：`(programId, instruction)` registry `P1` (Deps: D13)
  - AC: `solana_instruction` 编译时优先命中 registry；未命中则走 SPL/ATA 内置 + generic bytes fallback。
  - AC: 可用 `createDefaultSolanaInstructionCompilerRegistry()` 扩展内置集合，不引入全局可变状态。

- [x] T420 SolanaRpcExecutor：执行 `solana_instruction`（send/confirm） `P1` (Deps: T410)
  - AC: 支持注入 transport（mockable）与 signer；产出 tx signature/slot/confirmation；写回 `nodes.<id>.outputs`。

- [x] T421 Solana Read 支持（建议新增核心 `solana_read` 或插件实现） `P1` (Deps: D13, T420)
  - AC: 能查询账户余额/代币账户/程序状态等基础读；可与 `until/retry` 配合实现“到账等待”。

- [x] T422 Bridge 参考实现（最小可用）：以“协议 action/query 组合”跑通 `send → wait_arrival → deposit` `P1` (Deps: T411, T420, T421)
  - AC: 不要求统一所有桥；先选定 1 个桥协议做 reference spec + executor 或纯 spec（如果桥本身在链上）。

### Spec 文档与 Examples（P1）

- [x] T430 发布 1 个端到端 example：Aave 借出 → 分叉转账 + 跨链 → Solana 存入 `P1` (Deps: T420-T422)
  - AC: example 明确标注每个 node 的 chain、deps、前置检查、轮询节点与 until。

- [x] T431 Examples/Specs 增加“最小运行时上下文”说明模板（inputs/ctx/contracts） `P1` (Carry: REFACTOR_TODO#T021)

---

## 4) 插件体系（非核心链的扩展点）：精准但强扩展（P1→P2）

> 目标：AIS 核心只保证 EVM+Solana；其它链通过插件挂载 schema/validator/compiler/executor。

- [x] T440 TS SDK 插件注册 API（最小可用） `P1` (Deps: D13, D14)
  - AC: `registerExecutionType({ type, schema, readinessRefsCollector?, compiler?, executor? })`（接口形状可调整）。
  - AC: validator/loader/plan builder 能识别插件 schema；未知 type 若无插件 → 明确错误。

- [x] T443 （可选，P2）SDK 适配器：JSONL 事件输出 / RPC 模式 `P2` (Deps: T413)
  - AC: 这不是 AIS 核心能力；只是为了上层系统更易集成 `runPlan` 事件流。
  - AC: 不引入会话/对话系统；只做 engine 事件的进程间/文件流适配。

- [x] T441 Validator 插件点：自定义 lint/validate 规则 `P1` (Carry: REFACTOR_TODO#T180)
  - AC: 核心 validate 只做规范错误；lint 做建议；插件可注入链/协议特定规则。

- [x] T442 Loader 插件点：目录加载与错误定位增强 `P1` (Carry: REFACTOR_TODO#T102)
  - AC: loadDirectory 输出可定位到文件/字段；并能按插件 schema 分类。

---

## 5) 仍然重要的未完成事项（以 `docs/TODO.md` 为准）

以下条目在“多链 + 插件化”新方向下仍然关键，建议保留并纳入上述里程碑（详情见 `docs/TODO.md`）：

- [ ] T011/T012/T013/T014/T015/T016/T017/T019（Spec 语义与扩展点定稿、schemas 发布）`P0/P1`
- [x] T163/T164（composite → plan 执行计划生成、Solana 接入 plan）`P0`
- [x] T165（detect provider 插件接口，支持 async detect + runner/executor 贯通）`P1`
- [x] T181/T182（跨文件引用校验 + CLI 输出）`P1/P2`
- [x] T200/T201/T204（conformance vectors + golden tests）`P1`

> 注：本文件是阶段性 Roadmap；权威 TODO 以 `docs/TODO.md` 为准。

---

## 6) 下一步（建议执行顺序）

1) 做 T443（可选适配器）：把 engine 事件流更容易接到外部系统（RPC/JSONL/stream），但不引入“会话系统”。  
