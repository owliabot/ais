# `ais-runner`

CLI wrapper for AIS SDK/engine workflows.

## Responsibility

- Provide CLI command skeleton for `run plan`, `run workflow`, `plan diff`, `replay`
- Implement dry-run output (`text` default, `json` optional)
- Load plan/runtime/workspace files and delegate to `ais-sdk` parse/validate/planner APIs
- For `run workflow`, merge `workflow.inputs.*.default` into runtime `inputs` when missing (runtime explicit values take precedence)
- Build runner chain config (`ais-runner/0.0.1`) and assemble exact-chain router executors
- Bridge engine run statuses (`completed|paused|stopped`) to CLI output/rendering

## Public entry points

- Binary: `ais-runner`
- Config APIs:
  - `load_runner_config(path)`
  - `validate_runner_config(config)`
  - `build_router_executor(config)`
  - `build_router_executor_for_plan(plan, config)`
- Commands:
  - `ais-runner run plan --plan <file> [--config <runner-config>] [--runtime <file>] [--dry-run] [--events-jsonl <path|->] [--trace <path>] [--checkpoint <path>] [--commands-stdin-jsonl] [--verbose] [--format text|json]`
  - `ais-runner run workflow --workflow <file> [--workspace <dir>] [--config <runner-config>] [--runtime <file>] [--dry-run] [--events-jsonl <path|->] [--trace <path>] [--checkpoint <path>] [--outputs <json-file>] [--commands-stdin-jsonl] [--verbose] [--format text|json]`
  - `ais-runner plan diff --before <plan> --after <plan> [--format text|json]`
  - `ais-runner replay [--trace-jsonl <file>] [--checkpoint <file> --plan <plan> --config <runner-config>] [--until-node <id>] [--format text|json]`

## Dependencies

- `ais-sdk`: parse + dry-run planner APIs
- `clap`: CLI parsing
- `serde_json`, `serde_yaml`: runtime file decoding
- `thiserror`: CLI/domain errors

## Current status

- Implemented:
  - `AISRS-RUN-001` (CLI 命令骨架 + `--help` smoke test)
  - `AISRS-RUN-002` (workspace 目录加载与分类：protocol/pack/workflow/plan，含 issues 输出)
  - `AISRS-RUN-003` (runner config 解析/校验 + EVM/Solana executor 装配 + plan chain 缺失校验)
  - `AISRS-RUN-010` (run plan dry-run text/json, includes `main.rs` CLI dispatch and `run_test.rs`)
  - `AISRS-RUN-011` (run plan execute loop + events-jsonl sink + trace sink + checkpoint save/restore)
  - `AISRS-RUN-012` (optional stdin JSONL command ingestion, supports apply_patches/user_confirm/cancel, emits command accepted/rejected events)
  - `AISRS-RUN-020` (plan diff text/json path wired to engine diff)
  - `AISRS-RUN-021` (replay trace/checkpoint path with until-node, text/json output)
  - `AISRS-RUN-022` (run workflow 0.0.3 mode: workspace+workflow validation, compile_workflow, dry-run or execute via engine)
  - workflow execute mode can evaluate top-level `workflow.outputs` against final runtime and write them to a dedicated JSON file via `--outputs`.
  - runner `rpc_url` validation accepts `http(s)` and `ws(s)` for chain endpoints.
  - chain `timeout_ms` now maps to EVM RPC client request timeout middleware.
  - Workspace loader tests keep schema-valid protocol fixtures to match strict parser+schema validation behavior
  - `--verbose` runtime event printing for `run plan` / `run workflow` (stderr event lines for easier trace/debug); `error` events additionally print full event `data` JSON for assert/condition/executor diagnostics.
  - Minor cleanup: simplified parser error mappers and state init branches to reduce boilerplate and clippy noise.
- Runner delegates EVM read/call/rpc transport to `ais-evm-executor` Alloy-backed sender adapters (no local duplicate EVM transport implementation in `ais-runner`)
- Planned next:
  - wire real Solana RPC client factory into `config` executor assembly path
  - runner integration polish and fixtures coverage
