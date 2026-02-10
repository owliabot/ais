# AIS 0.0.2 — Authoritative TODO (No Legacy Dependencies)

日期：2026-02-09  
适用范围：`specs/`、`schemas/`、`examples/`、`ts-sdk/`、`docs/`

本文件是 **唯一权威 TODO**（可破坏性改动；不考虑历史兼容）。旧 TODO 文件将被删除，因此本文件 **不引用任何旧 TODO**。

## 0) 追踪方式（必须遵守）

- 每个条目都有唯一 ID（`T###` / `D#`）。
- 进度用 Markdown checkbox：`[ ]` 未完成、`[x]` 已完成。
- 依赖用 `Deps:` 标注前置条目 ID。
- 验收标准用 `AC:`（Acceptance Criteria）定义“完成的定义”（可自动/手动验证）。
- 优先级标签：
  - `P0`：阻塞（不做会导致 spec/SDK 长期不一致或无法互操作）
  - `P1`：重要增强（显著提升覆盖/可用性/安全性）
  - `P2`：可选优化（不增加核心复杂度前提下再做）

## 0) 不变目标（硬约束）

- AIS 是 **agent 可接入的组件**（协议 + SDK + 校验/执行闭环），不是 agent 产品本体。
- Core chain 支持：**EVM + Solana + Bitcoin(PSBT)**；其它链/协议能力通过插件扩展。
- 设计信条：**简洁、优雅、低耦合、可扩展、可验证、可审计**；避免 spec 膨胀。

## 1) 当前基线（已具备；用于对齐范围，不作为 TODO）

（非 TODO，仅做现状摘要）

- `ValueRef`（`lit/ref/cel/detect/object/array`）+ CEL 数值安全（BigInt/decimal）已实现
- ExecutionPlan IR（DAG + readiness）+ Engine runner（并发、checkpoint、trace、until/retry/timeout）已实现
- EVM JSON ABI 编码/解码（tuple-safe）+ EVM/Solana RPC executors 已实现
- 插件执行类型注册（schema/readiness/compiler/executor）+ workspace validator + CLI（validate/lint/check）已实现
- Detect provider registry（支持 async）+ JSONL 事件/JSONL peer 适配器已实现

## 2) 决策（已按推荐默认采用；后续实现以此为准）

### D1（已采用 A）`evm_multiread` / `evm_multicall` 的定位
- [x] D1 = A
- A（推荐）**从 core 移出**，改为插件 execution types（保持 core 只含 `evm_read/evm_call`），避免 spec/SDK 绑死 router 生态细节。
- B 保持 core，并实现 compiler + executor（需要定义 multicall3 地址、失败语义、返回写入规则）。

### D2（已采用 A）`bitcoin_psbt` 的最小闭环范围
- [x] D2 = A
- A（推荐）core 只保证 **PSBT 组装（pure compiler）** + engine 事件（`need_user_confirm`）交给外部 signer/broadcaster；broadcast 走插件 executor。
- B core 还要提供 broadcast executor（需要定义 transport / fee / UTXO sourcing 等，复杂度很高）。

## 3) P0：必须完成（否则 spec 与 SDK 会长期不一致）

- [x] T001 Schema 严格性 + 扩展点策略定稿（Spec + TS SDK）`P0`
- Deps: 无
  - Scope: `specs/ais-1-*.md`、`ts-sdk/src/schema/*`
  - Problem: 多数 Zod `z.object(...)` 默认会 **strip** 未知字段，导致错误静默吞掉；与“严格字段 + 扩展点”目标冲突。
  - AC:
    - Protocol/Pack/Workflow/Plan 等所有核心对象默认 **strict**（未知字段报错）
    - 允许扩展的唯一入口为 `extensions`（或 `x_*`，二选一，推荐 `extensions`）
    - 插件 execution types 保持 `.passthrough()`（仅对 `execution.type` 非 core 的对象）
    - 增加至少 10 个负例测试：未知字段应报错且定位到路径

- [x] T002 发布权威 JSON Schemas（`schemas/0.0.2/*.json`）`P0`
- Deps: T001
- Scope: `schemas/`、`ts-sdk/` build scripts
- AC:
  - 输出 Protocol/Pack/Workflow/Plan/Conformance 的 JSON Schema 文件到 `schemas/0.0.2/`
  - 提供脚本：`npm run build:schema`（或同等）可一键生成/更新
  - `examples/` 可被 JSON Schema 校验通过（至少包含结构与必填字段）

- [x] T003 Capabilities 正文补齐（避免“有字段但无语义”）`P0`
- Deps: 无
- Scope: `specs/ais-1-capabilities.md`
- AC:
  - 明确定义 capabilities 的命名、作用域（engine/runtime/chain/wallet/provider）
  - 规定 pack 如何声明/限制 detect providers、quote providers、execution plugins
  - 规定 engine 在 capabilities 不满足时的标准行为（error vs need_user_confirm）

- [x] T004 Chain pattern matching 规范化 + conformance 向量 `P0`
- Deps: 无
- Scope: `specs/ais-2-evm.md`、`specs/ais-2-solana.md`（或抽到共享章节）、`specs/conformance/vectors/*`
- AC:
  - 写清算法：exact → `<namespace>:*` → `*`，无匹配时报错
  - 冲突与歧义处理（例如多个同级匹配）必须有明确规则
  - 增加 10+ conformance cases（含负例）

- [x] T005 Query `returns` ↔ ABI `outputs` 的映射与写入规范 `P0`
- Deps: 无
- Scope: `specs/ais-1-protocol.md`、`specs/ais-2-evm.md`、`ts-sdk/src/execution/*`、`ts-sdk/src/validator/*`
- Problem: SDK decode 目前主要以 ABI outputs 命名为准，但 spec 里还有 `query.returns`；两者未严格绑定会导致互操作分歧。
- AC:
  - 规定 `returns` 与 `abi.outputs` 的一致性要求（命名/顺序/tuple 展开规则）
  - 规定写入路径：默认写入 `nodes.<workflowNodeId>.outputs`；禁止“按 queryId 聚合覆盖”的歧义
  - validator 提前报错：returns 与 abi.outputs 不一致时必须定位到字段路径

- [x] T006 Numeric model 补齐（`to_atomic/to_human/mul_div`）+ conformance `P0`
- Deps: 无
- Scope: `specs/ais-1-types.md`、`specs/ais-4-conformance.md`、`specs/conformance/vectors/*`
- AC:
  - 完整定义：decimal 字符串格式、截断/舍入规则、错误条件（小数位过多等）
  - 明确 `mul_div`/`min_out` 推荐公式与边界（溢出、负数、零分母）
  - 追加 20+ conformance cases（含 6/8/9/18 decimals、边界与负例）

- [ ] T007 Detect provider 协商与交互规范（从“实现”提升为“可互操作”）`P0`
- Deps: T003
- Scope: `specs/ais-1-expressions.md`、`specs/ais-1-pack.md`、`specs/ais-1-capabilities.md`
- AC:
  - 明确 detect 输入字段（candidates/constraints/requires_capabilities）与输出契约（允许返回 ValueRef-like 并递归求值）
  - pack 如何启用/禁用 detect providers 的规则（kind/provider/priority/chain scope）
  - engine 在 detect 缺失/失败时的标准事件（`need_user_confirm` 的 `reason/details` 结构化约定）

- [ ] T008 Registry `specHash` canonicalization + hash 算法定稿（并与 SDK 对齐）`P0`
- Deps: 无
- Scope: `specs/ais-3-registry.md`、`specs/ais-4-conformance.md`、`specs/conformance/vectors/*`
- AC:
  - 明确：canonicalization = RFC 8785 JCS（精确定义输入对象范围）
  - 明确：默认 hash = keccak256（并说明可扩展 hash 的策略）
  - 5+ 对照向量：canonical string + hash 输出

## 4) P1：高价值（补齐“core 能力承诺”与工程闭环）

- [ ] T101 `Action.requires_queries` 的规划/去重/缓存语义 + SDK 支持 `P1`
- Deps: T005（query outputs 规范化后再把注入语义写严谨）
  - Scope: `specs/ais-1-protocol.md`、`specs/ais-1-workflow.md`、`ts-sdk/src/execution/plan.ts`
  - AC:
    - spec 说明 planner 是否可自动注入 requires_queries（推荐：planner 注入、可关闭）
    - 规定去重 key（skill+query+args hash）与 cache 策略（至少“同一 plan 内去重”）
    - SDK 实现一个可选 planner 开关：`buildWorkflowExecutionPlan(..., { inject_requires_queries: true })`

- [ ] T102 Bitcoin PSBT（core）最小实现（取决于 D2）`P1`
- Deps: D2
- Scope: `specs/ais-2-bitcoin.md`（新增）、`ts-sdk/src/execution/bitcoin/*`（新增）、`ts-sdk/src/engine/executors/*`（可选）
- AC（按 D2=A）:
  - compiler：`bitcoin_psbt` → `psbt_base64`（纯函数、无 IO）
  - engine/executor：产出 `need_user_confirm`，details 包含 psbt 与推荐签名策略字段
  - 10+ 单测覆盖（UTXO/outputs/fee/错误路径）

- [ ] T103 Core EVM 执行类型对齐（取决于 D1）`P1`
- Deps: D1
- Scope: `specs/ais-2-evm.md`、`ts-sdk/src/schema/execution.ts`、`ts-sdk/src/engine/executors/evm-jsonrpc.ts`
- AC:
  - 若 D1=A：从 core 移出 `evm_multiread/evm_multicall`（spec/schema/README/examples 全对齐）
  - 若 D1=B：实现 compiler + executor，明确 outputs 写入规则与错误语义，并加 conformance/golden tests

- [ ] T104 Conformance suite 拆分与扩容（可被其它语言 SDK 复用）`P1`
- Deps: T004/T006/T008
- Scope: `specs/conformance/vectors/*`、`specs/ais-4-conformance.md`、`ts-sdk/tests/*`
- AC:
  - vectors 拆分：`numeric.json`、`abi.json`、`pattern.json`、`registry.json`、`detect.json`、`plan.json`（或同等主题拆分）
  - 每类至少 10 个用例（含负例）；TS SDK 测试按主题跑
  - `ais-4-conformance.md` 写清 “实现必须通过哪些向量才算 conformant”

- [ ] T105 Examples：逐文件补齐“最小运行时上下文”说明 + golden snapshots `P1`
- Deps: T002（JSON Schemas）或至少 T001（strict schema），否则说明难以稳定
- Scope: `examples/*`、`ts-sdk/tests/examples-directory.test.ts`
- AC:
  - 每个 `.ais.yaml/.ais-pack.yaml/.ais-flow.yaml` 有对应说明（文件头注释或同名 `.md`）
  - 增加 golden snapshots：解析→验证→plan 输出（稳定可追踪）

## 5) P2：可选（在不增加核心复杂度的前提下提升易用性）

- [ ] T201 Solana 派生账户表达（ATA/PDA）策略收敛 `P2`
- Deps: 无
- Scope: `specs/ais-2-solana.md`、`ts-sdk/src/execution/solana/*`
- Goal: 不让 AIS core 变复杂，但要让“写 Solana spec 不靠硬编码/手算地址”。
- AC（推荐路径）:
  - spec：不引入复杂 `accounts.source/derived` 结构；改为推荐使用 `calculated.*` + SDK helper
  - SDK：提供 `solana.deriveAta(owner, mint, tokenProgram?)` / `solana.derivePda(seeds, programId)` helper（纯函数）
  - examples 更新为使用 helper 产出的 `calculated.*`

## 6) 下一步（按推荐顺序执行）

1) `T001/T002` 已完成：schema strict + `extensions` 扩展点，以及权威 JSON Schemas 已发布。  
2) 优先补齐 `T007/T008` 的规范条款 + conformance 向量（锁死互操作语义）。  

## 7) Handoff（联调测试清单）

日期：2026-02-09  
目标：你专注联调与集成测试时，可以用下面清单快速定位“哪里该看/怎么跑/改动点”。

### 7.1 当前版本与基线

- Spec/SDK 版本：`0.0.2`（不考虑历史兼容）。
- 数值模型（T006）已定稿并有 conformance：`specs/ais-1-types.md` + `specs/conformance/vectors/numeric.json`。

### 7.2 一键自检（推荐）

- 单测（含 conformance vectors）：`cd ts-sdk && npm test`
- 生成/刷新 JSON Schemas：`cd ts-sdk && npm run build:schema`

### 7.3 CLI 联调（workspace 级）

先构建 CLI：

- `cd ts-sdk && npm run build`

然后在 repo root 做检查：

- `node ts-sdk/dist/cli/index.js validate examples --recursive`
- `node ts-sdk/dist/cli/index.js lint examples --recursive`
- `node ts-sdk/dist/cli/index.js check examples --recursive`

（CI/脚本用 JSON 输出：在命令末尾加 `--json`。）

### 7.4 联调重点与常见坑

- YAML 缩进：`parser` 严格拒绝 tab 缩进（只允许空格）。
- Numeric 严格化（T006）：
  - DecimalString 只接受 `^\d+(\.\d+)?$`（拒绝 `.5`、`1.`、指数、空白、`+1`）。
  - `to_human()` / `mul_div()` 对负数输入会报错；`mul_div()` 要求 `denom > 0`。
- Conformance vectors：
  - 目录：`specs/conformance/vectors/`
  - 新增：`numeric.json`（覆盖 `to_atomic/to_human/mul_div`，含负例）
  - runner：`ts-sdk/tests/conformance-vectors.test.ts`

### 7.5 下一步（联调后最该推进）

- `T007`（Detect provider 协商/交互规范）：把“实现可用”提升到“跨实现可互操作”。
- `T008`（Registry specHash 定稿）：锁死 canonicalization + hash 算法与向量覆盖。
