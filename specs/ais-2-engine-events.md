# AIS-2E: Engine Event Protocol (`ais-engine-event/0.0.3`)

Status: Draft  
Spec Version: 0.0.2  

定义 agent/runner/UI 的稳定事件协议，支持回放、审计和跨进程集成。

## 1. 事件输出形态（JSONL）

每行一个 JSON 对象：

- `schema`: `"ais-engine-event/0.0.3"`
- `run_id`: string
- `seq`: number（单调递增）
- `ts`: RFC3339 timestamp
- `event`: `{ type, node_id?, data, extensions? }`

## 2. 最小事件集合（AGT002）

- `plan_ready`
- `node_ready`
- `node_blocked`
- `need_user_confirm`
- `need_user_input`
- `query_result`
- `tx_prepared`
- `tx_sent`
- `tx_confirmed`
- `node_waiting`
- `checkpoint_saved`
- `engine_paused`
- `error`

可扩展事件（不影响最小集合契约）：

- `solver_applied`
- `node_paused`
- `skipped`
- `plan_replaced`
- `command_accepted`
- `command_rejected`
- `patch_applied`
- `patch_rejected`
- `side_effect_observed`

## 3. `need_user_confirm` 结构化字段约束

`event.type = need_user_confirm` 时，`event.data.details` 至少应包含：

- `node_id`
- `action_ref`
- `hit_reasons: string[]`

推荐包含：

- `workflow_node_id`
- `chain`
- `execution_type`
- `pack_summary`（pack 名称/版本/协议上下文）
- `policy_summary`（策略模式、风险阈值、缺失字段）

## 3.1 `need_user_input` 结构化字段约束

`event.type = need_user_input` 时，`event.data` 至少应包含：

- `reason_code`
- `reason`
- `details`（对象）

推荐在 `details` 中包含：

- `command_id`（触发该补参请求/校验失败的命令 id）
- `input_id`
- `question_id`

## 4. 错误事件约束

`error` 事件应包含：

- `reason`
- `retryable`（boolean）
- `error`（经 AIS JSON codec 序列化后的错误对象）

## 4.3 Side-effect 事件约束（side_effect_observed）

当执行器/引擎观察到可持久化 side-effect 时，推荐输出：

- `event.type = side_effect_observed`
- `event.data.record` MUST conform to `ais-side-effect-record/0.1.0`

该事件用于 checkpoint ledger 的事件驱动重建，避免 runner 通过 runtime 输出字段猜测 side-effect。

## 4.1 命令回执事件约束（command_accepted / command_rejected）

当 runner 以 JSONL 方式接收 commands（见 `specs/ais-2-engine-commands.md`）时，引擎应输出命令回执事件用于 agent-loop 对账与重放。

`event.type = command_accepted | command_rejected` 时，`event.data` 至少应包含：

- `command_id: string`（对应 command envelope 的 `command.id`）
- `command_type: string`（例如 `apply_patches` / `user_confirm` / `cancel` / `replace_plan`）
- `command_type: string`（例如 `apply_patches` / `user_confirm` / `user_input` / `user_select` / `cancel` / `replace_plan`）
- `duplicate: boolean`（是否为重复 id）
- `noop: boolean`（是否为 no-op；推荐 duplicate 时为 true）

推荐包含：

- `reason: string`（例如 `duplicate_command_id`）

## 4.2 Plan 替换事件约束（plan_replaced）

当引擎接受并应用 `replace_plan` 命令后，应输出 `plan_replaced` 事件用于审计与回放对账。

`event.type = plan_replaced` 时，`event.data` 至少应包含：

- `before_plan_hash: string`
- `after_plan_hash: string`

推荐包含：

- `command_id: string`（触发替换的命令 id）
- `reason: string`（人类可读摘要）

## 5. Redaction

- 默认模式必须脱敏私钥、seed、原始签名材料、完整 RPC payload、PII。
- 审计模式可在显式配置下保留更多字段。
- runner 可通过 `--trace-redact <default|audit|off>` 指定。

## 6. Runner CLI 输出

- `--events-jsonl <path|stdout>` 输出原始事件 JSONL（`stdout` 或 `-` 表示标准输出）。
- 默认文本事件输出行为保持不变。

## 7. 权威 Schema

- JSON Schema: `schemas/0.0.2/engine-event.schema.json`
- Agent checkpoint contract: `specs/ais-2-agent-checkpoint.md`
- Side-effect record contract: `specs/ais-2-side-effects.md`
