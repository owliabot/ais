use super::super::input_normalize::normalize_input_slot_key;
use super::super::*;
use ais_engine::{
    DefaultSolver, EngineEventRecord, EngineRunStatus, EngineRunnerOptions, EngineRunnerState,
};
use ais_sdk::documents::PlanSketchSegment;
use serde_json::{Map, Value};
use std::collections::BTreeSet;

#[derive(Debug, Clone)]
pub(crate) struct ExecuteRoundOutcome {
    pub(crate) status: EngineRunStatus,
    pub(crate) iterations: usize,
    pub(crate) round_events: Vec<EngineEventRecord>,
    pub(crate) last_iteration_events: Vec<EngineEventRecord>,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn execute_round(
    command: &AgentCommand,
    run_id: &str,
    config: &RunnerConfig,
    engine_options: &EngineRunnerOptions,
    decision_policy: &mut dyn crate::agent::brain::DecisionPolicy,
    command_builder: &mut CommandBuilder,
    checkpoint_ledger: &mut RunnerCheckpointLedger,
    state: &mut EngineRunnerState,
    active_plan: &mut PlanDocument,
    active_plan_hash: &mut String,
    segment: &PlanSketchSegment,
    segment_plan: &PlanDocument,
    planning_memory: Option<Value>,
    fact_store: &mut InputStore,
    checkpoint_extensions: &checkpoint_ext::AgentCheckpointExtensions,
    total_events: &mut usize,
    todo_id: &str,
) -> Result<ExecuteRoundOutcome, RunnerError> {
    let replacement = super::super::merge_segment_plan(active_plan, segment_plan)?;
    let replace_reason = format!("segment:{}", segment.segment_id);
    let replace_command = super::super::build_replace_plan_command(
        command_builder,
        &replacement,
        Some(replace_reason.as_str()),
    )?;
    let processed = process_replace_plan_commands(
        run_id,
        config,
        &[replace_command],
        engine_options,
        state,
        active_plan,
        active_plan_hash,
    )?;
    let mut round_events = Vec::<EngineEventRecord>::new();
    if !processed.events.is_empty() {
        let annotated = annotate_events_with_todo(processed.events.as_slice(), segment, todo_id);
        *total_events = total_events.saturating_add(annotated.len());
        super::super::write_event_sinks(command, annotated.as_slice())?;
        checkpoint_ledger.absorb_events(annotated.as_slice());
        checkpoint_ledger.mark_approved_nodes(
            &state.approved_node_ids,
            wall_clock_timestamp_rfc3339().as_str(),
        );
        super::super::record_side_effect_lifecycle(&mut state.runtime, checkpoint_ledger);
        round_events.extend(annotated);
        super::super::checkpoint_flow::checkpoint_round(
            command,
            run_id,
            active_plan_hash,
            active_plan,
            state,
            checkpoint_ledger,
            planning_memory.clone(),
            fact_store,
            checkpoint_extensions,
        )?;
    }
    if !processed.plan_replaced {
        return Err(RunnerError::Llm(
            "replace_plan failed while applying segment".to_string(),
        ));
    }

    let router = build_router_executor_for_plan(active_plan, config)
        .map_err(RunnerError::ConfigInvalidForPlan)?;
    state.paused_reason = None;

    let max_iterations = command
        .max_iterations
        .unwrap_or_else(|| active_plan.nodes.len().saturating_mul(8).max(16));
    let loop_config = AgentLoopConfig { max_iterations };
    let mut last_iteration_events = Vec::<EngineEventRecord>::new();
    let loop_result = run_agent_loop(
        run_id,
        active_plan,
        state,
        &router,
        &DefaultSolver,
        engine_options,
        &loop_config,
        command_builder,
        decision_policy,
        |state, events| {
            let annotated = annotate_events_with_todo(events, segment, todo_id);
            *total_events = total_events.saturating_add(annotated.len());
            last_iteration_events = annotated.clone();
            round_events.extend(annotated.clone());
            super::super::write_event_sinks(command, annotated.as_slice())?;
            apply_segment_stores_from_runtime(segment, state, fact_store, command.verbose_llm);
            checkpoint_ledger.absorb_events(annotated.as_slice());
            checkpoint_ledger.mark_approved_nodes(
                &state.approved_node_ids,
                wall_clock_timestamp_rfc3339().as_str(),
            );
            super::super::checkpoint_flow::checkpoint_round(
                command,
                run_id,
                active_plan_hash,
                active_plan,
                state,
                checkpoint_ledger,
                planning_memory.clone(),
                fact_store,
                checkpoint_extensions,
            )?;
            Ok(())
        },
    )?;
    super::super::record_side_effect_lifecycle(&mut state.runtime, checkpoint_ledger);

    Ok(ExecuteRoundOutcome {
        status: loop_result.status,
        iterations: loop_result.iterations,
        round_events,
        last_iteration_events,
    })
}

pub(crate) fn bind_segment_todo_id(segment: &mut PlanSketchSegment, todo_id: &str) {
    segment
        .extensions
        .insert("todo_id".to_string(), Value::String(todo_id.to_string()));
}

pub(crate) fn annotate_events_with_todo(
    events: &[EngineEventRecord],
    segment: &PlanSketchSegment,
    todo_id: &str,
) -> Vec<EngineEventRecord> {
    events
        .iter()
        .cloned()
        .map(|mut record| {
            let agent = record
                .event
                .extensions
                .entry("agent".to_string())
                .or_insert_with(|| Value::Object(Map::new()));
            if !agent.is_object() {
                *agent = Value::Object(Map::new());
            }
            if let Some(agent_obj) = agent.as_object_mut() {
                agent_obj.insert("todo_id".to_string(), Value::String(todo_id.to_string()));
                agent_obj.insert(
                    "segment_id".to_string(),
                    Value::String(segment.segment_id.clone()),
                );
                if let Some(step_id) = step_id_for_segment_node(
                    record.event.node_id.as_deref(),
                    segment.segment_id.as_str(),
                ) {
                    agent_obj.insert("step_id".to_string(), Value::String(step_id.to_string()));
                }
            }
            record
        })
        .collect::<Vec<_>>()
}

fn step_id_for_segment_node<'a>(node_id: Option<&'a str>, segment_id: &str) -> Option<&'a str> {
    let node_id = node_id?;
    let prefix = format!("{segment_id}/");
    node_id
        .strip_prefix(prefix.as_str())
        .and_then(|value| (!value.trim().is_empty()).then_some(value))
}

pub(crate) fn build_todo_receipt(
    todo_id: &str,
    segment: &PlanSketchSegment,
    status: EngineRunStatus,
    state: &EngineRunnerState,
    round_events: &[EngineEventRecord],
) -> super::super::todos::TodoReceipt {
    let node_ids = segment
        .steps
        .iter()
        .map(|step| format!("{}/{}", segment.segment_id, step.id))
        .collect::<Vec<_>>();
    let completed_node_set = state
        .completed_node_ids
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let completed_node_ids = node_ids
        .iter()
        .filter(|node_id| completed_node_set.contains((*node_id).as_str()))
        .cloned()
        .collect::<Vec<_>>();
    let tx_hashes = collect_segment_tx_hashes(state, node_ids.as_slice());
    let event_types = round_events
        .iter()
        .map(event_type_name)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();

    super::super::todos::TodoReceipt {
        schema: "ais-agent-todo-receipt/0.0.1".to_string(),
        todo_id: todo_id.to_string(),
        segment_id: segment.segment_id.clone(),
        status: run_status_name(status).to_string(),
        paused_reason: state.paused_reason.clone(),
        node_ids,
        completed_node_ids,
        tx_hashes,
        event_types,
        event_count: round_events.len() as u64,
    }
}

pub(crate) fn run_status_name(status: EngineRunStatus) -> &'static str {
    match status {
        EngineRunStatus::Completed => "completed",
        EngineRunStatus::Paused => "paused",
        EngineRunStatus::Stopped => "stopped",
    }
}

fn event_type_name(record: &EngineEventRecord) -> String {
    serde_json::to_value(record.event.event_type)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_else(|| format!("{:?}", record.event.event_type).to_lowercase())
}

fn collect_segment_tx_hashes(state: &EngineRunnerState, node_ids: &[String]) -> Vec<String> {
    let mut tx_hashes = BTreeSet::<String>::new();
    for node_id in node_ids {
        let Some(outputs) = runtime_node_outputs(state, node_id.as_str()) else {
            continue;
        };
        collect_tx_hash_like_strings(outputs, &mut tx_hashes, 0);
    }
    tx_hashes.into_iter().collect::<Vec<_>>()
}

fn collect_tx_hash_like_strings(value: &Value, output: &mut BTreeSet<String>, depth: usize) {
    if depth > 8 {
        return;
    }
    match value {
        Value::Object(map) => {
            for key in ["tx_hash", "signed_tx_hash", "signature"] {
                if let Some(hash) = map.get(key).and_then(Value::as_str) {
                    let trimmed = hash.trim();
                    if !trimmed.is_empty() {
                        output.insert(trimmed.to_string());
                    }
                }
            }
            for nested in map.values() {
                collect_tx_hash_like_strings(nested, output, depth.saturating_add(1));
            }
        }
        Value::Array(items) => {
            for nested in items {
                collect_tx_hash_like_strings(nested, output, depth.saturating_add(1));
            }
        }
        _ => {}
    }
}

pub(crate) fn apply_segment_stores_from_runtime(
    segment: &PlanSketchSegment,
    state: &EngineRunnerState,
    fact_store: &mut InputStore,
    verbose_llm: bool,
) {
    for step in &segment.steps {
        if step.stores.is_empty() {
            continue;
        }
        let node_id = format!("{}/{}", segment.segment_id, step.id);
        let Some(node_outputs) = runtime_node_outputs(state, node_id.as_str()) else {
            continue;
        };
        for (return_field, slot_name) in &step.stores {
            let Some(value) = extract_store_value(node_outputs, return_field.as_str()) else {
                continue;
            };
            let provenance = format!("segment_store.{node_id}.{}", return_field.trim());
            if let Some(canonical_slot) = normalize_input_slot_key(slot_name) {
                let upsert_result = super::super::upsert_store_value_with_source(
                    fact_store,
                    canonical_slot.as_str(),
                    value.clone(),
                    super::super::input_store::InputValueLayer::Observed,
                    "query",
                    90,
                    provenance.clone(),
                );
                if verbose_llm {
                    eprintln!(
                        "[agent] stores mapped node={} field={} -> slot=inputs.{} upsert={:?}",
                        node_id, return_field, canonical_slot, upsert_result
                    );
                }
                continue;
            }
            let upsert_result = super::super::upsert_store_value_with_source(
                fact_store,
                slot_name,
                value.clone(),
                super::super::input_store::InputValueLayer::Observed,
                "query",
                90,
                provenance,
            );
            if verbose_llm {
                eprintln!(
                    "[agent] stores mapped node={} field={} -> slot={} upsert={:?}",
                    node_id, return_field, slot_name, upsert_result
                );
            }
        }
    }
}

fn runtime_node_outputs<'a>(state: &'a EngineRunnerState, node_id: &str) -> Option<&'a Value> {
    let escaped = node_id.replace('~', "~0").replace('/', "~1");
    state
        .runtime
        .pointer(format!("/nodes/{escaped}/outputs").as_str())
}

fn extract_store_value(node_outputs: &Value, field: &str) -> Option<Value> {
    let field = field.trim();
    if field.is_empty() {
        return None;
    }
    if let Some(value) = value_at_dot_path(node_outputs, field) {
        return Some(value.clone());
    }
    if let Some(outputs_value) = node_outputs.get("outputs") {
        if let Some(value) = value_at_dot_path(outputs_value, field) {
            return Some(value.clone());
        }
    }
    None
}

fn value_at_dot_path<'a>(value: &'a Value, dot_path: &str) -> Option<&'a Value> {
    let mut current = value;
    for segment in dot_path.split('.').filter(|item| !item.is_empty()) {
        current = current.get(segment)?;
    }
    Some(current)
}

#[cfg(test)]
#[path = "../tests/phase_machine/segment_exec.rs"]
mod tests;
