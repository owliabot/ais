# AIS 0.0.2 多链 Workflow 执行闭环：精准表达 + 可恢复编排（设计重构方案）

日期：2026-02-06  
范围：AIS Spec（`specs/`）+ AIS TS SDK（`ts-sdk/`）  
前提：**不考虑历史兼容**；可以删除/收缩冗余；优先“最优雅、最简洁、最可扩展”的抽象。

---

## 0. 你提出的目标（我对需求的精确复述）

用户对 Agent 发出复杂意图，例如：

> 在以太坊 Aave 抵押 1000U 等值的 ETH，借出 850U；其中 500U 跨链到 Solana 存入某个 DeFi（JLP），剩余 350U 转到交易所地址。  
> 全流程要有前置/后置检查；中途出错时要把问题丢给 AI 判断怎么处理；有些步骤可并行。

你期望的工作流模式：

1) **LLM/Agent 负责**：理解意图 → 选择 AIS 协议 action/query → 组装成一个“多步 Workflow（DAG）”。  
2) **代码执行器负责**：只要 Workflow 构建正确且没有报错/需要确认，就能自动按 DAG 运行（查询→交易→后置检查→下一步）。  
3) **AI 介入点**：仅在 `need_user_confirm`（需要用户批复）或 `error`（异常/分叉决策）时介入。

关键点：
- Workflow 要支持 **多链**（EVM ↔ Solana）；
- Workflow 要支持 **并行**（deps 分叉）；
- Workflow 要支持 **轮询/直到满足**（跨链到账、交易确认、状态达标）；
- 每一步要有 **前置/后置检查**，但不能让 AIS 变得冗余繁杂。

---

## 1. 当前实现与缺口（结论先行）

### 1.1 已经对齐的方向

TS SDK 已经形成了正确的闭环骨架（可复现、可 checkpoint）：

- Planner：`buildWorkflowExecutionPlan()` 把 Workflow 编译成 `ExecutionPlan(DAG)`  
- Readiness：`getNodeReadiness()` 判断一个节点是否可执行（缺 ref / detect）  
- Solver：对 `runtime.*` 打补丁（缺 contracts/inputs/detect）  
- Executor：做链上 IO（示例：`EvmJsonRpcExecutor`）并写回 outputs  
- Engine：`runPlan()` 串起以上，并输出事件流 + checkpoint

### 1.2 目前无法表达你要的“Bridge 多链闭环”的根因

**Workflow nodes 没有 per-node chain**：`ais-flow/0.0.2` 的 node 结构缺 `chain` 字段；planner 只能对全 workflow 用一个 chain。  
结果：同一个 workflow 里无法把某些节点放在 `eip155:1`，另一些放在 `solana:*`。

### 1.3 目前执行闭环还缺的关键语义

即便补了多链，还缺两个“不可缺的”执行语义，否则只能靠外部 while-loop：

1) **轮询/直到满足（poll/until）**：跨链到账、状态达标、等待确认  
2) **结构化错误/决策协议**：把“错误丢给 AI 处理”变成可靠、可审计的接口，而不是把字符串丢给模型猜

---

## 2. 设计原则（避免 AIS 越来越冗余繁杂）

1) **把复杂性放在“可复用的抽象”里**，不要放在“每个 workflow 都要重复写的模板”里。  
2) **workflow 只表达 DAG 与参数绑定**，不要引入一堆“临时语法糖”。  
3) **跨链不是特例**：Bridge 只是“某协议在不同链上的 action/query 节点”组合；核心仍然是 DAG。  
4) **把循环变成语义**（poll/until），不要让执行器写死 while(true)。  
5) **AI 只处理决策**：确认/异常/策略分叉；正常执行必须 deterministic。

---

## 3. 最小但强表达的 Spec 改造（推荐）

### 3.1 WorkflowNode 增加 `chain`（多链表达的最小闭环）

在 `ais-flow/0.0.2` 的 node 上新增可选字段：

```yaml
chain: "eip155:1"     # 可选；缺省时继承 workflow 的 default_chain（或 runner 传入的默认 chain）
```

语义：
- 每个 node 的执行链独立；
- DAG 依赖仍由 `deps` + `nodes.<id>.*` 引用推断；
- planner/engine 以 node.chain 作为该 node 的最终 chain。

这一个字段足以把“Bridge=多链 DAG”表达出来，而不引入任何新的文档类型。

### 3.2 引入“可恢复轮询”的统一语义（避免桥接场景靠外部循环）

建议在 **node 层**增加两个可选字段（仅影响执行器/引擎，不增加 execution type）：

```yaml
retry:
  interval_ms: 2000
  max_attempts: 300
  backoff: "fixed"     # fixed | linear | exp（可选，默认 fixed）
timeout_ms: 600000      # 可选：总超时

until: { cel: "nodes.wait.outputs.arrived == true" }  # 可选：仅对 query_ref 推荐
```

语义（引擎负责）：
- 对于带 `until` 的 query 节点：重复执行 executor → 写回 outputs → 计算 `until`；
- `until=true` 才算该节点 completed；否则按 retry 继续；
- 超时或 attempts 超限 → `error`（带结构化诊断）。

这样“跨链到账检查”不需要额外的 action 类型，也不需要外部 while-loop。

> 备注：这比新增 `wait_until` execution type 更“收敛”，因为循环是引擎能力而不是协议 execution 的一种。

### 3.3 前置/后置检查：不引入新 execution type，优先用“标准 query + until/condition”

为了避免 AIS 变冗余，建议：
- **检查依旧用 query_ref 表达**（余额/allowance/状态/到账），不要新增 `assert` 类型；
- 条件分支用现有 `condition`（如“allowance 不够就 approve”）；
- 后置达标用 `until`（如“到账了再继续”）。

这样 spec 增量非常小，但表达能力覆盖你描述的大部分“前置/后置检查 + 轮询”。

> 可选增强（后续）：在 protocol action 上提供 `recommended_prechecks/recommended_postchecks` 作为 lint/建议，而不是强制语义，避免 spec 过早膨胀。

---

## 4. 多链 Bridge Workflow 如何表达（在上述最小改动下）

核心思想：**Bridge 不需要“特殊跨链动作”**。它是多个 node 的组合：

1) 源链：余额/allowance check（query）  
2) 源链：必要时 approve（action，带 condition）  
3) 源链：bridge send（action）→ 输出 message_id/tx_hash  
4) 目的链：poll bridge status / mint 余额 / receiver account（query + until + retry）  
5) 目的链：deposit（action）

并行：抵押借出完成后，`transfer_to_cex` 与 `bridge_flow` 仅依赖同一个 borrow 节点即可并行。

### 4.1 示例（结构示意，非最终字段名）

```yaml
schema: "ais-flow/0.0.2"
meta: { name: "demo", version: "0.0.2" }
default_chain: "eip155:1"         # 新增：可选（也可以继续由 runner 传入默认 chain）

inputs:
  collateral_amount: { type: token_amount, required: true }
  borrow_amount: { type: token_amount, required: true }
  bridge_amount: { type: token_amount, required: true }
  cex_address: { type: address, required: true }

nodes:
  - id: aave_supply
    chain: "eip155:1"
    type: action_ref
    protocol: "aave@0.0.2"
    action: "supply_eth"
    args: { amount: { ref: "inputs.collateral_amount" } }

  - id: aave_borrow
    chain: "eip155:1"
    type: action_ref
    protocol: "aave@0.0.2"
    action: "borrow_usdc"
    deps: ["aave_supply"]
    args: { amount: { ref: "inputs.borrow_amount" } }

  # 并行分支 A：转账到交易所
  - id: transfer_to_cex
    chain: "eip155:1"
    type: action_ref
    protocol: "erc20@0.0.2"
    action: "transfer"
    deps: ["aave_borrow"]
    args:
      token: { ref: "calculated.usdc" }
      to: { ref: "inputs.cex_address" }
      amount: { cel: "to_atomic(inputs.borrow_amount, calculated.usdc) - to_atomic(inputs.bridge_amount, calculated.usdc)" }

  # 并行分支 B：跨链
  - id: bridge_allowance
    chain: "eip155:1"
    type: query_ref
    protocol: "erc20@0.0.2"
    query: "allowance"
    deps: ["aave_borrow"]
    args:
      token: { ref: "calculated.usdc" }
      owner: { ref: "ctx.wallet_address" }
      spender: { ref: "contracts.bridge_spender" }

  - id: bridge_approve
    chain: "eip155:1"
    type: action_ref
    protocol: "erc20@0.0.2"
    action: "approve"
    deps: ["bridge_allowance"]
    condition: { cel: "nodes.bridge_allowance.outputs.allowance < to_atomic(inputs.bridge_amount, calculated.usdc)" }
    args:
      token: { ref: "calculated.usdc" }
      spender: { ref: "contracts.bridge_spender" }
      amount: { ref: "inputs.bridge_amount" }

  - id: bridge_send
    chain: "eip155:1"
    type: action_ref
    protocol: "some-bridge@0.0.2"
    action: "send"
    deps: ["bridge_allowance", "bridge_approve"]  # approve 若 skipped，不会阻塞
    args:
      token: { ref: "calculated.usdc" }
      amount: { ref: "inputs.bridge_amount" }
      to_chain: { lit: "solana:5eyk..." }
      to_address: { ref: "ctx.wallet_address_solana" }

  - id: wait_arrival
    chain: "solana:5eyk..."
    type: query_ref
    protocol: "some-bridge@0.0.2"
    query: "arrival_status"
    deps: ["bridge_send"]
    args:
      message_id: { ref: "nodes.bridge_send.outputs.message_id" }
    retry: { interval_ms: 2000, max_attempts: 600 }
    until: { cel: "nodes.wait_arrival.outputs.arrived == true" }

  - id: solana_deposit
    chain: "solana:5eyk..."
    type: action_ref
    protocol: "jlp@0.0.2"
    action: "deposit"
    deps: ["wait_arrival"]
    args:
      amount: { ref: "inputs.bridge_amount" }
```

这套表达具备：
- 多链（node.chain）  
- 并行（deps 分叉）  
- 前置/后置检查（query + condition + until）  
- 可恢复轮询（retry/until）  
- 引擎可 checkpoint/resume（无需 AI 常驻）

---

## 5. TS SDK 重构/扩展方案（与 Spec 改动一一对应）

### 5.1 Planner：按 node.chain 选择 execution（替换“单一 chain”假设）

改造点：
- `WorkflowNodeSchema` 增加 `chain?: string` + `WorkflowSchema` 增加 `default_chain?: string`（可选）
- `buildWorkflowExecutionPlan()`：对每个 node 使用 `node.chain ?? workflow.default_chain ?? options.chain`

收益：
- ExecutionPlan 天然成为多链 DAG；
- 现有调度器按 chain 限制并发/串行能直接复用。

### 5.2 Readiness：支持 until 表达式（并把“重试”交给引擎）

改造点：
- `getNodeReadiness()` 仍然只判断“单次执行所需 ValueRef 是否就绪”
- `until` 的判定不应放在 readiness（否则会把“可执行”与“是否完成”混在一起）；应放在 engine 执行后。

### 5.3 Engine：引入 poll/until、blocked 不阻塞全局、幂等恢复

#### (a) poll/until
- 对 query_ref 节点：执行一次 executor 后，写回 outputs，再评估 `until`：
  - true → completed
  - false → 进入 retry 计数，等待 interval 再执行（期间不阻塞其他可运行节点）

#### (b) blocked 处理策略（并行友好）
当前参考 runner 在 `need_user_confirm` 时会整体 return。更理想的默认策略：
- `need_user_confirm` 的节点被标记为“暂停/待确认”，但引擎继续推进其他不依赖它的分支；
- 当所有可推进分支都耗尽且仍存在待确认节点 → 引擎输出 “需要确认汇总” 并暂停。

#### (c) 幂等/恢复
建议引入最小幂等策略（不膨胀 spec）：
- 若 node outputs 已包含 `tx_hash` / `message_id` 等“外部可定位标识”，executor 支持 `resume` 路径：优先查 receipt/status，而不是重发。

### 5.4 Executors：多链闭环需要的最小集合

现状：只有 `EvmJsonRpcExecutor`。要跑通你描述的场景，需要至少：

- `SolanaRpcExecutor`：执行 `solana_instruction`（发送/确认/模拟） + 后续补 `solana_read`（余额/账户）  
- `BridgeExecutor`：不是强制要“通用桥执行器”，可以先用某 bridge 协议的 action/query spec；关键是配套 executor 能跑对应的 execution types。  

### 5.5 错误与 AI 交互：把“丢给 AI”变成可控协议

引擎/执行器输出事件时，必须包含足够的结构化诊断，让 AI 能做“可解释决策”：
- node id / chain / execution type
- RPC method/request/response（脱敏/裁剪）
- revert reason / logs / program error code
- attempts/elapsed/last_success_checkpoint

输出给 AI 的不是“日志文本”，而是“诊断对象 + 可选动作列表（retry/patch/ask user/abort）”。

---

## 6. 收缩/删除建议（防止 AIS 膨胀）

以下是“为了更简洁”建议收缩的方向（在不影响表达能力前提下）：

1) **避免新增一堆 execution types**：优先把“循环/重试/直到满足”放在 node 通用字段（retry/until/timeout）。  
2) **避免把 pre/post checks 内置进 action 强语义**：先用 workflow 节点表达（可 lint 建议），避免 spec 过早绑定某些 DeFi 习惯。  
3) **桥不是一级类型**：不引入 `bridge:*` chain 或特殊全局语法；桥协议就是 protocol spec，跨链由 node.chain 组合完成。  
4) **减少“重复写 outputs path”**：默认写回 `nodes.<id>.outputs`，仅在需要时使用 `writes` 覆盖。

---

## 7. 实施路线图（不考虑兼容，直接落地）

P0（让“Bridge 多链 workflow”先能表达并跑通最小闭环）：
1) Workflow node 增加 `chain` + workflow 增加 `default_chain`  
2) Planner 按 node.chain 选择 execution spec  
3) Engine 支持 `retry/until/timeout`（仅先支持 query_ref + until）  
4) 增强事件/错误诊断 payload（供 AI/产品接入）

P1（让 Solana 与跨链闭环更完整）：
1) `SolanaRpcExecutor`（send/confirm）  
2) `solana_read`（或最小 RPC query）+ poll/until  
3) 幂等恢复：基于 `tx_hash/message_id` 的 resume 策略

P2（减少 agent 手工模板，提高 “workflow 构建正确性”）：
1) protocol lint：为常见 action 提供 recommended checks（余额/allowance/到账）  
2) 可选的 action expansion（把模板下沉到 spec 作者，而不是每次由 LLM 临时生成）

---

## 8. 判断标准（“是不是最佳设计”）

当以下条件都成立时，就达到了你要的“最佳实践”：

1) LLM 只需构建 workflow（少量、清晰、可审计），正常执行不再需要 AI 常驻。  
2) 多链/并行/轮询都是 workflow+engine 的一等语义，而不是外部脚本胶水。  
3) 每一步的失败都能产出结构化诊断，AI/用户可做可控决策（重试/改参/批复/终止）。  
4) Spec 增量极小（主要是 `node.chain` + `retry/until/timeout`），表达力却足以覆盖“桥接 + 到账等待 + 目的链操作”的主路径。

