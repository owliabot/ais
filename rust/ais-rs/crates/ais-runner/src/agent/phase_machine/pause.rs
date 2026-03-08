use super::super::*;
use ais_engine::{EngineEventRecord, EngineEventType, EngineRunnerState};
use serde_json::{Map, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PauseReasonKind {
    MissingRequiredInput,
    NeedUserConfirm,
    Other,
}

#[derive(Debug, Clone)]
pub(crate) enum MissingRequiredInputBackflow {
    ResolvedByUserInput { answers: Map<String, Value> },
    Paused,
}

#[derive(Debug, Clone)]
pub(crate) enum MissingRequiredInputRecoveryBackflow {
    Retry {
        state_changed: bool,
        answers: Option<Map<String, Value>>,
    },
    Paused,
}

#[derive(Debug, Clone)]
pub(crate) enum ResolvePauseBackflow {
    MissingRequiredInputResolved { answers: Map<String, Value> },
    MissingRequiredInputPaused,
    PauseTerminal { blocked_reason: String },
    RepairScheduled { previous_error: Value },
}

pub(crate) fn classify_pause_reason(paused_reason: Option<&str>) -> PauseReasonKind {
    let Some(reason) = paused_reason else {
        return PauseReasonKind::Other;
    };
    if reason == "missing_required_input" || reason.starts_with("need_user_input:") {
        return PauseReasonKind::MissingRequiredInput;
    }
    if reason == "need_user_confirm" || reason.starts_with("need_user_confirm:") {
        return PauseReasonKind::NeedUserConfirm;
    }
    PauseReasonKind::Other
}

pub(crate) fn resolve_missing_required_input_payload(
    state: &mut EngineRunnerState,
    fact_store: &mut InputStore,
    payload: &Value,
    record_payload_before_collect: bool,
) -> Result<MissingRequiredInputBackflow, RunnerError> {
    let normalized_payload =
        super::super::missing_input::normalize_missing_required_input_payload(payload);
    if record_payload_before_collect {
        super::super::missing_input::record(&mut state.runtime, &normalized_payload);
    }
    let questions = normalized_payload
        .get("questions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if let Some(answers) = super::super::missing_input::maybe_collect_and_apply_answers(
        state,
        fact_store,
        questions.as_slice(),
        &normalized_payload,
    )? {
        super::super::missing_input::mark_consumed(&mut state.runtime);
        state.paused_reason = None;
        return Ok(MissingRequiredInputBackflow::ResolvedByUserInput { answers });
    }
    super::super::missing_input::pause_with_payload(state, &normalized_payload);
    Ok(MissingRequiredInputBackflow::Paused)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn recover_missing_required_input_payload(
    command: &AgentCommand,
    state: &mut EngineRunnerState,
    context: &mut super::super::orchestrator::SegmentedAgentContext,
    candidate_context: &CandidateContext,
    readonly_autofill_router: Option<&ais_engine::RouterExecutor>,
    payload: &Value,
    scope_id: &str,
    done: bool,
    phase_hint: &'static str,
    record_payload_before_collect: bool,
    collect_user_answers: bool,
) -> Result<MissingRequiredInputRecoveryBackflow, RunnerError> {
    let normalized_payload =
        super::super::missing_input::normalize_missing_required_input_payload(payload);
    let recovery_outcome =
        super::super::missing_resolution::missing_resolution_recover_missing_refs(
            command,
            state,
            context,
            &normalized_payload,
            candidate_context,
            readonly_autofill_router,
            scope_id,
            done,
            phase_hint,
        );
    if matches!(
        recovery_outcome,
        super::super::missing_resolution::MissingResolutionOutcome::Recovered
    ) {
        return Ok(MissingRequiredInputRecoveryBackflow::Retry {
            state_changed: true,
            answers: None,
        });
    }
    if recovery_outcome.should_retry_round() {
        return Ok(MissingRequiredInputRecoveryBackflow::Retry {
            state_changed: false,
            answers: None,
        });
    }
    let gated_payload =
        recovery_payload_from_outcome(&normalized_payload, &recovery_outcome, phase_hint, scope_id);
    if !can_prompt_user_missing_input(&gated_payload) {
        super::super::missing_input::pause_with_payload(state, &gated_payload);
        return Ok(MissingRequiredInputRecoveryBackflow::Paused);
    }
    if !collect_user_answers {
        if record_payload_before_collect {
            super::super::missing_input::record(&mut state.runtime, &gated_payload);
        }
        super::super::missing_input::pause_with_payload(state, &gated_payload);
        return Ok(MissingRequiredInputRecoveryBackflow::Paused);
    }
    match resolve_missing_required_input_payload(
        state,
        context.input_store_mut(),
        &gated_payload,
        record_payload_before_collect,
    )? {
        MissingRequiredInputBackflow::ResolvedByUserInput { answers } => {
            Ok(MissingRequiredInputRecoveryBackflow::Retry {
                state_changed: true,
                answers: Some(answers),
            })
        }
        MissingRequiredInputBackflow::Paused => Ok(MissingRequiredInputRecoveryBackflow::Paused),
    }
}

pub(crate) fn can_prompt_user_missing_input(payload: &Value) -> bool {
    let status = payload
        .pointer("/recovery_exhaustion/status")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let allowed_status = matches!(
        status,
        "need_user_input" | "exhausted_unavailable" | "compile_autofill_exhausted"
    );
    if !allowed_status {
        return false;
    }
    let unresolved_refs = payload
        .pointer("/recovery_exhaustion/unresolved_refs")
        .and_then(Value::as_array)
        .map(|items| !items.is_empty())
        .unwrap_or(false);
    let reasons = payload
        .pointer("/recovery_exhaustion/reasons")
        .and_then(Value::as_array)
        .map(|items| !items.is_empty())
        .unwrap_or(false);
    let attempt_trace_id = payload
        .pointer("/recovery_exhaustion/attempt_trace_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .is_some_and(|value| !value.is_empty());
    let questions = payload
        .get("questions")
        .and_then(Value::as_array)
        .map(|items| !items.is_empty())
        .unwrap_or(false);
    (questions || unresolved_refs) && reasons && attempt_trace_id
}

pub(crate) fn attach_missing_input_recovery(
    payload: &Value,
    status: &str,
    reason: &str,
    source: &str,
    phase_hint: &str,
    scope_id: &str,
    missing_refs: &[String],
) -> Value {
    let recovery_exhaustion = build_recovery_exhaustion_payload(
        status,
        reason,
        source,
        phase_hint,
        scope_id,
        missing_refs,
    );
    let mut out = payload.clone();
    if let Some(object) = out.as_object_mut() {
        object.insert("recovery_exhaustion".to_string(), recovery_exhaustion);
    }
    out
}

fn build_recovery_exhaustion_payload(
    status: &str,
    reason: &str,
    source: &str,
    phase_hint: &str,
    scope_id: &str,
    missing_refs: &[String],
) -> Value {
    let reason_text = reason.trim();
    let reasons = if reason_text.is_empty() {
        vec!["recovery_exhausted".to_string()]
    } else {
        vec![reason_text.to_string()]
    };
    let scope = scope_id.trim();
    let phase = phase_hint.trim();
    let attempt_trace_id = format!(
        "{source}:{phase}:{scope}:{status}",
        phase = if phase.is_empty() {
            "unknown_phase"
        } else {
            phase
        },
        scope = if scope.is_empty() {
            "unknown_scope"
        } else {
            scope
        },
    );
    serde_json::json!({
        "status": status,
        "source": source,
        "unresolved_refs": missing_refs,
        "reasons": reasons,
        "attempt_trace_id": attempt_trace_id,
    })
}

fn recovery_payload_from_outcome(
    payload: &Value,
    outcome: &super::super::missing_resolution::MissingResolutionOutcome,
    phase_hint: &str,
    scope_id: &str,
) -> Value {
    match outcome {
        super::super::missing_resolution::MissingResolutionOutcome::NeedUserInput {
            missing_refs,
            reason,
        } => attach_missing_input_recovery(
            payload,
            "need_user_input",
            reason,
            "missing_resolution",
            phase_hint,
            scope_id,
            missing_refs,
        ),
        super::super::missing_resolution::MissingResolutionOutcome::ExhaustedUnavailable {
            missing_refs,
            reason,
        } => attach_missing_input_recovery(
            payload,
            "exhausted_unavailable",
            reason,
            "missing_resolution",
            phase_hint,
            scope_id,
            missing_refs,
        ),
        super::super::missing_resolution::MissingResolutionOutcome::Recovered
        | super::super::missing_resolution::MissingResolutionOutcome::RetryScheduled => {
            payload.clone()
        }
    }
}

pub(crate) fn resolve_execution_pause_backflow(
    state: &mut EngineRunnerState,
    fact_store: &mut InputStore,
    events: &[EngineEventRecord],
    round: u8,
) -> Result<ResolvePauseBackflow, RunnerError> {
    if let Some(payload) = missing_required_input_payload_from_pause(state, events, round) {
        return match resolve_missing_required_input_payload(state, fact_store, &payload, true)? {
            MissingRequiredInputBackflow::ResolvedByUserInput { answers } => {
                Ok(ResolvePauseBackflow::MissingRequiredInputResolved { answers })
            }
            MissingRequiredInputBackflow::Paused => {
                Ok(ResolvePauseBackflow::MissingRequiredInputPaused)
            }
        };
    }

    let blocked_reason = state
        .paused_reason
        .clone()
        .unwrap_or_else(|| "paused".to_string());
    match classify_pause_reason(state.paused_reason.as_deref()) {
        PauseReasonKind::MissingRequiredInput | PauseReasonKind::NeedUserConfirm => {
            return Ok(ResolvePauseBackflow::PauseTerminal { blocked_reason });
        }
        PauseReasonKind::Other => {}
    }
    if !super::super::should_attempt_intent_repair(state.paused_reason.as_deref()) {
        return Ok(ResolvePauseBackflow::PauseTerminal { blocked_reason });
    }

    Ok(ResolvePauseBackflow::RepairScheduled {
        previous_error: super::super::intent_execution_error_payload(
            state.paused_reason.as_deref(),
            events,
            round,
        ),
    })
}

pub(crate) fn missing_required_input_payload_from_pause(
    state: &EngineRunnerState,
    events: &[EngineEventRecord],
    round: u8,
) -> Option<Value> {
    let from_events = super::super::missing_input::payload_from_pause(state, events, round);
    if from_events.is_some() {
        return from_events;
    }
    runtime_missing_required_input_payload(state, events)
}

fn runtime_missing_required_input_payload(
    state: &EngineRunnerState,
    events: &[EngineEventRecord],
) -> Option<Value> {
    if classify_pause_reason(state.paused_reason.as_deref())
        != PauseReasonKind::MissingRequiredInput
    {
        return None;
    }
    if let Some(latest_need_user_input) = events
        .iter()
        .rev()
        .find(|record| record.event.event_type == EngineEventType::NeedUserInput)
    {
        let reason_code = latest_need_user_input
            .event
            .data
            .get("reason_code")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if reason_code != "missing_required_input" {
            return None;
        }
    }
    let payload = state
        .runtime
        .pointer("/agent/missing_required_input")?
        .clone();
    if payload
        .get("consumed")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return None;
    }
    let reason_code = payload
        .get("reason_code")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if reason_code != "missing_required_input" {
        return None;
    }
    Some(super::super::missing_input::normalize_missing_required_input_payload(&payload))
}

#[cfg(test)]
#[path = "../tests/phase_machine/pause.rs"]
mod tests;
