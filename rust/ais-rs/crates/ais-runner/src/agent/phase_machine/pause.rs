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
    if record_payload_before_collect {
        super::super::missing_input::record(&mut state.runtime, payload);
    }
    let questions = payload
        .get("questions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if let Some(answers) = super::super::missing_input::maybe_collect_and_apply_answers(
        state,
        fact_store,
        questions.as_slice(),
    )? {
        super::super::missing_input::mark_consumed(&mut state.runtime);
        state.paused_reason = None;
        return Ok(MissingRequiredInputBackflow::ResolvedByUserInput { answers });
    }
    super::super::missing_input::pause_with_payload(state, payload);
    Ok(MissingRequiredInputBackflow::Paused)
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
    Some(payload)
}

#[cfg(test)]
#[path = "../tests/phase_machine/pause.rs"]
mod tests;
