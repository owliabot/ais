use super::{
    ensure_monotonic_sequence, EngineEvent, EngineEventRecord, EngineEventSequenceError,
    EngineEventStream, EngineEventType, ENGINE_EVENT_SCHEMA_0_0_3,
};
use crate::events::{encode_event_jsonl_line, parse_event_jsonl_line};
use serde_json::json;

#[test]
fn jsonl_roundtrip_produces_valid_envelope() {
    let mut event = EngineEvent::new(EngineEventType::PlanReady);
    event.data.insert("node_count".to_string(), json!(3));

    let record = EngineEventRecord::new(
        "run-1",
        0,
        "2026-02-13T00:00:00Z",
        event,
    );
    let line = encode_event_jsonl_line(&record).expect("must encode");
    assert!(line.ends_with('\n'));

    let decoded = parse_event_jsonl_line(&line).expect("must decode");
    assert_eq!(decoded.schema, ENGINE_EVENT_SCHEMA_0_0_3);
    assert_eq!(decoded.run_id, "run-1");
    assert_eq!(decoded.seq, 0);
    assert_eq!(decoded.ts, "2026-02-13T00:00:00Z");
    assert_eq!(decoded.event.event_type, EngineEventType::PlanReady);
}

#[test]
fn stream_emits_monotonic_sequence() {
    let mut stream = EngineEventStream::new("run-2");
    let first = stream.next_record("2026-02-13T00:00:00Z", EngineEvent::new(EngineEventType::PlanReady));
    let second = stream.next_record("2026-02-13T00:00:01Z", EngineEvent::new(EngineEventType::NodeReady));
    let third = stream.next_record("2026-02-13T00:00:02Z", EngineEvent::new(EngineEventType::TxSent));

    assert_eq!(first.seq, 0);
    assert_eq!(second.seq, 1);
    assert_eq!(third.seq, 2);
    ensure_monotonic_sequence(&[first, second, third]).expect("must be monotonic");
}

#[test]
fn sequence_validator_rejects_gap() {
    let records = vec![
        EngineEventRecord::new(
            "run-3",
            0,
            "2026-02-13T00:00:00Z",
            EngineEvent::new(EngineEventType::PlanReady),
        ),
        EngineEventRecord::new(
            "run-3",
            2,
            "2026-02-13T00:00:01Z",
            EngineEvent::new(EngineEventType::NodeReady),
        ),
    ];

    let error = ensure_monotonic_sequence(&records).expect_err("must fail");
    assert_eq!(
        error,
        EngineEventSequenceError::NonMonotonic {
            index: 1,
            expected: 1,
            actual: 2,
        }
    );
}
