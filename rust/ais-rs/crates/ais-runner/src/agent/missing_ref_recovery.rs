use super::candidates::CandidateContext;
use super::input_normalize;
use super::orchestrator::SegmentedAgentContext;
use crate::cli::AgentCommand;
use ais_engine::{run_plan_once, DefaultSolver, EngineRunnerOptions, EngineRunnerState, RouterExecutor};
use ais_sdk::documents::PlanSketchSegment;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};

const HOST_QUERY_AUTOFILL_MAX_TOTAL_ATTEMPTS: usize = 6;
const HOST_QUERY_AUTOFILL_MAX_ATTEMPTS_PER_REF: usize = 2;
const HOST_QUERY_AUTOFILL_EMPTY_STREAK_FUSE: usize = 2;
const HOST_QUERY_AUTOFILL_MAX_ROUNDS: usize = 3;
const RECOVERY_MAX_NO_PROGRESS_ROUNDS: usize = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RecoveryOutcome {
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

impl RecoveryOutcome {
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
pub(super) fn preserve_autofill_context(previous_error: Option<&Value>, payload: &mut Value) {
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
pub(super) fn split_query_recoverable_questions(
    candidate_context: &CandidateContext,
    questions: &[Value],
    limit_per_ref: usize,
) -> (Vec<Value>, Vec<Value>) {
    let missing_refs = question_missing_refs(questions);
    if missing_refs.is_empty() {
        return (Vec::new(), questions.to_vec());
    }
    let resolution =
        super::intent_segmented::resolve_missing_facts_for_refs(candidate_context, &missing_refs, limit_per_ref);
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
        let Some(slot) = input_normalize::normalize_missing_input_ref(id) else {
            unresolved.push(question.clone());
            continue;
        };
        let canonical_ref = format!("inputs.{slot}");
        if recoverable_refs.contains(&canonical_ref) {
            recoverable.push(question.clone());
        } else {
            unresolved.push(question.clone());
        }
    }
    (recoverable, unresolved)
}

pub(super) fn query_recoverable_missing_refs(resolution_payload: &Value) -> BTreeSet<String> {
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

pub(super) fn selected_query_refs_from_missing_resolution(resolution_payload: &Value) -> Vec<String> {
    let mut refs = BTreeSet::<String>::new();
    for query_ref in resolution_payload
        .get("resolved")
        .and_then(Value::as_array)
        .into_iter()
        .flat_map(|items| items.iter())
        .filter_map(|item| {
            item.get("query_candidates")
                .and_then(Value::as_array)
                .and_then(|items| items.first())
                .and_then(|candidate| candidate.get("query_ref"))
                .and_then(Value::as_str)
        })
    {
        refs.insert(query_ref.to_string());
    }
    refs.into_iter().collect::<Vec<_>>()
}

pub(super) fn query_candidate_pool_from_missing_resolution(resolution_payload: &Value) -> Vec<Value> {
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

pub(super) fn payload_question_input_refs(payload: &Value) -> Vec<String> {
    let mut refs = BTreeSet::<String>::new();
    for question_id in payload
        .get("questions")
        .and_then(Value::as_array)
        .into_iter()
        .flat_map(|items| items.iter())
        .filter_map(|question| question.get("id").and_then(Value::as_str))
    {
        collect_missing_input_ref(question_id, None, &mut refs);
    }
    refs.into_iter().collect::<Vec<_>>()
}

pub(super) fn missing_required_input_refs(payload: &Value) -> Vec<String> {
    let mut refs = BTreeSet::<String>::new();
    for raw in payload
        .get("missing_refs")
        .and_then(Value::as_array)
        .into_iter()
        .flat_map(|items| items.iter())
    {
        if let Some(raw_ref) = raw.as_str() {
            collect_missing_input_ref(raw_ref, Some(raw), &mut refs);
            continue;
        }
        let reference = raw
            .get("ref")
            .or_else(|| raw.get("missing_ref"))
            .or_else(|| raw.get("path"))
            .and_then(Value::as_str);
        if let Some(reference) = reference {
            collect_missing_input_ref(reference, Some(raw), &mut refs);
        }
    }
    refs.into_iter().collect::<Vec<_>>()
}

pub(super) fn emit_missing_input_autofill_unresolved(
    trace_enabled: bool,
    state: &mut EngineRunnerState,
    phase_hint: &str,
    scope_id: &str,
    missing_refs: &[String],
    reason: &str,
) {
    super::runtime_store::record_runtime_agent_field(
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
    super::trace::emit(
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
    let attempt_trace_id = format!(
        "missing_ref_recovery:{phase_hint}:{scope_id}:{stage}:{attempt}"
    );
    super::runtime_store::record_runtime_agent_field(
        &mut state.runtime,
        "missing_ref_recovery",
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
    super::trace::emit(
        trace_enabled,
        phase_hint,
        "missing_ref_recovery_terminal",
        &[
            ("scope_id", scope_id.to_string()),
            ("stage", stage.to_string()),
            ("attempt", attempt.to_string()),
            ("terminal_reason", terminal_reason.to_string()),
            ("unresolved_refs", unresolved_refs.join(",")),
        ],
    );
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn recover_missing_refs(
    command: &AgentCommand,
    state: &mut EngineRunnerState,
    context: &mut SegmentedAgentContext,
    missing_input_payload: &Value,
    candidate_context: &CandidateContext,
    readonly_autofill_router: Option<&RouterExecutor>,
    scope_id: &str,
    done: bool,
    phase_hint: &'static str,
) -> RecoveryOutcome {
    let trace_enabled = command.verbose || command.verbose_llm;
    let mut missing_refs = missing_required_input_refs(missing_input_payload);
    if missing_refs.is_empty() {
        missing_refs = payload_question_input_refs(missing_input_payload);
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
        return RecoveryOutcome::Recovered;
    }

    let static_outcome = super::orchestrator::apply_static_missing_ref_refill(
        state,
        context,
        missing_refs.as_slice(),
        phase_hint,
        scope_id,
    );
    let mut recovered_refs = static_outcome.resolved_refs.clone();
    missing_refs.retain(|path| {
        !super::orchestrator::runtime_has_input_ref(context.state_summary().as_ref(), path)
    });

    let ambiguous_bindings = static_outcome
        .ambiguous_bindings
        .iter()
        .filter(|item| missing_refs.iter().any(|missing| missing == &item.missing_ref))
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
        super::runtime_store::record_runtime_agent_field(
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
        return RecoveryOutcome::Recovered;
    }

    super::trace::emit(
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
    let mut no_progress_rounds = 0usize;
    let mut terminal_reason = "no_query_candidates".to_string();
    let mut hard_fail_type = None::<String>;
    let mut query_stage_attempted = false;

    while !missing_refs.is_empty() && query_round < HOST_QUERY_AUTOFILL_MAX_ROUNDS {
        query_round = query_round.saturating_add(1);
        let resolution = super::intent_segmented::resolve_missing_facts_for_refs(
            candidate_context,
            missing_refs.as_slice(),
            3,
        );
        query_candidate_pool = query_candidate_pool_from_missing_resolution(&resolution);
        last_resolution = resolution.clone();
        let selected_query_refs = selected_query_refs_from_missing_resolution(&resolution);
        if selected_query_refs.is_empty() {
            terminal_reason = "no_query_candidates".to_string();
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
            &resolution,
            query_round,
        );
        for resolved in query_autofill_stats.resolved_refs.iter() {
            if !recovered_refs.contains(resolved) {
                recovered_refs.push(resolved.clone());
            }
        }
        missing_refs = before_round_missing
            .into_iter()
            .filter(|path| {
                !super::orchestrator::runtime_has_input_ref(context.state_summary().as_ref(), path)
            })
            .collect::<Vec<_>>();
        if missing_refs.len() < before_round_missing_len {
            no_progress_rounds = 0;
        } else {
            no_progress_rounds = no_progress_rounds.saturating_add(1);
        }
        terminal_reason = query_autofill_stats.terminal_reason.clone();
        if query_autofill_stats.hard_fail_type.is_some() {
            hard_fail_type = query_autofill_stats.hard_fail_type;
            break;
        }
        if missing_refs.is_empty() {
            break;
        }
        if no_progress_rounds >= RECOVERY_MAX_NO_PROGRESS_ROUNDS {
            break;
        }
    }

    if missing_refs.is_empty() {
        super::runtime_store::record_runtime_agent_field(
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
        return RecoveryOutcome::Recovered;
    }

    let adjudicate_retry_key = format!("binding_adjudicate:{phase_hint}:{scope_id}");
    if (!ambiguous_bindings.is_empty() || query_stage_attempted || !query_candidate_pool.is_empty())
        && !context.has_compile_autofill_attempt(adjudicate_retry_key.as_str())
    {
        context.mark_compile_autofill_attempt(adjudicate_retry_key);
        let available_input_refs =
            super::orchestrator::available_input_ref_catalog(context.state_summary().as_ref());
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
                    "query_candidate_pool": query_candidate_pool,
                    "resolver": last_resolution,
                    "query_rounds": query_round,
                }),
            );
        }
        context.set_previous_error_and_refresh(state, done, previous_error);
        super::runtime_store::record_runtime_agent_field(
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
        return RecoveryOutcome::RetryScheduled;
    }

    if terminal_reason.is_empty() {
        terminal_reason = "no_query_candidates".to_string();
    }
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
        return RecoveryOutcome::ExhaustedUnavailable {
            missing_refs,
            reason: terminal_reason,
        };
    }
    RecoveryOutcome::NeedUserInput {
        missing_refs,
        reason: terminal_reason,
    }
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
    round: usize,
) -> QueryAutofillRoundStats {
    let mut stats = QueryAutofillRoundStats {
        round,
        ..QueryAutofillRoundStats::default()
    };
    let trace_enabled = command.verbose || command.verbose_llm;
    let Some(resolved_items) = resolution_payload.get("resolved").and_then(Value::as_array) else {
        stats.terminal_reason = "resolver_empty".to_string();
        record_query_autofill_round_stats(state, phase_hint, scope_id, &stats);
        return stats;
    };
    if readonly_autofill_router.is_none() {
        for item in resolved_items {
            let Some(missing_ref) = item.get("missing_ref").and_then(Value::as_str) else {
                continue;
            };
            stats.unresolved_refs.push(missing_ref.to_string());
            push_query_failure_reason(
                &mut stats.failure_reasons,
                missing_ref,
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
    'outer: for item in resolved_items {
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
        let Some(missing_ref) = item.get("missing_ref").and_then(Value::as_str) else {
            continue;
        };
        let per_ref_attempts = stats
            .per_missing_ref_attempts
            .entry(missing_ref.to_string())
            .or_default();
        let query_candidates = item
            .get("query_candidates")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if query_candidates.is_empty() {
            stats.unresolved_refs.push(missing_ref.to_string());
            continue;
        }
        let mut resolved_this_ref = false;
        for query_candidate in query_candidates {
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
            let query_ref = query_candidate
                .get("query_ref")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
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
            super::trace::emit(
                trace_enabled,
                phase_hint,
                "autofill_query_attempt_start",
                &[
                    ("scope_id", scope_id.to_string()),
                    ("missing_ref", missing_ref.to_string()),
                    ("query_ref", query_ref.clone()),
                    ("attempt", stats.attempts_total.to_string()),
                ],
            );

            match execute_query_autofill_candidate(
                state,
                context,
                candidate_context,
                router,
                missing_ref,
                &query_candidate,
            ) {
                Ok(Some(value)) => {
                    stats.resolved_refs.push(missing_ref.to_string());
                    resolved_this_ref = true;
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
                    super::trace::emit(
                        trace_enabled,
                        phase_hint,
                        "autofill_query_attempt_end",
                        &[
                            ("scope_id", scope_id.to_string()),
                            ("missing_ref", missing_ref.to_string()),
                            ("query_ref", query_ref),
                            ("status", "resolved".to_string()),
                        ],
                    );
                    break;
                }
                Ok(None) => {
                    stats.empty_or_invalid_streak = stats.empty_or_invalid_streak.saturating_add(1);
                    push_query_failure_reason(
                        &mut stats.failure_reasons,
                        missing_ref,
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
                        stats.empty_or_invalid_streak =
                            stats.empty_or_invalid_streak.saturating_add(1);
                    } else {
                        stats.empty_or_invalid_streak = 0;
                    }
                    push_query_failure_reason(&mut stats.failure_reasons, missing_ref, reason);
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
        if !resolved_this_ref {
            stats.unresolved_refs.push(missing_ref.to_string());
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
        stats.hard_fail_type = Some(QueryAutofillFailureReason::RouterUnavailable.as_str().to_string());
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
    let step_inputs =
        build_query_autofill_step_inputs(context.state_summary().as_ref(), query_detail, missing_ref)
            .ok_or(QueryAutofillFailureReason::ParamBuildFailed)?;
    let segment = build_query_autofill_segment(context, query_ref, step_inputs)
        .ok_or(QueryAutofillFailureReason::ParamBuildFailed)?;
    let chain_scope = query_autofill_chain_scope(query_detail);
    let known_input_refs = super::known_input_refs_from_state_summary(context.state_summary().as_ref());
    let plan = super::compile_segment_plan_with_snapshot_hash(
        context.intent(),
        context.session().session_id.as_str(),
        context.session().cursor.as_str(),
        &segment,
        candidate_context,
        context.session().snapshot_hash.as_str(),
        chain_scope.as_slice(),
        known_input_refs.as_slice(),
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
    let Some(slot) = input_normalize::normalize_input_slot_key(missing_ref) else {
        return Err(QueryAutofillFailureReason::ParamBuildFailed);
    };
    input_normalize::set_runtime_input_value(&mut state.runtime, slot.as_str(), value.clone());
    let _ = super::upsert_store_value_with_source(
        context.input_store_mut(),
        slot.as_str(),
        value.clone(),
        super::input_store::InputValueLayer::Derived,
        "host.query_autofill",
        88,
        format!("autofill.query.{missing_ref}.{query_ref}"),
    );
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

fn build_query_autofill_step_inputs(
    state_summary: Option<&Value>,
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
        let param_type = param.get("type").and_then(Value::as_str).unwrap_or_default();
        let value = build_query_param_value(state_summary, missing_ref, name, param_type);
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

fn build_query_param_value(
    state_summary: Option<&Value>,
    missing_ref: &str,
    param_name: &str,
    param_type: &str,
) -> Option<Value> {
    if let Some(summary) = state_summary {
        if let Some(candidate) =
            select_best_query_param_ref_candidate(summary, missing_ref, param_name, param_type)
        {
            return Some(encode_query_param_ref_binding(param_type, &candidate));
        }
    }
    for slot in query_param_fallback_slots(missing_ref, param_name, param_type) {
        if let Some(value) =
            super::orchestrator::resolve_static_input_value_for_slot(state_summary, slot.as_str())
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
    summary: &Value,
    missing_ref: &str,
    param_name: &str,
    param_type: &str,
) -> Option<QueryParamBindingCandidate> {
    let requirement = query_param_binding_requirement(missing_ref, param_name, param_type);
    let mut scored = collect_query_param_binding_candidates(summary)
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
    let mut tokens = semantic_tokens(param_name);
    if let Some(slot) = input_normalize::normalize_input_slot_key(missing_ref) {
        for token in semantic_tokens(slot.as_str()) {
            if !tokens.contains(&token) {
                tokens.push(token);
            }
        }
    }
    let normalized_key = normalize_semantic_key(param_name);
    QueryParamBindingRequirement {
        normalized_key,
        tokens,
        expected_type: infer_param_binding_type(param_name, param_type),
    }
}

fn collect_query_param_binding_candidates(summary: &Value) -> Vec<QueryParamBindingCandidate> {
    let mut out = Vec::<QueryParamBindingCandidate>::new();
    let Some(facts) = summary.pointer("/input_store/facts").and_then(Value::as_object) else {
        return out;
    };
    let meta = summary.pointer("/input_store/meta").and_then(Value::as_object);
    for (raw_key, raw_value) in facts {
        let Some(slot) = input_normalize::normalize_input_slot_key(raw_key) else {
            continue;
        };
        let value = extract_input_value(raw_value);
        let source_priority = meta
            .and_then(|map| map.get(raw_key.as_str()))
            .and_then(|entry| entry.get("source_priority"))
            .and_then(Value::as_u64)
            .unwrap_or(60)
            .min(u16::MAX as u64) as u16;
        push_query_param_binding_candidate(
            &mut out,
            format!("inputs.{slot}"),
            value.clone(),
            source_priority,
        );
        if let Some(address) = value.get("address") {
            let candidate_ref = format!("inputs.{slot}.address");
            push_query_param_binding_candidate(
                &mut out,
                candidate_ref,
                address.clone(),
                source_priority,
            );
        }
    }
    out
}

fn push_query_param_binding_candidate(
    out: &mut Vec<QueryParamBindingCandidate>,
    reference: String,
    value: Value,
    source_priority: u16,
) {
    let normalized_key = normalize_semantic_key(reference.as_str());
    let tokens = semantic_tokens(reference.as_str());
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
        if !is_generic_semantic_token(token.as_str()) {
            overlap.shared_non_generic = overlap.shared_non_generic.saturating_add(1);
        }
    }
    overlap
}

fn encode_query_param_ref_binding(param_type: &str, candidate: &QueryParamBindingCandidate) -> Value {
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

fn query_param_fallback_slots(missing_ref: &str, param_name: &str, param_type: &str) -> Vec<String> {
    let mut slots = BTreeSet::<String>::new();
    if let Some(slot) = input_normalize::normalize_input_slot_key(param_name) {
        slots.insert(slot.clone());
        if param_type.eq_ignore_ascii_case("asset") || param_type.eq_ignore_ascii_case("address") {
            slots.insert(format!("{slot}.address"));
        }
    }
    if let Some(slot) = input_normalize::normalize_input_slot_key(missing_ref) {
        slots.insert(slot.clone());
        if let Some((prefix, _)) = slot.rsplit_once('.') {
            slots.insert(prefix.to_string());
            if param_type.eq_ignore_ascii_case("asset") || param_type.eq_ignore_ascii_case("address")
            {
                slots.insert(format!("{prefix}.address"));
            }
        }
    }
    slots.into_iter().collect::<Vec<_>>()
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
    let tokens = semantic_tokens(param_name);
    if tokens
        .iter()
        .any(|token| matches!(token.as_str(), "address" | "owner" | "wallet" | "recipient"))
    {
        return ParamBindingType::Address;
    }
    if tokens
        .iter()
        .any(|token| matches!(token.as_str(), "amount" | "decimals" | "threshold" | "limit"))
    {
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
            if is_evm_address(text) {
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

fn semantic_tokens(raw: &str) -> Vec<String> {
    raw.to_ascii_lowercase()
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|segment| !segment.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>()
}

fn normalize_semantic_key(raw: &str) -> String {
    semantic_tokens(raw).join("")
}

fn is_generic_semantic_token(token: &str) -> bool {
    matches!(token, "inputs" | "input" | "value" | "field" | "data" | "ref")
}

fn is_evm_address(value: &str) -> bool {
    value.len() == 42
        && value.starts_with("0x")
        && value
            .as_bytes()
            .iter()
            .skip(2)
            .all(|byte| byte.is_ascii_hexdigit())
}

fn query_autofill_chain_scope(query_detail: &Value) -> Vec<String> {
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
        explicit.push("eip155:1".to_string());
    }
    explicit
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

fn append_query_autofill_attempt(state: &mut EngineRunnerState, attempt: &Value) {
    let Some(agent) = state.runtime.get_mut("agent").and_then(Value::as_object_mut) else {
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
    let Some(agent) = state.runtime.get_mut("agent").and_then(Value::as_object_mut) else {
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
    let mut refs = BTreeSet::<String>::new();
    for id in questions
        .iter()
        .filter_map(|question| question.get("id").and_then(Value::as_str))
    {
        if let Some(slot) = input_normalize::normalize_missing_input_ref(id) {
            refs.insert(format!("inputs.{slot}"));
        }
    }
    refs.into_iter().collect::<Vec<_>>()
}

fn collect_missing_input_ref(raw: &str, metadata: Option<&Value>, missing_refs: &mut BTreeSet<String>) {
    if let Some(path) = input_normalize::normalize_missing_input_ref(raw) {
        for leaf in input_normalize::expand_missing_input_slot(path.as_str(), metadata) {
            missing_refs.insert(format!("inputs.{leaf}"));
        }
    }
}

#[cfg(test)]
#[path = "tests/missing_ref_recovery_module.rs"]
mod tests;
