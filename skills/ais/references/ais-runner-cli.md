# `ais-runner` CLI Reference

Primary sources:
- `/home/ocbot/.openclaw/workspace/repos/ais/docs/ais-rust-cli.md`
- `/home/ocbot/.openclaw/workspace/repos/ais/rust/ais-rs/crates/ais-runner/src/cli.rs`
- `/home/ocbot/.openclaw/workspace/repos/ais/rust/ais-rs/crates/ais-runner/src/run.rs`
- `/home/ocbot/.openclaw/workspace/repos/ais/rust/ais-rs/crates/ais-runner/src/agent/mod.rs`
- `/home/ocbot/.openclaw/workspace/repos/ais/rust/ais-rs/crates/ais-runner/src/error.rs`

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

Notes:
- `--config` is optional only for `--dry-run`; required for execution mode.
- `--events-jsonl -` sends raw event JSONL to stdout and suppresses summary output.

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
  - `paused_reason`
  - `resumed_from_checkpoint`
  - `iterations`
  - `events_emitted`
  - `command_accepted`
  - `command_rejected`
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
  - `paused_reason`

## `agent`
- `json` schema: `ais-runner-agent/0.0.1`
  - `schema`
  - `status` (`completed|paused|stopped`)
  - `paused_reason`
  - `resumed_from_checkpoint`
  - `iterations`
  - `events_emitted`
  - `llm_usage`

## 3) JSONL boundary contracts

## `--events-jsonl`
- Available on `run plan`, `run workflow`, `agent`.
- Output record schema: `ais-engine-event/0.0.3` (`EngineEventRecord`), one JSON object per line.
- `--events-jsonl -` behavior:
  - no text/json summary rendering
  - stdout is raw event JSONL stream

## `--trace`
- Available on `run plan`, `run workflow`, `agent`.
- Output is one redacted event JSON object per line (`encode_trace_jsonl_line` with default redaction).

## `--commands-stdin-jsonl`
- Available on `run plan`, `run workflow`.
- Input schema: `ais-engine-command/0.0.1` (`EngineCommandEnvelope`), one JSON object per line via stdin.
- Empty lines are ignored.
- Decode failure format: `commands stdin jsonl decode failed at line <n>: <reason>`.

## 4) Replay/checkpoint contracts

- Replay inputs are mutually exclusive by source:
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
# plan dry-run JSON
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

# agent intent mode
ais-runner agent --intent "swap 1 ETH to USDC" --config ./runner.yaml \
  --workspace ./workspace --approvals-mode safe --format json
```
