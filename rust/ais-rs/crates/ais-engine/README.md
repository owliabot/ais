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
- Provide policy gate extract/enforce (allowlist + risk-threshold + missingness)
- Provide confirmation summary/hash for `need_user_confirm` and emit schema-compatible confirmation details
- Provide plan-first execution loop (`readiness -> solver -> policy gate -> materialize -> executor`)
- Provide workflow condition/assert + preflight.simulate semantics in execution loop
- Provide deterministic scheduler with global/per-chain limits
- Provide plan diff output (`text/json`)
- Provide replay helpers (`trace` playback + `checkpoint` resume)
- Define executor output contract (`result/writes/side_effects`) and route execution by chain + execution.type
- Define executor-side side-effect reconciliation contract (router dispatch by `side_effect.{chain,execution_type}`)
- Emit `side_effect_observed` events from normalized executor-provided side-effect records
- Provide unified execution-type capabilities registry (`core/plugin`, write semantics, route presets)

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
- `CheckpointApprovalLedgerEntry`
- `CheckpointSideEffectRecord`
- `SIDE_EFFECT_RECORD_SCHEMA_0_1_0`
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
- `ExecutionHandlerKind`
- `RouterExecuteResult`
- `RouterExecuteError`
- `RouterReconcileResult`
- `RouterReconcileError`
- `ExecutionTypeKind`
- `ExecutionTypeRoutePreset`
- `ExecutionTypeCapabilities`
- `execution_type_capabilities`
- `execution_type_kind`
- `is_core_execution_type`
- `is_write_execution_type`
- `execution_types_for_route_preset`
- `PluginExecutionTypeCapabilities`
- `RuntimeExecutionTypeRegistry`
- `PolicyGateInput`
- `PolicyGateOutput`
- `PolicyGateReasonCode`
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
- `EngineSafetyOptions`
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
  - checkpoint contract now includes `approvals_ledger` + `side_effects` ledgers (tx/approval persistence for idempotent resume); decode path performs deterministic dedup normalization.
  - checkpoint document now exposes `extensions` (opaque map) for runner-level resume metadata (for example segmented planner memory snapshots), while core engine replay semantics remain driven by typed checkpoint fields.
  - `AISNEXT-ARCH-002` (SideEffect contract alignment): introduces normalized side-effect record contract used by engine events/checkpoint and executor integrations.
  - side-effect status is canonicalized at engine/checkpoint boundary to `prepared|sent|confirmed|reverted|unknown` (legacy `failed` inputs are normalized to `reverted`).
  - `AISNEXT-ARCH-003` (execution type capability registry): centralizes execution-type semantics (`is_write`, `core/plugin`, route presets, side-effect-adapter support metadata), and supports runtime plugin-type registration (for example `offchain_apy_query`) instead of hardcoding plugin types into presets.
  - `AISNEXT-ARCH-004` (event-driven side-effect ledger source): engine execution loop now emits `side_effect_observed` events from executor-provided side-effect records, with required fields normalized before emission.
  - `AISNEXT-ARCH-005` (compat hard-delete): checkpoint side-effect ledger dedup no longer derives fallback keys from `tx_hash`; records missing `idempotency_key` are dropped during checkpoint decode normalization.
  - side-effect producer boundary is now executor-first: engine consumes `ExecutorOutput.side_effects` directly and emits `side_effect_observed`; it no longer infers side-effects from executor `result` payloads.
  - `AISRS-ENG-004` (engine command stdin JSONL + command id dedupe + accepted/rejected events)
  - `AISRS-FT-012` (input-interaction command/event protocol): adds `user_input` / `user_select` commands and `need_user_input` event for structured runtime input补参与错误回执。
  - `AISRS-ENG-005` (runtime patch apply with forced guard + patch_applied/patch_rejected audit events)
  - `AISRS-ENG-006` (solver trait + default solver: auto contracts / need_user_confirm)
  - blocked readiness due unresolved `missing_refs` now pauses as `need_user_input` (`reason_code=missing_required_input`) instead of `need_user_confirm`, so runtime补参与风险确认语义分离。
  - `AISRS-ENG-007` (executor trait + exact chain router with mismatch rejection)
  - executor router now uses explicit `chain + execution.type` registration with `core|plugin` handler kind, and rejects unregistered execution types with a stable `UnregisteredExecutionType` error.
  - executor/router now expose side-effect reconciliation routing (`reconcile_side_effect`) keyed by side-effect `chain + execution_type`, so resume reconciliation can be delegated to chain executors.
  - `AISRS-ENG-008` (policy gate extract + enforce with missing/unknown semantics, allowlist, risk-threshold)
  - policy gate缺参类确认（`missing_fields`/`unknown_fields`）在执行环统一路由为 `need_user_input`（`reason_code=missing_required_input`），并标准化输出 `details.missing_refs/suggested_paths/questions`，将补参与风险确认分离。
  - policy gate missing/unknown extraction is driven by `node.extensions.policy.{param_roles,required_fields}` + canonical `action_ref` (no swap/approve method-name heuristics or param alias fallback).
  - policy gate now consumes `node.extensions.policy.constraint_templates` and enforces built-in template decisions (`max_spend`, `max_slippage_bps`, `disallow_unlimited_approval`) before allowlist/threshold checks, with stable reason codes for violation/unknown/invalid params.
  - policy gate threshold contract is slimmed to approval-risk boundary (`max_risk_level`) and no longer includes pack-level fixed amount/slippage/unlimited-approval threshold compatibility fields.
  - plugin allowlist gating treats only `evm_read|evm_call|solana_read|solana_instruction` as core; non-core execution types (for example `evm_rpc`) are evaluated under plugin allowlist rules.
  - pack-style plugin execution allowlisting is supported via `PolicyEnforcementOptions.enforce_plugin_execution_allowlist` (unlisted plugin types hard-block when enabled).
  - `AISRS-ENG-009` (confirmation_summary + confirmation_hash stable over summary ignoring timestamps)
  - `AISSLIM-RS-004` (reason_code stabilization): policy gate outputs now emit stable `reason_code`; engine paused/error/confirm events carry `event.data.reason_code`; confirmation summary binds `reason_code` for deterministic automation.
  - `AISSLIM-TEST-001` (资金安全核心路径矩阵): engine tests now cover `need_user_confirm` approve/deny branches and verify `hard_block` cannot be bypassed by user confirmation commands.
  - `AISNEXT-TEST-003` (policy gate confirm UX + 模板约束): engine tests now cover threshold-driven `need_user_confirm` confirmation summary payload (`reason_code/risk_level/hit_reasons`) and template-required-fields hard-block behavior (`hard_block_on_missing=true` + stable `missing_fields` details).
  - `AISRS-ENG-010` (plan-first runner loop with command apply, policy gate before execution, and engine_paused on no progress)
  - `AISNEXT-RS-005` (safety governance chain): execution loop now includes before-execute safety hook (`blocked_execution_types`), executor-output sanitization (sensitive key redaction + bounded string length), and prompt-injection hard-block for suspicious executor payloads.
  - `AISRS-CMD-001` (`replace_plan` command/event surface): engine command type includes `replace_plan`; event type includes `plan_replaced`; checkpoint state tracks `plan_epoch` and `plan_hash_history`, with optional `plan_snapshot` for deterministic resume after plan replacement.
  - `need_user_confirm` events now always include required `details` fields (`node_id`, `action_ref`, `hit_reasons`, `confirmation_summary`, `confirmation_hash`) for both solver and policy-gate pauses (schema-aligned for agents/runners).
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
