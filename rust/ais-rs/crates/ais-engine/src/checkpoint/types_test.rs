use super::{
    create_checkpoint_document, decode_checkpoint_json, encode_checkpoint_json, CheckpointEngineState,
    CHECKPOINT_SCHEMA_0_0_1,
};
use crate::trace::{TraceRedactMode, TraceRedactOptions};
use serde_json::json;

#[test]
fn checkpoint_roundtrip_json() {
    let document = create_checkpoint_document(
        "run-1",
        "plan-hash-1",
        CheckpointEngineState {
            completed_node_ids: vec!["b".to_string(), "a".to_string(), "a".to_string()],
            paused_reason: Some("node_blocked".to_string()),
            seen_command_ids: vec!["cmd-2".to_string(), "cmd-1".to_string(), "cmd-1".to_string()],
            pending_retries: serde_json::Map::from_iter([("node-1".to_string(), json!({"attempt": 2}))]),
        },
        Some(json!({
            "inputs": {"amount": "100"}
        })),
        None,
    );

    let encoded = encode_checkpoint_json(&document).expect("must encode");
    let decoded = decode_checkpoint_json(&encoded).expect("must decode");
    assert_eq!(decoded.schema, CHECKPOINT_SCHEMA_0_0_1);
    assert_eq!(decoded.run_id, "run-1");
    assert_eq!(decoded.plan_hash, "plan-hash-1");
    assert_eq!(decoded.engine_state.completed_node_ids, vec!["a".to_string(), "b".to_string()]);
    assert_eq!(decoded.engine_state.seen_command_ids, vec!["cmd-1".to_string(), "cmd-2".to_string()]);
}

#[test]
fn checkpoint_redacted_payload_still_deserializes() {
    let document = create_checkpoint_document(
        "run-2",
        "plan-hash-2",
        CheckpointEngineState::default(),
        Some(json!({
            "wallet": {
                "private_key": "0xabc",
                "mnemonic": "alpha beta gamma"
            },
            "rpc_payload": {
                "method": "eth_call",
                "params": ["0x123"]
            }
        })),
        Some(&TraceRedactOptions {
            mode: TraceRedactMode::Default,
            allow_path_patterns: vec![],
        }),
    );

    let encoded = encode_checkpoint_json(&document).expect("must encode");
    let decoded = decode_checkpoint_json(&encoded).expect("must decode");
    let wallet = decoded
        .runtime_snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.get("wallet"))
        .expect("wallet exists");
    assert_eq!(wallet.get("private_key"), Some(&json!("[REDACTED]")));
    assert_eq!(wallet.get("mnemonic"), Some(&json!("[REDACTED]")));
    assert_eq!(
        decoded
            .runtime_snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.get("rpc_payload")),
        Some(&json!("[REDACTED]"))
    );
}
