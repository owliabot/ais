# AGT103: Fragments (Micro-Templates) And Composition Rules

Fragments 是针对 agent 的“微模板/片段库”：复用稳定的 DAG 结构与控制语义，避免让 agent 直接生成长篇 workflow 文本。

本仓库的落地形式以 `PlanSkeleton`（见 `docs/ais-plan-skeleton.md`）为目标输出：fragment 组合得到一个 `ais-plan-skeleton/0.0.1`，再由 SDK 编译为 `ExecutionPlan`。

## 1. Fragment 的契约

每个 fragment 必须明确：

- 输入槽位：需要哪些语义输入（映射到 `inputs.*` / `ctx.*` / 上游 `nodes.*.outputs.*`）
- 输出槽位：该片段产生哪些可被引用的输出（`nodes.*.outputs.*`）
- 失败策略：缺字段/引用错误/执行失败时如何推进（hard block / need confirm / retry / skip）
- 风险标签：用于 pack/policy gate 做确认与审计（例如 `approval/slippage/bridge`）
- 适用前置条件：要求协议侧存在的 action/query、链支持、依赖关系等

注意：

- fragment 不是绕过 pack 的机制。pack 的 allowlist + policy gate 仍是最终边界。
- fragment 不要求和任何具体协议绑定；绑定发生在组合时填写 `protocol@version` 与 action/query id。

## 2. 组合规则（Composition Rules）

### 2.1 命名空间与 ID

- 组合多个 fragment 时，必须保证 `nodes[].id` 唯一。
- 推荐规则：`<fragKey>__<localNodeId>`，例如 `approve__allowance`、`swap__quote`。

### 2.2 依赖连边

- 显式依赖：通过 `deps` 指定。
- 数据依赖：通过 `args` 里的 `ref: "nodes.<id>.outputs.<field>"` 形成隐式依赖（planner 会推断）。
- 推荐：关键安全链路（approve -> swap）用显式 deps 固化顺序。

### 2.3 链与跨链

- 同一片段默认在同一条链上运行，使用 `default_chain` 或 `nodes[].chain`。
- 跨链组合必须显式设置各节点 `chain`，并通过 deps 串联（例如 bridge send -> wait/destination）。

### 2.4 条件与分支

- 使用 `condition` 实现“可选节点”（例如 allowance 足够则跳过 approve）。
- 条件表达式建议仅引用已完成依赖节点的 outputs 与 inputs/ctx。

### 2.5 轮询等待

- 使用 `until + retry (+timeout_ms)` 实现“wait-until”类片段。
- `until` 失败不会 hard fail，而是进入 `node_waiting`，直到超时/最大尝试。

### 2.6 Gate/确认点（与 AGT005/AGT010 关系）

- “guardrail-gate”片段的作用不是生成一个单独的节点，而是约定写节点必须携带足够信息让 policy gate 生效：
  - swap-like: `slippage_bps`、`spend_amount`
  - approve-like: `approval_amount`、`spender_address`、`unlimited_approval`
- 若 preview 编译失败，policy gate 会 hard block（见 `docs/ais-policy-gate-input-rules.md`）。

## 3. Fragment 库（10+）

下列片段以 PlanSkeleton 片段展示。组合时请按 2.1 做 id 重命名。

### F01 `read`

- 输入槽位：`protocol_ref`、`query_id`、`args`
- 输出槽位：`nodes.<id>.outputs.*`
- 失败策略：query 执行失败 -> `error`（由 executor）
- 风险标签：`read_only`
- 前置条件：协议存在 query；链支持 query execution

```json
{
  "id": "read",
  "type": "query_ref",
  "protocol": "demo@0.0.2",
  "query": "quote",
  "args": { "amount_in": { "ref": "inputs.amount_in" } }
}
```

### F02 `write`

- 输入槽位：`protocol_ref`、`action_id`、`args`
- 输出槽位：`nodes.<id>.outputs.*`（若 action 定义 returns/或执行器写 outputs）
- 失败策略：写失败 -> `error`；若 pack/policy 命中 -> `need_user_confirm/hard_block`
- 风险标签：`write`
- 前置条件：协议存在 action；链支持 action execution

```json
{
  "id": "write",
  "type": "action_ref",
  "protocol": "demo@0.0.2",
  "action": "swap",
  "args": { "amount_in": { "ref": "inputs.amount_in" } }
}
```

### F03 `read-then-write`

- 输入槽位：read args、write args（可引用 read outputs）
- 输出槽位：write outputs
- 失败策略：read 失败 -> 停；write 可能触发 gate
- 风险标签：`write`
- 前置条件：read 输出字段可满足 write 参数

```json
{
  "nodes": [
    { "id": "q", "type": "query_ref", "protocol": "demo@0.0.2", "query": "quote", "args": { "x": { "ref": "inputs.x" } } },
    { "id": "a", "type": "action_ref", "protocol": "demo@0.0.2", "action": "swap", "deps": ["q"], "args": { "amount_in": { "ref": "inputs.x" }, "min_out": { "ref": "nodes.q.outputs.y" } } }
  ]
}
```

### F04 `quote-then-swap`

- 输入槽位：`amount_in`、token in/out（协议决定）
- 输出槽位：swap outputs
- 失败策略：缺 quote 输出/缺 slippage -> gate confirm 或 hard block（preview_compile）
- 风险标签：`slippage`
- 前置条件：协议同时提供 quote query 与 swap action

模板同 `examples/plan-skeleton/swap.json`。

### F05 `approve-if-needed`

- 输入槽位：`owner`、`spender`、`amount`
- 输出槽位：approve 结果（可忽略）
- 失败策略：allowance query 失败 -> 停；approve 触发 approval gate
- 风险标签：`approval`
- 前置条件：协议提供 allowance query + approve action；approve condition 可表达

模板同 `examples/plan-skeleton/approve-if-needed.json`。

### F06 `guardrail-gate`（policy gate 适配片段）

- 输入槽位：按写类型需要（swap/approve/transfer/bridge）
- 输出槽位：无
- 失败策略：缺关键字段 -> `need_user_confirm`；preview 编译失败 -> `hard_block`
- 风险标签：`policy_gate`
- 前置条件：pack.policy 存在且 runner 开启 policy gate wrapper

说明：这是“约定型片段”，用于指导 agent 选择参数字段与命名，确保 gate 可稳定提取。

### F07 `wait-until`

- 输入槽位：poll query args、until 条件、retry 参数
- 输出槽位：最终 poll outputs
- 失败策略：超时/最大尝试 -> `error`
- 风险标签：`wait`
- 前置条件：query 支持幂等轮询

模板同 `examples/plan-skeleton/wait-until.json`。

### F08 `read-then-assert`

- 输入槽位：query args、assert 表达式
- 输出槽位：无（或 query outputs）
- 失败策略：assert falsy -> `error`（engine assert）
- 风险标签：`verification`
- 前置条件：assert 表达式可用（引用 query outputs）

```json
{
  "nodes": [
    { "id": "q", "type": "query_ref", "protocol": "demo@0.0.2", "query": "status", "args": { "id": { "ref": "inputs.id" } } }
  ],
  "post": { "assert": { "cel": "nodes.q.outputs.ok == true" }, "assert_message": "status not ok" }
}
```

注：当前 PlanSkeleton 只承载 node-level 字段；若要 assert，请在组合时把 `assert` 放到对应 node 上（与 workflow/plan 字段一致）。

### F09 `write-then-verify`

- 输入槽位：write args、verify query args（引用 write outputs 或 inputs）
- 输出槽位：verify outputs
- 失败策略：verify 可配 until/retry（等待链上最终一致）
- 风险标签：`verification`
- 前置条件：存在可验证的 query（例如 balance/receipt/status）

```json
{
  "nodes": [
    { "id": "a", "type": "action_ref", "protocol": "demo@0.0.2", "action": "send", "args": { "x": { "ref": "inputs.x" } } },
    { "id": "q", "type": "query_ref", "protocol": "demo@0.0.2", "query": "receipt", "deps": ["a"], "args": { "tx": { "ref": "nodes.a.outputs.tx_hash" } }, "until": { "cel": "nodes.q.outputs.done == true" }, "retry": { "interval_ms": 2000, "max_attempts": 30 } }
  ]
}
```

### F10 `send-then-wait`（receipt 等待）

- 输入槽位：send args
- 输出槽位：receipt/status outputs
- 失败策略：wait 超时 -> error；policy gate 仍按 send 的写节点裁决
- 风险标签：`wait`
- 前置条件：协议提供 send action + receipt/status query

等价于 F09 的特化版。

## 4. 长尾意图（5 个组合示例）

本目录给出“组合后输出 PlanSkeleton”的示例（不输出 workflow YAML）：

- `examples/fragments/intent-01-swap-with-approve.json`
- `examples/fragments/intent-02-bridge-send-wait.json`
- `examples/fragments/intent-03-swap-with-price-guard.json`
- `examples/fragments/intent-04-withdraw-then-verify.json`
- `examples/fragments/intent-05-two-step-write-verify.json`

