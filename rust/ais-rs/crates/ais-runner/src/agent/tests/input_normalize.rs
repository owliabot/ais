use super::*;
use serde_json::json;

#[test]
fn normalize_input_slot_key_rejects_non_input_roots() {
    assert!(normalize_input_slot_key("facts.owner").is_none());
    assert!(normalize_input_slot_key("nodes.balance").is_none());
    assert!(normalize_input_slot_key("  runtime.inputs.owner  ").is_some());
    assert_eq!(
        normalize_input_slot_key(" inputs.token.address ").unwrap(),
        "token.address"
    );
}

#[test]
fn set_runtime_input_value_reuses_nested_slots() {
    let mut runtime = json!({});
    set_runtime_input_value(&mut runtime, "token.address", json!("0xabc"));
    assert_eq!(
        runtime.pointer("/inputs/token/address"),
        Some(&json!("0xabc"))
    );
}

#[test]
fn normalize_missing_input_ref_rejects_legacy_input_prefix() {
    assert!(normalize_missing_input_ref("input.owner").is_none());
    assert_eq!(
        normalize_missing_input_ref("inputs.owner"),
        Some("owner".to_string())
    );
}
