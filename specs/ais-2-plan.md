# AIS-2D: Execution Plan Contract (`ais-plan/0.0.3`)

Status: Draft  
Spec Version: 0.0.2  

`ExecutionPlan` 是 runner 的执行契约（plan-first），用于跨实现互操作。

对于自然语言入口（`agent --intent`）的输入/输出 contract，见 `specs/ais-2-agent-intent.md`。

## 1. 角色与边界

- `workflow`：内容资产/模板（面向编排与复用）。
- `plan`：执行契约（面向调度与执行）。
- 可以 `workflow -> plan` 导出；`plan -> workflow` 仅限受限场景，不保证无损往返。

不保证无损的典型项：

- composite 展开后的 `step_id` 与中间节点
- 运行时补齐字段（如 `writes`、`bindings`、`source`）
- 非决定性元信息（如 `meta.created_at`）

## 2. Determinism 约束

在相同 `workflow + pack + inputs + ctx + registry/plugin set` 前提下，`workflow -> plan` 结果必须一致，允许以下字段差异：

- `meta.created_at`
- `extensions` 下的非语义字段（如 trace 标签）

## 3. 最小字段集

顶层：

- `schema: "ais-plan/0.0.3"`
- `nodes: ExecutionPlanNode[]`
- `meta?`
- `extensions?`

节点：

- `id`
- `chain`
- `kind`
- `execution`
- 可选：`deps`、`condition`、`assert`、`assert_message`、`until`、`retry`、`timeout_ms`、`writes`、`bindings`、`source`、`extensions`

## 4. `kind` 语义

- `query_ref`: 来源于 workflow query 节点，默认 read 路径。
- `action_ref`: 来源于 workflow action 节点，默认 write 路径。
- `execution`: 由 planner 展开/合成的执行节点（例如 composite step）。

## 4.1 `execution.type` 与可执行性（core vs plugin）

`execution` 对象必须包含 `type` 字段。`execution.type` 是引擎路由执行器的 **路由键**。

Normative rules:

- 引擎 MUST 维护一个以 `execution.type` 为 key 的 handler/ executor 注册表。
- AIS-2 定义的 core execution types（EVM/Solana/Composite 等）可视为默认注册；除此之外均为 plugin execution types。
- 引擎 MUST NOT 执行任何未注册 handler 的 `execution.type`。
- 当 pack 处于激活状态时：
  - 引擎 MUST 将 pack 的 `plugins.execution.enabled` 作为 plugin execution types 的 allowlist。
  - 引擎 MUST 拒绝任何不在 allowlist 内的 plugin execution type（即使该 handler 已安装/注册）。

Recommended behavior:

- 在 planning/validation 阶段尽早拒绝（fail-fast），避免生成“看似可执行但运行期必停”的 plan。
- 若 plan 是外部直接输入且无法提前校验，引擎 MUST 输出结构化错误（事件或 issues），并不得隐式降级为其他执行路径。

## 4.2 Plan mutation（`replace_plan`）与可追踪性建议

AIS 允许通过 engine commands 在运行期替换 plan（见 `specs/ais-2-engine-commands.md` 的 `replace_plan`）。

为了保证可重放与审计，推荐实现遵循以下约定（推荐项，非强制）：

- 引擎/runner 在每次启用新 plan 时计算并记录 `plan_hash`（基于稳定序列化）。
- 若 plan 在运行期被替换，记录“父子关系”（plan epoch）：
  - `parent_plan_hash`：替换前的 plan hash
  - `next_plan_hash`：替换后的 plan hash
- 建议把上述关系写入：
  - checkpoint（最推荐），或
  - events（例如 `plan_replaced`），或
  - plan `extensions`（仅作为附加提示，不应成为唯一审计来源）

注意：
- plan mutation 不能绕过 pack allowlist、handler 注册、以及 policy gate。
- 推荐在替换前输出 plan diff 并触发相应的确认策略（risk-based）。

## 5. `writes` 语义

- `mode=set`：整体覆盖目标路径。
- `mode=merge`：浅层对象合并。
- 多节点写同一路径由执行时序决定（后写覆盖先写）；若需要严格控制，必须通过 `deps` 强制顺序。
- 默认安全可写路径由 RuntimePatch guard 决定（见 AGT008，默认 `inputs/ctx/contracts/policy`）。

## 6. 版本策略（AGT001A）

`schema` 版本（如 `ais-plan/0.0.3`）必须在以下情况 bump：

- 字段新增/删除且影响语义
- 字段语义变化
- 校验规则变更导致行为变化

不需要 bump `schema` 的场景：

- 仅新增 `meta` 内容
- 仅新增 `extensions` 的实现私有字段（不影响规范语义）

## 7. 稳定序列化建议

- 统一使用 AIS JSON codec（见 `docs/ais-json-codec-profile.md`）。
- 建议对象键按字典序输出，数组保持原序。
- `BigInt/bytes/Error` 表示必须遵循统一 profile，避免跨语言语义漂移。

## 8. 权威 Schema

- JSON Schema: `schemas/0.0.2/plan.schema.json`
