pub mod checkpoint;
pub mod commands;
pub mod engine;
pub mod executor;
pub mod events;
pub mod plan_diff;
pub mod policy;
pub mod solver;
pub mod trace;

pub use checkpoint::{
    create_checkpoint_document, decode_checkpoint_json, encode_checkpoint_json,
    load_checkpoint_from_path, save_checkpoint_to_path, CheckpointDocument, CheckpointEngineState,
    CheckpointStoreError, CHECKPOINT_SCHEMA_0_0_1,
};
pub use commands::{
    apply_command_with_dedupe, decode_command_jsonl_line, encode_command_jsonl_line,
    CommandApplyResult, CommandDeduper, DuplicateCommandMode, EngineCommand, EngineCommandEnvelope,
    EngineCommandType, ENGINE_COMMAND_SCHEMA_0_0_1,
};
pub use engine::{
    apply_patches_from_command, run_plan_once, ApplyPatchesCommandError, ApplyPatchesExecution,
    EngineRunResult, EngineRunnerOptions, EngineRunnerState, EngineRunStatus, schedule_ready_nodes,
    ScheduleBatch, ScheduledNode, SchedulerOptions,
};
pub use executor::{
    Executor, ExecutorOutput, RouterExecuteError, RouterExecuteResult, RouterExecutor,
    RouterExecutorRegistration,
};
pub use events::{
    encode_event_jsonl_line, ensure_monotonic_sequence, parse_event_jsonl_line, EngineEvent,
    EngineEventRecord, EngineEventSequenceError, EngineEventStream, EngineEventType,
    ENGINE_EVENT_SCHEMA_0_0_3,
};
pub use plan_diff::{
    diff_plans_json, diff_plans_text, PlanChange, PlanDiffJson, PlanDiffNodeChanged,
    PlanDiffNodeIdentity, PlanDiffSummary,
};
pub use policy::{
    build_confirmation_summary, confirmation_hash, enforce_policy_gate,
    enrich_need_user_confirm_output, ConfirmationHashError, ConfirmationSummary,
    extract_policy_gate_input, PolicyEnforcementOptions, PolicyGateInput, PolicyGateOutput,
    PolicyPackAllowlist, PolicyThresholdRules,
};
pub use solver::{build_solver_event, DefaultSolver, Solver, SolverContext, SolverDecision};
pub use trace::{
    encode_trace_jsonl_line, redact_engine_event_record, redact_value, TraceEncodeError,
    replay_from_checkpoint, replay_trace_events, replay_trace_jsonl, ReplayError, ReplayOptions,
    ReplayResult, ReplayStatus, TraceRedactMode, TraceRedactOptions,
};
