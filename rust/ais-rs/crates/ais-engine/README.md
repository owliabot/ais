# `ais-engine`

Engine runtime primitives for AIS execution loop.

## Responsibility

- Define engine event envelope types (`ais-engine-event/0.0.3`)
- Provide JSONL encode/decode helpers for event streaming
- Provide sequence utilities to enforce monotonic `seq`
- Provide trace JSONL encoding with redact hook (`default|audit|off`)
- Provide checkpoint format + file store (recoverable engine state)
- Provide command JSONL envelope + idempotent dedupe hook
- Provide guarded runtime patch application + patch audit events
- Provide executor trait + exact chain router
- Provide policy gate extract/enforce (allowlist + thresholds)
- Provide confirmation summary/hash for `need_user_confirm`
- Provide plan-first execution loop (`readiness -> solver -> policy gate -> materialize -> executor`)
- Provide workflow condition/assert + preflight.simulate semantics in execution loop
- Provide deterministic scheduler with global/per-chain limits
- Provide plan diff output (`text/json`)
- Provide replay helpers (`trace` playback + `checkpoint` resume)

## Public entry points

- `EngineEventType`
- `EngineEvent`
- `EngineEventRecord`
- `EngineEventStream`
- `EngineEventSequenceError`
- `ensure_monotonic_sequence`
- `encode_event_jsonl_line`
- `parse_event_jsonl_line`
- `TraceRedactMode`
- `TraceRedactOptions`
- `redact_engine_event_record`
- `redact_value`
- `encode_trace_jsonl_line`
- `CheckpointDocument`
- `CheckpointEngineState`
- `create_checkpoint_document`
- `encode_checkpoint_json`
- `decode_checkpoint_json`
- `save_checkpoint_to_path`
- `load_checkpoint_from_path`
- `EngineCommandType`
- `EngineCommand`
- `EngineCommandEnvelope`
- `encode_command_jsonl_line`
- `decode_command_jsonl_line`
- `DuplicateCommandMode`
- `CommandDeduper`
- `apply_command_with_dedupe`
- `apply_patches_from_command`
- `ApplyPatchesExecution`
- `ApplyPatchesCommandError`
- `Solver` / `DefaultSolver`
- `SolverContext`
- `SolverDecision`
- `build_solver_event`
- `Executor`
- `ExecutorOutput`
- `RouterExecutor`
- `RouterExecutorRegistration`
- `RouterExecuteResult`
- `RouterExecuteError`
- `PolicyGateInput`
- `PolicyGateOutput`
- `PolicyPackAllowlist`
- `PolicyThresholdRules`
- `PolicyEnforcementOptions`
- `extract_policy_gate_input`
- `enforce_policy_gate`
- `ConfirmationSummary`
- `build_confirmation_summary`
- `confirmation_hash`
- `enrich_need_user_confirm_output`
- `EngineRunnerState`
- `EngineRunnerOptions`
- `EngineRunStatus`
- `EngineRunResult`
- `run_plan_once`
- `SchedulerOptions`
- `ScheduledNode`
- `ScheduleBatch`
- `schedule_ready_nodes`
- `PlanDiffSummary`
- `PlanDiffNodeIdentity`
- `PlanDiffNodeChanged`
- `PlanDiffJson`
- `PlanChange`
- `diff_plans_json`
- `diff_plans_text`
- `ReplayStatus`
- `ReplayOptions`
- `ReplayResult`
- `ReplayError`
- `replay_trace_events`
- `replay_trace_jsonl`
- `replay_from_checkpoint`

## Dependencies

- `ais-core`: runtime patch model/apply/guard policy + audit hash
- `ais-sdk`: readiness model + ValueRef evaluation for pre-executor execution materialization
- `serde`, `serde_json`: JSON schema-compatible serialization
- `thiserror`: typed sequence validation error

## Test fixtures

- Fixture-backed tests consume `rust/ais-rs/fixtures/plan-events` for plan diff, replay, checkpoint, and redaction regression coverage.

## Current status

- Implemented:
  - `AISRS-ENG-001` (EngineEvent types + JSONL envelope + seq monotonic checks)
  - `AISRS-ENG-002` (trace JSONL + redaction hook + allow_path_patterns)
  - `AISRS-ENG-003` (checkpoint format + store + redacted runtime snapshot compatibility)
  - `AISRS-ENG-004` (engine command stdin JSONL + command id dedupe + accepted/rejected events)
  - `AISRS-ENG-005` (runtime patch apply with forced guard + patch_applied/patch_rejected audit events)
  - `AISRS-ENG-006` (solver trait + default solver: auto contracts / need_user_confirm / select_provider)
  - `AISRS-ENG-007` (executor trait + exact chain router with mismatch rejection)
  - `AISRS-ENG-008` (policy gate extract + enforce with missing/unknown semantics, allowlist, thresholds)
  - `AISRS-ENG-009` (confirmation_summary + confirmation_hash stable over summary ignoring timestamps)
  - `AISRS-ENG-010` (plan-first runner loop with command apply, policy gate before execution, and engine_paused on no progress)
  - `AISRS-ENG-011` (deterministic scheduler with reads parallel and writes per-chain serial by default)
  - `AISRS-ENG-020` (plan diff text/json with added/removed/changed and key-field change detection)
  - `AISRS-ENG-021` (replay from trace/checkpoint with until-node stopping behavior)
  - `AISRS-ENG-022` (workflow assert fail-fast + pause/stop strategy + preflight.simulate execution path)
  - `AISRS-ENG-023` (workflow condition pre-check semantics; false => skipped; invalid => paused)
  - `AISRS-ENG-024` (workflow until/retry semantics; until false enters retry loop with max-attempt guard)
  - `AISRS-ENG-025` (workflow timeout_ms semantics; retry lifecycle timeout produces deterministic pause reason/events)
  - Engine runner now materializes node execution ValueRef (including `bindings.params` root override) before dispatching to chain executors, keeping executor layer transport-focused.
  - For query nodes (identified by `type=query_ref` or `source.query`), default write path `nodes.<id>.outputs` projects `executor_result.outputs` when present, so workflow expressions can consistently use `nodes.<id>.outputs.<field>`.
  - `assert_failed` engine error events now include `message`, `phase`, and original `assert` payload to support runtime troubleshooting in runner verbose logs.
