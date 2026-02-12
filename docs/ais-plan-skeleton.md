# AGT102: PlanSkeleton (`ais-plan-skeleton/0.0.1`)

目标：让 agent 输出最小、结构化的“骨架计划”，由 SDK 编译为 `ExecutionPlan`，避免生成整份 workflow YAML。

## 文档形态

- `schema: "ais-plan-skeleton/0.0.1"`
- `default_chain?`
- `nodes[]`
- `policy_hints?`（提示，不可绕过 pack/policy）

## Node 形态

`type: "action_ref"`:

- `id`
- `protocol`（如 `uniswap-v3@1.0.0`）
- `action`
- `chain?`（缺省使用 `default_chain`）
- `args?`（ValueRef）
- `deps?`

`type: "query_ref"`:

- `id`
- `protocol`
- `query`
- 其余同上

可选控制字段：

- `condition` / `until` / `retry` / `timeout_ms`

## 编译

SDK：`compilePlanSkeleton(input, ctx, { default_chain? })`

- 成功：返回 `{ ok: true, plan, workflow }`
  - `workflow` 是内部合成物（不会写文件）
  - `plan.extensions.plan_skeleton` 会携带 `policy_hints`
- 失败：返回 `{ ok: false, issues[] }`
  - `issues[]` 结构：`{ kind, severity, node_id?, field_path, message, reference? }`

## 示例

见：

- `examples/plan-skeleton/swap.json`
- `examples/plan-skeleton/approve-if-needed.json`
- `examples/plan-skeleton/wait-until.json`
