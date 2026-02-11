# AIS TS SDK: Internal Runner `main` (Workflow/Action/Query) Design (0.0.2)

目标：实现一个**内部验证用**的可执行 `main`，用于端到端跑通 `ts-sdk` 的闭环能力：
加载 workspace -> 解析 workflow/action/query -> DAG 并发调度 -> 前置检查 -> 构造交易 -> 上链执行并等待成功 -> 后置检查/轮询 -> 继续下一步，直到完成。

本 `main` 的定位是 “SDK 自测/验收工具”，不是对外产品能力；对外场景中，网络访问与签名通常由调用方/agent-host 承担。

---

## 1. 现有 SDK 能力对齐点 (不重复造轮子)

当前 `ts-sdk` 已经提供了 runner 所需的核心拼图：

- 加载与解析
  - `loadDirectory()` / `loadDirectoryAsContext()` (`ts-sdk/src/loader.ts`)
  - `parseWorkflow()` / `parseProtocolSpec()` (`ts-sdk/src/parser.ts`)
- 运行时上下文 (refs/CEL 的根对象)
  - `createContext()` + `ctx.runtime.{inputs,params,ctx,contracts,nodes,...}` (`ts-sdk/src/resolver/context.ts`)
- 规划与依赖
  - `buildWorkflowExecutionPlan()` 生成稳定拓扑的 `ExecutionPlan` (`ts-sdk/src/execution/plan.ts`)
  - DAG 包含显式 `deps` + 从 `ValueRef` 推断的隐式依赖 (`ts-sdk/src/workflow/dag.ts`)
- Readiness (前置检查的一部分)
  - `getNodeReadiness()` / `getNodeReadinessAsync()`：缺失 refs、detect 需求、condition 过滤 (`ts-sdk/src/execution/plan.ts`)
- 执行引擎 (并发、checkpoint、until/retry)
  - `runPlan()`：并发调度、按链限制读写并发、blocked->solver、executor IO、until 轮询、checkpoint/trace (`ts-sdk/src/engine/runner.ts`)
- 参考执行器 (网络 IO)
  - EVM: `EvmJsonRpcExecutor` (`ts-sdk/src/engine/executors/evm-jsonrpc.ts`)
  - Solana: `SolanaRpcExecutor` (`ts-sdk/src/engine/executors/solana-rpc.ts`)
- 参考 solver (补齐 contracts / 引导缺失输入)
  - `solver` / `createSolver()` (`ts-sdk/src/engine/solvers/solver.ts`)

结论：内部 `main` 应该做的主要是 “**配置 + 组装 + 安全护栏 + 人类可读输出**”，而不是重写 engine。

---

## 2. `main` 的职责边界

### 2.1 `main` 必须做

- Workspace 加载：从目录加载 `.ais.yaml` / `.ais-pack.yaml` / `.ais-flow.yaml`
- 选择入口：workflow / action / query 三种入口统一为 `ExecutionPlan`
- 运行时注入：把输入与环境注入 `ctx.runtime.inputs` / `ctx.runtime.ctx`
- 链配置：按 CAIP-2 `chain` 选择正确的 RPC、signer、并发限制
- 运行与观测：调用 `runPlan()`，把事件流转成可读日志，并保存 checkpoint/trace
- 完成态输出：计算 workflow `outputs`（`ValueRef`）并打印/落盘

### 2.2 `main` 不应该做 (保持 SDK 意图一致)

- 不在 `ts-sdk` 核心库里绑定具体钱包 UI 或账户管理体系
- 不把策略智能塞进 engine：策略/风控仍通过 solver/executor wrapper 或调用方实现
- 不让 spec “为了跑通 demo” 变得冗余：前置/后置检查优先用 workflow 节点表达

---

## 3. 三种入口如何统一成一个执行管线

内部 runner 支持三种运行模式，但实现上应该统一为 “构造 workflow -> build plan -> run plan”：

1. `workflow`：读取一个 `.ais-flow.yaml`，直接 `buildWorkflowExecutionPlan(workflow, ctx)`
2. `action`：用 `--action <protocol@version>/<actionId>` + `--args <json>` 构造一个**合成 workflow**：
   - 一个 `action_ref` 节点，`nodes[].args` 来自 `--args` (转为 `ValueRef` 的 `lit`)
   - `buildWorkflowExecutionPlan()` 会自动把 action 的 `composite` 展开成多节点 (step nodes)
3. `query`：同理，合成一个只包含一个 `query_ref` 的 workflow；如果要轮询，可直接在该节点上加 `until/retry/timeout_ms`

这样可以避免：
- 重新实现 composite expansion (它在 planner 内部已实现)
- 单独维护 “执行 action/query” 的旁路逻辑

---

## 4. 运行时上下文注入 (inputs/ctx/contracts/params)

### 4.1 运行时根对象 (参考 `examples/README.md`)

`ctx.runtime` 的关键字段：

- `inputs.*`：workflow inputs，用户/脚本提供
- `ctx.*`：环境 (地址、时间、能力、链相关)
- `contracts.*`：合约地址 bag，常由 solver 从 `protocol.deployments[].contracts` 自动填充
- `nodes.<id>.outputs.*`：引擎写回的 node 输出 (query decode、tx hash、receipt 等)

### 4.2 输入类型与数值约束 (关键点)

AIS 0.0.2 的执行关键路径里 **不应该出现 JS `number`** (尤其是 uint256/金额/decimal)。

runner 的输入策略建议：

- CLI/配置读取到的所有整数类值：优先接受字符串或 BigInt，并在注入 runtime 前转为 `bigint`
- 复杂类型 (asset/token_amount 等)：优先 JSON 输入，然后保持对象字段的 string/bigint 语义
- 对 `workflow.inputs` 做 “按声明 type 的 coercion”，不做隐式猜测

---

## 5. 链 RPC 与签名配置 (多链、可插拔、可审计)

### 5.1 配置文件建议格式

建议 runner 引入一个独立配置文件，例如 `ais-runner.config.yaml` (仅内部使用)：

```yaml
schema: "ais-runner/0.0.1"

engine:
  max_concurrency: 8
  per_chain:
    "eip155:8453": { max_read_concurrency: 8, max_write_concurrency: 1 }
    "solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp": { max_read_concurrency: 16, max_write_concurrency: 1 }

chains:
  "eip155:8453":
    rpc_url: "${EVM_RPC_BASE}"
    wait_for_receipt: true
    receipt_poll: { interval_ms: 1000, max_attempts: 120 }
    signer:
      type: "evm_private_key"
      private_key_env: "EVM_PRIVATE_KEY"

  "solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp":
    rpc_url: "${SOLANA_RPC_MAINNET}"
    commitment: "confirmed"
    wait_for_confirmation: true
    signer:
      type: "solana_keypair_file"
      keypair_path: "~/.config/solana/id.json"

runtime:
  ctx:
    # runner 会自动补齐 ctx.wallet_address / ctx.solana_address (如 signer 可用)
    capabilities: []
```

说明：
- `chains` 的 key 必须是 CAIP-2 chain id，与 workflow node 的 `chain` 精确一致
- signer 信息不写死在 workflow/spec 里，避免污染 spec 的可移植性
- `${ENV}` 展开由 runner 负责 (纯工具层)

### 5.2 Transport/Signer 适配策略 (复用现有 executors)

- EVM
  - `EvmJsonRpcExecutor` 需要：
    - `transport.request(method, params)`：建议用 `ethers.JsonRpcProvider#send` 适配
    - `signer.signTransaction({chainId,to,data,value})`：建议用 `ethers.Wallet` + provider，在 signer 内部完成 nonce/gas 填充
- Solana
  - `SolanaRpcExecutor` 需要：
    - `connection`：可直接用 `@solana/web3.js` 的 `Connection` (满足 required methods)
    - `signer`：Keypair wrapper 实现 `publicKey + signTransaction`

### 5.3 重要缺口：多链路由与 executor 选择

当前 `EvmJsonRpcExecutor.supports()` 只判断 `node.chain.startsWith("eip155:")`，不区分具体 chain。
如果 runner 同时配置多个 EVM chain 的 executor，`pickExecutor()` 会总是选第一个，存在 “把 eip155:1 发到 base RPC” 的风险。

runner 应该提供一个链路由层 (推荐实现方式)：

- `ChainRouterExecutor`：内部维护 `Map<chainId, Executor>`，`supports()` 必须精确匹配 `node.chain`
- 这样既保持现有 executor 的简单性，也避免修改 engine 的 `pickExecutor()` 策略

Solana 也同理 (多个 cluster 时)。

---

## 6. 前置检查 / 构造交易 / 等待成功 / 后置检查 的落地语义

### 6.1 前置检查

runner 的前置检查来自 3 层：

1. 静态校验
   - `validateWorkspaceReferences()`：workflow -> pack -> protocol 关系是否自洽
   - `validateWorkflow()`：node protocol/action/query 是否存在、ValueRef 引用是否合法、DAG 无环
2. Readiness (执行前)
   - `getNodeReadiness*()`：缺失 refs、detect 需求、condition=false 跳过
3. 业务前置检查 (workflow 显式建模)
   - 例如 allowance/balance/quote 等，应该是 workflow 中的 `query_ref` 节点
   - `condition` 用于 “不足才 approve” 等可跳过分支 (参见 `examples/aave-branch-bridge-solana-deposit.ais-flow.yaml`)

### 6.2 Action 的 `requires_queries` + `calculated_fields` (当前 runner 必须补齐的关键语义)

协议 action 可能包含：

- `requires_queries: ["<queryId>", ...]`
- `calculated_fields.<name>.expr`（ValueRef，可包含 `{detect: ...}`）
- execution spec 内引用 `query["<queryId>"].*` 与 `calculated.<name>`（见 `examples/uniswap-v3.ais.yaml`）

但 `ExecutionPlan` 目前只携带 “选中的 execution spec”，并不会自动：

1. 执行 required queries 并填充 `runtime.query[...]`
2. 计算 calculated_fields 并填充 `runtime.calculated.*` 或 `nodes.<id>.calculated.*`

因此，内部 runner 的最小闭环必须提供一个 “ActionPreflight” 层（推荐以 executor wrapper 方式实现）：

- 在执行任意 `write` 节点前（`action_ref` / composite steps）：
  1. 根据 `node.source.protocol/action` 解析 action spec
  2. 确保 `requires_queries` 的结果存在：
     - 若 workflow 已有对应 `query_ref` 节点：将其结果 fan-out 到 `runtime.query[queryId]`（见 6.3）
     - 若缺失：runner 可选 `--auto-required-queries`，用 action params 自动构造并执行这些 queries（失败则报错/need_user_confirm）
  3. 计算 `calculated_fields`：
     - 以 action 的 `calculated_fields[*].inputs` 作为依赖提示，按依赖顺序求值（ValueRef + 可选 detect）
     - 写入 `runtime.calculated`（供 execution spec 引用），并同步写入 `nodes.<nodeId>.calculated`（供 workflow outputs 引用）

并发注意：
- `runtime.query` 与 `runtime.calculated` 是共享 bag。若 workflow 同时并行执行多个 action，可能发生覆盖。
- v1（内部验证优先跑通）：对所有 `write` 节点设置“全局串行”（不仅 per-chain=1），避免 calculated/query 互相污染。
- v2（正确并发）：需要在 compiler/readiness 中支持 node-local `root_overrides`（例如把 calculated/query 做成 per-node 覆盖），避免依赖全局共享 bag。

### 6.3 Workflow `query_ref` 的结果写入位置 (nodes vs query bag)

`ExecutionPlan` 对 workflow `query_ref` 节点默认写入 `nodes.<id>.outputs`（planner 已设置 `writes`）。
但部分协议 action 的 execution/calc 仍使用 `runtime.query["<queryId>"]`（例如 `examples/uniswap-v3.ais.yaml`）。

为兼容这两种风格，runner 建议增加一个 “QueryResultFanout” wrapper：

- 当执行完成一个 `query_ref` 节点且 `node.source.query` 存在时：
  - 除了 `nodes.<id>.outputs`，额外 `set runtime.query[queryId] = outputs`

这样 workflow 可以用 `nodes.<id>.outputs` 做编排，而 action/calculated_fields 仍能读到 `query[...]`。

### 6.4 构造交易与上链执行

- 交易构造由 execution compiler 完成：
  - EVM: `compileEvmExecution*()` (executor 内部调用)
  - Solana: `compileSolanaInstruction*()` (executor 内部调用)
- 网络发送与等待：
  - EVM: `eth_sendRawTransaction` + 可选 `eth_getTransactionReceipt` 轮询
  - Solana: `sendRawTransaction` + `confirmTransaction`

### 6.5 “等待成功”的判定

runner 需要把 “等待 receipt/confirmation” 和 “判定成功” 区分开：

- EVM receipt 存在不代表成功，runner 应检查：
  - `receipt.status == 0x1` (成功)；否则当作失败并抛出 error
- Solana `confirmTransaction` 返回 `err != null` 则失败

建议实现为 executor wrapper（不修改 core executor）：
- `StrictSuccessExecutor`：执行后检查 outputs.receipt/confirmation，失败则返回 error

### 6.6 后置检查与轮询 (until/retry/timeout_ms)

AIS workflow 规范把 “等待达标” 放在 node 字段而不是 execution type：

- `until`：在 node 每次执行成功后评估；truthy 才算该节点完成
- `retry`：轮询间隔与最大次数
- `timeout_ms`：总体超时

`runPlan()` 已实现该语义，并且**只允许 read 节点**使用 `until/retry`：
适用场景：跨链到账、余额变化、状态达标等（参见 `examples/bridge-send-wait-deposit.ais-flow.yaml`）。

---

## 7. 并发与调度策略 (并行时并行)

runner 的并行策略应直接使用 `runPlan()` 的参数：

- `max_concurrency`：全局并发上限
- `per_chain[chain].max_read_concurrency`
- `per_chain[chain].max_write_concurrency`

推荐默认值：
- reads 并行，writes 串行（EVM 同账户 nonce 顺序要求）
- 跨链 writes 可并行（不同链不同 nonce domain）

---

## 8. “内部验证用”安全护栏 (必要但简洁)

为了能在内部环境安全地真发交易，同时保持人类可读与可控，建议 runner 提供：

- `--dry-run`：只编译与打印将要发送的 tx，不广播
- `--broadcast`：显式允许广播（默认不广播）
- `--stop-on-error/--continue-on-error`：错误策略
- `--checkpoint <path> --resume`：可恢复运行
- `--trace <path>`：写 JSONL trace（便于审计与复现）

对于 pack/workflow policy 的 approval 门槛（`auto_execute_max_risk_level` 等），建议：
- runner 在准备执行 `write` 节点前，读取 action 的 `risk_level`，与 pack policy 对比
- 超过门槛则发出 `need_user_confirm`（内部验证可用 `--yes` 自动通过）

注：SDK 当前的内置 solver 主要处理 missing refs 与 contracts auto-fill；approval gate 更适合做成 executor wrapper 或 runner 层逻辑。

---

## 9. `main` 的模块化结构建议 (简洁可读)

建议把内部 runner 放在仓库根目录的 `tools/` 下，避免污染 SDK 公共 CLI：

```
tools/ais-runner/
  src/
    main.ts                # 入口：解析 args -> 选择模式 -> 组装 ctx/plan -> runPlan
    workspace.ts           # loadDirectoryAsContext + workspace/workflow 校验
    config.ts              # runner config (yaml/json) + env 展开 + 默认值
    runtime.ts             # inputs/ctx 注入 + 类型 coercion
    executors.ts           # ChainRouterExecutor + StrictSuccessExecutor + 具体链适配 (ethers/solana)
    output.ts              # workflow.outputs 计算 + 结果打印/落盘
  README.md                # 内部使用说明
```

关键点：
- `ts-sdk` 保持 “SDK for agents” 的定位；runner 是独立工具
- runner 只依赖 SDK 的公开 API：`loadDirectoryAsContext` / `buildWorkflowExecutionPlan` / `runPlan` / executors/solver

---

## 10. 端到端运行示例 (对齐仓库现有 examples)

以 `examples/aave-branch-bridge-solana-deposit.ais-flow.yaml` 为例：

1. workspace: 指向仓库根的 `examples/`，其中包含 protocol/pack/workflow
2. runner config: 提供 Base 的 EVM RPC + Solana mainnet RPC + 两条链的 signer
3. inputs: 按 workflow 声明注入 `inputs.collateral_amount_atomic` 等

预期行为：
- allowance query -> conditionally approve -> supply -> borrow
- 分支并发：transfer_to_cex 与 bridge_send 并行（deps 满足后 engine 允许并行）
- solana balance 轮询直到 `until` 成功 -> solana_deposit
- 输出 workflow `outputs` 汇总结果（tx hash / signature）

---

## 11. 设计小结

- runner 不需要实现新的编排引擎：`runPlan()` 已经覆盖并发、blocked/solver、until 轮询、checkpoint/trace
- 核心工程点在 “链配置与路由”：
  - 按 `node.chain` 精确选择 RPC + signer
  - 多链情况下必须避免 executor 误选（建议 `ChainRouterExecutor`）
- 前置/后置检查应尽量在 workflow 中显式表达（query + condition + until），runner 只提供护栏与可观测性
