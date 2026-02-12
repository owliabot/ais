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
- `command_accepted`
- `command_rejected`
- `patch_applied`
- `patch_rejected`

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

## 4. 错误事件约束

`error` 事件应包含：

- `reason`
- `retryable`（boolean）
- `error`（经 AIS JSON codec 序列化后的错误对象）

## 5. Redaction

- 默认模式必须脱敏私钥、seed、原始签名材料、完整 RPC payload、PII。
- 审计模式可在显式配置下保留更多字段。
- runner 可通过 `--trace-redact <default|audit|off>` 指定。

## 6. Runner CLI 输出

- `--events-jsonl <path|stdout>` 输出原始事件 JSONL（`stdout` 或 `-` 表示标准输出）。
- 默认文本事件输出行为保持不变。

## 7. 权威 Schema

- JSON Schema: `schemas/0.0.2/engine-event.schema.json`
