use super::{
    ensure_monotonic_sequence, format_unix_seconds_rfc3339, wall_clock_timestamp_rfc3339,
    EngineEvent, EngineEventRecord, EngineEventSequenceError, EngineEventStream, EngineEventType,
    ENGINE_EVENT_SCHEMA_0_0_3,
};
use crate::events::{encode_event_jsonl_line, parse_event_jsonl_line};
use serde_json::json;

#[test]
fn jsonl_roundtrip_produces_valid_envelope() {
    let mut event = EngineEvent::new(EngineEventType::PlanReady);
    event.data.insert("node_count".to_string(), json!(3));

    let record = EngineEventRecord::new("run-1", 0, "2026-02-13T00:00:00Z", event);
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
    let first = stream.next_record(
        "2026-02-13T00:00:00Z",
        EngineEvent::new(EngineEventType::PlanReady),
    );
    let second = stream.next_record(
        "2026-02-13T00:00:01Z",
        EngineEvent::new(EngineEventType::NodeReady),
    );
    let third = stream.next_record(
        "2026-02-13T00:00:02Z",
        EngineEvent::new(EngineEventType::TxSent),
    );

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

#[test]
fn side_effect_observed_variant_serializes_with_stable_name() {
    let value = serde_json::to_value(EngineEventType::SideEffectObserved).expect("serialize");
    assert_eq!(value.as_str(), Some("side_effect_observed"));
}

#[test]
fn need_user_input_variant_serializes_with_stable_name() {
    let value = serde_json::to_value(EngineEventType::NeedUserInput).expect("serialize");
    assert_eq!(value.as_str(), Some("need_user_input"));
}

#[test]
fn unix_seconds_rfc3339_format_is_stable() {
    assert_eq!(format_unix_seconds_rfc3339(0), "1970-01-01T00:00:00Z");
    assert_eq!(
        format_unix_seconds_rfc3339(1_709_251_200),
        "2024-03-01T00:00:00Z"
    );
}

#[test]
fn wall_clock_timestamp_is_not_epoch_and_uses_rfc3339_utc_seconds_shape() {
    let ts = wall_clock_timestamp_rfc3339();
    assert_ne!(ts, "1970-01-01T00:00:00Z");
    assert_eq!(ts.len(), 20);
    assert_eq!(&ts[4..5], "-");
    assert_eq!(&ts[7..8], "-");
    assert_eq!(&ts[10..11], "T");
    assert_eq!(&ts[13..14], ":");
    assert_eq!(&ts[16..17], ":");
    assert_eq!(&ts[19..20], "Z");
    assert!(ts[..4].chars().all(|ch| ch.is_ascii_digit()));
    assert!(ts[5..7].chars().all(|ch| ch.is_ascii_digit()));
    assert!(ts[8..10].chars().all(|ch| ch.is_ascii_digit()));
    assert!(ts[11..13].chars().all(|ch| ch.is_ascii_digit()));
    assert!(ts[14..16].chars().all(|ch| ch.is_ascii_digit()));
    assert!(ts[17..19].chars().all(|ch| ch.is_ascii_digit()));
}
