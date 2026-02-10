# AIS TS SDK：Solver / Executor / Engine 设计方案（0.0.2，无历史兼容）

日期：2026-02-05  
适用范围：`ts-sdk/`（以及与之配套的 `specs/` / `examples/`）  

本文是对你提出的需求的落地设计：**在 SDK 内提供“可执行闭环”的参考实现**——包含示例 `Solver`、链上查询/发交易的 `Executor`、以及编排它们的 `Engine`。同时保证解耦：SDK 既可以“自带默认实现”，也可以被外部 solver/executor 替换或接管。

---

## 1. 需求解读（你的话→可实现的目标）

你描述的链路（我理解为“执行闭环”）：

1) Agent 获取用户语义  
2) Agent 查询 `ais-spec`，借助 SDK 组装出 `action` 或 `workflow`（YAML/JSON）  
3) 这时还缺少部分链上读取信息（allowance/quote/route/decimals/余额…）  
4) Agent 调用一个具体的 **solver** 来补全 query / detect / 缺失输入（可能还涉及用户确认）  
5) 对多步 workflow：有些 query 要等前一步执行完（例如 approve 后再 quote/再 swap），因此需要 **异步编排**  
6) workflow 的 step 有依赖关系：有些必须串行，有些可并行（同时发起 reads 或多个独立链的步骤）

目标抽象为 SDK 内的 3 个职责：

- **Planner（已具备雏形）**：把 `workflow` 编译成稳定 DAG 的 `ExecutionPlan`（可 checkpoint）  
- **Solver（要补齐）**：把 `getNodeReadiness()` 给出的 “missing refs / needs_detect” 变成一组对 `runtime.*` 的补丁（可能需要用户确认）  
- **Executor（要补齐）**：执行 plan 节点：`evm_read` 做 `eth_call` 并 decode；`evm_call` 发交易；把结果按 `writes` 写回 runtime  
- **Engine（要补齐）**：串起 Planner/Solver/Executor，提供同步/异步事件协议，支持并行调度与可恢复

---

## 2. 当前设计基础（已存在的“关键拼图”）

SDK 当前已经具备以下构建块（0.0.2）：

- `ValueRef` + 结构化运行时：`runtime.{inputs,params,ctx,contracts,calculated,policy,query,nodes}`  
- `ExecutionPlan` IR（`ais-plan/0.0.2`）：可 JSON 序列化、可 readiness 检查  
- Workflow DAG：`deps` + 从 `ValueRef` 推断的 `nodes.*` 引用依赖，稳定拓扑序  
- EVM JSON ABI 编码：tuple-safe + args 对齐校验  
- EVM `eth_call` 返回 decode（JSON ABI outputs）

以及关键的约定（D10）：

- **Workflow 的 query 结果**以 node id 为主：写入 `runtime.nodes.<nodeId>.outputs`  
- 不使用全局 `runtime.query.<queryName>` 作为 workflow 级别绑定（避免同名 query 冲突）

这意味着：SDK 已经可以“生成 plan + 判断缺什么 + 编译 calldata + decode 输出”，下一步就是把**网络执行**与**缺失补全**接起来。

---

## 3. 设计原则（保证解耦与可对接外部实现）

### 3.1 必须解耦的边界

- **核心 IR/编译/校验必须纯粹**：不依赖网络，不依赖钱包，不依赖 UI  
- **网络与签名是可插拔的**：SDK 提供参考实现，但外部可完全替换  
- **Solver 是可插拔的**：既能用“SDK 内示例 solver”，也能用外部 solver（例如更强的路由器/价格引擎/MEV 风控）

### 3.2 Engine 只做“编排”，不做“业务智能”

- Engine 负责：DAG 调度、并行控制、事件输出、checkpoint、重试策略框架  
- Solver 负责：补齐缺失 refs、解决 detect、提出用户确认点、选择候选  
- Executor 负责：链上读取 / 发交易 / 等确认 / 提取输出

---

## 4. 新增的接口设计（建议作为 SDK 的“可执行闭环 API”）

下面是建议落地的 TypeScript 概念模型（文档层 API 形状，具体可在 TODO 实现）。

### 4.1 RuntimePatch：solver/executor 写回 runtime 的统一方式

建议用“最小自定义 patch”，而非 RFC6902 JSON Patch（避免引入复杂度）：

- `set(path, value)`：覆盖写入  
- `merge(path, object)`：对象浅合并（常用于 outputs 增量写入）

path 统一使用现有 `setRef()` 的 dot-path：`inputs.* / ctx.* / contracts.* / nodes.<id>.outputs.* ...`

### 4.2 Solver 接口（同步/异步 + 允许用户交互）

Solver 的输入来自 `getNodeReadiness()`：

- `missing_refs[]`：缺少的引用（例如 `contracts.router`、`inputs.amount`）  
- `needs_detect`：出现 `{detect: ...}` 且无法自动选择  
- `errors[]`：表达式/类型错误（solver 可以尝试修复或直接上抛）

建议：

- `solve(node, readiness, ctx) -> SolverResult`
- `SolverResult` 允许三类输出：
  1) `patches[]`（可直接应用）  
  2) `need_user_confirm`（给 engine 输出事件，让外部 UI/agent 提示用户）  
  3) `cannot_solve`（带解释）

### 4.3 Executor 接口（链适配器）

Executor 的目标：给定一个 `ExecutionPlanNode`（已就绪）与当前 `ResolverContext.runtime`，完成一次“链交互”，并把可追踪结果写回 runtime。

为了保证解耦，建议把 Executor 拆成两层：

1) **Compiler（纯）**：把 `ExecutionSpec` 编译为链侧请求（SDK 已具备 EVM compiler）  
2) **Transport/Signer（可插拔）**：负责 RPC、签名、发交易、等待确认

建议的执行器接口形状：

- `Executor.execute(node, ctx, options) -> ExecutorResult`
  - `ExecutorResult` 至少包含：
    - `writes[]`：对 runtime 的 patch（或直接输出 `outputs` 由 engine 根据 plan.writes 写回）
    - `telemetry`：请求/响应元信息（用于调试与 checkpoint）
    - `need_user_confirm?`：例如发交易前展示风险、gas、模拟结果

#### 4.3.1 EVM Executor：读与写的最小能力

- Read（`evm_read`）：
  - `compileEvmRead()` → `{to,data,chainId}`  
  - `eth_call` → `returnData`  
  - `decodeJsonAbiFunctionResult(abi, returnData)` → `{ outputs... }`  
  - 写回：`nodes.<nodeId>.outputs = decoded`

- Call（`evm_call`）：
  - `compileEvmCall()` → `{to,data,value,chainId}`  
  - 交易生命周期建议拆分事件：
    - `tx_prepared`（已得到 tx request）
    - `need_user_confirm`（可选：让外部展示并确认）
    - `tx_sent`（得到 txHash）
    - `tx_confirmed`（可选：得到 receipt）
  - 写回（建议最小可用）：
    - `nodes.<nodeId>.outputs.tx_hash`
    - `nodes.<nodeId>.outputs.receipt?`

> 注意：签名本质不应该强绑定某个钱包 SDK。建议在 Executor 中注入 `Signer`：
> - `Signer.signEvmTransaction(txRequest) -> signedTxHex`
> - 或者允许外部直接发送交易：Executor 只返回 `txRequest`，由外部广播（更解耦，但不满足“SDK 自己发交易”的闭环）。

#### 4.3.2 Solana Executor（后续）

Solana 的最小能力类似：

- Instruction（`solana_instruction`）编译：program/accounts/data/compute_units/lookup_tables  
- 交易组装与签名：同样用可插拔 signer/provider  

（Solana 的统一接入建议仍以 `T163/T164` 为主，此处只定义接口一致性。）

---

## 5. Engine：把 Planner/Solver/Executor 串起来

Engine 是“执行闭环”的核心：它不关心链细节，只关心 DAG 调度、readiness、补全与 checkpoint。

### 5.1 推荐的最小 API

建议 Engine 面向 `ExecutionPlan` 工作（而不是直接面向 workflow），因为 plan：

- 稳定拓扑序（可并行调度）
- readiness 可计算
- 可 JSON 序列化（checkpoint/恢复）

最小入口：

- `buildWorkflowExecutionPlan(workflow, ctx, { default_chain? }) -> plan`（chain 来自 `nodes[].chain` / `workflow.default_chain`，也可由 planner 注入）
- `runPlan(plan, ctx, { solver, executors, scheduler, checkpoint }) -> AsyncIterable<EngineEvent>`

### 5.2 事件协议（对接 agent/外部系统）

Engine 以事件流驱动，以支持：

- 同步（直接 await）
- 异步（边跑边产出进度；中间需要用户确认时暂停）
- 可恢复（事件 + checkpoint 可重放/续跑）

建议事件（与 TODO T301 一致，但补充 payload 结构）：

- `plan_ready`：plan 生成完成（输出 plan hash / nodes 列表）
- `node_ready`：某个节点满足 deps（尚未 readiness 检查）
- `node_blocked`：readiness 返回缺失 refs / needs_detect（附 `missing_refs[]`）
- `solver_applied`：solver patches 已应用（附 patches）
- `query_result`：read 节点完成（附 decoded outputs）
- `tx_prepared`：写交易准备完成（附 txRequest）
- `need_user_confirm`：需要用户确认（附原因、展示字段、建议文案）
- `tx_sent`：已广播（附 txHash）
- `tx_confirmed`：已确认（附 receipt / slot）
- `skipped`：条件为 false
- `error`：错误（可附 retryable 标记）
- `checkpoint_saved`：可选（每个关键阶段保存）

### 5.3 并行/串行调度策略

Engine 的调度只依赖 DAG 与节点类型：

- **DAG 约束**：一个节点仅在所有 `deps` 完成后进入候选队列  
- **readiness 约束**：候选节点需 `getNodeReadiness()==ready` 才可执行，否则交给 solver  
- **并行策略**（建议默认）：
  - `evm_read`：可高并发（受 RPC 限速与 provider 配置影响）
  - `evm_call`：同一 `eip155:<chainId>` 默认串行（nonce/用户体验），跨链可并行
  - 未来 `evm_multiread`：对同 chain 的 reads 可合并（可选优化）

可配置项建议：

- `max_concurrency`（全局并发）
- `per_chain.max_read_concurrency`
- `per_chain.max_write_concurrency`（EVM 默认 1）

---

## 6. SDK 内“参考实现”的建议形态

你希望 SDK 内就能跑通闭环，同时还能对接外部实现。建议分 3 层交付：

### 6.1 纯接口层（稳定、可对接外部）

在 `ts-sdk/src/engine/`（或 `src/runtime/`）定义：

- `RuntimePatch` / `applyRuntimePatch()`  
- `Solver` / `SolverResult`  
- `Executor` / `ExecutorResult`  
- `EngineEvent` / `EngineOptions`  
- `CheckpointStore`（in-memory/file/db 皆可）

### 6.2 默认实现（示例级别，不追求覆盖所有协议）

- `solver`（最小可用内置 solver，可被外部替换/接管）：
  - 只处理：
    - `detect.kind=choose_one`（已内建候选）
    - `missing_refs` 中常见的 `contracts.*`（如果 protocol deployment 已加载则可填）
    - `inputs.*`：若 workflow inputs 有 default 则填，否则 `need_user_confirm`
- `EvmJsonRpcExecutor`：
  - 支持 `eth_call`（read + decode）
  - 支持 `eth_sendRawTransaction` + `eth_getTransactionReceipt`（write）
  - 依赖注入 `Signer`（默认空实现，示例用私钥 signer 仅用于 dev）

### 6.3 外部对接（生产级）

外部系统（agent 执行器、钱包、报价/路由服务）可以：

- 替换 `Solver`：更强 detect/route/quote 能力 + 策略风控  
- 替换 `Executor`：使用 viem/ethers、使用托管签名服务、带模拟与风控  
- 只订阅 Engine 事件：把 `need_user_confirm` 接入 UI/聊天交互

---

## 7. 结果写回约定（与当前实现保持一致）

### 7.1 Workflow query_ref（已定）

- decode 的结果写回：`runtime.nodes.<nodeId>.outputs`
- 引用读取：`{ ref: "nodes.<nodeId>.outputs.<field>" }`

### 7.2 Workflow action_ref（建议）

最小写回：

- `runtime.nodes.<nodeId>.outputs.tx_hash`
- `runtime.nodes.<nodeId>.outputs.receipt?`

后续可扩展：

- `calculated` / `policy` 的动态收敛
- `writes` 显式字段（D10）用于把 action 输出映射到指定路径

---

## 8. 下一步（从“能跑”到“好用”）

1) 把接口层与参考实现落进 `ts-sdk/src/engine/`（新增模块 README）  
2) 把 `ExecutorPlan.writes` 贯彻到 engine：统一用 patch 写回 runtime  
3) 完成 `T300/T301/T302`（solver/executor/scheduler）并给一个端到端 demo（本地 JSON-RPC + mock signer）

---

## 附录 A：端到端 Demo（T308）

仓库提供一个可直接运行的 demo 脚本，演示闭环：

1) 加载 protocol + workflow（YAML）  
2) `buildWorkflowExecutionPlan()` 生成 plan  
3) `runPlan()` 执行：readiness → solver 补全 `contracts.*` → executor 执行 `eth_call` / 发交易  
4) 输出 checkpoint（可恢复）

### 运行方式

```bash
cd ts-sdk
npm run build
node examples/engine-runner-demo.mjs
```

说明：
- demo 使用 mock JSON-RPC transport + mock signer（仅演示事件流与 checkpoint 形态）。
- checkpoint 默认写到 `/tmp/ais-engine-checkpoint.json`。
