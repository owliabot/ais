use super::*;
use serde_json::json;

#[test]
fn parse_valid_token_decimals_accepts_integer_number_and_string() {
    assert_eq!(parse_valid_token_decimals(&json!(18), 36), Some(18));
    assert_eq!(parse_valid_token_decimals(&json!("6"), 36), Some(6));
    assert_eq!(
        parse_valid_token_decimals(&json!({"value":"8"}), 36),
        Some(8)
    );
}

#[test]
fn parse_valid_token_decimals_rejects_invalid_or_out_of_range() {
    assert_eq!(parse_valid_token_decimals(&json!("18.5"), 36), None);
    assert_eq!(parse_valid_token_decimals(&json!(-1), 36), None);
    assert_eq!(parse_valid_token_decimals(&json!(255), 36), None);
}

#[test]
fn value_contains_valid_asset_decimals_requires_typed_valid_value() {
    let valid = json!({
        "object": {
            "address": {"lit":"0x8464135c8F25Da09e49BC8782676a84730C318bC"},
            "decimals": "18"
        }
    });
    let invalid = json!({
        "object": {
            "address": {"lit":"0x8464135c8F25Da09e49BC8782676a84730C318bC"},
            "decimals": "18.5"
        }
    });
    assert!(value_contains_valid_asset_decimals(&valid, 36));
    assert!(!value_contains_valid_asset_decimals(&invalid, 36));
}

#[test]
fn static_input_alias_slots_include_token_and_erc20_aliases() {
    let aliases = static_input_alias_slots("token");
    assert!(aliases.contains(&"erc20_token".to_string()));
    assert!(aliases.contains(&"token.address".to_string()));

    let erc20_aliases = static_input_alias_slots("erc20_token");
    assert!(erc20_aliases.contains(&"token".to_string()));
    assert!(erc20_aliases.contains(&"token.address".to_string()));
}
