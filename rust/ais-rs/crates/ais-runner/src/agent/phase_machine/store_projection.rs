use super::super::input_normalize::normalize_input_slot_key;
use super::super::runtime_facts_store::RuntimeFactsStore;
use super::super::*;
use ais_core::{stable_hash_hex, StableJsonOptions};
use ais_engine::EngineRunnerState;
use ais_sdk::documents::PlanSketchSegment;
use serde_json::{Map, Value};

#[derive(Debug, Clone)]
pub(crate) struct InputStoreSyncReport {
    pub(crate) synced_refs: Vec<String>,
    pub(crate) hash_changed: bool,
    pub(crate) previous_hash: Option<String>,
    pub(crate) current_hash: Option<String>,
}

pub(crate) fn sync_runtime_inputs_from_input_store(
    runtime: &mut Value,
    fact_store: &InputStore,
) -> InputStoreSyncReport {
    let previous_hash = stable_inputs_hash(runtime.pointer("/inputs"));
    if !runtime.is_object() {
        *runtime = Value::Object(Map::new());
    }
    if let Some(root) = runtime.as_object_mut() {
        root.insert("inputs".to_string(), Value::Object(Map::new()));
    }
    let mut synced_refs = Vec::<String>::new();
    for slot in fact_store.list_semantic_ref_strings() {
        let Some(value) = fact_store
            .get_semantic(slot.as_str())
            .map(|entry| entry.value.clone())
        else {
            continue;
        };
        super::super::input_normalize::set_runtime_input_value(runtime, slot.as_str(), value);
        synced_refs.push(format!("inputs.{slot}"));
    }
    let current_hash = stable_inputs_hash(runtime.pointer("/inputs"));
    InputStoreSyncReport {
        synced_refs,
        hash_changed: previous_hash != current_hash,
        previous_hash,
        current_hash,
    }
}

pub(crate) fn apply_segment_stores_from_runtime(
    segment: &PlanSketchSegment,
    state: &EngineRunnerState,
    runtime_facts_store: &mut RuntimeFactsStore,
    input_store: &mut InputStore,
    verbose_llm: bool,
) {
    for step in &segment.steps {
        if step.stores.is_empty() {
            continue;
        }
        let node_id = format!("{}/{}", segment.segment_id, step.id);
        let Some(node_outputs) = runtime_node_outputs(state, node_id.as_str()) else {
            continue;
        };
        for (return_field, slot_name) in &step.stores {
            let Some(value) = extract_store_value(node_outputs, return_field.as_str()) else {
                continue;
            };
            let provenance = format!("segment_store.{node_id}.{}", return_field.trim());
            let (target_ref, upsert_result) =
                if let Some(canonical_slot) = normalize_input_slot_key(slot_name) {
                    let target_ref = format!("inputs.{canonical_slot}");
                    let upsert_result = super::super::upsert_store_value_with_source(
                        input_store,
                        canonical_slot.as_str(),
                        value.clone(),
                        super::super::input_store::InputValueLayer::Observed,
                        "query",
                        90,
                        provenance.clone(),
                    );
                    (target_ref, upsert_result)
                } else if slot_name.trim().starts_with("facts.")
                    || slot_name.trim().starts_with("fact:")
                {
                    let fact_ref = slot_name.trim().replace("fact:", "facts.");
                    let upsert_result = super::super::upsert_runtime_fact_with_source(
                        runtime_facts_store,
                        fact_ref.as_str(),
                        value.clone(),
                        super::super::input_store::InputValueLayer::Observed,
                        "query",
                        90,
                        provenance.clone(),
                    );
                    (fact_ref, upsert_result)
                } else {
                    continue;
                };
            if verbose_llm {
                eprintln!(
                    "[agent] stores mapped node={} field={} -> ref={} upsert={:?}",
                    node_id, return_field, target_ref, upsert_result
                );
            }
        }
    }
}

/// Auto-project query node outputs to `InputStore` when they form bindable `inputs.*`.
/// Reusable semantic `facts.*` remain owned by `RuntimeFactsStore`.
pub(crate) fn auto_project_query_outputs_to_input_store(
    segment: &PlanSketchSegment,
    state: &EngineRunnerState,
    _runtime_facts_store: &mut RuntimeFactsStore,
    input_store: &mut InputStore,
    verbose_llm: bool,
) {
    for step in &segment.steps {
        if step.kind != "query" {
            continue;
        }
        let node_id = format!("{}/{}", segment.segment_id, step.id);
        let Some(node_outputs) = runtime_node_outputs(state, node_id.as_str()) else {
            continue;
        };
        let Some(outputs_obj) = node_outputs.as_object() else {
            continue;
        };
        let already_stored_fields = step
            .stores
            .keys()
            .map(|key| key.trim().to_string())
            .collect::<std::collections::BTreeSet<_>>();
        for (field_name, field_value) in outputs_obj {
            let field_name = field_name.trim();
            if field_name.is_empty() {
                continue;
            }
            if already_stored_fields.contains(field_name) {
                continue;
            }
            if !is_projectable_output_value(field_value) {
                continue;
            }
            let slot = derive_input_slot_from_step_output(step.id.as_str(), field_name);
            let input_ref = format!("inputs.{slot}");
            if input_store.get(slot.as_str()).is_some() {
                continue;
            }
            let provenance = format!("auto_project.{node_id}.{field_name}");
            let upsert_result = super::super::upsert_store_value_with_source(
                input_store,
                slot.as_str(),
                field_value.clone(),
                super::super::input_store::InputValueLayer::Observed,
                "query.auto_project",
                85,
                provenance.clone(),
            );
            if verbose_llm {
                eprintln!(
                    "[agent] auto_project node={} field={} -> ref={} upsert={:?}",
                    node_id, field_name, input_ref, upsert_result
                );
            }
        }
    }
}

fn is_projectable_output_value(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Array(items) => !items.is_empty(),
        Value::Object(object) => !object.is_empty(),
        Value::Bool(_) | Value::Number(_) | Value::String(_) => true,
    }
}

fn derive_input_slot_from_step_output(step_id: &str, field_name: &str) -> String {
    let clean_step = step_id
        .trim()
        .strip_prefix("q_")
        .or_else(|| step_id.trim().strip_prefix("query_"))
        .unwrap_or(step_id.trim());
    if field_name == "balance" || field_name == "result" || field_name == "value" {
        return clean_step.to_string();
    }
    if clean_step.ends_with(field_name) || field_name.contains(clean_step) {
        return field_name.to_string();
    }
    format!("{clean_step}_{field_name}")
}

fn stable_inputs_hash(value: Option<&Value>) -> Option<String> {
    stable_hash_hex(value?, &StableJsonOptions::default()).ok()
}

fn runtime_node_outputs<'a>(state: &'a EngineRunnerState, node_id: &str) -> Option<&'a Value> {
    let escaped = node_id.replace('~', "~0").replace('/', "~1");
    state
        .runtime
        .pointer(format!("/nodes/{escaped}/outputs").as_str())
}

fn extract_store_value(node_outputs: &Value, field: &str) -> Option<Value> {
    let field = field.trim();
    if field.is_empty() {
        return None;
    }
    if let Some(value) = value_at_dot_path(node_outputs, field) {
        return Some(value.clone());
    }
    if let Some(outputs_value) = node_outputs.get("outputs") {
        if let Some(value) = value_at_dot_path(outputs_value, field) {
            return Some(value.clone());
        }
    }
    None
}

fn value_at_dot_path<'a>(value: &'a Value, dot_path: &str) -> Option<&'a Value> {
    let mut current = value;
    for segment in dot_path.split('.').filter(|item| !item.is_empty()) {
        current = current.get(segment)?;
    }
    Some(current)
}
