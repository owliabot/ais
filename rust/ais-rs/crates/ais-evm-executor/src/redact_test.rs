use super::redact_evm_value;
use ais_engine::TraceRedactMode;
use serde_json::json;

#[test]
fn default_mode_redacts_raw_signed_tx_and_rpc_params() {
    let payload = json!({
        "method": "eth_sendRawTransaction",
        "params": ["0xdeadbeef"],
        "raw_tx": "0xabcdef",
        "private_key": "0x1234",
        "tx": {
            "to": "0x0000000000000000000000000000000000000001",
            "signature": "0x9999"
        }
    });

    let redacted = redact_evm_value(&payload, TraceRedactMode::Default);
    assert_eq!(redacted.get("method"), Some(&json!("eth_sendRawTransaction")));
    assert_eq!(redacted.get("params"), Some(&json!("[REDACTED]")));
    assert_eq!(redacted.get("raw_tx"), Some(&json!("[REDACTED]")));
    assert_eq!(redacted.get("private_key"), Some(&json!("[REDACTED]")));
    assert_eq!(
        redacted
            .get("tx")
            .and_then(|value| value.get("signature")),
        Some(&json!("[REDACTED]"))
    );
}

#[test]
fn audit_mode_keeps_structure_and_trims_params() {
    let payload = json!({
        "method": "eth_call",
        "params": [{
            "to": "0x0000000000000000000000000000000000000001",
            "data": "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef"
        }],
        "mnemonic": "alpha beta gamma delta epsilon zeta eta theta iota"
    });

    let redacted = redact_evm_value(&payload, TraceRedactMode::Audit);
    assert_eq!(redacted.get("method"), Some(&json!("eth_call")));
    assert!(redacted.get("params").and_then(|value| value.as_array()).is_some());
    assert_eq!(redacted.get("mnemonic"), Some(&json!("[REDACTED]")));
    let data = redacted
        .get("params")
        .and_then(|value| value.as_array())
        .and_then(|items| items.first())
        .and_then(|value| value.get("data"))
        .and_then(|value| value.as_str())
        .expect("trimmed data must exist");
    assert!(data.contains("…(len="));
}

#[test]
fn off_mode_keeps_full_payload() {
    let payload = json!({
        "method": "eth_getTransactionReceipt",
        "params": ["0xabcd"],
        "raw_tx": "0xdeadbeef"
    });
    let redacted = redact_evm_value(&payload, TraceRedactMode::Off);
    assert_eq!(redacted, payload);
}
