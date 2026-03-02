use super::*;
use serde_json::json;

#[test]
fn context_envelope_round_trip_through_compat_summary() {
    let payload = json!({
        "done": false,
        "input_registry": {"known_refs": ["inputs.owner"]}
    });
    let envelope = ContextEnvelope::from_payload(&payload, 3, None);
    let summary = envelope.to_compat_summary(payload.clone());
    let parsed = ContextEnvelope::from_summary(&summary).expect("parse envelope");
    assert_eq!(parsed.version, 3);
    assert_eq!(parsed.hash, envelope.hash);
    assert_eq!(payload_from_summary(&summary), payload);
}

#[test]
fn context_envelope_reads_legacy_summary_shape() {
    let summary = json!({
        "done": false,
        "context_version": 4,
        "context_hash": "legacy-hash",
        "context_unchanged": true,
    });
    let envelope = ContextEnvelope::from_summary(&summary).expect("legacy parse");
    assert_eq!(envelope.schema, CONTEXT_ENVELOPE_SCHEMA);
    assert_eq!(envelope.schema_version, CONTEXT_ENVELOPE_SCHEMA_VERSION);
    assert_eq!(envelope.version, 4);
    assert_eq!(envelope.hash, "legacy-hash");
    assert!(envelope.unchanged);
}

#[test]
fn context_envelope_invalid_schema_falls_back_to_legacy() {
    let summary = json!({
        "done": false,
        "context_version": 5,
        "context_hash": "legacy-hash",
        "context_unchanged": true,
        "context_envelope": {
            "schema": "foreign-envelope",
            "schema_version": 1,
            "version": 999,
            "hash": "wrong",
            "unchanged": false
        }
    });
    let envelope = ContextEnvelope::from_summary(&summary).expect("legacy fallback");
    assert_eq!(envelope.version, 5);
    assert_eq!(envelope.hash, "legacy-hash");
    assert!(envelope.unchanged);
}

#[test]
fn context_envelope_rejects_foreign_schema_without_legacy_shape() {
    let summary = json!({
        "done": false,
        "context_envelope": {
            "schema": "foreign-envelope",
            "schema_version": 1,
            "version": 1,
            "hash": "x",
            "unchanged": false
        }
    });
    assert!(ContextEnvelope::from_summary(&summary).is_none());
}

#[test]
fn context_envelope_hash_validation_is_optional() {
    let payload = json!({
        "done": false,
        "input_registry": {"known_refs": ["inputs.owner"]}
    });
    let envelope = ContextEnvelope::from_payload(&payload, 1, None);
    let mut summary = envelope.to_compat_summary(payload);
    summary["done"] = Value::Bool(true);

    let parsed = ContextEnvelope::from_summary(&summary).expect("non-strict parse");
    assert_eq!(parsed.version, 1);
    assert!(ContextEnvelope::from_summary_with_options(&summary, true).is_none());
}

#[test]
fn context_envelope_hash_validation_can_fallback_to_legacy() {
    let payload = json!({
        "done": false,
        "input_registry": {"known_refs": ["inputs.owner"]}
    });
    let envelope = ContextEnvelope::from_payload(&payload, 1, None);
    let mut summary = envelope.to_compat_summary(payload);
    summary["context_envelope"]["hash"] = Value::String("tampered".to_string());

    let parsed = ContextEnvelope::from_summary_with_options(&summary, true)
        .expect("legacy fallback under hash verification");
    assert_eq!(parsed.version, 1);
    assert_eq!(parsed.hash, envelope.hash);
}
