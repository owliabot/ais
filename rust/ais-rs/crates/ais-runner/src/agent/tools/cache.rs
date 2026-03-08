use serde_json::{json, Value};

pub(crate) fn tool_cache_key(tool_name: &str, arguments: &Value) -> Option<String> {
    match tool_name {
        "get_candidate_detail"
        | "catalog.discover"
        | "catalog.resolve_missing_facts"
        | "guide.get"
        | "plan.check_segment" => {
            let normalized = normalize_tool_arguments(tool_name, arguments);
            let normalized_text =
                serde_json::to_string(&normalized).unwrap_or_else(|_| normalized.to_string());
            Some(format!("{tool_name}:{normalized_text}"))
        }
        _ => None,
    }
}

fn normalize_tool_arguments(tool_name: &str, arguments: &Value) -> Value {
    match tool_name {
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
        "catalog.discover" => {
            let query = arguments
                .get("query")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .and_then(normalize_catalog_search_query_for_cache)
                .map(Value::String)
                .unwrap_or(Value::Null);
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
            let kind = arguments
                .get("kind")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| Value::String(value.to_ascii_lowercase()))
                .unwrap_or(Value::Null);
            json!({
                "query": query,
                "chain": chain,
                "protocol": protocol,
                "kind": kind,
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
    super::super::input_normalize::canonical_missing_ref(raw)
}
