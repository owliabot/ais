use super::*;
use serde_json::json;

#[test]
fn decode_and_encode_preserves_unknown_extensions() {
    let mut input = Map::<String, Value>::new();
    input.insert(
        "resume_core".to_string(),
        json!({"planning_memory":{"snapshot":"abc"}}),
    );
    input.insert("legacy_block".to_string(), json!({"ignored":true}));
    let decoded = AgentCheckpointExtensions::decode(Some(&input));
    assert!(decoded.planning_memory().is_some());
    assert!(decoded.input_store().is_none());

    let mut input_store = InputStore::default();
    input_store.upsert_seed("owner", json!("0xabc"), "runtime.inputs.owner");
    let output = decoded.encode_updated_with_runtime_facts(
        Some(json!({"snapshot":"next"})),
        &input_store,
        &super::super::runtime_facts_store::RuntimeFactsStore::default(),
    );
    let output_value = Value::Object(output.clone());
    assert_eq!(
        output_value.pointer("/resume_core/planning_memory"),
        Some(&json!({"snapshot":"next"}))
    );
    assert_eq!(
        output_value.pointer("/resume_core/input_store/entries/owner/value"),
        Some(&json!("0xabc"))
    );
    assert!(output.get("derived_projections").is_none());
    assert!(output.get("todo_progress").is_none());
    assert!(output.get("intent_facts").is_none());
    assert!(output.get("legacy_block").is_none());
}

#[test]
fn decode_input_store_only_payload_rebuilds_fact_projection() {
    let mut input = Map::<String, Value>::new();
    input.insert(
        "resume_core".to_string(),
        json!({
            "input_store":{
              "entries":{
                "owner":{
                    "value":"0xdef",
                    "meta":{
                        "source":"user",
                        "source_priority":100,
                        "provenance":"checkpoint.input_store.owner",
                        "confidence":null
                    }
                },
                "token.decimals":{
                    "value":6,
                    "meta":{
                        "source":"runtime",
                        "source_priority":70,
                        "provenance":"checkpoint.input_store.token.decimals",
                        "confidence":null
                    }
                }
              }
            }
        }),
    );
    let decoded = AgentCheckpointExtensions::decode(Some(&input));
    let store = decoded.input_store().expect("input store");
    assert_eq!(
        store.get("owner").map(|entry| entry.value.clone()),
        Some(json!("0xdef"))
    );
    assert_eq!(
        store.get("owner").map(|entry| entry.value.clone()),
        Some(json!("0xdef"))
    );
    assert_eq!(
        store.get("token.decimals").map(|entry| entry.value.clone()),
        Some(json!(6))
    );
}
