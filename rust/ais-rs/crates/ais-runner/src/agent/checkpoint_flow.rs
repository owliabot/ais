use super::*;
use ais_engine::events::wall_clock_timestamp_rfc3339;
use ais_engine::{EngineEvent, EngineEventRecord, EngineEventType, EngineRunnerState};
use serde_json::Value;
use std::collections::BTreeMap;

#[allow(clippy::too_many_arguments)]
pub(super) fn record_planning_failure_event_and_checkpoint(
    command: &AgentCommand,
    run_id: &str,
    active_plan_hash: &str,
    active_plan: &PlanDocument,
    state: &mut EngineRunnerState,
    checkpoint_ledger: &RunnerCheckpointLedger,
    planning_memory: Option<Value>,
    input_store: &InputStore,
    checkpoint_extensions: &checkpoint_ext::AgentCheckpointExtensions,
    error: &RunnerError,
    round: u64,
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
    super::write_event_sinks(command, std::slice::from_ref(&record))?;
    checkpoint_round(
        command,
        run_id,
        active_plan_hash,
        active_plan,
        state,
        checkpoint_ledger,
        planning_memory,
        input_store,
        checkpoint_extensions,
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
    checkpoint_extensions: &checkpoint_ext::AgentCheckpointExtensions,
) -> Result<(), RunnerError> {
    let intent_facts = runtime_intent_facts(&state.runtime);
    super::maybe_save_checkpoint(
        command,
        run_id,
        active_plan_hash,
        active_plan,
        state,
        checkpoint_ledger,
        // Input checkpoint payload is emitted from InputStore directly.
        Some(checkpoint_extensions.encode_updated(
            planning_memory,
            input_store,
            state.runtime.pointer("/agent/todo_progress"),
            intent_facts.as_ref(),
        )),
    )
}

fn runtime_intent_facts(runtime: &Value) -> Option<BTreeMap<String, Value>> {
    runtime
        .pointer("/agent/intent_grounding/intent_facts")
        .and_then(Value::as_object)
        .map(|facts| {
            facts
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect::<BTreeMap<String, Value>>()
        })
}
