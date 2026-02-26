use super::{
    canonical_side_effect_status, create_checkpoint_document, decode_checkpoint_json,
    encode_checkpoint_json, CheckpointApprovalLedgerEntry, CheckpointEngineState,
    CheckpointSideEffectRecord, CHECKPOINT_SCHEMA_0_0_1, SIDE_EFFECT_STATUS_REVERTED,
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
            seen_command_ids: vec![
                "cmd-2".to_string(),
                "cmd-1".to_string(),
                "cmd-1".to_string(),
            ],
            pending_retries: serde_json::Map::from_iter([(
                "node-1".to_string(),
                json!({"attempt": 2}),
            )]),
            plan_epoch: 0,
            plan_hash_history: vec!["plan-hash-1".to_string()],
        },
        Some(json!({
            "inputs": {"amount": "100"}
        })),
        None,
        None,
    );

    let encoded = encode_checkpoint_json(&document).expect("must encode");
    let decoded = decode_checkpoint_json(&encoded).expect("must decode");
    assert_eq!(decoded.schema, CHECKPOINT_SCHEMA_0_0_1);
    assert_eq!(decoded.run_id, "run-1");
    assert_eq!(decoded.plan_hash, "plan-hash-1");
    assert_eq!(
        decoded.engine_state.completed_node_ids,
        vec!["a".to_string(), "b".to_string()]
    );
    assert_eq!(
        decoded.engine_state.seen_command_ids,
        vec!["cmd-1".to_string(), "cmd-2".to_string()]
    );
}

#[test]
fn checkpoint_roundtrip_with_ledgers_and_dedup() {
    let mut document = create_checkpoint_document(
        "run-ledger",
        "plan-hash-ledger",
        CheckpointEngineState::default(),
        None,
        None,
        None,
    );
    document.approvals_ledger = vec![
        CheckpointApprovalLedgerEntry {
            node_id: "swap-1".to_string(),
            confirmation_hash: Some("0xabc".to_string()),
            decision: "approve".to_string(),
            reason_code: Some("threshold_exceeded".to_string()),
            decided_at: "2026-02-23T00:00:00Z".to_string(),
        },
        CheckpointApprovalLedgerEntry {
            node_id: "swap-1".to_string(),
            confirmation_hash: Some("0xabc".to_string()),
            decision: "approve".to_string(),
            reason_code: Some("threshold_exceeded".to_string()),
            decided_at: "2026-02-23T00:00:00Z".to_string(),
        },
    ];
    document.side_effects = vec![
        CheckpointSideEffectRecord {
            schema: Some("ais-side-effect-record/0.1.0".to_string()),
            idempotency_key: "node:swap-1:0xtx1".to_string(),
            node_id: "swap-1".to_string(),
            effect_type: "tx".to_string(),
            chain: Some("eip155:1".to_string()),
            execution_type: Some("evm_call".to_string()),
            tx_hash: Some("0xtx1".to_string()),
            nonce: Some(9),
            provider_ref: None,
            reason_code: None,
            details: None,
            status: "sent".to_string(),
            observed_at: "2026-02-23T00:00:00Z".to_string(),
        },
        CheckpointSideEffectRecord {
            schema: Some("ais-side-effect-record/0.1.0".to_string()),
            idempotency_key: "node:swap-1:0xtx1".to_string(),
            node_id: "swap-1".to_string(),
            effect_type: "tx".to_string(),
            chain: Some("eip155:1".to_string()),
            execution_type: Some("evm_call".to_string()),
            tx_hash: Some("0xtx1".to_string()),
            nonce: Some(9),
            provider_ref: None,
            reason_code: None,
            details: None,
            status: "sent".to_string(),
            observed_at: "2026-02-23T00:00:00Z".to_string(),
        },
    ];
    let encoded = encode_checkpoint_json(&document).expect("must encode");
    let decoded = decode_checkpoint_json(&encoded).expect("must decode");
    assert_eq!(decoded.approvals_ledger.len(), 1);
    assert_eq!(decoded.side_effects.len(), 1);
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
        None,
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

#[test]
fn checkpoint_decode_drops_side_effect_without_idempotency_key() {
    let encoded = serde_json::to_string_pretty(&json!({
        "schema": "ais-checkpoint/0.0.1",
        "run_id": "run-1",
        "plan_hash": "hash-1",
        "engine_state": {},
        "side_effects": [
            {
                "schema": "ais-side-effect-record/0.1.0",
                "idempotency_key": "",
                "node_id": "swap-1",
                "effect_type": "tx",
                "chain": "eip155:1",
                "execution_type": "evm_call",
                "status": "sent",
                "observed_at": "2026-02-24T00:00:00Z",
                "tx_hash": "0xtx1"
            },
            {
                "schema": "ais-side-effect-record/0.1.0",
                "idempotency_key": "tx:swap-1:0xtx2",
                "node_id": "swap-1",
                "effect_type": "tx",
                "chain": "eip155:1",
                "execution_type": "evm_call",
                "status": "sent",
                "observed_at": "2026-02-24T00:00:00Z",
                "tx_hash": "0xtx2"
            }
        ]
    }))
    .expect("must encode");

    let decoded = decode_checkpoint_json(&encoded).expect("must decode");
    assert_eq!(decoded.side_effects.len(), 1);
    assert_eq!(decoded.side_effects[0].idempotency_key, "tx:swap-1:0xtx2");
}

#[test]
fn checkpoint_decode_normalizes_failed_status_to_reverted() {
    let encoded = serde_json::to_string_pretty(&json!({
        "schema": "ais-checkpoint/0.0.1",
        "run_id": "run-1",
        "plan_hash": "hash-1",
        "engine_state": {},
        "side_effects": [
            {
                "schema": "ais-side-effect-record/0.1.0",
                "idempotency_key": "tx:swap-1:0xtx1",
                "node_id": "swap-1",
                "effect_type": "tx",
                "chain": "eip155:1",
                "execution_type": "evm_call",
                "status": "failed",
                "observed_at": "2026-02-24T00:00:00Z",
                "tx_hash": "0xtx1"
            }
        ]
    }))
    .expect("must encode");
    let decoded = decode_checkpoint_json(&encoded).expect("must decode");
    assert_eq!(
        canonical_side_effect_status(decoded.side_effects[0].status.as_str()),
        SIDE_EFFECT_STATUS_REVERTED
    );
    assert_eq!(decoded.side_effects[0].status, "reverted");
}
