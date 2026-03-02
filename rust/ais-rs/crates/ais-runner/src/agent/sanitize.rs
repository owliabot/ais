use serde_json::Value;

const DEFAULT_MAX_STRING_CHARS: usize = 400;

pub fn sanitize_for_llm_payload(value: &Value) -> Value {
    sanitize_for_llm_payload_with_limit(value, DEFAULT_MAX_STRING_CHARS)
}

pub fn sanitize_for_llm_payload_with_limit(value: &Value, max_string_chars: usize) -> Value {
    match value {
        Value::Object(object) => {
            let mut out = serde_json::Map::new();
            for (key, child) in object {
                if is_sensitive_key(key) {
                    out.insert(key.clone(), Value::String("[REDACTED]".to_string()));
                    continue;
                }
                out.insert(
                    key.clone(),
                    sanitize_for_llm_payload_with_limit(child, max_string_chars),
                );
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| sanitize_for_llm_payload_with_limit(item, max_string_chars))
                .collect(),
        ),
        Value::String(text) => Value::String(sanitize_string(text, max_string_chars)),
        _ => value.clone(),
    }
}

fn sanitize_string(text: &str, max_string_chars: usize) -> String {
    let mut normalized = text.trim().to_string();
    if looks_like_prompt_injection(&normalized) {
        normalized = "[SANITIZED_POTENTIALLY_UNSAFE_TEXT]".to_string();
    }
    if normalized.chars().count() > max_string_chars {
        normalized = normalized
            .chars()
            .take(max_string_chars)
            .collect::<String>();
        normalized.push_str("...");
    }
    normalized
}

fn is_sensitive_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "private_key"
            | "mnemonic"
            | "seed"
            | "api_key"
            | "authorization"
            | "password"
            | "secret"
            | "access_token"
            | "refresh_token"
    )
}

fn looks_like_prompt_injection(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("ignore previous instructions")
        || lower.contains("ignore all previous")
        || lower.contains("system prompt")
        || lower.contains("developer message")
        || lower.contains("<script")
}

#[cfg(test)]
#[path = "tests/sanitize.rs"]
mod tests;
