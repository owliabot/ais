use super::redact_solana_value;
use ais_engine::TraceRedactMode;
use serde_json::json;

#[test]
fn default_mode_redacts_raw_tx_and_instruction_data() {
    let payload = json!({
        "instruction": "swap",
        "data": "base64:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        "lookup_tables": [{"address":"TableAddress111111111111111111111111111"}],
        "private_key": "dev-secret",
        "raw_tx": "base64:deadbeef"
    });
    let redacted = redact_solana_value(&payload, TraceRedactMode::Default);
    assert_eq!(redacted.get("instruction"), Some(&json!("swap")));
    assert_eq!(redacted.get("data"), Some(&json!("[REDACTED]")));
    assert_eq!(redacted.get("lookup_tables"), Some(&json!("[REDACTED]")));
    assert_eq!(redacted.get("private_key"), Some(&json!("[REDACTED]")));
    assert_eq!(redacted.get("raw_tx"), Some(&json!("[REDACTED]")));
}

#[test]
fn audit_mode_keeps_shape_and_trims_values() {
    let payload = json!({
        "instruction": "swap",
        "data": "base64:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        "lookup_tables": [{"address":"TableAddress111111111111111111111111111"}],
        "signature": "abc"
    });
    let redacted = redact_solana_value(&payload, TraceRedactMode::Audit);
    assert_eq!(redacted.get("instruction"), Some(&json!("swap")));
    let data = redacted.get("data").and_then(|value| value.as_str()).expect("data");
    assert!(data.contains("…(len="));
    assert!(redacted.get("lookup_tables").and_then(|value| value.as_array()).is_some());
    assert_eq!(redacted.get("signature"), Some(&json!("[REDACTED]")));
}

#[test]
fn off_mode_keeps_payload() {
    let payload = json!({
        "instruction": "swap",
        "data": "base64:AAA=",
        "lookup_tables": []
    });
    assert_eq!(redact_solana_value(&payload, TraceRedactMode::Off), payload);
}
