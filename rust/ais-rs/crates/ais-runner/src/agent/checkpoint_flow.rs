use super::runtime_facts_store::RuntimeFactsStore;
use super::*;
use ais_engine::events::wall_clock_timestamp_rfc3339;
use ais_engine::{EngineEvent, EngineEventRecord, EngineEventType, EngineRunnerState};
use serde_json::Value;

pub(super) struct CheckpointGuard<'a> {
    pub command: &'a AgentCommand,
    pub run_id: &'a str,
    pub active_plan_hash: &'a str,
    pub active_plan: &'a PlanDocument,
}

impl CheckpointGuard<'_> {
    pub fn save(
        &self,
        state: &EngineRunnerState,
        checkpoint_ledger: &RunnerCheckpointLedger,
        planning_memory: Option<Value>,
        input_store: &InputStore,
        runtime_facts_store: &RuntimeFactsStore,
        checkpoint_extensions: &checkpoint_ext::AgentCheckpointExtensions,
        audit_attempt: &crate::audit_contract::AuditStreamAttempt,
    ) -> Result<(), RunnerError> {
        checkpoint_round(
            self.command,
            self.run_id,
            self.active_plan_hash,
            self.active_plan,
            state,
            checkpoint_ledger,
            planning_memory,
            input_store,
            runtime_facts_store,
            checkpoint_extensions,
            audit_attempt,
        )
    }

    pub fn save_with_planning_failure(
        &self,
        state: &mut EngineRunnerState,
        checkpoint_ledger: &mut RunnerCheckpointLedger,
        planning_memory: Option<Value>,
        input_store: &InputStore,
        runtime_facts_store: &RuntimeFactsStore,
        checkpoint_extensions: &checkpoint_ext::AgentCheckpointExtensions,
        error: &RunnerError,
        round: u64,
        audit_attempt: &mut crate::audit_contract::AuditStreamAttempt,
    ) -> Result<(), RunnerError> {
        record_planning_failure_event_and_checkpoint(
            self.command,
            self.run_id,
            self.active_plan_hash,
            self.active_plan,
            state,
            checkpoint_ledger,
            planning_memory,
            input_store,
            runtime_facts_store,
            checkpoint_extensions,
            error,
            round,
            audit_attempt,
        )
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn record_planning_failure_event_and_checkpoint(
    command: &AgentCommand,
    run_id: &str,
    active_plan_hash: &str,
    active_plan: &PlanDocument,
    state: &mut EngineRunnerState,
    checkpoint_ledger: &mut RunnerCheckpointLedger,
    planning_memory: Option<Value>,
    input_store: &InputStore,
    runtime_facts_store: &RuntimeFactsStore,
    checkpoint_extensions: &checkpoint_ext::AgentCheckpointExtensions,
    error: &RunnerError,
    round: u64,
    audit_attempt: &mut crate::audit_contract::AuditStreamAttempt,
) -> Result<(), RunnerError> {
    let mut event = EngineEvent::new(EngineEventType::Error);
    event.data.insert(
        "reason".to_string(),
        Value::String("planner_round_failed".to_string()),
    );
    event.data.insert(
        "reason_code".to_string(),
        Value::String("planner_round_failed".to_string()),
    );
    event
        .data
        .insert("error".to_string(), Value::String(error.to_string()));
    event
        .data
        .insert("round".to_string(), Value::Number(round.into()));
    let record = EngineEventRecord::new(
        run_id.to_string(),
        state.next_seq,
        wall_clock_timestamp_rfc3339(),
        event,
    );
    state.next_seq = state.next_seq.saturating_add(1);
    super::write_event_sinks(command, std::slice::from_ref(&record), audit_attempt)?;
    checkpoint_ledger.absorb_events(std::slice::from_ref(&record));
    checkpoint_round(
        command,
        run_id,
        active_plan_hash,
        active_plan,
        state,
        checkpoint_ledger,
        planning_memory,
        input_store,
        runtime_facts_store,
        checkpoint_extensions,
        audit_attempt,
    )?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn checkpoint_round(
    command: &AgentCommand,
    run_id: &str,
    active_plan_hash: &str,
    active_plan: &PlanDocument,
    state: &EngineRunnerState,
    checkpoint_ledger: &RunnerCheckpointLedger,
    planning_memory: Option<Value>,
    input_store: &InputStore,
    runtime_facts_store: &RuntimeFactsStore,
    checkpoint_extensions: &checkpoint_ext::AgentCheckpointExtensions,
    audit_attempt: &crate::audit_contract::AuditStreamAttempt,
) -> Result<(), RunnerError> {
    let checkpoint_view = checkpoint_view::CheckpointView::from_state(state, checkpoint_ledger);
    super::maybe_save_checkpoint(
        command,
        run_id,
        active_plan_hash,
        active_plan,
        state,
        checkpoint_view.runtime(),
        checkpoint_ledger,
        // Input checkpoint payload is emitted from InputStore directly.
        Some(checkpoint_extensions.encode_updated_with_runtime_facts(
            planning_memory,
            input_store,
            runtime_facts_store,
        )),
        audit_attempt,
    )
}
