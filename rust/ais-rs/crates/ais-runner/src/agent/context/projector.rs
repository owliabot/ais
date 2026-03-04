use super::super::input_store::InputStore;
use super::super::intent_context::IntentContext;
use super::collector::{
    build_canonical_context_projection, build_input_registry_projection,
    build_input_slots_projection, build_node_output_refs_projection,
};
use ais_engine::EngineRunnerState;
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;
const INPUT_BINDABLE_SOURCE_OF_TRUTH: &str = "state_summary.input_store";
const INPUT_BINDABLE_REFS_PROJECTION_PATH: &str = "state_summary.input_registry.known_refs";
const INPUT_BINDING_SCHEMA: &str = "ais-agent-input-binding-contract/0.0.1";

pub(in super::super) fn build_projected_summary_base(
    state: &EngineRunnerState,
    completed_segments: usize,
    done: bool,
    previous_error: Option<&Value>,
    input_store: Option<&InputStore>,
    tool_memory_projection: Option<&Value>,
) -> Value {
    let input_slots = build_input_slots_projection(state, input_store);
    let input_binding = build_input_binding_contract();
    json!({
        "completed_segments": completed_segments,
        "completed_nodes": state.completed_node_ids.len(),
        "plan_epoch": state.plan_epoch,
        "paused_reason": state.paused_reason,
        "done": done,
        "previous_error": previous_error,
        "input_store": input_store.map(InputStore::to_projected_planning_value),
        "input_binding": input_binding,
        "input_slots": input_slots.value,
        "input_registry": build_input_registry_projection(&input_slots.resolved, input_slots.missing.as_slice()),
        "canonical_context": build_canonical_context_projection(&input_slots.resolved),
        "node_output_refs": build_node_output_refs_projection(state),
        "tool_memory_projection": tool_memory_projection,
        "intent_slots": build_intent_slots_projection(state),
        "intent_context": IntentContext::from_runtime(&state.runtime).map(|context| context.projection().clone()),
        "capability_view": state.runtime.pointer("/agent/capability_view"),
        "capability_ready": state.runtime.pointer("/agent/capability_ready"),
        "side_effect_lifecycle": state.runtime.pointer("/agent/side_effect_lifecycle"),
        "todo_state": state.runtime.pointer("/agent/todo_progress"),
    })
}

fn build_input_binding_contract() -> Value {
    json!({
        "schema": INPUT_BINDING_SCHEMA,
        "bindable_namespace": "inputs",
        "bindable_refs_source": INPUT_BINDABLE_SOURCE_OF_TRUTH,
        "bindable_refs_projection": INPUT_BINDABLE_REFS_PROJECTION_PATH,
        "known_refs_only": true,
        "facts_bindable": false,
    })
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
