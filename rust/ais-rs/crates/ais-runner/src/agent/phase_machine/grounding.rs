use super::super::orchestrator::SegmentedAgentContext;
use super::super::*;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};

const GROUND_INPUT_CONFIDENCE_THRESHOLD: u8 = 80;
const GROUND_FACT_CONFIDENCE_THRESHOLD: u8 = 65;
const INPUT_BINDABLE_SOURCE_OF_TRUTH: &str = "state_summary.input_store";
const INPUT_BINDABLE_REFS_PROJECTION_PATH: &str = "state_summary.input_registry.known_refs";

#[derive(Debug, Default)]
pub(crate) struct GroundingApplySummary {
    pub(crate) applied: Vec<String>,
    pub(crate) skipped_low_confidence: Vec<String>,
    pub(crate) deterministic_applied: Vec<String>,
    pub(crate) deterministic_skipped: Vec<String>,
    pub(crate) deterministic_conflicts: Vec<String>,
}

pub(crate) fn bootstrap_intent_grounding_if_needed<P: LlmProvider>(
    command: &AgentCommand,
    planner: &mut LlmSegmentedIntentPlanner<P>,
    state: &mut EngineRunnerState,
    context: &mut SegmentedAgentContext,
    candidate_context: &CandidateContext,
    readonly_autofill_router: Option<&ais_engine::RouterExecutor>,
    runtime_has_intent_grounding: bool,
) -> Result<bool, RunnerError> {
    let trace_enabled = command.verbose || command.verbose_llm;
    if runtime_has_intent_grounding {
        let ready = intent_grounding_ready_for_todos(state);
        super::super::trace::emit(
            trace_enabled,
            "grounding",
            "reuse_runtime_grounding",
            &[("ready_for_todos", ready.to_string())],
        );
        return Ok(ready);
    }
    let mut autofill_retry_budget = 1u8;
    loop {
        super::super::trace::emit(trace_enabled, "grounding", "planner_call_start", &[]);
        let draft_result = planner.ground_intent(IntentGroundingRequest {
            intent: context.intent().to_string(),
            session: context.session().clone(),
            state_summary: context.packed_summary().clone(),
            typed_summary: context.typed_summary().cloned(),
        });
        super::super::orchestrator::refresh_tool_memory_projection(context, planner, state);
        let is_retry_round = autofill_retry_budget == 0;
        match handle_grounding_draft(
            command,
            state,
            context,
            candidate_context,
            readonly_autofill_router,
            draft_result,
            is_retry_round,
        )? {
            GroundingDraftOutcome::Ready(ready) => return Ok(ready),
            outcome => match handle_grounding_retry_outcome(outcome, &mut autofill_retry_budget) {
                GroundingRetryAction::ReturnReady => return Ok(true),
                GroundingRetryAction::RetryWithTrace => {
                    super::super::trace::emit(
                        trace_enabled,
                        "grounding",
                        "autofill_retry",
                        &[("remaining_budget", autofill_retry_budget.to_string())],
                    );
                }
                GroundingRetryAction::RetrySilently => continue,
                GroundingRetryAction::StopNotReady => return Ok(false),
            },
        }
    }
}

enum GroundingDraftOutcome {
    Ready(bool),
    Retry {
        state_changed: bool,
        host_ready: bool,
    },
}

enum GroundingRetryAction {
    ReturnReady,
    RetryWithTrace,
    RetrySilently,
    StopNotReady,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GroundingFollowUpState {
    Actionable,
    NonActionable,
}

fn handle_grounding_retry_outcome(
    outcome: GroundingDraftOutcome,
    autofill_retry_budget: &mut u8,
) -> GroundingRetryAction {
    match outcome {
        GroundingDraftOutcome::Retry {
            state_changed,
            host_ready,
        } => {
            if host_ready {
                return GroundingRetryAction::ReturnReady;
            }
            if state_changed {
                return GroundingRetryAction::RetrySilently;
            }
            if *autofill_retry_budget == 0 {
                return GroundingRetryAction::StopNotReady;
            }
            *autofill_retry_budget = autofill_retry_budget.saturating_sub(1);
            GroundingRetryAction::RetryWithTrace
        }
        GroundingDraftOutcome::Ready(_) => GroundingRetryAction::StopNotReady,
    }
}

fn grounding_follow_up_state(
    questions: &[Value],
    missing_refs: &[String],
) -> GroundingFollowUpState {
    if questions.is_empty() && missing_refs.is_empty() {
        GroundingFollowUpState::NonActionable
    } else {
        GroundingFollowUpState::Actionable
    }
}

fn handle_grounding_draft(
    command: &AgentCommand,
    state: &mut EngineRunnerState,
    context: &mut SegmentedAgentContext,
    candidate_context: &CandidateContext,
    readonly_autofill_router: Option<&ais_engine::RouterExecutor>,
    draft_result: Result<IntentGroundingDraft, RunnerError>,
    is_retry_round: bool,
) -> Result<GroundingDraftOutcome, RunnerError> {
    let trace_enabled = command.verbose || command.verbose_llm;
    let draft = match draft_result {
        Ok(draft) => draft,
        Err(error) => {
            let error_message = error.to_string();
            if command.verbose_llm {
                eprintln!("[agent] intent grounding failed ({error}) (fallback continue)");
            }
            super::super::trace::emit(
                trace_enabled,
                "grounding",
                "planner_call_failed",
                &[("error", error_message.clone())],
            );
            super::super::runtime_store::record_runtime_agent_field(
                &mut state.runtime,
                "intent_grounding",
                json!({
                    "status":"fallback",
                    "ready_for_todos": true,
                    "reason_code": "planner_call_failed",
                    "message": error_message.as_str(),
                    "input_binding": grounding_input_binding_metadata(),
                }),
            );
            context.set_previous_error_and_refresh(
                state,
                false,
                super::super::grounding_phase_error_payload(
                    "planner_call_failed",
                    Some(error_message.as_str()),
                    &[],
                    &[],
                    context.completed_segments_u8(),
                ),
            );
            state.paused_reason = None;
            return Ok(GroundingDraftOutcome::Ready(true));
        }
    };

    match draft {
        IntentGroundingDraft::Proposed {
            summary,
            ready_for_todos,
            missing_refs,
            resolved_inputs,
            intent_facts,
            confidence,
            issues,
            questions,
        } => {
            let intent_text = context.intent().to_string();
            let apply_summary = apply_intent_grounding(
                state,
                context.input_store_mut(),
                &resolved_inputs,
                &intent_facts,
                &confidence,
                intent_text.as_str(),
            );
            if !apply_summary.deterministic_conflicts.is_empty() {
                super::super::trace::emit(
                    trace_enabled,
                    "grounding",
                    "deterministic_rule_conflict",
                    &[
                        (
                            "conflicts",
                            apply_summary.deterministic_conflicts.len().to_string(),
                        ),
                        ("policy", "rule_extracted_over_llm".to_string()),
                    ],
                );
            }
            let mut answered_questions = Map::new();
            let remaining_questions_raw = filter_unanswered_questions(
                questions.as_slice(),
                answered_questions.keys().collect::<Vec<_>>().as_slice(),
            );
            let (auto_answers, after_auto_answer_questions) = auto_answer_single_option_questions(
                remaining_questions_raw.as_slice(),
                &resolved_inputs,
                &intent_facts,
                context.input_store_mut(),
            );
            if !auto_answers.is_empty() {
                super::super::trace::emit(
                    trace_enabled,
                    "grounding",
                    "auto_answered_single_option_questions",
                    &[
                        ("count", auto_answers.len().to_string()),
                        (
                            "keys",
                            auto_answers.keys().cloned().collect::<Vec<_>>().join(","),
                        ),
                    ],
                );
                for (key, value) in &auto_answers {
                    answered_questions.insert(key.clone(), value.clone());
                }
            }
            let (query_recoverable_questions, remaining_questions) =
                super::super::missing_resolution::split_query_recoverable_questions(
                    candidate_context,
                    after_auto_answer_questions.as_slice(),
                    2,
                );
            let candidate = super::super::grounding_resolution::normalize_grounding_candidate(
                ready_for_todos,
                missing_refs.as_slice(),
                remaining_questions.as_slice(),
                &resolved_inputs,
                &intent_facts,
                &confidence,
                issues.as_slice(),
            );
            let resolution = super::super::grounding_resolution::reconcile_grounding_candidate(
                context.typed_summary(),
                &candidate,
            );
            if !query_recoverable_questions.is_empty() {
                super::super::trace::emit(
                    trace_enabled,
                    "grounding",
                    "query_recoverable_questions_filtered",
                    &[
                        ("count", query_recoverable_questions.len().to_string()),
                        ("remaining", remaining_questions.len().to_string()),
                    ],
                );
            }
            let ready = resolution.ready_for_todos;
            super::super::trace::emit(
                trace_enabled,
                "grounding",
                "draft_proposed",
                &[
                    (
                        "planner_ready_hint",
                        resolution.planner_ready_hint.to_string(),
                    ),
                    ("ready_for_todos", ready.to_string()),
                    (
                        "remaining_questions",
                        resolution.effective_questions.len().to_string(),
                    ),
                    (
                        "missing_refs",
                        resolution.effective_missing_refs.len().to_string(),
                    ),
                ],
            );
            record_grounding_proposed_runtime(
                state,
                summary.as_deref(),
                &candidate,
                &resolution,
                &answered_questions,
                query_recoverable_questions.as_slice(),
                &apply_summary,
            );
            context.refresh_state_summary(state, false);
            let mandatory_missing = collect_mandatory_grounding_missing_refs_host(
                &state.runtime,
                context.typed_summary(),
            );
            if !mandatory_missing.is_empty() {
                super::super::trace::emit(
                    trace_enabled,
                    "grounding",
                    "mandatory_missing_refs_detected",
                    &[("missing_refs", mandatory_missing.join(","))],
                );
            }
            if !ready {
                let mut payload_missing_refs = resolution.effective_missing_refs.clone();
                for mandatory_ref in &mandatory_missing {
                    if !payload_missing_refs.contains(mandatory_ref) {
                        payload_missing_refs.push(mandatory_ref.clone());
                    }
                }
                payload_missing_refs.sort();
                payload_missing_refs.dedup();
                if matches!(
                    grounding_follow_up_state(
                        resolution.effective_questions.as_slice(),
                        payload_missing_refs.as_slice()
                    ),
                    GroundingFollowUpState::NonActionable
                ) {
                    let payload = super::super::missing_input::payload_with_context(
                        Some("intent_grounding_missing_inputs"),
                        &[],
                        issues.as_slice(),
                        &[],
                        &[],
                        context.completed_segments_u8(),
                    );
                    super::super::runtime_store::record_runtime_agent_field(
                        &mut state.runtime,
                        "missing_required_input",
                        payload.clone(),
                    );
                    state.paused_reason = Some("missing_required_input".to_string());
                    context.set_previous_error_and_refresh(
                        state,
                        false,
                        super::super::grounding_phase_error_payload(
                            "missing_required_input",
                            Some("intent_grounding_missing_inputs"),
                            issues.as_slice(),
                            &[],
                            context.completed_segments_u8(),
                        ),
                    );
                    super::super::trace::emit(
                        trace_enabled,
                        "pause_resolution",
                        "paused_missing_required_input",
                        &[("phase_hint", "grounding".to_string())],
                    );
                    return Ok(GroundingDraftOutcome::Ready(false));
                }
                let payload = super::super::missing_input::payload_with_context(
                    Some("intent_grounding_missing_inputs"),
                    resolution.effective_questions.as_slice(),
                    &[],
                    payload_missing_refs.as_slice(),
                    payload_missing_refs.as_slice(),
                    context.completed_segments_u8(),
                );
                match super::super::phase_machine::pause::recover_missing_required_input_payload(
                    command,
                    state,
                    context,
                    candidate_context,
                    readonly_autofill_router,
                    &payload,
                    "grounding",
                    false,
                    "grounding",
                    false,
                    is_retry_round,
                )? {
                    super::super::phase_machine::pause::MissingRequiredInputRecoveryBackflow::Retry {
                        state_changed,
                        ..
                    } => {
                        context.refresh_state_summary(state, false);
                        let fast_resolution =
                            super::super::grounding_resolution::reconcile_grounding_candidate(
                                context.typed_summary(),
                                &candidate,
                            );
                        let fast_mandatory_missing = collect_mandatory_grounding_missing_refs_host(
                            &state.runtime,
                            context.typed_summary(),
                        );
                        let host_ready =
                            fast_resolution.ready_for_todos && fast_mandatory_missing.is_empty();
                        if host_ready {
                            record_grounding_proposed_runtime(
                                state,
                                summary.as_deref(),
                                &candidate,
                                &fast_resolution,
                                &answered_questions,
                                query_recoverable_questions.as_slice(),
                                &apply_summary,
                            );
                            state.paused_reason = None;
                            context.clear_previous_error_and_refresh(state, false);
                            super::super::trace::emit(
                                trace_enabled,
                                "grounding",
                                "post_recovery_fast_path_ready",
                                &[("source", "host_recovery".to_string())],
                            );
                            return Ok(GroundingDraftOutcome::Ready(true));
                        }
                        return Ok(GroundingDraftOutcome::Retry {
                            state_changed,
                            host_ready,
                        });
                    }
                    super::super::phase_machine::pause::MissingRequiredInputRecoveryBackflow::Paused => {
                        context.set_previous_error_and_refresh(
                            state,
                            false,
                            super::super::grounding_phase_error_payload(
                                "missing_required_input",
                                Some("intent_grounding_missing_inputs"),
                                &[],
                                resolution.effective_questions.as_slice(),
                                context.completed_segments_u8(),
                            ),
                        );
                        super::super::trace::emit(
                            trace_enabled,
                            "pause_resolution",
                            "paused_missing_required_input",
                            &[("phase_hint", "grounding".to_string())],
                        );
                        return Ok(GroundingDraftOutcome::Ready(false));
                    }
                }
            }
            if !mandatory_missing.is_empty() {
                let payload = super::super::missing_input::payload_with_context(
                    Some("grounding_mandatory_missing_facts"),
                    &[],
                    &[],
                    mandatory_missing.as_slice(),
                    mandatory_missing.as_slice(),
                    context.completed_segments_u8(),
                );
                let recovery_outcome =
                    super::super::missing_resolution::missing_resolution_recover_missing_refs(
                        command,
                        state,
                        context,
                        &payload,
                        candidate_context,
                        readonly_autofill_router,
                        "grounding_mandatory",
                        false,
                        "grounding",
                    );
                if recovery_outcome.should_retry_round() {
                    return Ok(GroundingDraftOutcome::Retry {
                        state_changed: true,
                        host_ready: false,
                    });
                }
            }
            state.paused_reason = None;
            context.clear_previous_error_and_refresh(state, false);
            super::super::trace::emit(trace_enabled, "grounding", "ready", &[]);
            Ok(GroundingDraftOutcome::Ready(true))
        }
        IntentGroundingDraft::Unavailable {
            reason_code,
            message,
            issues,
            questions,
            error_details,
        } => {
            if reason_code == "intent_aborted" {
                super::super::trace::emit(
                    trace_enabled,
                    "grounding",
                    "abort_intent",
                    &[("reason_code", reason_code.clone())],
                );
                super::super::runtime_store::record_runtime_agent_field(
                    &mut state.runtime,
                    "abort_intent",
                    json!({
                        "accepted": true,
                        "phase": "grounding",
                        "reason_code": reason_code,
                        "summary": message,
                        "evidence": error_details.as_ref().and_then(|value| value.get("evidence")).cloned().unwrap_or_else(|| json!({})),
                        "user_fix_hint": error_details.as_ref().and_then(|value| value.get("user_fix_hint")).cloned().unwrap_or(Value::Null),
                    }),
                );
                state.paused_reason = None;
                context.set_final_status(EngineRunStatus::Stopped);
                context.clear_previous_error_and_refresh(state, true);
                return Ok(GroundingDraftOutcome::Ready(false));
            }
            super::super::trace::emit(
                trace_enabled,
                "grounding",
                "draft_unavailable",
                &[
                    ("reason_code", reason_code.clone()),
                    ("questions", questions.len().to_string()),
                ],
            );
            if reason_code == "missing_required_input" {
                let payload = super::super::missing_input::payload_with_error_details(
                    message.as_deref(),
                    questions.as_slice(),
                    issues.as_slice(),
                    error_details.as_ref(),
                    context.completed_segments_u8(),
                );
                match super::super::phase_machine::pause::recover_missing_required_input_payload(
                    command,
                    state,
                    context,
                    candidate_context,
                    readonly_autofill_router,
                    &payload,
                    "grounding",
                    false,
                    "grounding",
                    false,
                    true,
                )? {
                    super::super::phase_machine::pause::MissingRequiredInputRecoveryBackflow::Retry {
                        state_changed,
                        answers,
                    } => {
                        let payload_missing_refs =
                            collect_grounding_payload_missing_refs(&payload);
                        if let Some(answers) = answers {
                            super::super::runtime_store::record_runtime_agent_field(
                                &mut state.runtime,
                                "intent_grounding",
                                json!({
                                    "status":"resolved_by_user_input",
                                    "ready_for_todos": true,
                                    "reason_code": reason_code,
                                    "answers": answers,
                                    "input_binding": grounding_input_binding_metadata(),
                                }),
                            );
                            context.refresh_state_summary(state, false);
                            super::super::trace::emit(
                                trace_enabled,
                                "pause_resolution",
                                "resolved_by_user_input",
                                &[("phase_hint", "grounding".to_string())],
                            );
                        }
                        context.refresh_state_summary(state, false);
                        let fast_missing_refs = collect_unresolved_grounding_payload_missing_refs(
                            &state.runtime,
                            context.typed_summary(),
                            payload_missing_refs.as_slice(),
                        );
                        let fast_mandatory_missing =
                            collect_mandatory_grounding_missing_refs_host(
                                &state.runtime,
                                context.typed_summary(),
                            );
                        if fast_missing_refs.is_empty() && fast_mandatory_missing.is_empty() {
                            record_grounding_unavailable_ready_runtime(
                                state,
                                reason_code.as_str(),
                                message.as_deref(),
                                issues.as_slice(),
                            );
                            state.paused_reason = None;
                            context.clear_previous_error_and_refresh(state, false);
                            super::super::trace::emit(
                                trace_enabled,
                                "grounding",
                                "post_recovery_fast_path_short_circuit",
                                &[("source", "unavailable_host_recovery".to_string())],
                            );
                        }
                        return Ok(GroundingDraftOutcome::Retry {
                            state_changed,
                            host_ready: fast_missing_refs.is_empty()
                                && fast_mandatory_missing.is_empty(),
                        });
                    }
                    super::super::phase_machine::pause::MissingRequiredInputRecoveryBackflow::Paused => {
                        super::super::runtime_store::record_runtime_agent_field(
                            &mut state.runtime,
                            "intent_grounding",
                            json!({
                                "status":"unavailable",
                                "ready_for_todos": false,
                                "reason_code": reason_code,
                                "message": message,
                                "issues": issues,
                                "questions": questions,
                                "input_binding": grounding_input_binding_metadata(),
                            }),
                        );
                        context.set_previous_error_and_refresh(
                            state,
                            false,
                            super::super::grounding_phase_error_payload(
                                "missing_required_input",
                                message.as_deref(),
                                issues.as_slice(),
                                questions.as_slice(),
                                context.completed_segments_u8(),
                            ),
                        );
                        super::super::trace::emit(
                            trace_enabled,
                            "pause_resolution",
                            "paused_missing_required_input",
                            &[("phase_hint", "grounding".to_string())],
                        );
                        return Ok(GroundingDraftOutcome::Ready(false));
                    }
                }
            }
            context.set_previous_error_and_refresh(
                state,
                false,
                super::super::grounding_phase_error_payload(
                    "unavailable",
                    message.as_deref(),
                    issues.as_slice(),
                    questions.as_slice(),
                    context.completed_segments_u8(),
                ),
            );
            Err(RunnerError::Llm(format!(
                "intent grounding unavailable reason_code={} message={} issues={} questions={}",
                reason_code,
                message.unwrap_or_default(),
                issues.len(),
                questions.len()
            )))
        }
        IntentGroundingDraft::Invalid {
            reason_code,
            message,
            issues,
        } => {
            super::super::trace::emit(
                trace_enabled,
                "grounding",
                "draft_invalid",
                &[("reason_code", reason_code.clone())],
            );
            context.set_previous_error_and_refresh(
                state,
                false,
                super::super::grounding_phase_error_payload(
                    "invalid",
                    message.as_deref(),
                    issues.as_slice(),
                    &[],
                    context.completed_segments_u8(),
                ),
            );
            Err(RunnerError::Llm(format!(
                "intent grounding invalid reason_code={} message={} issues={}",
                reason_code,
                message.unwrap_or_default(),
                issues.len()
            )))
        }
    }
}

fn record_grounding_proposed_runtime(
    state: &mut EngineRunnerState,
    summary: Option<&str>,
    candidate: &super::super::grounding_resolution::GroundingCandidate,
    resolution: &super::super::grounding_resolution::GroundingResolution,
    answered_questions: &Map<String, Value>,
    query_recoverable_questions: &[Value],
    apply_summary: &GroundingApplySummary,
) {
    super::super::runtime_store::record_runtime_agent_field(
        &mut state.runtime,
        "intent_grounding",
        json!({
            "status":"proposed",
            "summary": summary,
            "ready_for_todos": resolution.ready_for_todos,
            "resolved_inputs": candidate.resolved_inputs.clone(),
            "intent_facts": candidate.intent_facts.clone(),
            "confidence": candidate.confidence.clone(),
            "issues": candidate.issues.clone(),
            "questions": resolution.effective_questions.clone(),
            "missing_refs": resolution.effective_missing_refs.clone(),
            "answers": answered_questions,
            "query_recoverable_questions": query_recoverable_questions,
            "applied": apply_summary.applied,
            "skipped_low_confidence": apply_summary.skipped_low_confidence,
            "deterministic_rule_inputs": apply_summary.deterministic_applied,
            "deterministic_rule_skipped": apply_summary.deterministic_skipped,
            "deterministic_conflicts": apply_summary.deterministic_conflicts,
            "deterministic_conflict_policy": "rule_extracted_over_llm",
            "resolved_input_refs": collect_bindable_input_refs(&candidate.resolved_inputs),
            "host_recovery_satisfied": resolution.host_recovery_satisfied,
            "user_input_required": resolution.user_input_required,
            "planner_ready_hint": resolution.planner_ready_hint,
            "resolution_state": match resolution.state {
                super::super::grounding_resolution::GroundingResolutionState::Ready => "ready",
                super::super::grounding_resolution::GroundingResolutionState::NeedsUserInput => "needs_user_input",
            },
            "input_binding": grounding_input_binding_metadata(),
        }),
    );
}

fn record_grounding_unavailable_ready_runtime(
    state: &mut EngineRunnerState,
    reason_code: &str,
    message: Option<&str>,
    issues: &[Value],
) {
    super::super::runtime_store::record_runtime_agent_field(
        &mut state.runtime,
        "intent_grounding",
        json!({
            "status":"unavailable_recovered",
            "ready_for_todos": true,
            "reason_code": reason_code,
            "message": message,
            "issues": issues,
            "questions": [],
            "missing_refs": [],
            "host_recovery_satisfied": true,
            "user_input_required": false,
            "planner_ready_hint": false,
            "resolution_state": "ready",
            "input_binding": grounding_input_binding_metadata(),
        }),
    );
}

pub(crate) fn apply_intent_grounding(
    state: &mut EngineRunnerState,
    fact_store: &mut InputStore,
    resolved_inputs: &BTreeMap<String, Value>,
    intent_facts: &BTreeMap<String, Value>,
    confidence: &BTreeMap<String, u8>,
    intent_text: &str,
) -> GroundingApplySummary {
    let mut summary = GroundingApplySummary::default();
    for (raw_key, raw_value) in resolved_inputs {
        let key = super::super::input_normalize::normalize_grounding_input_key(raw_key.as_str());
        if key.is_empty() {
            continue;
        }
        let Some(key) = super::super::input_normalize::normalize_input_slot_key(key.as_str())
        else {
            summary
                .skipped_low_confidence
                .push(format!("inputs.{key}:invalid_input_slot"));
            continue;
        };
        if !is_bindable_input_slot(key.as_str()) {
            summary
                .skipped_low_confidence
                .push(format!("inputs.{key}:invalid_input_slot"));
            continue;
        }
        if is_decimals_slot(key.as_str()) {
            summary
                .skipped_low_confidence
                .push(format!("inputs.{key}:decimals_requires_query"));
            continue;
        }
        let (value, inline_confidence) = normalize_grounding_input_value(raw_value);
        let score = resolve_grounding_input_confidence(
            confidence,
            raw_key.as_str(),
            key.as_str(),
            inline_confidence,
        );
        if score < GROUND_INPUT_CONFIDENCE_THRESHOLD {
            summary
                .skipped_low_confidence
                .push(format!("inputs.{key}:{score}"));
            continue;
        }
        let upsert_result = super::super::upsert_seed_input_value(
            &mut state.runtime,
            key.as_str(),
            value.clone(),
            format!("intent_grounding.input.{key}"),
        );
        match upsert_result {
            InputStoreUpsertResult::Inserted | InputStoreUpsertResult::Replaced => {}
            _ => {
                continue;
            }
        }
        let provenance = format!("intent_grounding.input.{key}");
        super::super::upsert_store_value_with_source(
            fact_store,
            key.as_str(),
            value.clone(),
            super::super::input_store::InputValueLayer::Seed,
            "intent",
            50,
            provenance,
        );
        summary.applied.push(format!("inputs.{key}:{score}"));
    }
    for (key, value) in intent_facts {
        let score = confidence
            .get(format!("fact:{key}").as_str())
            .copied()
            .or_else(|| confidence.get(key.as_str()).copied())
            .unwrap_or(70);
        if score < GROUND_FACT_CONFIDENCE_THRESHOLD {
            summary
                .skipped_low_confidence
                .push(format!("fact:{key}:{score}"));
            continue;
        }
        super::super::upsert_store_value_with_source(
            fact_store,
            key.clone(),
            value.clone(),
            super::super::input_store::InputValueLayer::Seed,
            "intent",
            50,
            format!("intent_grounding.fact.{key}"),
        );
        summary.applied.push(format!("fact:{key}:{score}"));
    }

    apply_balance_threshold_rule(
        state,
        fact_store,
        resolved_inputs,
        intent_facts,
        intent_text,
        &mut summary,
    );

    summary
}

fn apply_balance_threshold_rule(
    state: &mut EngineRunnerState,
    fact_store: &mut InputStore,
    resolved_inputs: &BTreeMap<String, Value>,
    intent_facts: &BTreeMap<String, Value>,
    intent_text: &str,
    summary: &mut GroundingApplySummary,
) {
    let threshold = match extract_balance_threshold(intent_text, intent_facts) {
        Some(value) => value,
        None => {
            summary
                .deterministic_skipped
                .push("inputs.balance_threshold:no_high_confidence_match".to_string());
            return;
        }
    };

    let deterministic_value = json!(threshold);
    if let Some((raw_key, llm_value)) =
        find_resolved_input_value(resolved_inputs, "balance_threshold")
    {
        if !values_semantically_equal(llm_value, &deterministic_value) {
            summary.deterministic_conflicts.push(format!(
                "inputs.balance_threshold:llm={llm_value} rule={deterministic_value} policy=rule_extracted_over_llm source_key={raw_key}"
            ));
        }
    }

    let upsert_result = super::super::upsert_seed_input_value(
        &mut state.runtime,
        "balance_threshold",
        deterministic_value.clone(),
        "rule_extracted.balance_threshold",
    );
    if !matches!(
        upsert_result,
        InputStoreUpsertResult::Inserted | InputStoreUpsertResult::Replaced
    ) {
        return;
    }

    super::super::upsert_store_value_with_source(
        fact_store,
        "balance_threshold",
        deterministic_value,
        super::super::input_store::InputValueLayer::Derived,
        "derived",
        60,
        "rule_extracted.balance_threshold",
    );
    summary.deterministic_applied.push(format!(
        "inputs.balance_threshold:{threshold}:rule_extracted"
    ));
    summary
        .applied
        .push("inputs.balance_threshold:rule_extracted".to_string());
}

fn find_resolved_input_value<'a>(
    resolved_inputs: &'a BTreeMap<String, Value>,
    slot: &str,
) -> Option<(&'a str, &'a Value)> {
    resolved_inputs.iter().find_map(|(raw_key, raw_value)| {
        let key = super::super::input_normalize::normalize_grounding_input_key(raw_key.as_str());
        if key == slot {
            let (value, _) = normalize_grounding_input_value(raw_value);
            return Some((raw_key.as_str(), raw_value_to_borrowed(raw_value, value)));
        }
        None
    })
}

fn raw_value_to_borrowed<'a>(raw_value: &'a Value, normalized: Value) -> &'a Value {
    if normalized == *raw_value {
        raw_value
    } else if let Some(object) = raw_value.as_object() {
        object.get("value").unwrap_or(raw_value)
    } else {
        raw_value
    }
}

fn values_semantically_equal(left: &Value, right: &Value) -> bool {
    if left == right {
        return true;
    }
    parse_u128_value(left)
        .zip(parse_u128_value(right))
        .is_some_and(|(left_num, right_num)| left_num == right_num)
}

fn parse_u128_value(value: &Value) -> Option<u128> {
    if let Some(number) = value.as_u64() {
        return Some(number as u128);
    }
    let text = value.as_str()?;
    let normalized = text.replace([',', '_', ' '], "");
    if normalized.is_empty() || !normalized.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    normalized.parse::<u128>().ok()
}

fn extract_balance_threshold(
    intent_text: &str,
    intent_facts: &BTreeMap<String, Value>,
) -> Option<u64> {
    let mut candidates = BTreeSet::<u64>::new();
    for text in collect_threshold_text_candidates(intent_text, intent_facts) {
        for threshold in extract_thresholds_from_expression(text.as_str()) {
            candidates.insert(threshold);
        }
    }
    if candidates.len() == 1 {
        return candidates.into_iter().next();
    }
    None
}

fn collect_threshold_text_candidates(
    intent_text: &str,
    intent_facts: &BTreeMap<String, Value>,
) -> Vec<String> {
    let mut out = vec![intent_text.to_string()];
    for value in intent_facts.values() {
        collect_string_values(value, &mut out);
    }
    out
}

fn collect_string_values(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(text) => out.push(text.clone()),
        Value::Array(array) => {
            for item in array {
                collect_string_values(item, out);
            }
        }
        Value::Object(map) => {
            for item in map.values() {
                collect_string_values(item, out);
            }
        }
        _ => {}
    }
}

fn extract_thresholds_from_expression(expression: &str) -> Vec<u64> {
    let bytes = expression.as_bytes();
    let mut out = Vec::new();
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] != b'>' {
            index += 1;
            continue;
        }
        if index + 1 < bytes.len() && bytes[index + 1] == b'=' {
            index += 2;
            continue;
        }

        let left = capture_left_identifier(expression, index);
        let right = capture_right_number(expression, index + 1);
        if left
            .as_deref()
            .is_some_and(|candidate| candidate.to_ascii_lowercase().contains("balance"))
        {
            if let Some(threshold) = right {
                out.push(threshold);
            }
        }
        index += 1;
    }
    out
}

fn capture_left_identifier(expression: &str, operator_index: usize) -> Option<String> {
    let bytes = expression.as_bytes();
    if operator_index == 0 {
        return None;
    }

    let mut end = operator_index;
    while end > 0 && bytes[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    if end == 0 {
        return None;
    }

    let mut start = end;
    while start > 0 {
        let candidate = bytes[start - 1];
        if candidate.is_ascii_alphanumeric() || matches!(candidate, b'_' | b'.') {
            start -= 1;
            continue;
        }
        break;
    }
    if start == end {
        return None;
    }
    Some(expression[start..end].to_string())
}

fn capture_right_number(expression: &str, mut index: usize) -> Option<u64> {
    let bytes = expression.as_bytes();
    while index < bytes.len() && bytes[index].is_ascii_whitespace() {
        index += 1;
    }
    if index >= bytes.len() {
        return None;
    }

    let start = index;
    while index < bytes.len() {
        let ch = bytes[index];
        if ch.is_ascii_digit() || matches!(ch, b',' | b'_' | b' ') {
            index += 1;
            continue;
        }
        break;
    }
    if index == start {
        return None;
    }
    let normalized = expression[start..index].replace([',', '_', ' '], "");
    normalized.parse::<u64>().ok()
}

pub(crate) fn intent_grounding_ready_for_todos(state: &EngineRunnerState) -> bool {
    let Some(grounding) = state.runtime.pointer("/agent/intent_grounding") else {
        return false;
    };
    let ready_flag = grounding
        .get("ready_for_todos")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let has_questions = grounding
        .get("questions")
        .and_then(Value::as_array)
        .is_some_and(|questions| !questions.is_empty());
    if has_questions {
        return false;
    }
    let has_missing_refs = grounding
        .get("missing_refs")
        .and_then(Value::as_array)
        .is_some_and(|items| !items.is_empty());
    if has_missing_refs {
        return false;
    }
    if ready_flag {
        return true;
    }
    let has_resolved_inputs = grounding
        .get("resolved_inputs")
        .and_then(Value::as_object)
        .is_some_and(|resolved| !resolved.is_empty());
    let has_intent_facts = grounding
        .get("intent_facts")
        .and_then(Value::as_object)
        .is_some_and(|facts| !facts.is_empty());
    has_resolved_inputs || has_intent_facts
}

fn normalize_grounding_input_value(raw_value: &Value) -> (Value, Option<u8>) {
    let Some(object) = raw_value.as_object() else {
        return (raw_value.clone(), None);
    };

    let inline_confidence = object
        .get("confidence")
        .and_then(Value::as_u64)
        .and_then(|value| u8::try_from(value).ok());

    let Some(inner_value) = object.get("value") else {
        return (raw_value.clone(), inline_confidence);
    };

    let is_wrapper = object.keys().all(|key| {
        matches!(
            key.as_str(),
            "value" | "confidence" | "source" | "note" | "reason"
        )
    });
    if is_wrapper {
        return (inner_value.clone(), inline_confidence);
    }
    (raw_value.clone(), inline_confidence)
}

fn resolve_grounding_input_confidence(
    confidence: &std::collections::BTreeMap<String, u8>,
    raw_key: &str,
    canonical_key: &str,
    inline_confidence: Option<u8>,
) -> u8 {
    confidence
        .get(raw_key)
        .copied()
        .or_else(|| confidence.get(canonical_key).copied())
        .or_else(|| {
            confidence
                .get(format!("inputs.{canonical_key}").as_str())
                .copied()
        })
        .or(inline_confidence)
        .unwrap_or(85)
}

fn filter_unanswered_questions(questions: &[Value], answered_ids: &[&String]) -> Vec<Value> {
    let answered = answered_ids
        .iter()
        .map(|value| value.as_str())
        .collect::<BTreeSet<_>>();
    questions
        .iter()
        .filter(|question| {
            let Some(id) = question.get("id").and_then(Value::as_str) else {
                return true;
            };
            !answered.contains(id)
        })
        .cloned()
        .collect::<Vec<_>>()
}

fn grounding_input_binding_metadata() -> Value {
    json!({
        "bindable_namespace": "inputs",
        "bindable_refs_source": INPUT_BINDABLE_SOURCE_OF_TRUTH,
        "bindable_refs_projection": INPUT_BINDABLE_REFS_PROJECTION_PATH,
        "known_refs_only": true,
        "facts_bindable": false,
    })
}

fn collect_bindable_input_refs(resolved_inputs: &BTreeMap<String, Value>) -> Vec<String> {
    resolved_inputs
        .keys()
        .filter_map(|key| {
            let canonical = super::super::input_normalize::normalize_grounding_input_key(key);
            super::super::input_normalize::normalize_input_slot_key(canonical.as_str()).and_then(
                |slot| is_bindable_input_slot(slot.as_str()).then(|| format!("inputs.{slot}")),
            )
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
}

fn is_bindable_input_slot(slot: &str) -> bool {
    let lowered = slot.to_ascii_lowercase();
    !slot.contains(':')
        && !lowered.starts_with("facts.")
        && !lowered.starts_with("fact.")
        && !lowered.starts_with("fact:")
}

fn is_decimals_slot(slot: &str) -> bool {
    slot == "token.decimals"
        || slot == "decimals"
        || slot == "token_decimals"
        || slot.ends_with(".decimals")
}

fn auto_answer_single_option_questions(
    questions: &[Value],
    resolved_inputs: &BTreeMap<String, Value>,
    intent_facts: &BTreeMap<String, Value>,
    _fact_store: &mut InputStore,
) -> (Map<String, Value>, Vec<Value>) {
    let mut answers = Map::new();
    let mut remaining = Vec::new();
    for question in questions {
        let id = question
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let options = question
            .get("options")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if options.len() == 1 {
            let value = options[0]
                .get("value")
                .cloned()
                .unwrap_or_else(|| options[0].clone());
            answers.insert(id.to_string(), value);
            continue;
        }
        if options.len() > 1 {
            if let Some(matched_value) =
                match_question_option_to_known_values(&options, resolved_inputs, intent_facts)
            {
                answers.insert(id.to_string(), matched_value);
                continue;
            }
        }
        remaining.push(question.clone());
    }
    (answers, remaining)
}

fn match_question_option_to_known_values(
    options: &[Value],
    resolved_inputs: &BTreeMap<String, Value>,
    intent_facts: &BTreeMap<String, Value>,
) -> Option<Value> {
    let known_values: Vec<&Value> = resolved_inputs
        .values()
        .chain(intent_facts.values())
        .collect();
    let mut matched = Vec::new();
    for option in options {
        let option_value = option.get("value").unwrap_or(option);
        let option_str = option_value.as_str().unwrap_or_default();
        if option_str.is_empty() {
            continue;
        }
        for known in &known_values {
            let known_str = known.as_str().unwrap_or_default();
            if !known_str.is_empty() && known_str == option_str {
                matched.push(option_value.clone());
            }
        }
    }
    if matched.len() == 1 {
        return Some(matched.into_iter().next().unwrap());
    }
    None
}

fn collect_mandatory_grounding_missing_refs_host(
    runtime: &Value,
    typed_summary: Option<&super::super::state_summary::StateSummary>,
) -> Vec<String> {
    let mut mandatory = Vec::<String>::new();
    let has_token = grounding_payload_ref_resolved(runtime, typed_summary, "inputs.token")
        || grounding_payload_ref_resolved(runtime, typed_summary, "inputs.token_address")
        || grounding_payload_ref_resolved(runtime, typed_summary, "inputs.erc20_token");
    let has_decimals =
        grounding_payload_ref_resolved(runtime, typed_summary, "inputs.token.decimals")
            || grounding_payload_ref_resolved(runtime, typed_summary, "inputs.token_decimals")
            || grounding_payload_ref_resolved(runtime, typed_summary, "inputs.decimals");
    if has_token && !has_decimals {
        mandatory.push("inputs.token.decimals".to_string());
    }
    mandatory
}

fn collect_unresolved_grounding_payload_missing_refs(
    runtime: &Value,
    typed_summary: Option<&super::super::state_summary::StateSummary>,
    missing_refs: &[String],
) -> Vec<String> {
    missing_refs
        .iter()
        .filter(|reference| !grounding_payload_ref_resolved(runtime, typed_summary, reference))
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
}

fn collect_grounding_payload_missing_refs(payload: &Value) -> Vec<String> {
    super::super::missing_resolution::missing_required_input_refs(payload)
        .into_iter()
        .chain(super::super::missing_resolution::payload_question_refs(
            payload,
        ))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
}

fn grounding_payload_ref_resolved(
    runtime: &Value,
    typed_summary: Option<&super::super::state_summary::StateSummary>,
    reference: &str,
) -> bool {
    let Some(path) = super::super::input_normalize::canonical_missing_ref_path(reference) else {
        return false;
    };
    match path {
        super::super::ref_model::RefPath::Input { slot } => grounding_has_input_slot(
            runtime,
            typed_summary,
            format!("inputs.{slot}").as_str(),
            slot.as_str(),
        ),
        super::super::ref_model::RefPath::Fact { key } => typed_summary
            .and_then(|summary| summary.runtime_facts_facts())
            .and_then(|facts| {
                let canonical = format!("facts.{key}");
                facts
                    .get(canonical.as_str())
                    .or_else(|| value_at_dotted_path_object(facts, canonical.as_str()))
            })
            .is_some(),
        super::super::ref_model::RefPath::NodeOutput {
            step_id,
            field_path,
        } => {
            let expected = format!("nodes.{step_id}.outputs.{field_path}");
            typed_summary
                .map(|summary| {
                    summary
                        .node_output_refs_known_refs()
                        .iter()
                        .any(|raw_ref| *raw_ref == expected)
                })
                .unwrap_or(false)
        }
    }
}

fn grounding_has_input_slot(
    runtime: &Value,
    typed_summary: Option<&super::super::state_summary::StateSummary>,
    _full_ref: &str,
    slot: &str,
) -> bool {
    if runtime
        .pointer(format!("/inputs/{}", slot.replace('.', "/")).as_str())
        .is_some()
    {
        return true;
    }
    if let Some(summary) = typed_summary {
        let has_input_meta = summary.input_store_meta().and_then(|meta| {
            meta.get(slot)
                .or_else(|| value_at_dotted_path_object(meta, slot))
        });
        if has_input_meta.is_some()
            && summary
                .input_store_facts()
                .and_then(|facts| {
                    facts
                        .get(slot)
                        .or_else(|| value_at_dotted_path_object(facts, slot))
                })
                .is_some()
        {
            return true;
        }
    }
    typed_summary
        .and_then(|summary| summary.intent_slots_resolved_inputs())
        .and_then(|inputs| inputs.get(slot))
        .is_some()
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

#[cfg(test)]
#[path = "../tests/phase_machine/grounding.rs"]
mod tests;
