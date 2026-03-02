use super::*;
use ais_engine::{EngineEventRecord, EngineEventType, EngineRunnerState};
use serde_json::{Map, Value};
use std::collections::BTreeSet;

pub(super) fn payload(
    message: Option<&str>,
    questions: &[Value],
    issues: &[Value],
    round: u8,
) -> Value {
    payload_with_context(message, questions, issues, &[], &[], round)
}

pub(super) fn payload_with_context(
    message: Option<&str>,
    questions: &[Value],
    issues: &[Value],
    missing_refs: &[String],
    suggested_paths: &[String],
    round: u8,
) -> Value {
    compact_json_for_llm(&serde_json::json!({
        "phase": "planning",
        "reason_code": "missing_required_input",
        "message": message,
        "missing_refs": missing_refs,
        "suggested_paths": suggested_paths,
        "questions": questions,
        "issues": issues,
        "round": round,
    }))
}

pub(super) fn resolved_payload(answers: &Map<String, Value>, round: u8) -> Value {
    compact_json_for_llm(&serde_json::json!({
        "phase": "planning",
        "reason_code": "missing_required_input",
        "status": "resolved_by_user_input",
        "answers": answers,
        "round": round,
    }))
}

pub(super) fn apply_answers(
    state: &mut EngineRunnerState,
    input_store: &mut InputStore,
    answers: &Map<String, Value>,
) {
    for (raw_key, value) in answers {
        let key = super::input_normalize::canonical_input_slot_key(raw_key.as_str());
        if key.is_empty() {
            continue;
        }
        let result = super::upsert_user_input_value(
            &mut state.runtime,
            key.as_str(),
            value.clone(),
            "user.prompt",
        );
        if matches!(
            result,
            super::InputStoreUpsertResult::Rejected | super::InputStoreUpsertResult::Ignored
        ) {
            continue;
        }
        let provenance = format!("user.prompt.{key}");
        super::upsert_store_value_with_source(
            input_store,
            key.as_str(),
            value.clone(),
            super::input_store::InputValueLayer::Seed,
            "user",
            100,
            provenance,
        );
        if key == "owner" {
            super::upsert_store_value_with_source(
                input_store,
                "wallet.default",
                value.clone(),
                super::input_store::InputValueLayer::Seed,
                "user",
                100,
                "user.prompt.owner",
            );
        }
    }
}

pub(super) fn maybe_collect_and_apply_answers(
    state: &mut EngineRunnerState,
    input_store: &mut InputStore,
    questions: &[Value],
) -> Result<Option<Map<String, Value>>, RunnerError> {
    let Some(answers) = super::maybe_collect_missing_input_answers(questions)? else {
        return Ok(None);
    };
    apply_answers(state, input_store, &answers);
    Ok(Some(answers))
}

pub(super) fn record(runtime: &mut Value, payload: &Value) {
    super::runtime_store::record_runtime_agent_field(
        runtime,
        "missing_required_input",
        payload.clone(),
    );
}

pub(super) fn pause_with_payload(state: &mut EngineRunnerState, payload: &Value) {
    state.paused_reason = Some("missing_required_input".to_string());
    record(&mut state.runtime, payload);
}

pub(super) fn payload_from_pause(
    state: &EngineRunnerState,
    events: &[EngineEventRecord],
    round: u8,
) -> Option<Value> {
    let paused_reason = state.paused_reason.as_deref()?;
    if !paused_reason.starts_with("need_user_input:")
        && paused_reason != "need_user_input"
        && paused_reason != "missing_required_input"
    {
        return None;
    }

    let event = events
        .iter()
        .rev()
        .find(|record| record.event.event_type == EngineEventType::NeedUserInput)?;
    let reason_code = event
        .event
        .data
        .get("reason_code")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if reason_code != "missing_required_input" {
        return None;
    }

    let message = event.event.data.get("reason").and_then(Value::as_str);
    let details = event
        .event
        .data
        .get("details")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let missing_refs = string_array_field(details.get("missing_refs"));
    let suggested_paths = string_array_field(details.get("suggested_paths"));
    let questions = details
        .get("questions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let issues = details
        .get("issues")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    Some(payload_with_context(
        message,
        questions.as_slice(),
        issues.as_slice(),
        missing_refs.as_slice(),
        suggested_paths.as_slice(),
        round,
    ))
}

fn string_array_field(value: Option<&Value>) -> Vec<String> {
    let Some(items) = value.and_then(Value::as_array) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
}

#[cfg(test)]
#[path = "tests/missing_input_module.rs"]
mod tests;
