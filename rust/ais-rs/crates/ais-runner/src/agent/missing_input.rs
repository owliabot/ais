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

pub(super) fn payload_with_error_details(
    message: Option<&str>,
    questions: &[Value],
    issues: &[Value],
    error_details: Option<&Value>,
    round: u8,
) -> Value {
    let mut out = payload_with_context(message, questions, issues, &[], &[], round);
    merge_error_details_hints(&mut out, error_details);
    compact_json_for_llm(&out)
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
        "consumed": false,
        "message": message,
        "missing_refs": missing_refs,
        "suggested_paths": suggested_paths,
        "questions": questions,
        "issues": issues,
        "round": round,
    }))
}

fn merge_error_details_hints(payload: &mut Value, error_details: Option<&Value>) {
    let Some(details_object) = error_details.and_then(Value::as_object) else {
        return;
    };
    let Some(payload_object) = payload.as_object_mut() else {
        return;
    };
    payload_object.insert(
        "error_details".to_string(),
        Value::Object(details_object.clone()),
    );
    if payload_object.get("recovery_exhaustion").is_none() {
        if let Some(recovery_exhaustion) = details_object.get("recovery_exhaustion") {
            payload_object.insert(
                "recovery_exhaustion".to_string(),
                recovery_exhaustion.clone(),
            );
        }
    }
    for key in [
        "decisions",
        "binding_decisions",
        "query_decisions",
        "autofill",
    ] {
        if payload_object.get(key).is_some() {
            continue;
        }
        if let Some(value) = details_object.get(key) {
            payload_object.insert(key.to_string(), value.clone());
        }
    }
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

pub(super) fn mark_consumed(runtime: &mut Value) {
    let Some(agent) = runtime.get_mut("agent").and_then(Value::as_object_mut) else {
        return;
    };
    let Some(payload) = agent
        .get_mut("missing_required_input")
        .and_then(Value::as_object_mut)
    else {
        return;
    };
    payload.insert("consumed".to_string(), Value::Bool(true));
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
    payload: &Value,
) -> Result<Option<Map<String, Value>>, RunnerError> {
    let recovery_exhaustion = payload.get("recovery_exhaustion");
    let Some(answers) =
        super::maybe_collect_missing_input_answers_with_recovery(questions, recovery_exhaustion)?
    else {
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

    let payload = payload_with_context(
        message,
        questions.as_slice(),
        issues.as_slice(),
        missing_refs.as_slice(),
        suggested_paths.as_slice(),
        round,
    );
    let mut payload = payload;
    merge_error_details_hints(&mut payload, details.get("error_details"));
    let mut payload = normalize_missing_required_input_payload(&payload);
    if let Some(recovery_exhaustion) = details.get("recovery_exhaustion") {
        if let Some(object) = payload.as_object_mut() {
            object.insert(
                "recovery_exhaustion".to_string(),
                recovery_exhaustion.clone(),
            );
        }
    }
    Some(normalize_missing_required_input_payload(&payload))
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

pub(super) fn normalize_missing_required_input_payload(payload: &Value) -> Value {
    let mut out = payload.clone();
    let Some(object) = out.as_object_mut() else {
        return out;
    };
    let missing_refs_raw = string_array_field(object.get("missing_refs"));
    let suggested_paths_raw = string_array_field(object.get("suggested_paths"));
    let source_refs =
        normalize_source_refs(missing_refs_raw.as_slice(), suggested_paths_raw.as_slice());
    let questions = object
        .get("questions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let normalized_questions =
        normalize_question_entries(questions.as_slice(), source_refs.as_slice());
    object.insert(
        "missing_refs".to_string(),
        Value::Array(
            source_refs
                .iter()
                .map(|item| Value::String(item.clone()))
                .collect(),
        ),
    );
    object.insert(
        "suggested_paths".to_string(),
        Value::Array(
            source_refs
                .iter()
                .map(|item| Value::String(item.clone()))
                .collect(),
        ),
    );
    object.insert("questions".to_string(), Value::Array(normalized_questions));
    if let Some(recovery_exhaustion) =
        normalize_recovery_exhaustion(object.get("recovery_exhaustion"), source_refs.as_slice())
    {
        object.insert("recovery_exhaustion".to_string(), recovery_exhaustion);
    }
    out
}

fn normalize_recovery_exhaustion(
    recovery_exhaustion: Option<&Value>,
    source_refs: &[String],
) -> Option<Value> {
    let status = recovery_exhaustion
        .and_then(|value| value.get("status"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let source = recovery_exhaustion
        .and_then(|value| value.get("source"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let unresolved_refs_raw = recovery_exhaustion
        .and_then(|value| value.get("unresolved_refs"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut unresolved_refs =
        normalize_recovery_unresolved_refs(unresolved_refs_raw.as_slice(), source_refs);
    if unresolved_refs.is_empty() {
        unresolved_refs = source_refs.to_vec();
    }
    let mut reasons = recovery_exhaustion
        .and_then(|value| value.get("reasons"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if reasons.is_empty() {
        reasons.push("recovery_exhausted".to_string());
    }
    let attempt_trace_id = recovery_exhaustion
        .and_then(|value| value.get("attempt_trace_id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| "missing_resolution:unknown".to_string());
    let inferred_status = status.or_else(|| infer_recovery_status(attempt_trace_id.as_str()));
    let inferred_source = source.or_else(|| infer_recovery_source(attempt_trace_id.as_str()));
    if unresolved_refs.is_empty() {
        return None;
    }
    let mut normalized = serde_json::json!({
        "unresolved_refs": unresolved_refs,
        "reasons": reasons,
        "attempt_trace_id": attempt_trace_id,
    });
    if let Some(object) = normalized.as_object_mut() {
        if let Some(status) = inferred_status {
            object.insert("status".to_string(), Value::String(status));
        }
        if let Some(source) = inferred_source {
            object.insert("source".to_string(), Value::String(source));
        }
    }
    Some(normalized)
}

fn infer_recovery_status(attempt_trace_id: &str) -> Option<String> {
    let candidate = attempt_trace_id.rsplit(':').next()?.trim();
    matches!(
        candidate,
        "need_user_input" | "exhausted_unavailable" | "compile_autofill_exhausted"
    )
    .then(|| candidate.to_string())
}

fn infer_recovery_source(attempt_trace_id: &str) -> Option<String> {
    let mut parts = attempt_trace_id.split(':');
    let first = parts.next()?.trim();
    (!first.is_empty()).then(|| first.to_string())
}

fn normalize_recovery_unresolved_refs(raw_refs: &[String], source_refs: &[String]) -> Vec<String> {
    let mut out = BTreeSet::<String>::new();
    let mut source_pool = source_refs.iter().cloned().collect::<BTreeSet<_>>();
    for raw_ref in raw_refs {
        if let Some(canonical) = canonicalize_source_ref(raw_ref, &source_pool) {
            source_pool.insert(canonical.clone());
            out.insert(canonical);
            continue;
        }
        if let Some(canonical) = canonical_source_ref(raw_ref) {
            source_pool.insert(canonical.clone());
            out.insert(canonical);
        }
    }
    out.into_iter().collect::<Vec<_>>()
}

fn normalize_source_refs(missing_refs: &[String], suggested_paths: &[String]) -> Vec<String> {
    let mut out = BTreeSet::<String>::new();
    let mut source_pool = BTreeSet::<String>::new();
    for raw in missing_refs.iter().chain(suggested_paths.iter()) {
        if raw.trim().is_empty() || raw.trim().starts_with("params.") {
            continue;
        }
        if let Some(canonical) = canonicalize_source_ref(raw, &source_pool) {
            source_pool.insert(canonical.clone());
            out.insert(canonical);
        }
    }
    for raw in missing_refs.iter().chain(suggested_paths.iter()) {
        if let Some(canonical) = canonicalize_source_ref(raw, &source_pool) {
            source_pool.insert(canonical.clone());
            out.insert(canonical);
        }
    }
    out.into_iter().collect::<Vec<_>>()
}

fn normalize_question_entries(questions: &[Value], source_refs: &[String]) -> Vec<Value> {
    let source_set = source_refs.iter().cloned().collect::<BTreeSet<_>>();
    let mut seen_ids = BTreeSet::<String>::new();
    let mut normalized = Vec::<Value>::new();
    for question in questions {
        let Some(mut question_object) = question.as_object().cloned() else {
            continue;
        };
        let normalized_id = question_object
            .get("id")
            .and_then(Value::as_str)
            .and_then(|id| canonicalize_source_ref(id, &source_set));
        let has_internal_param_id = question_object
            .get("id")
            .and_then(Value::as_str)
            .is_some_and(|id| id.trim().starts_with("params."));
        if has_internal_param_id && normalized_id.is_none() {
            continue;
        }
        if let Some(id) = normalized_id {
            if !seen_ids.insert(id.clone()) {
                continue;
            }
            question_object.insert("id".to_string(), Value::String(id));
        }
        normalized.push(Value::Object(question_object));
    }
    normalized
}

fn canonicalize_source_ref(raw: &str, source_refs: &BTreeSet<String>) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.starts_with("params.") {
        return map_param_ref_to_source(trimmed, source_refs);
    }
    canonical_source_ref(trimmed).map(|canonical| remap_semantic_alias(canonical, source_refs))
}

fn canonical_source_ref(raw: &str) -> Option<String> {
    super::input_normalize::canonical_missing_ref(raw)
}

fn map_param_ref_to_source(raw: &str, source_refs: &BTreeSet<String>) -> Option<String> {
    let param_path = raw
        .trim()
        .strip_prefix("params.")
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let canonical = super::input_normalize::normalize_input_slot_key(param_path)
        .map(|slot| format!("inputs.{slot}"));
    if let Some(canonical_ref) = canonical.as_ref() {
        if source_refs.contains(canonical_ref) {
            return Some(canonical_ref.clone());
        }
        if canonical_ref == "inputs.token.address" {
            if let Some(mapped) = choose_token_address_source(source_refs) {
                return Some(mapped);
            }
        }
    }
    canonical
}

fn remap_semantic_alias(canonical: String, source_refs: &BTreeSet<String>) -> String {
    if source_refs.contains(canonical.as_str()) {
        return canonical;
    }
    if canonical == "inputs.token.address" {
        if let Some(mapped) = choose_token_address_source(source_refs) {
            return mapped;
        }
    }
    canonical
}

fn choose_token_address_source(source_refs: &BTreeSet<String>) -> Option<String> {
    let candidates = source_refs
        .iter()
        .filter(|reference| {
            reference.starts_with("inputs.")
                && reference.contains("token")
                && (reference.ends_with(".address") || reference.ends_with("_address"))
        })
        .cloned()
        .collect::<Vec<_>>();
    if candidates.len() == 1 {
        return candidates.first().cloned();
    }
    None
}

#[cfg(test)]
#[path = "tests/missing_input_module.rs"]
mod tests;
