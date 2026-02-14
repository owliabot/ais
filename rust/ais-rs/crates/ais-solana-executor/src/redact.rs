use ais_engine::TraceRedactMode;
use serde_json::{Map, Value};

const REDACTED: &str = "[REDACTED]";

pub fn redact_solana_value(value: &Value, mode: TraceRedactMode) -> Value {
    match mode {
        TraceRedactMode::Off => value.clone(),
        TraceRedactMode::Default => redact_default(value),
        TraceRedactMode::Audit => redact_audit(value),
    }
}

fn redact_default(value: &Value) -> Value {
    match value {
        Value::Object(object) => {
            let mut out = Map::<String, Value>::new();
            for (key, child) in object {
                let lower = key.to_lowercase();
                if is_secret_key(lower.as_str()) || lower == "data" {
                    out.insert(key.clone(), Value::String(REDACTED.to_string()));
                    continue;
                }
                if lower == "lookup_tables" {
                    out.insert(key.clone(), Value::String(REDACTED.to_string()));
                    continue;
                }
                out.insert(key.clone(), redact_default(child));
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(redact_default).collect()),
        Value::String(text) if looks_like_secret_string(text) => Value::String(REDACTED.to_string()),
        _ => value.clone(),
    }
}

fn redact_audit(value: &Value) -> Value {
    match value {
        Value::Object(object) => {
            let mut out = Map::<String, Value>::new();
            for (key, child) in object {
                let lower = key.to_lowercase();
                if is_secret_key(lower.as_str()) {
                    out.insert(key.clone(), Value::String(REDACTED.to_string()));
                    continue;
                }
                if lower == "data" {
                    out.insert(key.clone(), Value::String(trim_string(child.as_str().unwrap_or_default())));
                    continue;
                }
                if lower == "lookup_tables" {
                    out.insert(key.clone(), trim_lookup_tables(child));
                    continue;
                }
                out.insert(key.clone(), redact_audit(child));
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(redact_audit).collect()),
        Value::String(text) if looks_like_secret_string(text) => Value::String(REDACTED.to_string()),
        _ => value.clone(),
    }
}

fn trim_lookup_tables(value: &Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(
            items
                .iter()
                .enumerate()
                .map(|(index, item)| {
                    let address = item
                        .as_object()
                        .and_then(|object| object.get("address"))
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    Value::String(format!("table#{index}:{}", trim_string(address)))
                })
                .collect(),
        ),
        other => Value::String(trim_string(other.to_string().as_str())),
    }
}

fn trim_string(value: &str) -> String {
    if value.len() <= 18 {
        return value.to_string();
    }
    format!("{}…(len={})", &value[..10], value.len())
}

fn is_secret_key(key: &str) -> bool {
    key.contains("private_key")
        || key == "seed"
        || key == "mnemonic"
        || key == "secret"
        || key == "signature"
        || key == "raw_tx"
        || key == "signed_tx"
}

fn looks_like_secret_string(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains("private key")
        || lower.contains("seed phrase")
        || lower.contains("mnemonic")
        || (lower.starts_with("base64:") && text.len() > 120)
}

#[cfg(test)]
#[path = "redact_test.rs"]
mod tests;
