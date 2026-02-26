pub mod checkpoint;
pub mod commands;
pub mod engine;
pub mod events;
pub mod execution_type;
pub mod executor;
pub mod plan_diff;
pub mod policy;
pub mod solver;
pub mod trace;

pub use checkpoint::{
    canonical_side_effect_status, create_checkpoint_document, decode_checkpoint_json,
    encode_checkpoint_json, is_pending_side_effect_status, is_terminal_side_effect_status,
    load_checkpoint_from_path, save_checkpoint_to_path, CheckpointApprovalLedgerEntry,
    CheckpointDocument, CheckpointEngineState, CheckpointSideEffectRecord, CheckpointStoreError,
    CHECKPOINT_SCHEMA_0_0_1, SIDE_EFFECT_RECORD_SCHEMA_0_1_0, SIDE_EFFECT_STATUS_CONFIRMED,
    SIDE_EFFECT_STATUS_PREPARED, SIDE_EFFECT_STATUS_REVERTED, SIDE_EFFECT_STATUS_SENT,
    SIDE_EFFECT_STATUS_UNKNOWN,
};
pub use commands::{
    apply_command_with_dedupe, decode_command_jsonl_line, encode_command_jsonl_line,
    CommandApplyResult, CommandDeduper, DuplicateCommandMode, EngineCommand, EngineCommandEnvelope,
    EngineCommandType, ENGINE_COMMAND_SCHEMA_0_0_1,
};
pub use engine::{
    apply_patches_from_command, run_plan_once, schedule_ready_nodes, ApplyPatchesCommandError,
    ApplyPatchesExecution, EngineRunResult, EngineRunStatus, EngineRunnerOptions,
    EngineRunnerState, EngineSafetyOptions, ScheduleBatch, ScheduledNode, SchedulerOptions,
};
pub use events::{
    encode_event_jsonl_line, ensure_monotonic_sequence, parse_event_jsonl_line, EngineEvent,
    EngineEventRecord, EngineEventSequenceError, EngineEventStream, EngineEventType,
    ENGINE_EVENT_SCHEMA_0_0_3,
};
pub use execution_type::{
    execution_type_capabilities, execution_type_kind, execution_types_for_route_preset,
    is_core_execution_type, is_write_execution_type, ExecutionTypeCapabilities, ExecutionTypeKind,
    ExecutionTypeRoutePreset, PluginExecutionTypeCapabilities, RuntimeExecutionTypeRegistry,
};
pub use executor::{
    Executor, ExecutorOutput, RouterExecuteError, RouterExecuteResult, RouterExecutor,
    RouterExecutorRegistration, RouterReconcileError, RouterReconcileResult,
};
pub use plan_diff::{
    diff_plans_json, diff_plans_text, PlanChange, PlanDiffJson, PlanDiffNodeChanged,
    PlanDiffNodeIdentity, PlanDiffSummary,
};
pub use policy::{
    build_confirmation_summary, confirmation_hash, enforce_policy_gate,
    enrich_need_user_confirm_output, extract_policy_gate_input, ConfirmationHashError,
    ConfirmationSummary, PolicyConstraintTemplateRef, PolicyEnforcementOptions, PolicyGateInput,
    PolicyGateOutput, PolicyGateReasonCode, PolicyPackAllowlist, PolicyThresholdRules,
};
pub use solver::{build_solver_event, DefaultSolver, Solver, SolverContext, SolverDecision};
pub use trace::{
    encode_trace_jsonl_line, redact_engine_event_record, redact_value, replay_from_checkpoint,
    replay_trace_events, replay_trace_jsonl, ReplayError, ReplayOptions, ReplayResult,
    ReplayStatus, TraceEncodeError, TraceRedactMode, TraceRedactOptions,
};
