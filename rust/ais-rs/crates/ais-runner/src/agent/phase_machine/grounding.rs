use super::super::orchestrator::SegmentedAgentContext;
use super::super::*;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};

const GROUND_INPUT_CONFIDENCE_THRESHOLD: u8 = 80;
const GROUND_FACT_CONFIDENCE_THRESHOLD: u8 = 65;
const INPUT_BINDABLE_REFS_SOURCE_PATH: &str = "state_summary.input_registry.known_refs";

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
            state_summary: context.state_summary().clone(),
        });
        super::super::orchestrator::refresh_tool_memory_projection(context, planner, state);
        match handle_grounding_draft(command, state, context, candidate_context, draft_result)? {
            GroundingDraftOutcome::Ready(ready) => return Ok(ready),
            GroundingDraftOutcome::RetryAutofill => {
                if autofill_retry_budget == 0 {
                    return Ok(false);
                }
                autofill_retry_budget = autofill_retry_budget.saturating_sub(1);
                super::super::trace::emit(
                    trace_enabled,
                    "grounding",
                    "autofill_retry",
                    &[("remaining_budget", autofill_retry_budget.to_string())],
                );
            }
        }
    }
}

enum GroundingDraftOutcome {
    Ready(bool),
    RetryAutofill,
}

fn handle_grounding_draft(
    command: &AgentCommand,
    state: &mut EngineRunnerState,
    context: &mut SegmentedAgentContext,
    candidate_context: &CandidateContext,
    draft_result: Result<IntentGroundingDraft, RunnerError>,
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
            let answered_questions = if let Some(answers) =
                super::super::missing_input::maybe_collect_and_apply_answers(
                    state,
                    context.input_store_mut(),
                    questions.as_slice(),
                )? {
                answers
            } else {
                Map::new()
            };
            let remaining_questions = filter_unanswered_questions(
                questions.as_slice(),
                answered_questions.keys().collect::<Vec<_>>().as_slice(),
            );
            let ready = ready_for_todos && remaining_questions.is_empty();
            super::super::trace::emit(
                trace_enabled,
                "grounding",
                "draft_proposed",
                &[
                    ("ready_for_todos", ready.to_string()),
                    ("remaining_questions", remaining_questions.len().to_string()),
                ],
            );
            super::super::runtime_store::record_runtime_agent_field(
                &mut state.runtime,
                "intent_grounding",
                json!({
                    "status":"proposed",
                    "summary": summary,
                    "ready_for_todos": ready,
                    "resolved_inputs": resolved_inputs,
                    "intent_facts": intent_facts,
                    "confidence": confidence,
                    "issues": issues,
                    "questions": remaining_questions,
                    "answers": answered_questions,
                    "applied": apply_summary.applied,
                    "skipped_low_confidence": apply_summary.skipped_low_confidence,
                    "deterministic_rule_inputs": apply_summary.deterministic_applied,
                    "deterministic_rule_skipped": apply_summary.deterministic_skipped,
                    "deterministic_conflicts": apply_summary.deterministic_conflicts,
                    "deterministic_conflict_policy": "rule_extracted_over_llm",
                    "resolved_input_refs": collect_bindable_input_refs(&resolved_inputs),
                    "input_binding": grounding_input_binding_metadata(),
                }),
            );
            context.refresh_state_summary(state, false);
            if !ready {
                let payload = super::super::missing_input::payload(
                    Some("intent_grounding_missing_inputs"),
                    remaining_questions.as_slice(),
                    &[],
                    context.completed_segments_u8(),
                );
                if super::super::orchestrator::try_schedule_missing_input_query_autofill_round(
                    command,
                    state,
                    context,
                    &payload,
                    candidate_context,
                    "grounding",
                    false,
                    "grounding",
                ) {
                    return Ok(GroundingDraftOutcome::RetryAutofill);
                }
                context.set_previous_error_and_refresh(
                    state,
                    false,
                    super::super::grounding_phase_error_payload(
                        "missing_required_input",
                        Some("intent_grounding_missing_inputs"),
                        &[],
                        remaining_questions.as_slice(),
                        context.completed_segments_u8(),
                    ),
                );
                super::super::missing_input::pause_with_payload(state, &payload);
                super::super::trace::emit(
                    trace_enabled,
                    "pause_resolution",
                    "paused_missing_required_input",
                    &[("phase_hint", "grounding".to_string())],
                );
                return Ok(GroundingDraftOutcome::Ready(false));
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
        } => {
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
                let payload = super::super::missing_input::payload(
                    message.as_deref(),
                    questions.as_slice(),
                    issues.as_slice(),
                    context.completed_segments_u8(),
                );
                if super::super::orchestrator::try_schedule_missing_input_query_autofill_round(
                    command,
                    state,
                    context,
                    &payload,
                    candidate_context,
                    "grounding",
                    false,
                    "grounding",
                ) {
                    return Ok(GroundingDraftOutcome::RetryAutofill);
                }
                if let Some(answers) = super::super::missing_input::maybe_collect_and_apply_answers(
                    state,
                    context.input_store_mut(),
                    questions.as_slice(),
                )? {
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
                    return Ok(GroundingDraftOutcome::Ready(true));
                }
                super::super::missing_input::pause_with_payload(state, &payload);
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
    if ready_flag {
        return true;
    }
    let has_questions = grounding
        .get("questions")
        .and_then(Value::as_array)
        .is_some_and(|questions| !questions.is_empty());
    if has_questions {
        return false;
    }
    grounding
        .get("resolved_inputs")
        .and_then(Value::as_object)
        .is_some_and(|resolved| !resolved.is_empty())
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
        "bindable_refs_source": INPUT_BINDABLE_REFS_SOURCE_PATH,
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

#[cfg(test)]
#[path = "../tests/phase_machine/grounding.rs"]
mod tests;
