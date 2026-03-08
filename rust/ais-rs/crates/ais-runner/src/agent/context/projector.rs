use super::super::execution_view::build_reusable_output_inventory_projection;
use super::super::input_store::InputStore;
use super::super::intent_context::IntentContext;
use super::super::runtime_facts_store::RuntimeFactsStore;
use super::super::state_summary::{InputBindingContract, StateSummary};
use super::collector::{
    build_input_registry_projection, build_input_slots_projection,
    build_node_output_refs_projection,
};
use ais_engine::EngineRunnerState;
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;
const INPUT_BINDABLE_SOURCE_OF_TRUTH: &str = "state_summary.input_store";
const INPUT_BINDABLE_REFS_PROJECTION_PATH: &str = "state_summary.input_registry.known_refs";
const INPUT_BINDING_SCHEMA: &str = "ais-agent-input-binding-contract/0.0.1";
const INPUT_BINDING_CONTRACT: InputBindingContract = InputBindingContract {
    schema: INPUT_BINDING_SCHEMA,
    bindable_namespace: "inputs",
    bindable_refs_source: INPUT_BINDABLE_SOURCE_OF_TRUTH,
    bindable_refs_projection: INPUT_BINDABLE_REFS_PROJECTION_PATH,
    known_refs_only: true,
    facts_bindable: false,
};

pub(in super::super) fn build_projected_summary_base_with_runtime_facts(
    state: &EngineRunnerState,
    completed_segments: usize,
    done: bool,
    previous_error: Option<&Value>,
    input_store: Option<&InputStore>,
    runtime_facts_store: Option<&RuntimeFactsStore>,
    tool_memory_projection: Option<&Value>,
) -> StateSummary {
    let input_slots = build_input_slots_projection(state, input_store);
    let intent_context =
        IntentContext::from_runtime(&state.runtime).map(|context| context.projection().clone());
    StateSummary {
        completed_segments,
        completed_nodes: state.completed_node_ids.len(),
        plan_epoch: state.plan_epoch,
        paused_reason: state.paused_reason.clone(),
        done,
        previous_error: previous_error.cloned(),
        input_store: input_store.map(InputStore::to_projected_planning_value),
        runtime_facts: runtime_facts_store.map(RuntimeFactsStore::to_projected_planning_value),
        input_binding: INPUT_BINDING_CONTRACT,
        input_registry: build_input_registry_projection(
            &input_slots.resolved,
            input_slots.missing.as_slice(),
        ),
        node_output_refs: build_node_output_refs_projection(state),
        reusable_outputs: build_reusable_output_inventory_projection(
            runtime_facts_store,
            input_store,
        ),
        tool_memory_projection: tool_memory_projection.cloned(),
        intent_slots: Some(build_intent_slots_projection(state)).filter(|v| !v.is_null()),
        intent_context,
        capability_view: state.runtime.pointer("/agent/capability_view").cloned(),
        capability_ready: state
            .runtime
            .pointer("/agent/capability_ready")
            .and_then(Value::as_bool),
        side_effect_lifecycle: state
            .runtime
            .pointer("/agent/side_effect_lifecycle")
            .cloned(),
        todo_state: state.runtime.pointer("/agent/todo_progress").cloned(),
        recovery_diagnostics: build_recovery_diagnostics_projection(state, previous_error),
    }
}

fn build_recovery_diagnostics_projection(
    state: &EngineRunnerState,
    previous_error: Option<&Value>,
) -> Option<Value> {
    let query_round = state
        .runtime
        .pointer("/agent/missing_input_autofill/query_autofill_round")
        .cloned();
    let recent_attempts = state
        .runtime
        .pointer("/agent/missing_input_autofill/query_attempts")
        .and_then(Value::as_array)
        .map(|items| {
            let keep = items.len().saturating_sub(8);
            Value::Array(items.iter().skip(keep).cloned().collect::<Vec<_>>())
        });

    let mut available_attempt_keys = BTreeSet::<String>::new();
    if let Some(keys) = previous_error
        .and_then(|value| value.pointer("/autofill_history/attempt_keys"))
        .and_then(Value::as_array)
    {
        for key in keys.iter().filter_map(Value::as_str) {
            let key = key.trim();
            if !key.is_empty() {
                available_attempt_keys.insert(key.to_string());
            }
        }
    }
    if let Some(attempts) = recent_attempts.as_ref().and_then(Value::as_array) {
        for attempt in attempts {
            if let Some(mode) = attempt.get("status").and_then(Value::as_str) {
                available_attempt_keys.insert(format!("attempt_status:{mode}"));
            }
            if let Some(query_ref) = attempt.get("query_ref").and_then(Value::as_str) {
                if !query_ref.trim().is_empty() {
                    available_attempt_keys.insert(format!("query_ref:{query_ref}"));
                }
            }
            if let Some(missing_ref) = attempt.get("missing_ref").and_then(Value::as_str) {
                if !missing_ref.trim().is_empty() {
                    available_attempt_keys.insert(format!("missing_ref:{missing_ref}"));
                }
            }
        }
    }

    if let Some(mode) = previous_error
        .and_then(|value| value.pointer("/autofill/mode"))
        .and_then(Value::as_str)
    {
        available_attempt_keys.insert(format!("mode:{mode}"));
    }
    if let Some(refs) = previous_error
        .and_then(|value| value.pointer("/autofill/selected_query_refs"))
        .and_then(Value::as_array)
    {
        for query_ref in refs.iter().filter_map(Value::as_str) {
            let query_ref = query_ref.trim();
            if !query_ref.is_empty() {
                available_attempt_keys.insert(format!("query_ref:{query_ref}"));
            }
        }
    }

    let last_error_code = previous_error
        .and_then(|value| value.get("reason_code"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let last_error_reason = previous_error
        .and_then(|value| value.get("message"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            previous_error
                .and_then(|value| value.get("sub_reason_code"))
                .and_then(Value::as_str)
                .map(str::to_string)
        });

    let recoverable_candidates_remaining = query_round
        .as_ref()
        .and_then(|value| value.get("terminal_reason"))
        .and_then(Value::as_str)
        .map(|reason| reason != "no_query_candidates" && reason != "router_unavailable")
        .unwrap_or(false);

    if query_round.is_none()
        && recent_attempts.is_none()
        && available_attempt_keys.is_empty()
        && last_error_code.is_none()
        && last_error_reason.is_none()
    {
        return None;
    }

    Some(json!({
        "schema": "ais-agent-recovery-diagnostics/0.0.1",
        "recent_query_autofill_round": query_round,
        "recent_attempts": recent_attempts,
        "last_error_code": last_error_code,
        "last_error_reason": last_error_reason,
        "recoverable_candidates_remaining": recoverable_candidates_remaining,
        "available_attempt_keys": available_attempt_keys.into_iter().collect::<Vec<_>>(),
    }))
}

fn build_intent_slots_projection(state: &EngineRunnerState) -> Value {
    let Some(grounding) = state.runtime.pointer("/agent/intent_grounding") else {
        return Value::Null;
    };
    let Some(grounding_obj) = grounding.as_object() else {
        return grounding.clone();
    };

    let mut out = Map::<String, Value>::new();
    let (resolved_inputs, resolved_input_refs) = normalize_resolved_inputs_for_projection(
        grounding_obj
            .get("resolved_inputs")
            .and_then(Value::as_object),
    );
    out.insert(
        "resolved_inputs".to_string(),
        Value::Object(resolved_inputs),
    );
    out.insert(
        "resolved_input_refs".to_string(),
        Value::Array(
            resolved_input_refs
                .into_iter()
                .map(Value::String)
                .collect::<Vec<_>>(),
        ),
    );

    out.insert(
        "confidence".to_string(),
        json!({
            "inputs": normalize_grounding_confidence_for_projection(
            grounding_obj.get("confidence").and_then(Value::as_object),
        )}),
    );
    out.insert(
        "input_binding".to_string(),
        json!({
            "role": "grounding_intermediate",
            "bindable": false,
            "source_of_truth": "state_summary.input_store",
        }),
    );
    Value::Object(out)
}

fn normalize_resolved_inputs_for_projection(
    resolved_inputs: Option<&Map<String, Value>>,
) -> (Map<String, Value>, Vec<String>) {
    let mut normalized = Map::<String, Value>::new();
    let mut refs = BTreeSet::<String>::new();
    let Some(resolved_inputs) = resolved_inputs else {
        return (normalized, Vec::new());
    };
    for (raw_key, raw_value) in resolved_inputs {
        let canonical = super::super::input_normalize::normalize_grounding_input_key(raw_key);
        let Some(slot) =
            super::super::input_normalize::normalize_input_slot_key(canonical.as_str())
        else {
            continue;
        };
        if !is_bindable_input_slot(slot.as_str()) {
            continue;
        }
        normalized.insert(slot.clone(), raw_value.clone());
        refs.insert(format!("inputs.{slot}"));
    }
    (normalized, refs.into_iter().collect::<Vec<_>>())
}

fn normalize_grounding_confidence_for_projection(
    confidence: Option<&Map<String, Value>>,
) -> Map<String, Value> {
    let mut input_confidence = Map::<String, Value>::new();
    let Some(confidence) = confidence else {
        return input_confidence;
    };

    for (key, score) in confidence {
        let Some(score_u64) = score.as_u64() else {
            continue;
        };
        if key.starts_with("fact:") {
            continue;
        }
        let canonical = super::super::input_normalize::normalize_grounding_input_key(key.as_str());
        if let Some(slot) =
            super::super::input_normalize::normalize_input_slot_key(canonical.as_str())
        {
            if !is_bindable_input_slot(slot.as_str()) {
                continue;
            }
            input_confidence.insert(format!("inputs.{slot}"), Value::Number(score_u64.into()));
            continue;
        }
    }
    input_confidence
}

fn is_bindable_input_slot(slot: &str) -> bool {
    let lowered = slot.to_ascii_lowercase();
    !slot.contains(':')
        && !lowered.starts_with("facts.")
        && !lowered.starts_with("fact.")
        && !lowered.starts_with("fact:")
}
