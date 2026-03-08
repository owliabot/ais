use super::super::candidates::CandidateContext;
use super::super::input_normalize;
use super::super::orchestrator::SegmentedAgentContext;
use super::super::state_summary::StateSummary;
use crate::cli::AgentCommand;
use ais_engine::{
    run_plan_once, DefaultSolver, EngineRunnerOptions, EngineRunnerState, RouterExecutor,
};
use ais_sdk::documents::PlanSketchSegment;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};

const HOST_QUERY_AUTOFILL_MAX_TOTAL_ATTEMPTS: usize = 6;
const HOST_QUERY_AUTOFILL_MAX_ATTEMPTS_PER_REF: usize = 2;
const HOST_QUERY_AUTOFILL_EMPTY_STREAK_FUSE: usize = 2;
const HOST_QUERY_AUTOFILL_MAX_ROUNDS: usize = 3;
const RECOVERY_MAX_NO_PROGRESS_ROUNDS: usize = HOST_QUERY_AUTOFILL_MAX_ROUNDS + 1;
const RECOVERY_SAME_DECISION_HASH_ROUNDS: usize = HOST_QUERY_AUTOFILL_MAX_ROUNDS + 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MissingResolutionOutcome {
    Recovered,
    RetryScheduled,
    NeedUserInput {
        missing_refs: Vec<String>,
        reason: String,
    },
    ExhaustedUnavailable {
        missing_refs: Vec<String>,
        reason: String,
    },
}

impl MissingResolutionOutcome {
    pub(crate) fn should_retry_round(&self) -> bool {
        matches!(self, Self::Recovered | Self::RetryScheduled)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QueryAutofillFailureReason {
    RouterUnavailable,
    ParamBuildFailed,
    QueryExecFailed,
    QueryNoUsableOutput,
}

impl QueryAutofillFailureReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::RouterUnavailable => "router_unavailable",
            Self::ParamBuildFailed => "param_build_failed",
            Self::QueryExecFailed => "query_exec_failed",
            Self::QueryNoUsableOutput => "query_no_usable_output",
        }
    }

    fn is_hard_fail(self) -> bool {
        matches!(self, Self::RouterUnavailable | Self::QueryExecFailed)
    }
}

#[derive(Debug, Default)]
struct QueryAutofillRoundStats {
    round: usize,
    attempts_total: usize,
    per_missing_ref_attempts: BTreeMap<String, usize>,
    empty_or_invalid_streak: usize,
    resolved_refs: Vec<String>,
    unresolved_refs: Vec<String>,
    failure_reasons: BTreeMap<String, Vec<String>>,
    terminal_reason: String,
    hard_fail_type: Option<String>,
}

/// Preserve host-provided autofill recovery context across planner repair retries.
pub(crate) fn preserve_autofill_context(previous_error: Option<&Value>, payload: &mut Value) {
    let Some(autofill) = previous_error
        .and_then(|value| value.get("autofill"))
        .cloned()
    else {
        return;
    };
    if let Some(object) = payload.as_object_mut() {
        object.insert("autofill".to_string(), autofill);
    }
}

/// Split questions into query-recoverable and unresolved buckets via resolver candidates.
pub(crate) fn split_query_recoverable_questions(
    candidate_context: &CandidateContext,
    questions: &[Value],
    limit_per_ref: usize,
) -> (Vec<Value>, Vec<Value>) {
    let missing_refs = question_missing_refs(questions);
    if missing_refs.is_empty() {
        return (Vec::new(), questions.to_vec());
    }
    let resolution = super::super::intent_segmented::resolve_missing_facts_for_refs(
        candidate_context,
        &missing_refs,
        limit_per_ref,
    );
    let recoverable_refs = query_recoverable_missing_refs(&resolution);
    if recoverable_refs.is_empty() {
        return (Vec::new(), questions.to_vec());
    }
    let mut recoverable = Vec::new();
    let mut unresolved = Vec::new();
    for question in questions {
        let Some(id) = question.get("id").and_then(Value::as_str) else {
            unresolved.push(question.clone());
            continue;
        };
        let Some(canonical_ref) = input_normalize::canonical_missing_ref(id) else {
            unresolved.push(question.clone());
            continue;
        };
        if recoverable_refs.contains(&canonical_ref) {
            recoverable.push(question.clone());
        } else {
            unresolved.push(question.clone());
        }
    }
    (recoverable, unresolved)
}

pub(crate) fn query_recoverable_missing_refs(resolution_payload: &Value) -> BTreeSet<String> {
    resolution_payload
        .get("resolved")
        .and_then(Value::as_array)
        .into_iter()
        .flat_map(|items| items.iter())
        .filter_map(|item| {
            let missing_ref = item.get("missing_ref").and_then(Value::as_str)?;
            let has_candidates = item
                .get("query_candidates")
                .and_then(Value::as_array)
                .is_some_and(|items| !items.is_empty());
            if has_candidates {
                Some(missing_ref.to_string())
            } else {
                None
            }
        })
        .collect::<BTreeSet<_>>()
}

pub(crate) fn selected_query_refs_from_missing_resolution(
    resolution_payload: &Value,
) -> Vec<String> {
    let decisions = super::policy::build_missing_resolution_decisions(resolution_payload);
    super::policy::selected_query_refs_from_missing_resolution_decisions(decisions.as_slice())
}

pub(crate) fn query_candidate_pool_from_missing_resolution(
    resolution_payload: &Value,
) -> Vec<Value> {
    resolution_payload
        .get("resolved")
        .and_then(Value::as_array)
        .into_iter()
        .flat_map(|items| items.iter())
        .map(|item| {
            serde_json::json!({
                "missing_ref": item.get("missing_ref").and_then(Value::as_str).unwrap_or(""),
                "query_candidates": item
                    .get("query_candidates")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default(),
            })
        })
        .collect::<Vec<_>>()
}

pub(crate) fn payload_question_refs(payload: &Value) -> Vec<String> {
    let questions = payload
        .get("questions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    super::super::missing_registry::collect_question_refs(questions.as_slice())
}

pub(crate) fn missing_required_refs(payload: &Value) -> Vec<String> {
    super::super::missing_registry::collect_missing_refs_from_payload(payload)
}

pub(crate) fn missing_required_input_refs(payload: &Value) -> Vec<String> {
    missing_required_refs(payload)
        .into_iter()
        .filter(|reference| reference.starts_with("inputs."))
        .collect::<Vec<_>>()
}

fn runtime_has_ref_any(typed_summary: Option<&StateSummary>, reference: &str) -> bool {
    super::runtime_has_ref_typed(typed_summary, reference)
}

fn merged_ref_catalog(
    typed_summary: Option<&StateSummary>,
) -> super::super::ref_catalog::RefCatalog {
    super::super::ref_catalog::RefCatalog::build_typed(typed_summary)
}

pub(crate) fn precheck_missing_input_refs_for_current_todo(
    context: &SegmentedAgentContext,
    typed_summary: Option<&StateSummary>,
) -> Vec<String> {
    let Some(current_todo) = context.todo_board().current() else {
        return Vec::new();
    };
    super::super::missing_registry::collect_todo_precheck_missing_refs(
        current_todo.required_facts.as_slice(),
        |reference| runtime_has_ref_any(typed_summary, reference),
    )
}

pub(crate) fn precheck_missing_input_payload(missing_refs: &[String], round: u8) -> Value {
    let questions = missing_refs
        .iter()
        .filter(|reference| is_user_promptable_missing_ref(reference))
        .map(|reference| {
            serde_json::json!({
                "id": reference,
                "question": format!("Provide `{reference}`"),
                "required": true,
                "options": [],
            })
        })
        .collect::<Vec<_>>();
    super::super::missing_input::payload_with_context(
        Some("todo precheck missing required inputs"),
        questions.as_slice(),
        &[],
        missing_refs,
        missing_refs,
        round,
    )
}

pub(crate) fn emit_missing_input_autofill_unresolved(
    trace_enabled: bool,
    state: &mut EngineRunnerState,
    phase_hint: &str,
    scope_id: &str,
    missing_refs: &[String],
    reason: &str,
) {
    super::super::runtime_store::record_runtime_agent_field(
        &mut state.runtime,
        "missing_input_autofill",
        serde_json::json!({
            "status": "unresolved",
            "phase_hint": phase_hint,
            "scope_id": scope_id,
            "missing_refs": missing_refs,
            "reason": reason,
        }),
    );
    super::super::trace::emit(
        trace_enabled,
        phase_hint,
        "autofill_attempt_unresolved",
        &[
            ("scope_id", scope_id.to_string()),
            ("missing_refs", missing_refs.join(",")),
            ("reason", reason.to_string()),
        ],
    );
}

fn record_recovery_telemetry(
    trace_enabled: bool,
    state: &mut EngineRunnerState,
    phase_hint: &str,
    scope_id: &str,
    stage: &str,
    attempt: usize,
    terminal_reason: &str,
    resolved_refs: &[String],
    unresolved_refs: &[String],
) {
    let attempt_trace_id = format!("missing_resolution:{phase_hint}:{scope_id}:{stage}:{attempt}");
    super::super::runtime_store::record_runtime_agent_field(
        &mut state.runtime,
        "missing_resolution",
        serde_json::json!({
            "status": if unresolved_refs.is_empty() { "resolved" } else { "unresolved" },
            "phase_hint": phase_hint,
            "scope_id": scope_id,
            "stage": stage,
            "attempt": attempt,
            "terminal_reason": terminal_reason,
            "attempt_trace_id": attempt_trace_id,
            "resolved_refs": resolved_refs,
            "unresolved_refs": unresolved_refs,
        }),
    );
    super::super::trace::emit(
        trace_enabled,
        phase_hint,
        "missing_resolution_terminal",
        &[
            ("scope_id", scope_id.to_string()),
            ("stage", stage.to_string()),
            ("attempt", attempt.to_string()),
            ("terminal_reason", terminal_reason.to_string()),
            ("unresolved_refs", unresolved_refs.join(",")),
        ],
    );
}

fn record_recovery_termination(
    state: &mut EngineRunnerState,
    phase_hint: &str,
    scope_id: &str,
    reason: &str,
    query_round: usize,
    termination_state: &super::termination::MissingResolutionTerminationState,
) {
    super::super::runtime_store::record_runtime_agent_field(
        &mut state.runtime,
        "missing_ref_termination",
        serde_json::json!({
            "phase_hint": phase_hint,
            "scope_id": scope_id,
            "reason": reason,
            "query_round": query_round,
            "max_rounds": HOST_QUERY_AUTOFILL_MAX_ROUNDS,
            "no_progress_rounds": termination_state.no_progress_rounds,
            "same_decision_hash_rounds": termination_state.same_decision_hash_rounds,
            "total_attempts": termination_state.total_attempts,
            "last_decision_hash": termination_state.last_decision_hash,
        }),
    );
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn missing_resolution_recover_missing_refs(
    command: &AgentCommand,
    state: &mut EngineRunnerState,
    context: &mut SegmentedAgentContext,
    missing_input_payload: &Value,
    candidate_context: &CandidateContext,
    readonly_autofill_router: Option<&RouterExecutor>,
    scope_id: &str,
    done: bool,
    phase_hint: &'static str,
) -> MissingResolutionOutcome {
    let trace_enabled = command.verbose || command.verbose_llm;
    let mut missing_refs = missing_required_refs(missing_input_payload);
    if missing_refs.is_empty() {
        missing_refs = payload_question_refs(missing_input_payload);
    }
    if missing_refs.is_empty() {
        record_recovery_telemetry(
            trace_enabled,
            state,
            phase_hint,
            scope_id,
            "static_binding",
            0,
            "already_resolved",
            &[],
            &[],
        );
        return MissingResolutionOutcome::Recovered;
    }

    let static_outcome = super::apply_static_missing_ref_refill(
        state,
        context,
        missing_refs.as_slice(),
        phase_hint,
        scope_id,
    );
    let mut recovered_refs = static_outcome.resolved_refs.clone();
    missing_refs.retain(|path| !runtime_has_ref_any(context.typed_summary(), path));

    let ambiguous_bindings = static_outcome
        .ambiguous_bindings
        .iter()
        .filter(|item| {
            missing_refs
                .iter()
                .any(|missing| missing == &item.missing_ref)
        })
        .map(|item| {
            serde_json::json!({
                "missing_ref": item.missing_ref,
                "candidate_refs": item.candidate_refs,
            })
        })
        .collect::<Vec<_>>();

    if !static_outcome.resolved_refs.is_empty() {
        let mut previous_error = missing_input_payload.clone();
        if let Some(object) = previous_error.as_object_mut() {
            object.insert(
                "autofill".to_string(),
                serde_json::json!({
                    "mode": "host_static_refill_round",
                    "phase_hint": phase_hint,
                    "scope_id": scope_id,
                    "resolved_refs": static_outcome.resolved_refs.clone(),
                    "unresolved_refs": missing_refs.clone(),
                }),
            );
        }
        context.set_previous_error_and_refresh(state, done, previous_error);
        super::super::runtime_store::record_runtime_agent_field(
            &mut state.runtime,
            "missing_ref_refill",
            serde_json::json!({
                "status": if missing_refs.is_empty() { "resolved" } else { "resolved_partial" },
                "phase_hint": phase_hint,
                "scope_id": scope_id,
                "resolved_refs": static_outcome.resolved_refs.clone(),
                "unresolved_refs": missing_refs.clone(),
                "attempt": "static_intent_config",
            }),
        );
    }
    if missing_refs.is_empty() {
        record_recovery_telemetry(
            trace_enabled,
            state,
            phase_hint,
            scope_id,
            "static_binding",
            1,
            "resolved_after_static_binding",
            recovered_refs.as_slice(),
            &[],
        );
        return MissingResolutionOutcome::Recovered;
    }

    super::super::trace::emit(
        trace_enabled,
        phase_hint,
        "autofill_attempt_start",
        &[
            ("scope_id", scope_id.to_string()),
            ("missing_refs", missing_refs.join(",")),
        ],
    );

    let mut last_resolution = Value::Null;
    let mut query_candidate_pool = Vec::<Value>::new();
    let mut query_round = 0usize;
    let mut terminal_reason = "no_query_candidates".to_string();
    let mut hard_fail_type = None::<String>;
    let mut query_stage_attempted = false;
    let termination_policy = super::termination::MissingResolutionTerminationPolicy {
        max_no_progress_rounds: RECOVERY_MAX_NO_PROGRESS_ROUNDS,
        max_same_decision_hash_rounds: RECOVERY_SAME_DECISION_HASH_ROUNDS,
        max_total_attempts: HOST_QUERY_AUTOFILL_MAX_TOTAL_ATTEMPTS,
    };
    let mut termination_state = super::termination::MissingResolutionTerminationState::default();

    while !missing_refs.is_empty() && query_round < HOST_QUERY_AUTOFILL_MAX_ROUNDS {
        query_round = query_round.saturating_add(1);
        let resolution = super::super::intent_segmented::resolve_missing_facts_for_refs(
            candidate_context,
            missing_refs.as_slice(),
            3,
        );
        let resolution_with_decisions =
            merge_missing_resolution_decisions(&resolution, missing_input_payload);
        query_candidate_pool =
            query_candidate_pool_from_missing_resolution(&resolution_with_decisions);
        last_resolution = resolution_with_decisions.clone();
        let drafted_decisions =
            super::policy::build_missing_resolution_decisions(&resolution_with_decisions);
        let catalog = merged_ref_catalog(context.typed_summary());
        let validation = super::policy::validate_missing_resolution_decisions(
            drafted_decisions.as_slice(),
            missing_refs.as_slice(),
            &catalog,
        );
        let decisions = validation.accepted_decisions.clone();
        let rejected_decisions = validation.rejected_decisions.clone();
        let policy_status = if decisions.is_empty() {
            "rejected"
        } else if rejected_decisions.is_empty() {
            "accepted"
        } else {
            "partial"
        };
        super::super::runtime_store::record_runtime_agent_field(
            &mut state.runtime,
            "missing_ref_policy_validation",
            serde_json::json!({
                "status": policy_status,
                "phase_hint": phase_hint,
                "scope_id": scope_id,
                "round": query_round,
                "decisions_total": drafted_decisions.len(),
                "accepted_decisions": decisions.clone(),
                "rejected_decisions": rejected_decisions,
                "issues": validation.issues,
            }),
        );
        if decisions.is_empty() {
            terminal_reason = "policy_validation_failed".to_string();
            break;
        }
        let state_summary_snapshot = context.packed_summary().clone();
        let execution_plan =
            super::executor::build_missing_resolution_execution_plan(decisions.as_slice());
        let bind_execution = super::executor::apply_missing_resolution_bindings(
            &mut state.runtime,
            context.input_store_mut(),
            state_summary_snapshot.as_ref(),
            execution_plan.bindings.as_slice(),
            format!("{phase_hint}:{scope_id}:round_{query_round}").as_str(),
        );
        if !bind_execution.issues.is_empty() {
            super::super::runtime_store::record_runtime_agent_field(
                &mut state.runtime,
                "missing_ref_execution",
                serde_json::json!({
                    "phase_hint": phase_hint,
                    "scope_id": scope_id,
                    "round": query_round,
                    "issues": bind_execution.issues,
                }),
            );
        }
        if !bind_execution.resolved_targets.is_empty() {
            for resolved in bind_execution.resolved_targets.iter() {
                if !recovered_refs.contains(resolved) {
                    recovered_refs.push(resolved.clone());
                }
            }
            context.refresh_state_summary(state, false);
            missing_refs.retain(|path| !runtime_has_ref_any(context.typed_summary(), path));
            if missing_refs.is_empty() {
                terminal_reason = "resolved_after_bind".to_string();
                break;
            }
        }
        if let Some(reason) = execution_plan.abort_reason.as_ref() {
            terminal_reason = format!("policy_abort:{reason}");
            break;
        }
        if execution_plan.run_producers.is_empty() {
            terminal_reason = if execution_plan.ask_user.is_empty() {
                "no_query_candidates".to_string()
            } else {
                "policy_ask_user".to_string()
            };
            break;
        }
        query_stage_attempted = true;
        let before_round_missing = missing_refs.clone();
        let before_round_missing_len = before_round_missing.len();
        let query_autofill_stats = execute_host_query_autofill_round(
            command,
            state,
            context,
            candidate_context,
            readonly_autofill_router,
            phase_hint,
            scope_id,
            &resolution_with_decisions,
            execution_plan.run_producers.as_slice(),
            query_round,
        );
        for resolved in query_autofill_stats.resolved_refs.iter() {
            if !recovered_refs.contains(resolved) {
                recovered_refs.push(resolved.clone());
            }
        }
        missing_refs = before_round_missing
            .into_iter()
            .filter(|path| !runtime_has_ref_any(context.typed_summary(), path))
            .collect::<Vec<_>>();
        let made_progress = missing_refs.len() < before_round_missing_len;
        terminal_reason = query_autofill_stats.terminal_reason.clone();
        if query_autofill_stats.hard_fail_type.is_some() {
            hard_fail_type = query_autofill_stats.hard_fail_type;
            break;
        }
        if missing_refs.is_empty() {
            break;
        }
        if let Some(reason) = super::termination::observe_missing_resolution_round_progress(
            &mut termination_state,
            termination_policy,
            decisions.as_slice(),
            made_progress,
            query_autofill_stats.attempts_total,
        ) {
            terminal_reason = reason.clone();
            record_recovery_termination(
                state,
                phase_hint,
                scope_id,
                reason.as_str(),
                query_round,
                &termination_state,
            );
            break;
        }
        if query_round >= HOST_QUERY_AUTOFILL_MAX_ROUNDS {
            terminal_reason = "max_rounds_reached".to_string();
            record_recovery_termination(
                state,
                phase_hint,
                scope_id,
                terminal_reason.as_str(),
                query_round,
                &termination_state,
            );
            break;
        }
    }

    if missing_refs.is_empty() {
        super::super::runtime_store::record_runtime_agent_field(
            &mut state.runtime,
            "missing_ref_refill",
            serde_json::json!({
                "status": "resolved",
                "phase_hint": phase_hint,
                "scope_id": scope_id,
                "resolved_refs": recovered_refs,
                "unresolved_refs": [],
                "attempt": "dynamic_query",
                "query_rounds": query_round,
            }),
        );
        record_recovery_telemetry(
            trace_enabled,
            state,
            phase_hint,
            scope_id,
            "query_execute",
            query_round,
            "resolved_after_query",
            recovered_refs.as_slice(),
            &[],
        );
        return MissingResolutionOutcome::Recovered;
    }

    let adjudicate_retry_key = format!("binding_adjudicate:{phase_hint}:{scope_id}");
    if (!ambiguous_bindings.is_empty() || query_stage_attempted || !query_candidate_pool.is_empty())
        && !context.has_compile_autofill_attempt(adjudicate_retry_key.as_str())
    {
        context.mark_compile_autofill_attempt(adjudicate_retry_key);
        let catalog = merged_ref_catalog(context.typed_summary());
        let available_input_refs =
            super::super::ref_catalog::available_input_ref_catalog_typed(context.typed_summary());
        let available_refs = catalog
            .entries
            .iter()
            .map(|entry| {
                serde_json::json!({
                    "ref": entry.canonical_ref,
                    "has_value": entry.value_available,
                    "value_type": entry.value_type,
                    "source": entry.source,
                    "source_priority": entry.source_priority,
                    "producer_step": entry.producer_step,
                })
            })
            .collect::<Vec<_>>();
        let mut previous_error = missing_input_payload.clone();
        if let Some(object) = previous_error.as_object_mut() {
            object.insert(
                "autofill".to_string(),
                serde_json::json!({
                    "mode": "host_binding_adjudicate_round",
                    "phase_hint": phase_hint,
                    "scope_id": scope_id,
                    "resolved_refs": recovered_refs.clone(),
                    "unresolved_refs": missing_refs.clone(),
                    "ambiguous_bindings": ambiguous_bindings,
                    "available_input_refs": available_input_refs,
                    "available_refs": available_refs,
                    "query_candidate_pool": query_candidate_pool,
                    "resolver": last_resolution,
                    "query_rounds": query_round,
                }),
            );
        }
        context.set_previous_error_and_refresh(state, done, previous_error);
        super::super::runtime_store::record_runtime_agent_field(
            &mut state.runtime,
            "missing_ref_refill",
            serde_json::json!({
                "status": "adjudicate_scheduled",
                "phase_hint": phase_hint,
                "scope_id": scope_id,
                "resolved_refs": recovered_refs.clone(),
                "unresolved_refs": missing_refs.clone(),
                "attempt": "llm_binding_adjudicate",
                "reason": terminal_reason,
                "query_rounds": query_round,
            }),
        );
        record_recovery_telemetry(
            trace_enabled,
            state,
            phase_hint,
            scope_id,
            "llm_adjudicate",
            query_round,
            "adjudicate_scheduled",
            recovered_refs.as_slice(),
            missing_refs.as_slice(),
        );
        return MissingResolutionOutcome::RetryScheduled;
    }

    if terminal_reason.is_empty() {
        terminal_reason = "no_query_candidates".to_string();
    }
    record_recovery_termination(
        state,
        phase_hint,
        scope_id,
        terminal_reason.as_str(),
        query_round,
        &termination_state,
    );
    emit_missing_input_autofill_unresolved(
        trace_enabled,
        state,
        phase_hint,
        scope_id,
        missing_refs.as_slice(),
        terminal_reason.as_str(),
    );
    record_recovery_telemetry(
        trace_enabled,
        state,
        phase_hint,
        scope_id,
        "user_input",
        query_round,
        terminal_reason.as_str(),
        recovered_refs.as_slice(),
        missing_refs.as_slice(),
    );
    if hard_fail_type.is_some() {
        return MissingResolutionOutcome::ExhaustedUnavailable {
            missing_refs,
            reason: terminal_reason,
        };
    }
    if !missing_refs
        .iter()
        .any(|reference| is_user_promptable_missing_ref(reference))
    {
        return MissingResolutionOutcome::ExhaustedUnavailable {
            missing_refs,
            reason: terminal_reason,
        };
    }
    MissingResolutionOutcome::NeedUserInput {
        missing_refs,
        reason: terminal_reason,
    }
}

fn is_user_promptable_missing_ref(reference: &str) -> bool {
    matches!(
        input_normalize::canonical_missing_ref_path(reference),
        Some(
            super::super::ref_model::RefPath::Input { .. }
                | super::super::ref_model::RefPath::Fact { .. }
        )
    )
}

fn merge_missing_resolution_decisions(
    resolution_payload: &Value,
    missing_input_payload: &Value,
) -> Value {
    let decisions = collect_explicit_missing_resolution_decisions(missing_input_payload);
    if decisions.is_empty() {
        return resolution_payload.clone();
    }
    let mut merged = resolution_payload.clone();
    let Some(object) = merged.as_object_mut() else {
        return resolution_payload.clone();
    };
    object.insert("decisions".to_string(), Value::Array(decisions));
    merged
}

fn collect_explicit_missing_resolution_decisions(payload: &Value) -> Vec<Value> {
    let mut out = Vec::<Value>::new();
    extend_decision_array(&mut out, payload.get("decisions"));
    extend_decision_array(&mut out, payload.pointer("/autofill/decisions"));
    extend_decision_array(&mut out, payload.pointer("/error_details/decisions"));
    extend_decision_array(&mut out, payload.pointer("/details/decisions"));
    extend_decision_array(&mut out, payload.pointer("/error/details/decisions"));
    extend_legacy_binding_decisions(&mut out, payload.get("binding_decisions"));
    extend_legacy_binding_decisions(&mut out, payload.pointer("/autofill/binding_decisions"));
    extend_legacy_binding_decisions(
        &mut out,
        payload.pointer("/error_details/binding_decisions"),
    );
    extend_legacy_binding_decisions(&mut out, payload.pointer("/details/binding_decisions"));
    extend_legacy_binding_decisions(
        &mut out,
        payload.pointer("/error/details/binding_decisions"),
    );
    extend_legacy_query_decisions(&mut out, payload.get("query_decisions"));
    extend_legacy_query_decisions(&mut out, payload.pointer("/autofill/query_decisions"));
    extend_legacy_query_decisions(&mut out, payload.pointer("/error_details/query_decisions"));
    extend_legacy_query_decisions(&mut out, payload.pointer("/details/query_decisions"));
    extend_legacy_query_decisions(&mut out, payload.pointer("/error/details/query_decisions"));
    out.into_iter()
        .filter(|item| item.is_object())
        .collect::<Vec<_>>()
}

fn extend_decision_array(out: &mut Vec<Value>, source: Option<&Value>) {
    for item in source
        .and_then(Value::as_array)
        .into_iter()
        .flat_map(|items| items.iter())
    {
        out.push(item.clone());
    }
}

fn extend_legacy_binding_decisions(out: &mut Vec<Value>, source: Option<&Value>) {
    for item in source
        .and_then(Value::as_array)
        .into_iter()
        .flat_map(|items| items.iter())
    {
        let target = item
            .get("target")
            .or_else(|| item.get("missing_ref"))
            .or_else(|| item.get("ref"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();
        let source_ref = item
            .get("source")
            .or_else(|| item.get("source_ref"))
            .or_else(|| item.get("from_ref"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();
        if target.is_empty() || source_ref.is_empty() {
            continue;
        }
        out.push(serde_json::json!({
            "kind": "bind_from_ref",
            "target": target,
            "source": source_ref,
        }));
    }
}

fn extend_legacy_query_decisions(out: &mut Vec<Value>, source: Option<&Value>) {
    for item in source
        .and_then(Value::as_array)
        .into_iter()
        .flat_map(|items| items.iter())
    {
        let target = item
            .get("target")
            .or_else(|| item.get("missing_ref"))
            .or_else(|| item.get("ref"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();
        let query_ref = item
            .get("query_ref")
            .or_else(|| item.get("step_or_query_ref"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();
        if target.is_empty() || query_ref.is_empty() {
            continue;
        }
        out.push(serde_json::json!({
            "kind": "run_producer",
            "target": target,
            "query_ref": query_ref,
        }));
    }
}

fn build_query_candidate_for_run_producer(
    resolution_payload: &Value,
    run: &super::executor::MissingResolutionRunProducerAction,
) -> Value {
    let target_ref = run.target.as_canonical_str();
    let query_ref = run.query_ref.trim();
    if query_ref.is_empty() {
        return Value::Null;
    }
    if let Some(candidate) = resolution_payload
        .get("resolved")
        .and_then(Value::as_array)
        .into_iter()
        .flat_map(|items| items.iter())
        .find(|item| item.get("missing_ref").and_then(Value::as_str) == Some(target_ref.as_str()))
        .and_then(|item| item.get("query_candidates").and_then(Value::as_array))
        .into_iter()
        .flat_map(|items| items.iter())
        .find(|item| item.get("query_ref").and_then(Value::as_str) == Some(query_ref))
    {
        return candidate.clone();
    }
    serde_json::json!({ "query_ref": query_ref })
}

#[allow(clippy::too_many_arguments)]
fn execute_host_query_autofill_round(
    command: &AgentCommand,
    state: &mut EngineRunnerState,
    context: &mut SegmentedAgentContext,
    candidate_context: &CandidateContext,
    readonly_autofill_router: Option<&RouterExecutor>,
    phase_hint: &'static str,
    scope_id: &str,
    resolution_payload: &Value,
    run_producers: &[super::executor::MissingResolutionRunProducerAction],
    round: usize,
) -> QueryAutofillRoundStats {
    let mut stats = QueryAutofillRoundStats {
        round,
        ..QueryAutofillRoundStats::default()
    };
    let trace_enabled = command.verbose || command.verbose_llm;
    if run_producers.is_empty() {
        stats.terminal_reason = "no_query_candidates".to_string();
        record_query_autofill_round_stats(state, phase_hint, scope_id, &stats);
        return stats;
    }
    if readonly_autofill_router.is_none() {
        for run in run_producers {
            let missing_ref = run.target.as_canonical_str();
            stats.unresolved_refs.push(missing_ref.to_string());
            push_query_failure_reason(
                &mut stats.failure_reasons,
                missing_ref.as_str(),
                QueryAutofillFailureReason::RouterUnavailable,
            );
        }
        stats.terminal_reason = QueryAutofillFailureReason::RouterUnavailable
            .as_str()
            .to_string();
        stats.hard_fail_type = Some(
            QueryAutofillFailureReason::RouterUnavailable
                .as_str()
                .to_string(),
        );
        record_query_autofill_round_stats(state, phase_hint, scope_id, &stats);
        return stats;
    }
    let router = readonly_autofill_router.expect("checked is_some");
    let mut break_reason = None::<String>;
    let mut hard_fail = None::<QueryAutofillFailureReason>;
    'outer: for run in run_producers {
        if hard_fail.is_some() {
            break;
        }
        if stats.attempts_total >= HOST_QUERY_AUTOFILL_MAX_TOTAL_ATTEMPTS {
            break_reason.get_or_insert_with(|| "max_total_attempts_reached".to_string());
            break;
        }
        if stats.empty_or_invalid_streak >= HOST_QUERY_AUTOFILL_EMPTY_STREAK_FUSE {
            break_reason.get_or_insert_with(|| "empty_streak_fuse".to_string());
            break;
        }
        let missing_ref = run.target.as_canonical_str();
        let query_candidate = build_query_candidate_for_run_producer(resolution_payload, run);
        let query_ref = run.query_ref.trim().to_string();
        let per_ref_attempts = stats
            .per_missing_ref_attempts
            .entry(missing_ref.clone())
            .or_default();
        if *per_ref_attempts >= HOST_QUERY_AUTOFILL_MAX_ATTEMPTS_PER_REF {
            break_reason.get_or_insert_with(|| "max_attempts_per_ref_reached".to_string());
            break;
        }
        if stats.attempts_total >= HOST_QUERY_AUTOFILL_MAX_TOTAL_ATTEMPTS {
            break_reason.get_or_insert_with(|| "max_total_attempts_reached".to_string());
            break;
        }
        if stats.empty_or_invalid_streak >= HOST_QUERY_AUTOFILL_EMPTY_STREAK_FUSE {
            break_reason.get_or_insert_with(|| "empty_streak_fuse".to_string());
            break;
        }
        *per_ref_attempts = per_ref_attempts.saturating_add(1);
        stats.attempts_total = stats.attempts_total.saturating_add(1);
        let attempt_start = json!({
            "schema": "ais-agent-query-autofill-attempt/0.0.1",
            "status": "start",
            "phase_hint": phase_hint,
            "scope_id": scope_id,
            "round": round,
            "missing_ref": missing_ref,
            "query_ref": query_ref,
            "attempt_index": stats.attempts_total,
        });
        append_query_autofill_attempt(state, &attempt_start);
        super::super::trace::emit(
            trace_enabled,
            phase_hint,
            "autofill_query_attempt_start",
            &[
                ("scope_id", scope_id.to_string()),
                ("missing_ref", missing_ref.clone()),
                ("query_ref", query_ref.clone()),
                ("attempt", stats.attempts_total.to_string()),
            ],
        );

        match execute_query_autofill_candidate(
            state,
            context,
            candidate_context,
            router,
            missing_ref.as_str(),
            &query_candidate,
        ) {
            Ok(Some(value)) => {
                stats.resolved_refs.push(missing_ref.clone());
                stats.empty_or_invalid_streak = 0;
                append_query_autofill_attempt(
                    state,
                    &json!({
                        "schema": "ais-agent-query-autofill-attempt/0.0.1",
                        "status": "resolved",
                        "phase_hint": phase_hint,
                        "scope_id": scope_id,
                        "round": round,
                        "missing_ref": missing_ref,
                        "query_ref": query_ref,
                        "resolved_value": value,
                        "attempt_index": stats.attempts_total,
                    }),
                );
                super::super::trace::emit(
                    trace_enabled,
                    phase_hint,
                    "autofill_query_attempt_end",
                    &[
                        ("scope_id", scope_id.to_string()),
                        ("missing_ref", missing_ref),
                        ("query_ref", query_ref),
                        ("status", "resolved".to_string()),
                    ],
                );
            }
            Ok(None) => {
                stats.empty_or_invalid_streak = stats.empty_or_invalid_streak.saturating_add(1);
                stats.unresolved_refs.push(missing_ref.clone());
                push_query_failure_reason(
                    &mut stats.failure_reasons,
                    missing_ref.as_str(),
                    QueryAutofillFailureReason::QueryNoUsableOutput,
                );
                append_query_autofill_attempt(
                    state,
                    &json!({
                        "schema": "ais-agent-query-autofill-attempt/0.0.1",
                        "status": "failed",
                        "phase_hint": phase_hint,
                        "scope_id": scope_id,
                        "round": round,
                        "missing_ref": missing_ref,
                        "query_ref": query_ref,
                        "reason": QueryAutofillFailureReason::QueryNoUsableOutput.as_str(),
                        "attempt_index": stats.attempts_total,
                    }),
                );
            }
            Err(reason) => {
                if matches!(
                    reason,
                    QueryAutofillFailureReason::ParamBuildFailed
                        | QueryAutofillFailureReason::QueryNoUsableOutput
                ) {
                    stats.empty_or_invalid_streak = stats.empty_or_invalid_streak.saturating_add(1);
                } else {
                    stats.empty_or_invalid_streak = 0;
                }
                stats.unresolved_refs.push(missing_ref.clone());
                push_query_failure_reason(&mut stats.failure_reasons, missing_ref.as_str(), reason);
                append_query_autofill_attempt(
                    state,
                    &json!({
                        "schema": "ais-agent-query-autofill-attempt/0.0.1",
                        "status": "failed",
                        "phase_hint": phase_hint,
                        "scope_id": scope_id,
                        "round": round,
                        "missing_ref": missing_ref,
                        "query_ref": query_ref,
                        "reason": reason.as_str(),
                        "attempt_index": stats.attempts_total,
                    }),
                );
                if reason.is_hard_fail() {
                    hard_fail = Some(reason);
                    break_reason = Some(reason.as_str().to_string());
                    break 'outer;
                }
            }
        }
    }
    if let Some(reason) = hard_fail {
        stats.hard_fail_type = Some(reason.as_str().to_string());
    }
    stats.terminal_reason = if let Some(reason) = break_reason {
        reason
    } else if !stats.resolved_refs.is_empty() {
        "progress".to_string()
    } else if stats.attempts_total == 0 {
        "no_query_candidates".to_string()
    } else {
        "query_no_usable_output".to_string()
    };
    if stats.hard_fail_type.is_none()
        && stats
            .terminal_reason
            .as_str()
            .eq(QueryAutofillFailureReason::RouterUnavailable.as_str())
    {
        stats.hard_fail_type = Some(
            QueryAutofillFailureReason::RouterUnavailable
                .as_str()
                .to_string(),
        );
    }
    record_query_autofill_round_stats(state, phase_hint, scope_id, &stats);
    stats
}

fn execute_query_autofill_candidate(
    state: &mut EngineRunnerState,
    context: &mut SegmentedAgentContext,
    candidate_context: &CandidateContext,
    router: &RouterExecutor,
    missing_ref: &str,
    query_candidate: &Value,
) -> Result<Option<Value>, QueryAutofillFailureReason> {
    let query_ref = query_candidate
        .get("query_ref")
        .and_then(Value::as_str)
        .ok_or(QueryAutofillFailureReason::ParamBuildFailed)?;
    let query_detail = candidate_context
        .detail_by_ref
        .get(query_ref)
        .ok_or(QueryAutofillFailureReason::ParamBuildFailed)?;
    let step_inputs = match build_query_autofill_step_inputs_typed(
        context.typed_summary(),
        query_detail,
        missing_ref,
    ) {
        Some(inputs) => inputs,
        None => {
            let diag = diagnose_param_build_failure_typed(
                context.typed_summary(),
                query_detail,
                missing_ref,
            );
            if !diag.is_empty() {
                eprintln!(
                    "[missing_resolution] param_build_failed for query={}: {}",
                    query_ref,
                    diag.join("; ")
                );
            }
            return Err(QueryAutofillFailureReason::ParamBuildFailed);
        }
    };
    let segment = build_query_autofill_segment(context, query_ref, step_inputs)
        .ok_or(QueryAutofillFailureReason::ParamBuildFailed)?;
    let chain_scope = query_autofill_chain_scope_typed(context.typed_summary(), query_detail);
    let known_input_refs =
        super::super::known_input_refs_from_typed_summary(context.typed_summary())
            .into_iter()
            .chain(
                super::super::reference_inventory::ReferenceInventory::build(
                    context.packed_summary().as_ref(),
                )
                .input_refs()
                .into_iter(),
            )
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
    let plan = super::super::compile_segment_plan_with_snapshot_hash(
        context.intent(),
        context.session().session_id.as_str(),
        context.session().cursor.as_str(),
        &segment,
        candidate_context,
        context.session().snapshot_hash.as_str(),
        chain_scope.as_slice(),
        known_input_refs.as_slice(),
        Some(context.runtime_facts_store()),
        Some(context.input_store()),
    )
    .map_err(|_| QueryAutofillFailureReason::ParamBuildFailed)?;
    let Some(node) = plan.nodes.first() else {
        return Err(QueryAutofillFailureReason::QueryExecFailed);
    };
    let node_id = node
        .get("id")
        .and_then(Value::as_str)
        .ok_or(QueryAutofillFailureReason::QueryExecFailed)?
        .to_string();
    let mut query_state = EngineRunnerState {
        runtime: state.runtime.clone(),
        ..EngineRunnerState::default()
    };
    let run = run_plan_once(
        "run-host-query-autofill",
        &plan,
        &mut query_state,
        router,
        &DefaultSolver,
        &[],
        &EngineRunnerOptions::default(),
    );
    if !matches!(
        run.status,
        ais_engine::EngineRunStatus::Completed | ais_engine::EngineRunStatus::Stopped
    ) {
        return Err(QueryAutofillFailureReason::QueryExecFailed);
    }
    let output_value = extract_query_output_value(
        &query_state.runtime,
        node_id.as_str(),
        missing_ref,
        query_candidate,
    );
    let Some(value) = output_value else {
        return Ok(None);
    };
    let Some(target_ref) = input_normalize::canonical_missing_ref_path(missing_ref) else {
        return Err(QueryAutofillFailureReason::ParamBuildFailed);
    };
    match target_ref {
        super::super::ref_model::RefPath::Input { slot } => {
            input_normalize::set_runtime_input_value(
                &mut state.runtime,
                slot.as_str(),
                value.clone(),
            );
            let _ = super::super::upsert_store_value_with_source(
                context.input_store_mut(),
                slot.as_str(),
                value.clone(),
                super::super::input_store::InputValueLayer::Derived,
                "host.query_autofill",
                88,
                format!("autofill.query.{missing_ref}.{query_ref}"),
            );
        }
        super::super::ref_model::RefPath::Fact { key } => {
            super::executor::set_runtime_intent_fact(
                &mut state.runtime,
                key.as_str(),
                value.clone(),
            );
        }
        super::super::ref_model::RefPath::NodeOutput {
            step_id,
            field_path,
        } => {
            set_runtime_node_output_value(
                &mut state.runtime,
                step_id.as_str(),
                field_path.as_str(),
                value.clone(),
            );
        }
    }
    context.refresh_state_summary(state, false);
    Ok(Some(value))
}

fn build_query_autofill_segment(
    context: &SegmentedAgentContext,
    query_ref: &str,
    step_inputs: Map<String, Value>,
) -> Option<PlanSketchSegment> {
    let segment_value = json!({
        "segment_id": format!("seg_autofill_{}", stable_scope_slug(context.session().cursor.as_str())),
        "cursor_in": context.session().cursor,
        "cursor_out": context.session().cursor,
        "done": false,
        "summary": format!("host autofill query {query_ref}"),
        "steps": [
            {
                "id": "q_autofill",
                "kind": "query",
                "candidate_ref": query_ref,
                "inputs": step_inputs,
                "depends_on": []
            }
        ],
        "extensions": {}
    });
    serde_json::from_value(segment_value).ok()
}

fn build_query_autofill_step_inputs_typed(
    typed_summary: Option<&StateSummary>,
    query_detail: &Value,
    missing_ref: &str,
) -> Option<Map<String, Value>> {
    let mut inputs = Map::<String, Value>::new();
    let params = query_detail.get("params").and_then(Value::as_array)?;
    for param in params {
        let name = param.get("name").and_then(Value::as_str)?.trim();
        if name.is_empty() {
            continue;
        }
        let required = param
            .get("required")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let param_type = param
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let value = build_query_param_value_typed(typed_summary, missing_ref, name, param_type);
        match value {
            Some(value) => {
                inputs.insert(name.to_string(), value);
            }
            None if required => return None,
            None => {}
        }
    }
    Some(inputs)
}

fn diagnose_param_build_failure_typed(
    typed_summary: Option<&StateSummary>,
    query_detail: &Value,
    missing_ref: &str,
) -> Vec<String> {
    let mut diag = Vec::new();
    let Some(params) = query_detail.get("params").and_then(Value::as_array) else {
        diag.push("no params array in query detail".to_string());
        return diag;
    };
    for param in params {
        let name = match param.get("name").and_then(Value::as_str) {
            Some(n) => n.trim(),
            None => {
                diag.push("param missing name field".to_string());
                continue;
            }
        };
        if name.is_empty() {
            continue;
        }
        let required = param
            .get("required")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let param_type = param
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let value = build_query_param_value_typed(typed_summary, missing_ref, name, param_type);
        if value.is_none() && required {
            diag.push(format!(
                "required param '{}' (type={}) unresolvable for missing_ref='{}'",
                name, param_type, missing_ref
            ));
        }
    }
    diag
}

fn build_query_param_value_typed(
    typed_summary: Option<&StateSummary>,
    missing_ref: &str,
    param_name: &str,
    param_type: &str,
) -> Option<Value> {
    if let Some(candidate) =
        select_best_query_param_ref_candidate(typed_summary, missing_ref, param_name, param_type)
    {
        return Some(encode_query_param_ref_binding(param_type, &candidate));
    }
    for slot in query_param_fallback_slots(missing_ref, param_name, param_type) {
        if let Some(value) =
            super::resolve_static_input_value_for_slot_typed(typed_summary, None, slot.as_str())
        {
            return Some(encode_query_param_literal_binding(param_type, value));
        }
    }
    None
}

#[cfg(test)]
fn build_query_param_value(
    state_summary: Option<&Value>,
    missing_ref: &str,
    param_name: &str,
    param_type: &str,
) -> Option<Value> {
    if let Some(candidate) = select_best_query_param_ref_candidate_raw(
        state_summary,
        missing_ref,
        param_name,
        param_type,
    ) {
        return Some(encode_query_param_ref_binding(param_type, &candidate));
    }
    for slot in query_param_fallback_slots(missing_ref, param_name, param_type) {
        if let Some(value) =
            super::resolve_static_input_value_for_slot_typed(None, state_summary, slot.as_str())
        {
            return Some(encode_query_param_literal_binding(param_type, value));
        }
    }
    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParamBindingType {
    Address,
    Asset,
    Numeric,
    Boolean,
    Chain,
    Text,
    Unknown,
}

#[derive(Debug, Clone)]
struct QueryParamBindingRequirement {
    normalized_key: String,
    tokens: Vec<String>,
    expected_type: ParamBindingType,
}

#[derive(Debug, Clone)]
struct QueryParamBindingCandidate {
    reference: String,
    normalized_key: String,
    tokens: Vec<String>,
    value_type: ParamBindingType,
    source_priority: u16,
}

fn select_best_query_param_ref_candidate(
    typed_summary: Option<&StateSummary>,
    missing_ref: &str,
    param_name: &str,
    param_type: &str,
) -> Option<QueryParamBindingCandidate> {
    let requirement = query_param_binding_requirement(missing_ref, param_name, param_type);
    let mut scored = collect_query_param_binding_candidates_typed(typed_summary)
        .into_iter()
        .filter_map(|candidate| {
            score_query_param_candidate(&requirement, &candidate).map(|score| (score, candidate))
        })
        .collect::<Vec<_>>();
    if scored.is_empty() {
        return None;
    }
    scored.sort_by(|left, right| right.0.cmp(&left.0));
    let best = scored.first()?;
    if best.0 < 55 {
        return None;
    }
    let second_score = scored.get(1).map(|item| item.0).unwrap_or_default();
    if best.0.saturating_sub(second_score) < 10 {
        return None;
    }
    Some(best.1.clone())
}

fn query_param_binding_requirement(
    missing_ref: &str,
    param_name: &str,
    param_type: &str,
) -> QueryParamBindingRequirement {
    let mut tokens = super::heuristics::semantic_tokens(param_name);
    if let Some(slot) = input_normalize::normalize_input_slot_key(missing_ref) {
        for token in super::heuristics::semantic_tokens(slot.as_str()) {
            if !tokens.contains(&token) {
                tokens.push(token);
            }
        }
    }
    let normalized_key = super::heuristics::normalize_semantic_key(param_name);
    QueryParamBindingRequirement {
        normalized_key,
        tokens,
        expected_type: infer_param_binding_type(param_name, param_type),
    }
}

#[cfg(test)]
fn select_best_query_param_ref_candidate_raw(
    state_summary: Option<&Value>,
    missing_ref: &str,
    param_name: &str,
    param_type: &str,
) -> Option<QueryParamBindingCandidate> {
    let requirement = query_param_binding_requirement(missing_ref, param_name, param_type);
    let mut scored = collect_query_param_binding_candidates_raw(state_summary)
        .into_iter()
        .filter_map(|candidate| {
            score_query_param_candidate(&requirement, &candidate).map(|score| (score, candidate))
        })
        .collect::<Vec<_>>();
    if scored.is_empty() {
        return None;
    }
    scored.sort_by(|left, right| right.0.cmp(&left.0));
    let best = scored.first()?;
    if best.0 < 55 {
        return None;
    }
    let second_score = scored.get(1).map(|item| item.0).unwrap_or_default();
    if best.0.saturating_sub(second_score) < 10 {
        return None;
    }
    Some(best.1.clone())
}

fn collect_query_param_binding_candidates_typed(
    typed_summary: Option<&StateSummary>,
) -> Vec<QueryParamBindingCandidate> {
    let mut out = Vec::<QueryParamBindingCandidate>::new();
    let facts_typed = typed_summary.and_then(|summary| summary.input_store_facts());
    let meta_typed = typed_summary.and_then(|summary| summary.input_store_meta());
    collect_input_namespace_binding_candidates(&mut out, facts_typed, meta_typed, false);
    out
}

#[cfg(test)]
fn collect_query_param_binding_candidates_raw(
    state_summary: Option<&Value>,
) -> Vec<QueryParamBindingCandidate> {
    let mut out = Vec::<QueryParamBindingCandidate>::new();
    let facts_value = state_summary
        .and_then(|summary| summary.pointer("/input_store/facts"))
        .and_then(Value::as_object);
    let meta_value = state_summary
        .and_then(|summary| summary.pointer("/input_store/meta"))
        .and_then(Value::as_object);
    collect_input_namespace_binding_candidates(&mut out, facts_value, meta_value, false);
    out
}

fn collect_input_namespace_binding_candidates(
    out: &mut Vec<QueryParamBindingCandidate>,
    facts: Option<&serde_json::Map<String, Value>>,
    meta: Option<&serde_json::Map<String, Value>>,
    prefixed_inputs: bool,
) {
    let Some(facts) = facts else {
        return;
    };
    for (raw_key, raw_value) in facts {
        let normalized_slot = if prefixed_inputs {
            raw_key
                .strip_prefix("inputs.")
                .and_then(input_normalize::normalize_input_slot_key)
        } else {
            input_normalize::normalize_input_slot_key(raw_key)
        };
        let Some(slot) = normalized_slot else {
            continue;
        };
        let value = extract_input_value(raw_value);
        let meta_key = if prefixed_inputs {
            format!("inputs.{slot}")
        } else {
            slot.clone()
        };
        if !prefixed_inputs && !input_store_meta_allows_slot(meta, meta_key.as_str()) {
            continue;
        }
        let source_priority = meta
            .and_then(|map| map.get(meta_key.as_str()))
            .and_then(|entry| entry.get("source_priority"))
            .and_then(Value::as_u64)
            .unwrap_or(60)
            .min(u16::MAX as u64) as u16;
        push_query_param_binding_candidate(
            out,
            format!("inputs.{slot}"),
            value.clone(),
            source_priority,
        );
        if let Some(address) = value.get("address") {
            let candidate_ref = format!("inputs.{slot}.address");
            push_query_param_binding_candidate(
                out,
                candidate_ref,
                address.clone(),
                source_priority,
            );
        }
    }
}

fn input_store_meta_allows_slot(meta: Option<&serde_json::Map<String, Value>>, slot: &str) -> bool {
    let Some(meta) = meta else {
        return true;
    };
    if let Some(entry) = meta
        .get(slot)
        .or_else(|| value_at_dotted_path_object(meta, slot))
    {
        return meta_entry_has_any_source(entry);
    }
    let prefix = format!("{slot}.");
    let mut saw_descendant = false;
    let mut has_non_query_descendant = false;
    for (key, entry) in meta {
        if !key.starts_with(prefix.as_str()) {
            continue;
        }
        saw_descendant = true;
        if meta_entry_has_any_source(entry) {
            has_non_query_descendant = true;
            break;
        }
    }
    if !saw_descendant {
        return true;
    }
    has_non_query_descendant
}

fn meta_entry_has_any_source(entry: &Value) -> bool {
    if let Some(source) = entry.get("source").and_then(Value::as_str) {
        return !source.trim().is_empty();
    }
    entry
        .as_object()
        .is_none_or(|object| object.values().any(meta_entry_has_any_source))
}

fn value_at_dotted_path_object<'a>(
    map: &'a serde_json::Map<String, Value>,
    dotted: &str,
) -> Option<&'a Value> {
    let mut segments = dotted.split('.').filter(|part| !part.is_empty());
    let first = segments.next()?;
    let mut current = map.get(first)?;
    for segment in segments {
        current = current.get(segment)?;
    }
    Some(current)
}

fn push_query_param_binding_candidate(
    out: &mut Vec<QueryParamBindingCandidate>,
    reference: String,
    value: Value,
    source_priority: u16,
) {
    let normalized_key = super::heuristics::normalize_semantic_key(reference.as_str());
    let tokens = super::heuristics::semantic_tokens(reference.as_str());
    if tokens.is_empty() {
        return;
    }
    out.push(QueryParamBindingCandidate {
        reference,
        normalized_key,
        tokens,
        value_type: infer_value_binding_type(&value),
        source_priority,
    });
}

fn score_query_param_candidate(
    requirement: &QueryParamBindingRequirement,
    candidate: &QueryParamBindingCandidate,
) -> Option<u16> {
    if !binding_type_compatible(requirement.expected_type, candidate.value_type) {
        return None;
    }
    let mut score = candidate.source_priority.min(100) / 4;
    if requirement.normalized_key == candidate.normalized_key {
        score = score.saturating_add(170);
    }
    let overlap = semantic_overlap(requirement.tokens.as_slice(), candidate.tokens.as_slice());
    if overlap.shared_total == 0 {
        return None;
    }
    score = score.saturating_add((overlap.shared_non_generic as u16).saturating_mul(30));
    score = score.saturating_add((overlap.shared_total as u16).saturating_mul(8));
    if requirement.expected_type == candidate.value_type {
        score = score.saturating_add(22);
    }
    if requirement.expected_type == ParamBindingType::Address
        && candidate.reference.ends_with(".address")
    {
        score = score.saturating_add(16);
    }
    if requirement.expected_type == ParamBindingType::Asset
        && candidate.value_type == ParamBindingType::Asset
    {
        score = score.saturating_add(16);
    }
    Some(score)
}

#[derive(Default)]
struct SemanticOverlap {
    shared_total: usize,
    shared_non_generic: usize,
}

fn semantic_overlap(left: &[String], right: &[String]) -> SemanticOverlap {
    let mut overlap = SemanticOverlap::default();
    let right_set = right
        .iter()
        .map(|token| token.as_str())
        .collect::<BTreeSet<_>>();
    for token in left {
        if !right_set.contains(token.as_str()) {
            continue;
        }
        overlap.shared_total = overlap.shared_total.saturating_add(1);
        if !super::heuristics::is_generic_semantic_token(token.as_str()) {
            overlap.shared_non_generic = overlap.shared_non_generic.saturating_add(1);
        }
    }
    overlap
}

fn encode_query_param_ref_binding(
    param_type: &str,
    candidate: &QueryParamBindingCandidate,
) -> Value {
    if param_type.eq_ignore_ascii_case("asset")
        && (candidate.value_type == ParamBindingType::Address
            || candidate.reference.ends_with(".address"))
    {
        return json!({ "address": { "ref": candidate.reference } });
    }
    json!({ "ref": candidate.reference })
}

fn encode_query_param_literal_binding(param_type: &str, value: Value) -> Value {
    if param_type.eq_ignore_ascii_case("asset") {
        if value.is_object() {
            return value;
        }
        return json!({ "address": value });
    }
    value
}

fn query_param_fallback_slots(
    missing_ref: &str,
    param_name: &str,
    param_type: &str,
) -> Vec<String> {
    let mut slots = BTreeSet::<String>::new();
    if let Some(slot) = input_normalize::normalize_input_slot_key(param_name) {
        push_query_param_fallback_slot(&mut slots, slot.as_str());
        if param_type.eq_ignore_ascii_case("asset") || param_type.eq_ignore_ascii_case("address") {
            push_query_param_fallback_slot(&mut slots, format!("{slot}.address").as_str());
        }
    }
    if let Some(slot) = input_normalize::normalize_input_slot_key(missing_ref) {
        push_query_param_fallback_slot(&mut slots, slot.as_str());
        if let Some((prefix, _)) = slot.rsplit_once('.') {
            push_query_param_fallback_slot(&mut slots, prefix);
            if param_type.eq_ignore_ascii_case("asset")
                || param_type.eq_ignore_ascii_case("address")
            {
                push_query_param_fallback_slot(&mut slots, format!("{prefix}.address").as_str());
            }
        }
    }
    slots.into_iter().collect::<Vec<_>>()
}

fn push_query_param_fallback_slot(slots: &mut BTreeSet<String>, slot: &str) {
    let normalized = slot.trim();
    if normalized.is_empty() {
        return;
    }
    slots.insert(normalized.to_string());
    for alias in super::heuristics::static_input_alias_slots(normalized) {
        slots.insert(alias);
    }
}

fn infer_param_binding_type(param_name: &str, param_type: &str) -> ParamBindingType {
    if param_type.eq_ignore_ascii_case("asset") {
        return ParamBindingType::Asset;
    }
    if param_type.eq_ignore_ascii_case("address") {
        return ParamBindingType::Address;
    }
    if param_type.eq_ignore_ascii_case("bool") || param_type.eq_ignore_ascii_case("boolean") {
        return ParamBindingType::Boolean;
    }
    if matches!(
        param_type.to_ascii_lowercase().as_str(),
        "uint8"
            | "uint16"
            | "uint32"
            | "uint64"
            | "uint128"
            | "uint256"
            | "int8"
            | "int16"
            | "int32"
            | "int64"
            | "int128"
            | "int256"
            | "number"
    ) {
        return ParamBindingType::Numeric;
    }
    let tokens = super::heuristics::semantic_tokens(param_name);
    if tokens
        .iter()
        .any(|token| matches!(token.as_str(), "address" | "owner" | "wallet" | "recipient"))
    {
        return ParamBindingType::Address;
    }
    if tokens.iter().any(|token| {
        matches!(
            token.as_str(),
            "amount" | "decimals" | "threshold" | "limit"
        )
    }) {
        return ParamBindingType::Numeric;
    }
    if tokens
        .iter()
        .any(|token| matches!(token.as_str(), "chain" | "chainid" | "chainref"))
    {
        return ParamBindingType::Chain;
    }
    ParamBindingType::Unknown
}

fn infer_value_binding_type(value: &Value) -> ParamBindingType {
    match value {
        Value::Bool(_) => ParamBindingType::Boolean,
        Value::Number(_) => ParamBindingType::Numeric,
        Value::String(raw) => {
            let text = raw.trim();
            if super::heuristics::is_evm_address(text) {
                return ParamBindingType::Address;
            }
            if text.starts_with("eip155:") {
                return ParamBindingType::Chain;
            }
            if text.parse::<f64>().is_ok() {
                return ParamBindingType::Numeric;
            }
            ParamBindingType::Text
        }
        Value::Object(object) => {
            if object.contains_key("address") {
                ParamBindingType::Asset
            } else {
                ParamBindingType::Unknown
            }
        }
        _ => ParamBindingType::Unknown,
    }
}

fn binding_type_compatible(expected: ParamBindingType, actual: ParamBindingType) -> bool {
    expected == ParamBindingType::Unknown
        || actual == ParamBindingType::Unknown
        || expected == actual
        || (expected == ParamBindingType::Asset && actual == ParamBindingType::Address)
        || (expected == ParamBindingType::Numeric && actual == ParamBindingType::Text)
        || (expected == ParamBindingType::Boolean && actual == ParamBindingType::Text)
}

fn extract_input_value(value: &Value) -> Value {
    value
        .as_object()
        .and_then(|object| object.get("value"))
        .cloned()
        .unwrap_or_else(|| value.clone())
}

fn query_autofill_chain_scope_typed(
    typed_summary: Option<&StateSummary>,
    query_detail: &Value,
) -> Vec<String> {
    let mut explicit = query_detail
        .get("execution_chains")
        .and_then(Value::as_array)
        .into_iter()
        .flat_map(|items| items.iter())
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .filter(|value| !value.ends_with(":*"))
        .map(str::to_string)
        .collect::<Vec<_>>();
    if explicit.is_empty() {
        if let Some(chain) = query_autofill_runtime_chain_scope_typed(typed_summary) {
            explicit.push(chain);
        }
    }
    if explicit.is_empty() {
        explicit.push("eip155:1".to_string());
    }
    explicit
}

#[cfg(test)]
fn query_autofill_chain_scope(state_summary: Option<&Value>, query_detail: &Value) -> Vec<String> {
    let mut explicit = query_detail
        .get("execution_chains")
        .and_then(Value::as_array)
        .into_iter()
        .flat_map(|items| items.iter())
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .filter(|value| !value.ends_with(":*"))
        .map(str::to_string)
        .collect::<Vec<_>>();
    if explicit.is_empty() {
        for slot in ["chain", "chain_id", "chain_ref"] {
            let Some(value) =
                super::resolve_static_input_value_for_slot_typed(None, state_summary, slot)
            else {
                continue;
            };
            let Some(chain) = value.as_str().map(str::trim).filter(|raw| !raw.is_empty()) else {
                continue;
            };
            if chain.ends_with(":*") {
                continue;
            }
            explicit.push(chain.to_string());
            break;
        }
    }
    if explicit.is_empty() {
        explicit.push("eip155:1".to_string());
    }
    explicit
}

fn query_autofill_runtime_chain_scope_typed(
    typed_summary: Option<&StateSummary>,
) -> Option<String> {
    for slot in ["chain", "chain_id", "chain_ref"] {
        let Some(value) =
            super::resolve_static_input_value_for_slot_typed(typed_summary, None, slot)
        else {
            continue;
        };
        let Some(chain) = value.as_str().map(str::trim).filter(|raw| !raw.is_empty()) else {
            continue;
        };
        if chain.ends_with(":*") {
            continue;
        }
        return Some(chain.to_string());
    }
    if let Some(summary) = typed_summary {
        let view = summary.runtime_facts_view();
        for key in ["facts.chain", "facts.chain_id", "facts.chain_ref"] {
            if let Some(chain) = view
                .fact(key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|raw| !raw.is_empty() && !raw.ends_with(":*"))
            {
                return Some(chain.to_string());
            }
        }
    }
    None
}

fn extract_query_output_value(
    runtime: &Value,
    node_id: &str,
    missing_ref: &str,
    query_candidate: &Value,
) -> Option<Value> {
    let escaped = node_id.replace('~', "~0").replace('/', "~1");
    let outputs = runtime
        .pointer(format!("/nodes/{escaped}/outputs").as_str())
        .and_then(Value::as_object)?;
    for field in query_candidate
        .get("matched_return_fields")
        .and_then(Value::as_array)
        .into_iter()
        .flat_map(|items| items.iter())
        .filter_map(Value::as_str)
    {
        if let Some(value) = outputs.get(field) {
            return Some(value.clone());
        }
    }
    let leaf = missing_ref
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .trim()
        .to_string();
    if let Some(value) = outputs.get(leaf.as_str()) {
        return Some(value.clone());
    }
    if outputs.len() == 1 {
        return outputs.values().next().cloned();
    }
    None
}

fn set_runtime_node_output_value(
    runtime: &mut Value,
    step_id: &str,
    field_path: &str,
    value: Value,
) {
    if step_id.trim().is_empty() || field_path.trim().is_empty() {
        return;
    }
    if !runtime.is_object() {
        *runtime = Value::Object(Map::new());
    }
    let Some(root) = runtime.as_object_mut() else {
        return;
    };
    let nodes = root
        .entry("nodes".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if !nodes.is_object() {
        *nodes = Value::Object(Map::new());
    }
    let Some(nodes_obj) = nodes.as_object_mut() else {
        return;
    };
    let node_id = resolve_runtime_node_id_for_step(nodes_obj, step_id);
    let node_entry = nodes_obj
        .entry(node_id)
        .or_insert_with(|| Value::Object(Map::new()));
    if !node_entry.is_object() {
        *node_entry = Value::Object(Map::new());
    }
    let Some(node_obj) = node_entry.as_object_mut() else {
        return;
    };
    let outputs = node_obj
        .entry("outputs".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if !outputs.is_object() {
        *outputs = Value::Object(Map::new());
    }
    let Some(outputs_obj) = outputs.as_object_mut() else {
        return;
    };
    let segments = field_path
        .split('.')
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    if segments.is_empty() {
        return;
    }
    set_nested_object_value(outputs_obj, segments.as_slice(), value);
}

fn resolve_runtime_node_id_for_step(
    nodes_obj: &serde_json::Map<String, Value>,
    step_id: &str,
) -> String {
    if nodes_obj.contains_key(step_id) {
        return step_id.to_string();
    }
    for node_id in nodes_obj.keys() {
        if node_id
            .rsplit_once('/')
            .map(|(_, suffix)| suffix == step_id)
            .unwrap_or(false)
            || node_id
                .rsplit_once("__")
                .map(|(_, suffix)| suffix == step_id)
                .unwrap_or(false)
        {
            return node_id.to_string();
        }
    }
    format!("autofill/{step_id}")
}

fn set_nested_object_value(root: &mut Map<String, Value>, segments: &[&str], value: Value) {
    if segments.is_empty() {
        return;
    }
    if segments.len() == 1 {
        root.insert(segments[0].to_string(), value);
        return;
    }
    let child = root
        .entry(segments[0].to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if !child.is_object() {
        *child = Value::Object(Map::new());
    }
    if let Some(child_obj) = child.as_object_mut() {
        set_nested_object_value(child_obj, &segments[1..], value);
    }
}

fn append_query_autofill_attempt(state: &mut EngineRunnerState, attempt: &Value) {
    let Some(agent) = state
        .runtime
        .get_mut("agent")
        .and_then(Value::as_object_mut)
    else {
        return;
    };
    let section = agent
        .entry("missing_input_autofill".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if !section.is_object() {
        *section = Value::Object(Map::new());
    }
    let Some(section_object) = section.as_object_mut() else {
        return;
    };
    let attempts = section_object
        .entry("query_attempts".to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    if !attempts.is_array() {
        *attempts = Value::Array(Vec::new());
    }
    if let Some(items) = attempts.as_array_mut() {
        items.push(attempt.clone());
    }
}

fn push_query_failure_reason(
    failure_reasons: &mut BTreeMap<String, Vec<String>>,
    missing_ref: &str,
    reason: QueryAutofillFailureReason,
) {
    failure_reasons
        .entry(missing_ref.to_string())
        .or_default()
        .push(reason.as_str().to_string());
}

fn record_query_autofill_round_stats(
    state: &mut EngineRunnerState,
    phase_hint: &str,
    scope_id: &str,
    stats: &QueryAutofillRoundStats,
) {
    let round = json!({
        "schema": "ais-agent-query-autofill-round/0.0.1",
        "phase_hint": phase_hint,
        "scope_id": scope_id,
        "round": stats.round,
        "max_rounds": HOST_QUERY_AUTOFILL_MAX_ROUNDS,
        "attempts_total": stats.attempts_total,
        "max_total_attempts": HOST_QUERY_AUTOFILL_MAX_TOTAL_ATTEMPTS,
        "max_attempts_per_ref": HOST_QUERY_AUTOFILL_MAX_ATTEMPTS_PER_REF,
        "empty_streak_fuse": HOST_QUERY_AUTOFILL_EMPTY_STREAK_FUSE,
        "terminal_reason": stats.terminal_reason,
        "hard_fail_type": stats.hard_fail_type,
        "resolved_refs": stats.resolved_refs,
        "unresolved_refs": stats.unresolved_refs,
        "failure_reasons": stats.failure_reasons,
    });
    let Some(agent) = state
        .runtime
        .get_mut("agent")
        .and_then(Value::as_object_mut)
    else {
        return;
    };
    let section = agent
        .entry("missing_input_autofill".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if !section.is_object() {
        *section = Value::Object(Map::new());
    }
    if let Some(object) = section.as_object_mut() {
        object.insert("query_autofill_round".to_string(), round);
    }
}

fn stable_scope_slug(raw: &str) -> String {
    let mut out = String::new();
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push('_');
        }
    }
    out.trim_matches('_').to_string()
}

fn question_missing_refs(questions: &[Value]) -> Vec<String> {
    super::super::missing_registry::collect_question_refs(questions)
}

#[cfg(test)]
#[path = "../tests/missing_resolution_module.rs"]
mod tests;
