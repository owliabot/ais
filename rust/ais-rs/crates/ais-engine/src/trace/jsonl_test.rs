use super::encode_trace_jsonl_line;
use crate::events::{parse_event_jsonl_line, EngineEvent, EngineEventRecord, EngineEventType};
use crate::trace::{TraceRedactMode, TraceRedactOptions};
use serde_json::json;
use std::fs;
use std::path::PathBuf;

#[test]
fn encode_trace_jsonl_line_applies_redaction() {
    let mut event = EngineEvent::new(EngineEventType::TxPrepared);
    event.data.insert("private_key".to_string(), json!("0xabc"));
    event.data.insert(
        "rpc_payload".to_string(),
        json!({
            "method": "eth_sendRawTransaction",
            "params": ["0xf86c..."]
        }),
    );
    let record = EngineEventRecord::new("run-1", 0, "2026-02-13T00:00:00Z", event);

    let line = encode_trace_jsonl_line(
        &record,
        &TraceRedactOptions {
            mode: TraceRedactMode::Default,
            allow_path_patterns: vec![],
        },
    )
    .expect("must encode");

    assert!(line.ends_with('\n'));
    assert!(line.contains("\"schema\":\"ais-engine-event/0.0.3\""));
    assert!(line.contains("\"private_key\":\"[REDACTED]\""));
    assert!(line.contains("\"rpc_payload\":\"[REDACTED]\""));
}

#[test]
fn encode_trace_jsonl_line_fixture_default_mode_redacts_sensitive_fields() {
    let fixture = fs::read_to_string(fixture_root().join("redaction/trace-private.jsonl"))
        .expect("must read fixture");
    let record = parse_event_jsonl_line(fixture.as_str()).expect("must parse");
    let line = encode_trace_jsonl_line(
        &record,
        &TraceRedactOptions {
            mode: TraceRedactMode::Default,
            allow_path_patterns: vec![],
        },
    )
    .expect("must encode");

    assert!(line.contains("\"private_key\":\"[REDACTED]\""));
    assert!(line.contains("\"rpc_payload\":\"[REDACTED]\""));
}

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/plan-events")
}
