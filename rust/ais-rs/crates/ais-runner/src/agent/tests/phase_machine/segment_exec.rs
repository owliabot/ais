use super::*;
use ais_engine::{EngineEvent, EngineEventType};
use serde_json::json;

#[test]
fn apply_segment_stores_projects_query_and_action_outputs() {
    let segment: PlanSketchSegment = serde_json::from_value(json!({
        "segment_id":"seg_1",
        "cursor_in":"0",
        "cursor_out":"1",
        "done":false,
        "steps":[
            {
                "id":"q_balance",
                "kind":"query",
                "candidate_ref":"demo@0.0.2/quote",
                "inputs":{},
                "stores":{"balance":"facts.balance"}
            },
            {
                "id":"a_transfer",
                "kind":"action",
                "candidate_ref":"demo@0.0.2/swap",
                "inputs":{},
                "stores":{"tx_hash":"tx.hash","confirmed":"tx.confirmed"}
            }
        ]
    }))
    .expect("segment");
    let state = EngineRunnerState {
        runtime: json!({
            "nodes": {
                "seg_1/q_balance": {"outputs":{"balance":"100"}},
                "seg_1/a_transfer": {"outputs":{"outputs":{"tx_hash":"0xabc","confirmed":true}}}
            }
        }),
        ..EngineRunnerState::default()
    };
    let mut fact_store = InputStore::default();

    apply_segment_stores_from_runtime(&segment, &state, &mut fact_store, false);

    assert!(fact_store.get("facts.balance").is_none());
    assert_eq!(
        fact_store
            .get("inputs.tx.hash")
            .and_then(|entry| entry.value.as_str()),
        Some("0xabc")
    );
    assert_eq!(
        fact_store
            .get("inputs.tx.confirmed")
            .and_then(|entry| entry.value.as_bool()),
        Some(true)
    );
}

#[test]
fn apply_segment_stores_canonicalizes_inputs_slot_names() {
    let segment: PlanSketchSegment = serde_json::from_value(json!({
        "segment_id":"seg_input",
        "cursor_in":"0",
        "cursor_out":"1",
        "done":false,
        "steps":[
            {
                "id":"q_token",
                "kind":"query",
                "candidate_ref":"erc20@0.0.2/decimals",
                "inputs":{},
                "stores":{
                    "decimals":"inputs.token.decimals"
                }
            }
        ]
    }))
    .expect("segment");
    let state = EngineRunnerState {
        runtime: json!({
            "nodes": {
                "seg_input/q_token": {"outputs":{"decimals":18}},
            }
        }),
        ..EngineRunnerState::default()
    };
    let mut fact_store = InputStore::default();

    apply_segment_stores_from_runtime(&segment, &state, &mut fact_store, false);

    assert_eq!(
        fact_store
            .get("inputs.token.decimals")
            .and_then(|entry| entry.value.as_u64()),
        Some(18)
    );
    assert_eq!(
        fact_store
            .get("token.decimals")
            .and_then(|entry| entry.value.as_u64()),
        Some(18)
    );
    assert_eq!(
        fact_store
            .get("inputs.token.decimals")
            .map(|entry| entry.meta.source.as_str()),
        Some("query")
    );
    assert_eq!(
        fact_store
            .get("inputs.token.decimals")
            .and_then(|entry| entry.meta.provenance.as_deref()),
        Some("segment_store.seg_input/q_token.decimals")
    );
}

#[test]
fn bind_segment_todo_id_writes_segment_extension() {
    let mut segment: PlanSketchSegment = serde_json::from_value(json!({
        "segment_id":"seg_1",
        "cursor_in":"0",
        "cursor_out":"1",
        "done":false,
        "steps":[
            {"id":"q1","kind":"query","candidate_ref":"demo@0.0.2/quote","inputs":{}}
        ]
    }))
    .expect("segment");
    bind_segment_todo_id(&mut segment, "todo_1");
    assert_eq!(segment.extensions.get("todo_id"), Some(&json!("todo_1")));
}

#[test]
fn annotate_events_with_todo_adds_agent_extension() {
    let segment: PlanSketchSegment = serde_json::from_value(json!({
        "segment_id":"seg_1",
        "cursor_in":"0",
        "cursor_out":"1",
        "done":false,
        "steps":[
            {"id":"q1","kind":"query","candidate_ref":"demo@0.0.2/quote","inputs":{}}
        ]
    }))
    .expect("segment");
    let mut node_event = EngineEvent::new(EngineEventType::NodeReady);
    node_event.node_id = Some("seg_1/q1".to_string());
    let events = vec![
        EngineEventRecord::new("run-1", 1, "1970-01-01T00:00:00Z", node_event),
        EngineEventRecord::new(
            "run-1",
            2,
            "1970-01-01T00:00:00Z",
            EngineEvent::new(EngineEventType::PlanReplaced),
        ),
    ];

    let annotated = annotate_events_with_todo(events.as_slice(), &segment, "todo_1");
    let ext0 = Value::Object(annotated[0].event.extensions.clone());
    let ext1 = Value::Object(annotated[1].event.extensions.clone());
    assert_eq!(ext0.pointer("/agent/todo_id"), Some(&json!("todo_1")));
    assert_eq!(ext0.pointer("/agent/segment_id"), Some(&json!("seg_1")));
    assert_eq!(ext0.pointer("/agent/step_id"), Some(&json!("q1")));
    assert_eq!(ext1.pointer("/agent/todo_id"), Some(&json!("todo_1")));
}

#[test]
fn build_todo_receipt_collects_completed_nodes_and_tx_hashes() {
    let segment: PlanSketchSegment = serde_json::from_value(json!({
        "segment_id":"seg_1",
        "cursor_in":"0",
        "cursor_out":"1",
        "done":false,
        "steps":[
            {"id":"q1","kind":"query","candidate_ref":"demo@0.0.2/quote","inputs":{}},
            {"id":"a1","kind":"action","candidate_ref":"demo@0.0.2/swap","inputs":{}}
        ]
    }))
    .expect("segment");
    let state = EngineRunnerState {
        completed_node_ids: vec!["seg_1/q1".to_string()],
        paused_reason: Some("need_user_confirm:seg_1/a1".to_string()),
        runtime: json!({
            "nodes":{
                "seg_1/a1":{"outputs":{"tx_hash":"0xabc","nested":{"signed_tx_hash":"0xdef"}}}
            }
        }),
        ..EngineRunnerState::default()
    };
    let events = vec![EngineEventRecord::new(
        "run-1",
        3,
        "1970-01-01T00:00:00Z",
        EngineEvent::new(EngineEventType::NeedUserConfirm),
    )];
    let receipt = build_todo_receipt(
        "todo_1",
        &segment,
        EngineRunStatus::Paused,
        &state,
        events.as_slice(),
    );
    assert_eq!(receipt.todo_id, "todo_1");
    assert_eq!(receipt.segment_id, "seg_1");
    assert_eq!(receipt.status, "paused");
    assert_eq!(receipt.completed_node_ids, vec!["seg_1/q1".to_string()]);
    assert_eq!(
        receipt.tx_hashes,
        vec!["0xabc".to_string(), "0xdef".to_string()]
    );
    assert_eq!(receipt.event_types, vec!["need_user_confirm".to_string()]);
    assert_eq!(receipt.event_count, 1);
}
