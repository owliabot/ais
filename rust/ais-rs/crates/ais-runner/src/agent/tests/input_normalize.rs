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

#[test]
fn parse_missing_ref_path_keeps_ref_namespace() {
    assert_eq!(
        parse_missing_ref_path("facts.quote.slippage_bps"),
        Some(super::super::ref_model::RefPath::Fact {
            key: "quote.slippage_bps".to_string()
        })
    );
    assert_eq!(
        parse_missing_ref_path("nodes.q_balance.outputs.balance"),
        Some(super::super::ref_model::RefPath::NodeOutput {
            step_id: "q_balance".to_string(),
            field_path: "balance".to_string()
        })
    );
    assert_eq!(
        canonical_missing_ref("runtime.inputs.owner.value"),
        Some("inputs.owner".to_string())
    );
    assert_eq!(
        canonical_missing_ref("facts.quote.slippage_bps"),
        Some("facts.quote.slippage_bps".to_string())
    );
    assert_eq!(
        canonical_missing_ref("nodes.q_balance.outputs.balance"),
        Some("nodes.q_balance.outputs.balance".to_string())
    );
}

#[test]
fn set_runtime_input_value_leaf_then_subtree_preserves_leaf_as_value() {
    let mut runtime = json!({});
    set_runtime_input_value(&mut runtime, "owner", json!("0xABCD"));
    set_runtime_input_value(&mut runtime, "owner.balance.erc20", json!("999"));
    // The leaf value is preserved under `_value`
    assert_eq!(
        runtime.pointer("/inputs/owner/_value"),
        Some(&json!("0xABCD"))
    );
    // The subtree value is also present
    assert_eq!(
        runtime.pointer("/inputs/owner/balance/erc20"),
        Some(&json!("999"))
    );
}

#[test]
fn set_runtime_input_value_subtree_then_leaf_stores_leaf_as_value() {
    let mut runtime = json!({});
    set_runtime_input_value(&mut runtime, "owner.balance.erc20", json!("999"));
    set_runtime_input_value(&mut runtime, "owner", json!("0xABCD"));
    assert_eq!(
        runtime.pointer("/inputs/owner/_value"),
        Some(&json!("0xABCD"))
    );
    assert_eq!(
        runtime.pointer("/inputs/owner/balance/erc20"),
        Some(&json!("999"))
    );
}

#[test]
fn set_runtime_input_value_token_and_decimals_coexist() {
    let mut runtime = json!({});
    set_runtime_input_value(&mut runtime, "token", json!("0x8464"));
    set_runtime_input_value(&mut runtime, "token.decimals", json!("18"));
    assert_eq!(
        runtime.pointer("/inputs/token/_value"),
        Some(&json!("0x8464"))
    );
    assert_eq!(
        runtime.pointer("/inputs/token/decimals"),
        Some(&json!("18"))
    );
}

#[test]
fn set_runtime_input_value_pure_subtree_no_value_sentinel() {
    let mut runtime = json!({});
    set_runtime_input_value(&mut runtime, "owner.balance.native", json!("100"));
    set_runtime_input_value(&mut runtime, "owner.balance.erc20", json!("200"));
    // No `_value` key because there was never a leaf at `owner`
    assert!(runtime.pointer("/inputs/owner/_value").is_none());
    assert_eq!(
        runtime.pointer("/inputs/owner/balance/native"),
        Some(&json!("100"))
    );
    assert_eq!(
        runtime.pointer("/inputs/owner/balance/erc20"),
        Some(&json!("200"))
    );
}

#[test]
fn set_runtime_input_value_leaf_overwrite_updates_value_sentinel() {
    let mut runtime = json!({});
    set_runtime_input_value(&mut runtime, "owner", json!("0xOLD"));
    set_runtime_input_value(&mut runtime, "owner.balance.erc20", json!("999"));
    // overwrite the leaf
    set_runtime_input_value(&mut runtime, "owner", json!("0xNEW"));
    assert_eq!(
        runtime.pointer("/inputs/owner/_value"),
        Some(&json!("0xNEW"))
    );
    assert_eq!(
        runtime.pointer("/inputs/owner/balance/erc20"),
        Some(&json!("999"))
    );
}
