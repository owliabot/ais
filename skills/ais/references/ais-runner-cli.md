# `ais-runner` CLI Reference

## 1) Command surface

## `run plan`

```bash
ais-runner run plan \
  --plan <file> \
  [--config <runner-config>] \
  [--runtime <runtime-file>] \
  [--dry-run] \
  [--events-jsonl <path|->] \
  [--trace <path>] \
  [--checkpoint <path>] \
  [--commands-stdin-jsonl] \
  [--verbose] \
  [--format text|json]
```

### `--runtime <file>` (optional)

Optional JSON or YAML file providing runtime context for reference resolution. Use when a plan references `inputs.*` or other runtime namespaces:

```yaml
# example runtime.yaml
inputs:
  amount: "1000000"   # USDC atomic units
  recipient: "0xRecipient..."
```

Without this file, plans with unresolved `inputs.*` refs will fail at execution time.

Notes:
- `--config` is optional only for `--dry-run`; required for execution mode.
- `--events-jsonl -` sends raw event JSONL to stdout; suppresses summary on `run plan` / `run workflow`. For `agent`: event lines and final summary **both** go to stdout — use a file path instead to keep them separate.

## `run workflow`

```bash
ais-runner run workflow \
  --workflow <file> \
  [--workspace <dir>] \
  [--config <runner-config>] \
  [--runtime <runtime-file>] \
  [--dry-run] \
  [--events-jsonl <path|->] \
  [--trace <path>] \
  [--checkpoint <path>] \
  [--outputs <json-file>] \
  [--commands-stdin-jsonl] \
  [--verbose] \
  [--format text|json]
```

Notes:
- `--workspace` defaults to workflow file parent directory.
- `--outputs` writes evaluated workflow `outputs` after successful execution.

## `plan diff`

```bash
ais-runner plan diff --before <plan> --after <plan> [--format text|json]
```

## `replay`

```bash
ais-runner replay \
  [--trace-jsonl <file> | --checkpoint <file> --plan <file> --config <runner-config>] \
  [--until-node <id>] \
  [--format text|json]
```

## `agent`

```bash
ais-runner agent \
  (--plan <file> | --intent <text> | --intent-file <file>) \
  --config <runner-config> \
  [--workspace <dir>] \
  [--pack <pack-file>] \
  [--runtime <runtime-file>] \
  [--events-jsonl <path|->] \
  [--trace <path>] \
  [--checkpoint <path>] \
  [--profile standard|demo-scripted] \
  [--llm-script-jsonl <file>] \
  [--verbose] \
  [--verbose-llm] \
  [--approvals-mode safe|assist|yolo] \
  [--max-iterations <n>] \
  [--max-planner-rounds <n>] \
  [--max-tool-rounds <n>] \
  [--max-index-candidates <n>] \
  [--planner-context-token-budget <n>] \
  [--format text|json]
```

### `--profile` semantics

| Value | Behavior |
|-------|----------|
| `standard` (default) | Uses real LLM provider from `config.llm`; `--llm-script-jsonl` is forbidden |
| `demo-scripted` | Replays pre-recorded LLM responses from `--llm-script-jsonl` file (testing/demo use only); real LLM config is ignored |

## 2) Output formats and schemas

Global formatting:
- `--format text` (default)
- `--format json`

## `run plan`
- Dry-run `text`: `dry_run_text` rendering.
- Dry-run `json`: `dry_run_json` structure (from `ais-sdk`, no dedicated `ais-runner-*` schema id).
- Execute `json` schema: `ais-runner-run-plan/0.0.1`
  - `schema`
  - `status` (`completed|paused|stopped`)
  - `paused_reason` _(nullable string, open-ended — treat as opaque)_ — known prefixes include: `need_user_confirm:<node_id>`, `need_user_input:<node_id>`, `need_user_input:command`, `assert_failed:<node_id>`, `executor_error:<node_id>`, `hard_block:<node_id>`, `condition_failed:<node_id>`, `no_progress`, `cancelled_by_command`, `user_confirm_denied`, `until_failed:<node_id>`, `until_not_met:<node_id>`, `retry_exhausted:<node_id>`, `retry_timeout:<node_id>`, `replace_plan_rejected:<reason>`, `need_user_confirm:replace_plan`, `side_effect_reconcile_pending:<ids>`, `side_effect_reconcile_reverted:<ids>`, `missing_required_input`, `replay_step_limit`
  - `resumed_from_checkpoint`
  - `iterations`
  - `events_emitted`
  - `command_accepted` _(integer)_ — count of commands processed successfully
  - `command_rejected` _(integer)_ — count of commands rejected (unknown id, wrong type, etc.)
  - `completed_node_ids`

## `run workflow`
- Dry-run `json` schema: `ais-runner-run-workflow/0.0.1`
  - `schema`, `workflow`, `workspace`, `documents`, `plan`, `dry_run`, `issues`
- Execute mode output: same as `run plan` execution output.
- Optional outputs file schema (`--outputs`): `ais-runner-workflow-outputs/0.0.1`
  - `schema`
  - `outputs`

## `plan diff`
- `text`: `plan diff: added=... removed=... changed=...`
- `json`: structured diff payload from engine (`summary` plus node-level changes)

## `replay`
- `json` schema: `ais-runner-replay/0.0.1`
  - `schema`
  - `status` (`completed|paused|reached_until_node`)
  - `events_emitted`
  - `completed_node_ids`
  - `paused_reason` _(nullable string, open-ended — treat as opaque)_ — known prefixes include: `need_user_confirm:<node_id>`, `need_user_input:<node_id>`, `need_user_input:command`, `assert_failed:<node_id>`, `executor_error:<node_id>`, `hard_block:<node_id>`, `condition_failed:<node_id>`, `no_progress`, `cancelled_by_command`, `user_confirm_denied`, `until_failed:<node_id>`, `until_not_met:<node_id>`, `retry_exhausted:<node_id>`, `retry_timeout:<node_id>`, `replace_plan_rejected:<reason>`, `need_user_confirm:replace_plan`, `side_effect_reconcile_pending:<ids>`, `side_effect_reconcile_reverted:<ids>`, `missing_required_input`, `replay_step_limit`

## `agent`
- `json` schema: `ais-runner-agent/0.0.1`
  - `schema`
  - `status` (`completed|paused|stopped`)
  - `paused_reason` _(nullable string, open-ended — treat as opaque)_ — known prefixes include: `need_user_confirm:<node_id>`, `need_user_input:<node_id>`, `need_user_input:command`, `assert_failed:<node_id>`, `executor_error:<node_id>`, `hard_block:<node_id>`, `condition_failed:<node_id>`, `no_progress`, `cancelled_by_command`, `user_confirm_denied`, `until_failed:<node_id>`, `until_not_met:<node_id>`, `retry_exhausted:<node_id>`, `retry_timeout:<node_id>`, `replace_plan_rejected:<reason>`, `need_user_confirm:replace_plan`, `side_effect_reconcile_pending:<ids>`, `side_effect_reconcile_reverted:<ids>`, `missing_required_input`, `replay_step_limit`
  - `resumed_from_checkpoint`
  - `iterations`
  - `events_emitted`
  - `llm_usage`
  - `llm_usage` schema: `ais-agent-llm-usage/0.0.1` with fields: `calls`, `input_tokens`, `output_tokens`, `total_tokens`, `context_limit_tokens` (nullable)

## 3) JSONL boundary contracts

## `--events-jsonl`
Sample output line:
```json
{"schema":"ais-engine-event/0.0.3","run_id":"abc123","seq":0,"ts":"2026-03-05T00:00:00Z","event":{"type":"node_ready","node_id":"n1"}}
```

- Available on `run plan`, `run workflow`, `agent`.
- Output record schema: `ais-engine-event/0.0.3` (`EngineEventRecord`), one JSON object per line.
- `--events-jsonl -` behavior:
  - no text/json summary rendering
  - stdout is raw event JSONL stream

## `--trace`
- Available on `run plan`, `run workflow`, `agent`.
- Output is one redacted event JSON object per line (`encode_trace_jsonl_line` with default redaction).

## `--commands-stdin-jsonl`
**Available command types:**

| `type` | Purpose | Required `data` fields |
|--------|---------|----------------------|
| `user_confirm` | Approve or deny a node awaiting confirmation | `node_id`, `decision` (`"approve"` or `"deny"`) |
| `apply_patches` | Patch runtime values before next engine step | `patches: [{op, path, value}]` |
| `user_input` | Provide a value for a pending input request | `input_id`, `value` |
| `user_select` | Choose from a pending selection | `input_id`, `selected_index`, `options[]` |
| `cancel` | Cancel the run | _(empty data)_ |
| `replace_plan` | Swap the active plan mid-run | `plan` (full `ais-plan/0.0.3` object) |

Sample input lines:
```json
{"schema":"ais-engine-command/0.0.1","command":{"id":"cmd-1","type":"user_confirm","data":{"node_id":"n1","decision":"approve"}}}
{"schema":"ais-engine-command/0.0.1","command":{"id":"cmd-2","type":"user_confirm","data":{"node_id":"n2","decision":"deny"}}}
{"schema":"ais-engine-command/0.0.1","command":{"id":"cmd-3","type":"user_input","data":{"input_id":"recipient","value":"0xAbc..."}}}
{"schema":"ais-engine-command/0.0.1","command":{"id":"cmd-4","type":"cancel","data":{}}}
```

> ⚠️ Commands are read **once** before the engine loop starts (not streamed). Missing approvals cause a `paused` run requiring checkpoint resume.

- Available on `run plan`, `run workflow`.
- Input schema: `ais-engine-command/0.0.1` (`EngineCommandEnvelope`), one JSON object per line via stdin.
- Empty lines are ignored.
- Decode failure format: `commands stdin jsonl decode failed at line <n>: <reason>`.

## 4) Replay/checkpoint contracts

- Replay inputs are precedence by source (if both are provided, trace path runs first and checkpoint args are ignored):
- trace mode: `--trace-jsonl <file>`
- checkpoint mode: `--checkpoint <file> --plan <file> --config <file>`
- `--until-node <id>` can stop replay at a node boundary.

Checkpoint lifecycle:
- `run plan`/`run workflow` can write checkpoint via `--checkpoint`.
- Resume reads checkpoint state, approvals ledger, and side-effect ledger.
- On resume, pending side-effects may pause with:
- `side_effect_reconcile_pending:<node_ids>`
- `side_effect_reconcile_reverted:<node_ids>`

## 5) Error conventions

All errors are written to stderr. Exit code is non-zero on failure. Error messages below are stable CLI text that can be matched programmatically.

High-frequency user-facing errors (exact text from `RunnerError`):
- `runner config path is required for plan execution: pass --config <file>`
- `replay requires --trace-jsonl <file> or --checkpoint <file>`
- `replay from checkpoint requires --plan <file>`
- `replay from checkpoint requires --config <file>`
- `commands stdin jsonl decode failed at line <n>: <reason>`

Additional conventions:
- Parse/validation failures are emitted as `RunnerError` text variants (`plan parse failed`, `workflow parse failed`, `workflow validation failed`, etc.).
- File IO failures include path context (`read file failed`, `write file failed`, `checkpoint load/save failed`).
- JSON encode failures surface as `json encode failed: ...`.

## 6) Minimal usage examples

```bash
# plan dry-run JSON → returns ais-dry-run-report/0.0.1 object directly (not wrapped)
ais-runner run plan --plan ./plan.json --dry-run --format json

# plan execute with event/trace/checkpoint sinks
ais-runner run plan --plan ./plan.json --config ./runner.yaml \
  --events-jsonl ./events.jsonl --trace ./trace.jsonl --checkpoint ./checkpoint.json \
  --format json

# workflow execute and export outputs
ais-runner run workflow --workflow ./workflow.yaml --workspace ./workspace \
  --config ./runner.yaml --outputs ./workflow.outputs.json --format json

# replay from trace until a node
ais-runner replay --trace-jsonl ./trace.jsonl --until-node node-2 --format json
# Note: replace node-2 with the exact node id from your plan — this must be a precise match

# agent intent mode
ais-runner agent --intent "swap 1 ETH to USDC" --config ./runner.yaml \
  --workspace ./workspace --approvals-mode safe --format json
```
