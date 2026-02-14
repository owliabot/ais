use super::{redact_engine_event_record, TraceRedactMode, TraceRedactOptions};
use crate::events::{EngineEvent, EngineEventRecord, EngineEventType};
use serde_json::json;

fn sample_record() -> EngineEventRecord {
    let mut event = EngineEvent::new(EngineEventType::TxPrepared);
    event.node_id = Some("swap-1".to_string());
    event.data.insert("private_key".to_string(), json!("0xabc"));
    event.data.insert("seed_phrase".to_string(), json!("alpha beta gamma"));
    event.data.insert(
        "rpc_payload".to_string(),
        json!({
            "method": "eth_sendRawTransaction",
            "params": ["0xf86c..."],
            "chain": "eip155:1"
        }),
    );
    event.data.insert(
        "tx".to_string(),
        json!({
            "to": "0x0000000000000000000000000000000000000001",
            "value": "100",
            "signature": "0xdeadbeef"
        }),
    );
    EngineEventRecord::new("run-redact", 7, "2026-02-13T10:00:00Z", event)
}

#[test]
fn redact_default_mode_is_strong() {
    let options = TraceRedactOptions {
        mode: TraceRedactMode::Default,
        allow_path_patterns: vec![],
    };
    let record = redact_engine_event_record(&sample_record(), &options);
    let data = &record.event.data;

    assert_eq!(data.get("private_key"), Some(&json!("[REDACTED]")));
    assert_eq!(data.get("seed_phrase"), Some(&json!("[REDACTED]")));
    assert_eq!(data.get("rpc_payload"), Some(&json!("[REDACTED]")));
    assert_eq!(
        data.get("tx")
            .and_then(|value| value.get("signature")),
        Some(&json!("[REDACTED]"))
    );
}

#[test]
fn redact_audit_mode_keeps_rpc_structure_but_masks_secrets() {
    let options = TraceRedactOptions {
        mode: TraceRedactMode::Audit,
        allow_path_patterns: vec![],
    };
    let record = redact_engine_event_record(&sample_record(), &options);
    let rpc_payload = record
        .event
        .data
        .get("rpc_payload")
        .and_then(|value| value.as_object())
        .expect("rpc payload object must exist");

    assert_eq!(rpc_payload.get("method"), Some(&json!("eth_sendRawTransaction")));
    assert_eq!(
        record.event.data.get("private_key"),
        Some(&json!("[REDACTED]"))
    );
    assert_eq!(
        record
            .event
            .data
            .get("tx")
            .and_then(|value| value.get("signature")),
        Some(&json!("[REDACTED]"))
    );
}

#[test]
fn redact_off_mode_keeps_payload() {
    let options = TraceRedactOptions {
        mode: TraceRedactMode::Off,
        allow_path_patterns: vec![],
    };
    let original = sample_record();
    let record = redact_engine_event_record(&original, &options);
    assert_eq!(record, original);
}

#[test]
fn allow_path_patterns_can_unredact_specific_field() {
    let options = TraceRedactOptions {
        mode: TraceRedactMode::Default,
        allow_path_patterns: vec!["event.data.private_key".to_string()],
    };
    let record = redact_engine_event_record(&sample_record(), &options);
    assert_eq!(record.event.data.get("private_key"), Some(&json!("0xabc")));
    assert_eq!(record.event.data.get("seed_phrase"), Some(&json!("[REDACTED]")));
}
