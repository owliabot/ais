use super::*;
use serde_json::json;

#[test]
fn decode_and_encode_preserves_unknown_extensions() {
    let mut input = Map::<String, Value>::new();
    input.insert("planning_memory".to_string(), json!({"snapshot":"abc"}));
    input.insert(
        "todo_progress".to_string(),
        json!({"current_todo":{"id":"todo_1"}}),
    );
    input.insert(
        "intent_facts".to_string(),
        json!({"recipient":"0xabc","amount":"1"}),
    );
    let decoded = AgentCheckpointExtensions::decode(Some(&input));
    assert!(decoded.planning_memory().is_some());
    assert!(decoded.input_store().is_none());
    assert!(decoded.todo_progress().is_some());
    assert!(decoded.intent_facts().is_some());

    let mut input_store = InputStore::default();
    input_store.upsert_seed("owner", json!("0xabc"), "runtime.inputs.owner");
    let output = decoded.encode_updated(
        Some(json!({"snapshot":"next"})),
        &input_store,
        Some(&json!({"current_todo":{"id":"todo_2"}})),
        Some(&BTreeMap::from([("recipient".to_string(), json!("0xdef"))])),
    );
    assert_eq!(
        output.get("planning_memory"),
        Some(&json!({"snapshot":"next"}))
    );
    assert_eq!(
        output
            .get("input_store")
            .and_then(|value| value.pointer("/entries/owner/value")),
        Some(&json!("0xabc"))
    );
    assert_eq!(
        output
            .get("todo_progress")
            .and_then(|value| value.pointer("/current_todo/id")),
        Some(&json!("todo_2"))
    );
    assert_eq!(
        output
            .get("intent_facts")
            .and_then(|value| value.get("recipient")),
        Some(&json!("0xdef"))
    );
}

#[test]
fn decode_input_store_only_payload_rebuilds_fact_projection() {
    let mut input = Map::<String, Value>::new();
    input.insert(
        "input_store".to_string(),
        json!({
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

#[test]
fn todo_progress_receipt_tx_hashes_roundtrip() {
    let mut input = Map::<String, Value>::new();
    input.insert(
        "todo_progress".to_string(),
        json!({
            "schema":"ais-agent-todo-progress/0.0.1",
            "current_todo":{
                "id":"todo_1",
                "status":"in_progress",
                "receipt":{
                    "schema":"ais-agent-todo-receipt/0.0.1",
                    "todo_id":"todo_1",
                    "segment_id":"seg_1",
                    "status":"paused",
                    "tx_hashes":["0xabc","0xdef"]
                }
            },
            "todos":[],
            "progress":{"todo":0,"in_progress":1,"done":0,"blocked":0,"total":1},
            "next_seq":2
        }),
    );
    let decoded = AgentCheckpointExtensions::decode(Some(&input));
    let output =
        decoded.encode_updated(None, &InputStore::default(), decoded.todo_progress(), None);
    assert_eq!(
        output
            .get("todo_progress")
            .and_then(|value| value.pointer("/current_todo/receipt/tx_hashes/0")),
        Some(&json!("0xabc"))
    );
    assert_eq!(
        output
            .get("todo_progress")
            .and_then(|value| value.pointer("/current_todo/receipt/tx_hashes/1")),
        Some(&json!("0xdef"))
    );
}

#[test]
fn decode_todo_progress_normalizes_legacy_receipt_tx_hashes_shape() {
    let mut input = Map::<String, Value>::new();
    input.insert(
        "todo_progress".to_string(),
        json!({
            "schema":"ais-agent-todo-progress/0.0.1",
            "current_todo":{
                "id":"todo_1",
                "status":"in_progress",
                "receipt":{
                    "schema":"ais-agent-todo-receipt/0.0.1",
                    "todo_id":"todo_1",
                    "segment_id":"seg_1",
                    "status":"paused",
                    "tx_hashes":"0xabc"
                }
            },
            "todos":[
                {
                    "id":"todo_1",
                    "status":"in_progress",
                    "receipt":{
                        "schema":"ais-agent-todo-receipt/0.0.1",
                        "todo_id":"todo_1",
                        "segment_id":"seg_1",
                        "status":"paused",
                        "tx_hashes":null
                    }
                }
            ],
            "next_seq":2
        }),
    );
    let decoded = AgentCheckpointExtensions::decode(Some(&input));
    let output =
        decoded.encode_updated(None, &InputStore::default(), decoded.todo_progress(), None);
    assert_eq!(
        output
            .get("todo_progress")
            .and_then(|value| value.pointer("/current_todo/receipt/tx_hashes")),
        Some(&json!(["0xabc"]))
    );
    assert_eq!(
        output
            .get("todo_progress")
            .and_then(|value| value.pointer("/todos/0/receipt/tx_hashes")),
        Some(&json!([]))
    );
}
