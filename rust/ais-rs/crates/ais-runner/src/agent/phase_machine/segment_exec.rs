use super::super::input_normalize::canonical_missing_ref;
use super::super::ref_model::RefPath;
use super::super::runtime_facts_store::RuntimeFactsStore;
use super::super::*;
use super::super::{candidates::CandidateContext, write_gates};
use ais_engine::{
    DefaultSolver, EngineEventRecord, EngineRunStatus, EngineRunnerOptions, EngineRunnerState,
};
use ais_sdk::documents::PlanSketchSegment;
use serde_json::Map;
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
    candidate_context: &CandidateContext,
    planning_memory: Option<Value>,
    runtime_facts_store: &mut RuntimeFactsStore,
    fact_store: &mut InputStore,
    checkpoint_extensions: &checkpoint_ext::AgentCheckpointExtensions,
    audit_attempt: &mut crate::audit_contract::AuditStreamAttempt,
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
                    sync_report.previous_hash.unwrap_or_else(|| "-".to_string()),
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
        super::super::write_event_sinks(command, annotated.as_slice(), audit_attempt)?;
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
            runtime_facts_store,
            checkpoint_extensions,
            audit_attempt,
        )?;
    }
    if !processed.plan_replaced && has_duplicate_command_id_rejection(processed.events.as_slice()) {
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
            let annotated =
                annotate_events_with_todo(retry_processed.events.as_slice(), segment, todo_id);
            *total_events = total_events.saturating_add(annotated.len());
            super::super::write_event_sinks(command, annotated.as_slice(), audit_attempt)?;
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
                runtime_facts_store,
                checkpoint_extensions,
                audit_attempt,
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
    let mut invalidated_completed_write_nodes = segment_completed_write_node_ids(segment, state)
        .into_iter()
        .collect::<BTreeSet<_>>();
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
            super::super::write_event_sinks(command, annotated.as_slice(), audit_attempt)?;
            apply_segment_stores_from_runtime_with_runtime_facts(
                segment,
                state,
                runtime_facts_store,
                fact_store,
                command.verbose_llm,
            );
            checkpoint_ledger.absorb_events(annotated.as_slice());
            checkpoint_ledger.mark_approved_nodes(
                &state.approved_node_ids,
                wall_clock_timestamp_rfc3339().as_str(),
            );
            let invalidation_report = invalidate_post_write_volatile_facts(
                segment,
                state,
                candidate_context,
                runtime_facts_store,
                fact_store,
                &mut invalidated_completed_write_nodes,
            );
            if invalidation_report.has_effect() && (command.verbose || command.verbose_llm) {
                super::super::trace::emit(
                    true,
                    "execute_round",
                    "post_write_volatile_facts_invalidated",
                    &[
                        ("segment_id", segment.segment_id.clone()),
                        (
                            "completed_nodes",
                            invalidation_report.completed_write_node_ids.join(","),
                        ),
                        ("signals", invalidation_report.invalidated_signals.join(",")),
                        (
                            "input_refs",
                            invalidation_report.invalidated_input_refs.join(","),
                        ),
                        (
                            "runtime_fact_refs",
                            invalidation_report.invalidated_runtime_fact_refs.join(","),
                        ),
                    ],
                );
            }
            super::super::checkpoint_flow::checkpoint_round(
                command,
                run_id,
                active_plan_hash,
                active_plan,
                state,
                checkpoint_ledger,
                planning_memory.clone(),
                fact_store,
                runtime_facts_store,
                checkpoint_extensions,
                audit_attempt,
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
) -> super::store_projection::InputStoreSyncReport {
    super::store_projection::sync_runtime_inputs_from_input_store(runtime, fact_store)
}

pub(crate) fn collect_segment_ref_closure(segment: &PlanSketchSegment) -> Vec<String> {
    let mut refs = BTreeSet::<String>::new();
    for step in &segment.steps {
        collect_refs_from_value(&Value::Object(step.inputs.clone()), &mut refs);
        if let Some(when) = step.when.as_ref() {
            collect_refs_from_text(when.cel.as_str(), &mut refs);
        }
        if let Some(until) = step.until.as_ref() {
            collect_refs_from_value(until, &mut refs);
        }
        for template in &step.constraint_templates {
            collect_refs_from_value(&Value::Object(template.params.clone()), &mut refs);
        }
    }
    refs.into_iter().collect::<Vec<_>>()
}

pub(crate) fn collect_segment_missing_refs<F>(
    segment: &PlanSketchSegment,
    mut has_ref: F,
) -> Vec<String>
where
    F: FnMut(&str) -> bool,
{
    let in_segment_step_ids = segment
        .steps
        .iter()
        .map(|step| step.id.as_str())
        .collect::<BTreeSet<_>>();
    collect_segment_ref_closure(segment)
        .into_iter()
        .filter(|reference| {
            let Some(RefPath::NodeOutput { step_id, .. }) = RefPath::parse(reference) else {
                return true;
            };
            !in_segment_step_ids.contains(step_id.as_str())
        })
        .filter(|reference| !has_ref(reference))
        .collect::<Vec<_>>()
}

fn collect_refs_from_value(value: &Value, refs: &mut BTreeSet<String>) {
    match value {
        Value::Object(object) => {
            if let Some(reference) = object.get("ref").and_then(Value::as_str) {
                if let Some(canonical_ref) = canonical_missing_ref(reference) {
                    refs.insert(canonical_ref);
                }
            }
            for nested in object.values() {
                collect_refs_from_value(nested, refs);
            }
        }
        Value::Array(items) => {
            for nested in items {
                collect_refs_from_value(nested, refs);
            }
        }
        Value::String(text) => collect_refs_from_text(text.as_str(), refs),
        _ => {}
    }
}

fn collect_refs_from_text(text: &str, refs: &mut BTreeSet<String>) {
    for prefix in ["inputs.", "facts.", "nodes.", "fact:", "fact."] {
        collect_refs_from_text_with_prefix(text, prefix, refs);
    }
}

fn collect_refs_from_text_with_prefix(text: &str, prefix: &str, refs: &mut BTreeSet<String>) {
    let bytes = text.as_bytes();
    let mut offset = 0usize;
    while offset < bytes.len() {
        let Some(relative) = text[offset..].find(prefix) else {
            break;
        };
        let start = offset + relative;
        let mut end = start + prefix.len();
        while end < bytes.len() {
            let ch = bytes[end] as char;
            if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | ':') {
                end = end.saturating_add(1);
                continue;
            }
            break;
        }
        if let Some(canonical_ref) = canonical_missing_ref(&text[start..end]) {
            refs.insert(canonical_ref);
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct PostWriteInvalidationReport {
    completed_write_node_ids: Vec<String>,
    invalidated_signals: Vec<String>,
    invalidated_input_refs: Vec<String>,
    invalidated_runtime_fact_refs: Vec<String>,
}

impl PostWriteInvalidationReport {
    fn has_effect(&self) -> bool {
        !self.invalidated_input_refs.is_empty() || !self.invalidated_runtime_fact_refs.is_empty()
    }
}

fn invalidate_post_write_volatile_facts(
    segment: &PlanSketchSegment,
    state: &EngineRunnerState,
    candidate_context: &CandidateContext,
    runtime_facts_store: &mut RuntimeFactsStore,
    fact_store: &mut InputStore,
    invalidated_completed_write_nodes: &mut BTreeSet<String>,
) -> PostWriteInvalidationReport {
    let mut report = PostWriteInvalidationReport::default();
    let mut signals = BTreeSet::new();
    let completed_node_ids = state
        .completed_node_ids
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    for step in &segment.steps {
        if step.kind != "action" {
            continue;
        }
        let node_id = format!("{}/{}", segment.segment_id, step.id);
        if !completed_node_ids.contains(node_id.as_str())
            || !invalidated_completed_write_nodes.insert(node_id.clone())
        {
            continue;
        }
        report.completed_write_node_ids.push(node_id);
        for signal in write_gates::required_action_volatile_signals(step, candidate_context) {
            signals.insert(signal);
        }
    }

    let signals = signals.into_iter().collect::<Vec<_>>();
    if signals.is_empty() {
        return report;
    }

    report.invalidated_signals = signals
        .iter()
        .map(|signal| write_gates::volatile_signal_name(*signal).to_string())
        .collect();
    report.invalidated_input_refs = fact_store.invalidate_volatile_signals(signals.as_slice());
    report.invalidated_runtime_fact_refs =
        runtime_facts_store.invalidate_volatile_signals(signals.as_slice());
    report
}

fn segment_completed_write_node_ids(
    segment: &PlanSketchSegment,
    state: &EngineRunnerState,
) -> Vec<String> {
    let completed_node_ids = state
        .completed_node_ids
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    segment
        .steps
        .iter()
        .filter(|step| step.kind == "action")
        .map(|step| format!("{}/{}", segment.segment_id, step.id))
        .filter(|node_id| completed_node_ids.contains(node_id.as_str()))
        .collect::<Vec<_>>()
}

pub(crate) fn run_status_name(status: EngineRunStatus) -> &'static str {
    match status {
        EngineRunStatus::Completed => "completed",
        EngineRunStatus::Paused => "paused",
        EngineRunStatus::Stopped => "stopped",
    }
}

pub(crate) fn apply_segment_stores_from_runtime_with_runtime_facts(
    segment: &PlanSketchSegment,
    state: &EngineRunnerState,
    runtime_facts_store: &mut RuntimeFactsStore,
    fact_store: &mut InputStore,
    verbose_llm: bool,
) {
    super::store_projection::apply_segment_stores_from_runtime(
        segment,
        state,
        runtime_facts_store,
        fact_store,
        verbose_llm,
    );
    super::store_projection::auto_project_query_outputs_to_input_store(
        segment,
        state,
        runtime_facts_store,
        fact_store,
        verbose_llm,
    );
}

#[cfg(test)]
#[path = "../tests/phase_machine/segment_exec.rs"]
mod tests;
