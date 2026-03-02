use ais_core::{stable_hash_hex, StableJsonOptions};
use serde_json::{json, Value};

pub(crate) fn tool_cache_key(tool_name: &str, arguments: &Value) -> Option<String> {
    match tool_name {
        "list_candidates"
        | "get_candidate_detail"
        | "catalog.search"
        | "catalog.resolve_missing_facts"
        | "guide.get"
        | "plan.check_segment" => {
            let normalized = normalize_tool_arguments(tool_name, arguments);
            let hash = stable_hash_hex(&normalized, &StableJsonOptions::default())
                .unwrap_or_else(|_| serde_json::to_string(&normalized).unwrap_or_default());
            Some(format!("{tool_name}:{hash}"))
        }
        _ => None,
    }
}

fn normalize_tool_arguments(tool_name: &str, arguments: &Value) -> Value {
    match tool_name {
        "list_candidates" => {
            let chain = arguments
                .get("chain")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| Value::String(value.to_ascii_lowercase()))
                .unwrap_or(Value::Null);
            let protocol = arguments
                .get("protocol")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| Value::String(value.to_ascii_lowercase()))
                .unwrap_or(Value::Null);
            let filter_chain = arguments
                .get("filter")
                .and_then(Value::as_object)
                .and_then(|filter| filter.get("chain"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| Value::String(value.to_ascii_lowercase()))
                .unwrap_or(Value::Null);
            let filter_protocol = arguments
                .get("filter")
                .and_then(Value::as_object)
                .and_then(|filter| filter.get("protocol"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| Value::String(value.to_ascii_lowercase()))
                .unwrap_or(Value::Null);
            json!({
                "chain": if chain.is_null() { filter_chain } else { chain },
                "protocol": if protocol.is_null() { filter_protocol } else { protocol },
            })
        }
        "get_candidate_detail" => {
            let mut refs = arguments
                .get("refs")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            refs.sort();
            refs.dedup();
            json!({ "refs": refs })
        }
        "catalog.search" => {
            let query = arguments
                .get("query")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .and_then(normalize_catalog_search_query_for_cache)
                .map(Value::String)
                .unwrap_or(Value::Null);
            let kind = arguments
                .get("kind")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| Value::String(value.to_ascii_lowercase()))
                .unwrap_or(Value::Null);
            let chain = arguments
                .get("chain")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| Value::String(value.to_ascii_lowercase()))
                .unwrap_or(Value::Null);
            json!({
                "query": query,
                "kind": kind,
                "chain": chain,
                "min_risk_level": arguments.get("min_risk_level").cloned().unwrap_or(Value::Null),
                "max_risk_level": arguments.get("max_risk_level").cloned().unwrap_or(Value::Null),
                "limit": arguments.get("limit").cloned().unwrap_or(Value::Null),
            })
        }
        "catalog.resolve_missing_facts" => {
            let mut missing_refs = arguments
                .get("missing_refs")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(Value::as_str)
                        .filter_map(normalize_missing_fact_ref_for_cache)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            missing_refs.sort();
            missing_refs.dedup();
            let limit_per_ref = arguments
                .get("limit_per_ref")
                .and_then(Value::as_u64)
                .map(|value| value.clamp(1, 8));
            json!({
                "missing_refs": missing_refs,
                "limit_per_ref": limit_per_ref,
            })
        }
        "guide.get" => {
            let schema = arguments
                .get("schema")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| Value::String(value.to_string()))
                .unwrap_or(Value::Null);
            let topic = arguments
                .get("topic")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| Value::String(value.to_ascii_lowercase()))
                .unwrap_or(Value::Null);
            json!({
                "schema": schema,
                "topic": topic,
            })
        }
        "plan.check_segment" => json!({
            "segment": arguments.get("segment").cloned().unwrap_or(Value::Null),
        }),
        _ => arguments.clone(),
    }
}

fn normalize_catalog_search_query_for_cache(query: &str) -> Option<String> {
    let stop_words = ["a", "an", "the", "for", "on", "to", "my", "me", "please"];
    let lowered = query
        .to_ascii_lowercase()
        .replace("erc-20", "erc20")
        .replace("erc 20", "erc20")
        .replace("balanceof", "balance");
    let mut tokens = lowered
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(|token| match token {
            "erc20" | "token" | "tokens" => "token".to_string(),
            "native" | "eth" => "native".to_string(),
            "balanceof" | "balance" | "balances" => "balance".to_string(),
            "transfer" | "send" | "payment" => "transfer".to_string(),
            other => other.to_string(),
        })
        .filter(|token| !stop_words.contains(&token.as_str()))
        .collect::<Vec<_>>();
    if tokens.is_empty() {
        return None;
    }
    tokens.sort();
    tokens.dedup();
    Some(tokens.join(" "))
}

fn normalize_missing_fact_ref_for_cache(raw: &str) -> Option<String> {
    let trimmed = raw.trim_matches(|ch: char| {
        ch.is_whitespace() || matches!(ch, '`' | '"' | '\'' | ',' | ';' | '.' | ')' | '(')
    });
    let right_of_equals = trimmed
        .rsplit_once('=')
        .map(|(_, value)| value)
        .unwrap_or(trimmed);
    let normalized = right_of_equals
        .strip_prefix("runtime.")
        .unwrap_or(right_of_equals);
    let key = if let Some(key) = normalized.strip_prefix("inputs.") {
        key
    } else if let Some(key) = normalized.strip_prefix("input.") {
        key
    } else {
        normalized
    };
    let compacted = key.trim_matches('.');
    if compacted.is_empty() {
        return None;
    }
    Some(format!("inputs.{}", compacted.to_ascii_lowercase()))
}
