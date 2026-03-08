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
    let mut runtime_facts_store =
        super::super::super::runtime_facts_store::RuntimeFactsStore::default();

    apply_segment_stores_from_runtime_with_runtime_facts(
        &segment,
        &state,
        &mut runtime_facts_store,
        &mut fact_store,
        false,
    );

    assert!(fact_store.get("facts.balance").is_none());
    assert_eq!(
        runtime_facts_store
            .get("facts.balance")
            .and_then(|entry| entry.value.as_str()),
        Some("100")
    );
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
    let mut runtime_facts_store =
        super::super::super::runtime_facts_store::RuntimeFactsStore::default();

    apply_segment_stores_from_runtime_with_runtime_facts(
        &segment,
        &state,
        &mut runtime_facts_store,
        &mut fact_store,
        false,
    );

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
        runtime_facts_store
            .get("inputs.token.decimals")
            .and_then(|entry| entry.value.as_u64()),
        None
    );
    assert_eq!(
        runtime_facts_store
            .get("inputs.token.decimals")
            .and_then(|entry| entry.value.as_u64()),
        None
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
fn build_todo_receipt_without_ledger_keeps_tx_hashes_empty() {
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
    let receipt = super::super::super::receipt_view::build_segment_todo_receipt(
        "todo_1",
        &segment,
        EngineRunStatus::Paused,
        &state,
        events.as_slice(),
        None,
    );
    assert_eq!(receipt.todo_id, "todo_1");
    assert_eq!(receipt.segment_id, "seg_1");
    assert_eq!(receipt.status, "paused");
    assert_eq!(receipt.completed_node_ids, vec!["seg_1/q1".to_string()]);
    assert!(receipt.tx_hashes.is_empty());
    assert_eq!(receipt.event_types, vec!["need_user_confirm".to_string()]);
    assert_eq!(receipt.event_count, 1);
}

#[test]
fn has_duplicate_command_id_rejection_detects_rejected_event_reason() {
    let mut duplicate = EngineEvent::new(EngineEventType::CommandRejected);
    duplicate.data =
        serde_json::Map::from_iter([("reason_code".to_string(), json!("duplicate_command_id"))]);
    let records = vec![EngineEventRecord::new(
        "run-1",
        1,
        "1970-01-01T00:00:00Z",
        duplicate,
    )];
    assert!(has_duplicate_command_id_rejection(records.as_slice()));

    let mut other = EngineEvent::new(EngineEventType::CommandRejected);
    other.data =
        serde_json::Map::from_iter([("reason_code".to_string(), json!("validation_failed"))]);
    let records = vec![EngineEventRecord::new(
        "run-1",
        2,
        "1970-01-01T00:00:00Z",
        other,
    )];
    assert!(!has_duplicate_command_id_rejection(records.as_slice()));
}

#[test]
fn sync_runtime_inputs_from_input_store_rebuilds_inputs_tree() {
    let segment: PlanSketchSegment = serde_json::from_value(json!({
        "segment_id":"seg_1",
        "cursor_in":"0",
        "cursor_out":"1",
        "done":false,
        "steps":[
            {
                "id":"q_balance",
                "kind":"query",
                "candidate_ref":"erc20@0.0.2/balance-of",
                "inputs":{
                    "owner":{"ref":"inputs.owner"},
                    "token":{"object":{"address":{"ref":"inputs.tst_token_address"}}}
                }
            }
        ]
    }))
    .expect("segment");
    let mut store = InputStore::default();
    store.upsert_seed(
        "owner",
        json!("0x70997970c51812dc3a010c7d01b50e0d17dc79c8"),
        "test.owner",
    );
    store.upsert_seed(
        "tst_token_address",
        json!("0x8464135c8F25Da09e49BC8782676a84730C318bC"),
        "test.tst",
    );
    let mut runtime = json!({
        "inputs": {
            "stale_value": true
        }
    });

    let sync = sync_runtime_inputs_from_input_store(&mut runtime, &store);
    assert_eq!(sync.synced_refs.len(), 2);
    assert!(sync.hash_changed);
    assert_eq!(
        runtime.pointer("/inputs/owner"),
        Some(&json!("0x70997970c51812dc3a010c7d01b50e0d17dc79c8"))
    );
    assert_eq!(
        runtime.pointer("/inputs/tst_token_address"),
        Some(&json!("0x8464135c8F25Da09e49BC8782676a84730C318bC"))
    );
    assert!(runtime.pointer("/inputs/stale_value").is_none());
    assert!(collect_segment_ref_closure(&segment).contains(&"inputs.owner".to_string()));
}

#[test]
fn collect_segment_input_ref_closure_includes_when_until_and_constraints() {
    let segment: PlanSketchSegment = serde_json::from_value(json!({
        "segment_id":"seg_closure",
        "cursor_in":"0",
        "cursor_out":"1",
        "done":false,
        "steps":[
            {
                "id":"q1",
                "kind":"query",
                "candidate_ref":"demo@0.0.2/read",
                "inputs":{"owner":{"ref":"inputs.owner"}},
                "when":{"cel":"inputs.balance_threshold > 0 && nodes.q0.outputs.value > inputs.min_required"},
                "until":{"cel":"inputs.retry_limit > 0"},
                "constraint_templates":[
                    {"name":"max_amount","params":{"amount":{"ref":"inputs.max_amount"}}}
                ]
            }
        ]
    }))
    .expect("segment");
    let refs = collect_segment_ref_closure(&segment);
    assert!(refs.contains(&"inputs.owner".to_string()));
    assert!(refs.contains(&"inputs.balance_threshold".to_string()));
    assert!(refs.contains(&"inputs.min_required".to_string()));
    assert!(refs.contains(&"inputs.retry_limit".to_string()));
    assert!(refs.contains(&"inputs.max_amount".to_string()));
}

#[test]
fn collect_segment_ref_closure_includes_facts_and_nodes_refs() {
    let segment: PlanSketchSegment = serde_json::from_value(json!({
        "segment_id":"seg_refs",
        "cursor_in":"0",
        "cursor_out":"1",
        "done":false,
        "steps":[
            {
                "id":"q1",
                "kind":"query",
                "candidate_ref":"demo@0.0.2/read",
                "inputs":{
                    "owner":{"ref":"inputs.owner"},
                    "price":{"ref":"facts.quote.price"},
                    "balance":{"ref":"nodes.q_balance.outputs.balance"}
                },
                "when":{"cel":"facts.quote.price > 0 && nodes.q_balance.outputs.balance > inputs.min_required"}
            }
        ]
    }))
    .expect("segment");
    let refs = collect_segment_ref_closure(&segment);
    assert!(refs.contains(&"inputs.owner".to_string()));
    assert!(refs.contains(&"inputs.min_required".to_string()));
    assert!(refs.contains(&"facts.quote.price".to_string()));
    assert!(refs.contains(&"nodes.q_balance.outputs.balance".to_string()));
}

#[test]
fn collect_segment_missing_refs_uses_runtime_ref_availability_for_all_namespaces() {
    let segment: PlanSketchSegment = serde_json::from_value(json!({
        "segment_id":"seg_missing",
        "cursor_in":"0",
        "cursor_out":"1",
        "done":false,
        "steps":[
            {
                "id":"q1",
                "kind":"query",
                "candidate_ref":"demo@0.0.2/read",
                "inputs":{
                    "owner":{"ref":"inputs.owner"},
                    "price":{"ref":"facts.quote.price"},
                    "balance":{"ref":"nodes.q_balance.outputs.balance"}
                }
            }
        ]
    }))
    .expect("segment");
    let missing = collect_segment_missing_refs(&segment, |reference| {
        matches!(reference, "inputs.owner" | "facts.quote.price")
    });
    assert_eq!(missing, vec!["nodes.q_balance.outputs.balance".to_string()]);
}

#[test]
fn collect_segment_missing_refs_skips_node_outputs_produced_in_same_segment() {
    let segment: PlanSketchSegment = serde_json::from_value(json!({
        "segment_id":"seg_missing_local_nodes",
        "cursor_in":"0",
        "cursor_out":"1",
        "done":false,
        "steps":[
            {
                "id":"q_balance",
                "kind":"query",
                "candidate_ref":"demo@0.0.2/read",
                "inputs":{"owner":{"ref":"inputs.owner"}}
            },
            {
                "id":"a_transfer",
                "kind":"action",
                "candidate_ref":"demo@0.0.2/write",
                "inputs":{
                    "gate_balance":{"ref":"nodes.q_balance.outputs.balance"}
                },
                "depends_on":["q_balance"]
            }
        ]
    }))
    .expect("segment");
    let missing = collect_segment_missing_refs(&segment, |reference| reference == "inputs.owner");
    assert!(
        missing.is_empty(),
        "same-segment query output refs should not block execute precheck"
    );
}

#[test]
fn invalidate_post_write_volatile_facts_clears_fresh_query_observations_for_completed_writes() {
    let segment: PlanSketchSegment = serde_json::from_value(json!({
        "segment_id":"seg_write",
        "cursor_in":"0",
        "cursor_out":"1",
        "done":false,
        "steps":[
            {
                "id":"q_native_balance",
                "kind":"query",
                "candidate_ref":"wallet@0.0.1/native-balance",
                "inputs":{}
            },
            {
                "id":"a_transfer",
                "kind":"action",
                "candidate_ref":"wallet@0.0.1/native-transfer",
                "depends_on":["q_native_balance"],
                "inputs":{"amount":{"lit":"1"},"to":{"ref":"inputs.recipient"}}
            }
        ]
    }))
    .expect("segment");
    let state = EngineRunnerState {
        completed_node_ids: vec!["seg_write/a_transfer".to_string()],
        ..EngineRunnerState::default()
    };
    let mut candidate_context = super::super::super::candidates::CandidateContext::default();
    candidate_context.detail_by_ref.insert(
        "wallet@0.0.1/native-transfer".to_string(),
        json!({
            "kind":"action",
            "risk_tags":["native_transfer"]
        }),
    );

    let mut input_store = InputStore::default();
    input_store.upsert(
        "inputs.native_balance",
        json!("100"),
        InputValueMeta {
            source: "query".to_string(),
            source_priority: 80,
            provenance: Some("segment_store.seg_prev/q_native_balance.balance".to_string()),
            confidence: None,
            layer: InputValueLayer::Observed,
            stability: InputValueStability::Volatile,
            observed_at_ms: Some(123),
        },
    );
    let mut runtime_facts_store =
        super::super::super::runtime_facts_store::RuntimeFactsStore::default();
    runtime_facts_store.upsert(
        "facts.native_balance",
        json!("100"),
        InputValueMeta {
            source: "query".to_string(),
            source_priority: 80,
            provenance: Some("segment_store.seg_prev/q_native_balance.balance".to_string()),
            confidence: None,
            layer: InputValueLayer::Observed,
            stability: InputValueStability::Volatile,
            observed_at_ms: Some(123),
        },
    );

    let mut seen = std::collections::BTreeSet::new();
    let report = invalidate_post_write_volatile_facts(
        &segment,
        &state,
        &candidate_context,
        &mut runtime_facts_store,
        &mut input_store,
        &mut seen,
    );

    assert_eq!(
        report.completed_write_node_ids,
        vec!["seg_write/a_transfer".to_string()]
    );
    assert_eq!(report.invalidated_signals, vec!["balance".to_string()]);
    assert_eq!(
        input_store
            .get("inputs.native_balance")
            .and_then(|entry| entry.meta.observed_at_ms),
        None
    );
    assert_eq!(
        runtime_facts_store
            .get("facts.native_balance")
            .and_then(|entry| entry.meta.observed_at_ms),
        None
    );
}

#[test]
fn post_write_invalidation_forces_follow_up_write_to_refresh_balance() {
    let completed_segment: PlanSketchSegment = serde_json::from_value(json!({
        "segment_id":"seg_write",
        "cursor_in":"0",
        "cursor_out":"1",
        "done":false,
        "steps":[
            {
                "id":"a_transfer",
                "kind":"action",
                "candidate_ref":"wallet@0.0.1/native-transfer",
                "inputs":{"amount":{"lit":"1"},"to":{"ref":"inputs.recipient"}}
            }
        ]
    }))
    .expect("segment");
    let follow_up_segment: PlanSketchSegment = serde_json::from_value(json!({
        "segment_id":"seg_follow_up",
        "cursor_in":"1",
        "cursor_out":"2",
        "done":false,
        "steps":[
            {
                "id":"q_decimals",
                "kind":"query",
                "candidate_ref":"erc20@0.0.2/decimals",
                "inputs":{"token":{"ref":"inputs.token.address"}}
            },
            {
                "id":"check_preconditions",
                "kind":"assert",
                "depends_on":["q_decimals"],
                "inputs":{},
                "when":{"cel":"inputs.native_balance > 0"}
            },
            {
                "id":"a_transfer_again",
                "kind":"action",
                "candidate_ref":"wallet@0.0.1/native-transfer",
                "depends_on":["check_preconditions"],
                "inputs":{"amount":{"lit":"1"},"to":{"ref":"inputs.recipient"}}
            }
        ]
    }))
    .expect("segment");
    let state = EngineRunnerState {
        completed_node_ids: vec!["seg_write/a_transfer".to_string()],
        ..EngineRunnerState::default()
    };
    let mut candidate_context = super::super::super::candidates::CandidateContext::default();
    candidate_context.detail_by_ref.insert(
        "wallet@0.0.1/native-transfer".to_string(),
        json!({
            "kind":"action",
            "risk_tags":["native_transfer"]
        }),
    );
    candidate_context.detail_by_ref.insert(
        "erc20@0.0.2/decimals".to_string(),
        json!({
            "kind":"query",
            "returns":[{"name":"decimals","type":"uint8"}]
        }),
    );

    let mut input_store = InputStore::default();
    input_store.upsert(
        "inputs.native_balance",
        json!("100"),
        InputValueMeta {
            source: "query".to_string(),
            source_priority: 80,
            provenance: Some("segment_store.seg_prev/q_native_balance.balance".to_string()),
            confidence: None,
            layer: InputValueLayer::Observed,
            stability: InputValueStability::Volatile,
            observed_at_ms: Some(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|duration| duration.as_millis() as u64)
                    .unwrap_or(0),
            ),
        },
    );
    let mut runtime_facts_store =
        super::super::super::runtime_facts_store::RuntimeFactsStore::default();
    let mut seen = std::collections::BTreeSet::new();
    let _ = invalidate_post_write_volatile_facts(
        &completed_segment,
        &state,
        &candidate_context,
        &mut runtime_facts_store,
        &mut input_store,
        &mut seen,
    );

    let error = super::super::super::write_gates::validate_segment_write_gates_with_policy(
        &follow_up_segment,
        &candidate_context,
        Some(&runtime_facts_store),
        Some(&input_store),
        crate::policy::VolatileFactsPolicy::default(),
    )
    .expect_err("follow-up write should require refresh after prior write invalidated balance");
    assert_eq!(
        error
            .pointer("/issues/0/reason_code")
            .and_then(serde_json::Value::as_str),
        Some("stale_volatile_fact")
    );
    assert_eq!(
        error
            .pointer("/issues/0/required_signal")
            .and_then(serde_json::Value::as_str),
        Some("balance")
    );
}
