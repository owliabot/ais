use serde_json::Value;
use std::env;

pub(crate) const DEFAULT_TOKEN_DECIMALS_MAX: u32 = 36;
const TOKEN_DECIMALS_MAX_ENV: &str = "AIS_RUNNER_TOKEN_DECIMALS_MAX";

pub(crate) fn token_decimals_max() -> u32 {
    env::var(TOKEN_DECIMALS_MAX_ENV)
        .ok()
        .and_then(|raw| raw.trim().parse::<u32>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_TOKEN_DECIMALS_MAX)
}

pub(crate) fn parse_valid_token_decimals(value: &Value, max: u32) -> Option<u32> {
    let parsed = parse_u32_like(value)?;
    if parsed <= max {
        Some(parsed)
    } else {
        None
    }
}

pub(crate) fn value_contains_valid_asset_decimals(value: &Value, max: u32) -> bool {
    match value {
        Value::Object(object) => {
            if let Some(decimals) = object.get("decimals") {
                if parse_valid_token_decimals(decimals, max).is_some() {
                    return true;
                }
            }
            if let Some(lit) = object.get("lit") {
                if value_contains_valid_asset_decimals(lit, max) {
                    return true;
                }
            }
            if let Some(inner_object) = object.get("object") {
                return value_contains_valid_asset_decimals(inner_object, max);
            }
            false
        }
        Value::Array(values) => values
            .iter()
            .any(|item| value_contains_valid_asset_decimals(item, max)),
        _ => false,
    }
}

pub(crate) fn semantic_tokens(raw: &str) -> Vec<String> {
    raw.to_ascii_lowercase()
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|segment| !segment.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>()
}

pub(crate) fn normalize_semantic_key(raw: &str) -> String {
    semantic_tokens(raw).join("")
}

pub(crate) fn is_generic_semantic_token(token: &str) -> bool {
    matches!(
        token,
        "inputs" | "input" | "value" | "field" | "data" | "ref" | "address" | "amount"
    )
}

pub(crate) fn semantic_has_any(tokens: &[String], expected: &[&str]) -> bool {
    tokens
        .iter()
        .any(|token| expected.contains(&token.as_str()))
}

pub(crate) fn is_evm_address(value: &str) -> bool {
    value.len() == 42
        && value.starts_with("0x")
        && value
            .as_bytes()
            .iter()
            .skip(2)
            .all(|byte| byte.is_ascii_hexdigit())
}

pub(crate) fn static_input_alias_slots(slot: &str) -> Vec<String> {
    let mut aliases = Vec::<String>::new();
    match slot {
        "native_transfer_amount" => aliases.push("native_amount".to_string()),
        "token_transfer_amount" => aliases.push("token_amount".to_string()),
        "recipient" => aliases.push("recipient_address".to_string()),
        "recipient_address" => aliases.push("recipient".to_string()),
        "token.address" => {
            aliases.push("token_address".to_string());
            aliases.push("token".to_string());
            aliases.push("erc20_token".to_string());
            aliases.push("erc20_token.address".to_string());
        }
        "token" => {
            aliases.push("token.address".to_string());
            aliases.push("token_address".to_string());
            aliases.push("erc20_token".to_string());
            aliases.push("erc20_token.address".to_string());
        }
        "token_address" => {
            aliases.push("token.address".to_string());
            aliases.push("token".to_string());
            aliases.push("erc20_token".to_string());
            aliases.push("erc20_token.address".to_string());
        }
        "erc20_token" => {
            aliases.push("token".to_string());
            aliases.push("token.address".to_string());
            aliases.push("token_address".to_string());
            aliases.push("erc20_token.address".to_string());
        }
        "erc20_token.address" => {
            aliases.push("erc20_token".to_string());
            aliases.push("token.address".to_string());
            aliases.push("token_address".to_string());
            aliases.push("token".to_string());
        }
        "chain_ref" => {
            aliases.push("chain".to_string());
            aliases.push("chain_id".to_string());
        }
        "chain_id" => {
            aliases.push("chain".to_string());
            aliases.push("chain_ref".to_string());
        }
        "chain" => {
            aliases.push("chain_id".to_string());
            aliases.push("chain_ref".to_string());
        }
        "addr" => {
            aliases.push("owner".to_string());
            aliases.push("wallet.default".to_string());
        }
        "owner" => {
            aliases.push("addr".to_string());
            aliases.push("wallet.default".to_string());
        }
        "wallet.default" => {
            aliases.push("owner".to_string());
            aliases.push("addr".to_string());
        }
        _ => {}
    }
    if slot.ends_with(".address") {
        aliases.push(slot.replace(".address", "_address"));
    }
    if slot.ends_with("_address") {
        aliases.push(slot.replace("_address", ".address"));
    }
    aliases
}

fn parse_u32_like(value: &Value) -> Option<u32> {
    if let Some(inner) = value.as_object().and_then(|object| object.get("value")) {
        return parse_u32_like(inner);
    }
    match value {
        Value::Number(number) => {
            let value = number.as_u64()?;
            u32::try_from(value).ok()
        }
        Value::String(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return None;
            }
            if trimmed.contains('.') || trimmed.starts_with('+') || trimmed.starts_with('-') {
                return None;
            }
            trimmed.parse::<u32>().ok()
        }
        _ => None,
    }
}

#[cfg(test)]
#[path = "../tests/missing_resolution_heuristics.rs"]
mod tests;
