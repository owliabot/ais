use ais_core::{stable_hash_hex, StableJsonOptions};
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

#[derive(Debug, Clone)]
struct InputStoreSyncReport {
    synced_refs: Vec<String>,
    hash_changed: bool,
    previous_hash: Option<String>,
    current_hash: Option<String>,
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
    let sync_report = sync_runtime_inputs_from_input_store(&mut state.runtime, fact_store);
    if command.verbose || command.verbose_llm {
        super::super::trace::emit(
            true,
            "execute_round",
            "input_store_sync_applied",
            &[
                ("segment_id", segment.segment_id.clone()),
                ("synced_count", sync_report.synced_refs.len().to_string()),
                ("hash_changed", sync_report.hash_changed.to_string()),
                (
                    "previous_hash",
                    sync_report
                        .previous_hash
                        .unwrap_or_else(|| "-".to_string()),
                ),
                (
                    "current_hash",
                    sync_report.current_hash.unwrap_or_else(|| "-".to_string()),
                ),
            ],
        );
    }
    let replacement = super::super::merge_segment_plan(active_plan, segment_plan)?;
    let replace_reason = format!("segment:{}", segment.segment_id);
    let replace_command = super::super::build_replace_plan_command(
        command_builder,
        &replacement,
        Some(replace_reason.as_str()),
    )?;
    let mut processed = process_replace_plan_commands(
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
    if !processed.plan_replaced
        && has_duplicate_command_id_rejection(processed.events.as_slice())
    {
        super::super::trace::emit(
            command.verbose || command.verbose_llm,
            "execute_round",
            "command_id_repair_attempt",
            &[
                ("segment_id", segment.segment_id.clone()),
                ("todo_id", todo_id.to_string()),
            ],
        );
        let retry_command = super::super::build_replace_plan_command(
            command_builder,
            &replacement,
            Some(replace_reason.as_str()),
        )?;
        let retry_processed = process_replace_plan_commands(
            run_id,
            config,
            &[retry_command],
            engine_options,
            state,
            active_plan,
            active_plan_hash,
        )?;
        if !retry_processed.events.is_empty() {
            let annotated = annotate_events_with_todo(retry_processed.events.as_slice(), segment, todo_id);
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
        processed = retry_processed;
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

fn has_duplicate_command_id_rejection(events: &[EngineEventRecord]) -> bool {
    events.iter().any(|record| {
        record.event.event_type == ais_engine::EngineEventType::CommandRejected
            && record
                .event
                .data
                .get("reason_code")
                .and_then(Value::as_str)
                .is_some_and(|reason| reason == "duplicate_command_id")
    })
}

fn sync_runtime_inputs_from_input_store(
    runtime: &mut Value,
    fact_store: &InputStore,
) -> InputStoreSyncReport {
    let previous_hash = stable_inputs_hash(runtime.pointer("/inputs"));
    if !runtime.is_object() {
        *runtime = Value::Object(Map::new());
    }
    if let Some(root) = runtime.as_object_mut() {
        root.insert("inputs".to_string(), Value::Object(Map::new()));
    }
    let mut synced_refs = Vec::<String>::new();
    for slot in fact_store.list_ref_strings() {
        let Some(value) = fact_store.get(slot.as_str()).map(|entry| entry.value.clone()) else {
            continue;
        };
        super::super::input_normalize::set_runtime_input_value(runtime, slot.as_str(), value);
        synced_refs.push(format!("inputs.{slot}"));
    }
    let current_hash = stable_inputs_hash(runtime.pointer("/inputs"));
    InputStoreSyncReport {
        synced_refs,
        hash_changed: previous_hash != current_hash,
        previous_hash,
        current_hash,
    }
}

fn stable_inputs_hash(value: Option<&Value>) -> Option<String> {
    stable_hash_hex(value?, &StableJsonOptions::default()).ok()
}

pub(crate) fn collect_segment_input_ref_closure(segment: &PlanSketchSegment) -> Vec<String> {
    let mut refs = BTreeSet::<String>::new();
    for step in &segment.steps {
        collect_input_refs_from_value(&Value::Object(step.inputs.clone()), &mut refs);
        if let Some(when) = step.when.as_ref() {
            collect_input_refs_from_text(when.cel.as_str(), &mut refs);
        }
        if let Some(until) = step.until.as_ref() {
            collect_input_refs_from_value(until, &mut refs);
        }
        for template in &step.constraint_templates {
            collect_input_refs_from_value(&Value::Object(template.params.clone()), &mut refs);
        }
    }
    refs.into_iter().collect::<Vec<_>>()
}

pub(crate) fn collect_segment_missing_input_refs(
    segment: &PlanSketchSegment,
    fact_store: &InputStore,
) -> Vec<String> {
    collect_segment_input_ref_closure(segment)
        .into_iter()
        .filter(|reference| {
            let Some(slot) = normalize_input_slot_key(reference.as_str()) else {
                return false;
            };
            !fact_store.has(slot.as_str())
        })
        .collect::<Vec<_>>()
}

fn collect_input_refs_from_value(value: &Value, refs: &mut BTreeSet<String>) {
    match value {
        Value::Object(object) => {
            if let Some(reference) = object.get("ref").and_then(Value::as_str) {
                let trimmed = reference.trim();
                if trimmed.starts_with("inputs.") {
                    refs.insert(trimmed.to_string());
                }
            }
            for nested in object.values() {
                collect_input_refs_from_value(nested, refs);
            }
        }
        Value::Array(items) => {
            for nested in items {
                collect_input_refs_from_value(nested, refs);
            }
        }
        Value::String(text) => collect_input_refs_from_text(text.as_str(), refs),
        _ => {}
    }
}

fn collect_input_refs_from_text(text: &str, refs: &mut BTreeSet<String>) {
    let bytes = text.as_bytes();
    let mut offset = 0usize;
    while offset < bytes.len() {
        let Some(relative) = text[offset..].find("inputs.") else {
            break;
        };
        let start = offset + relative;
        let mut end = start + "inputs.".len();
        while end < bytes.len() {
            let ch = bytes[end] as char;
            if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | ':') {
                end = end.saturating_add(1);
                continue;
            }
            break;
        }
        if let Some(slot) = normalize_input_slot_key(&text[start..end]) {
            refs.insert(format!("inputs.{slot}"));
        }
        offset = end;
    }
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
