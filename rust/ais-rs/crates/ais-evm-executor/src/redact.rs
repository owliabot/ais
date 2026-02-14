use ais_engine::TraceRedactMode;
use serde_json::{Map, Value};

const REDACTED: &str = "[REDACTED]";

pub fn redact_evm_value(value: &Value, mode: TraceRedactMode) -> Value {
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
                if is_secret_key(lower.as_str()) || lower == "params" {
                    out.insert(key.clone(), Value::String(REDACTED.to_string()));
                    continue;
                }
                out.insert(key.clone(), redact_default(child));
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(redact_default).collect()),
        Value::String(text) if looks_like_secret_string(text) => {
            Value::String(REDACTED.to_string())
        }
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
                if lower == "params" {
                    out.insert(key.clone(), audit_trim_params(child));
                    continue;
                }
                out.insert(key.clone(), redact_audit(child));
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(redact_audit).collect()),
        Value::String(text) if looks_like_secret_string(text) => {
            Value::String(REDACTED.to_string())
        }
        _ => value.clone(),
    }
}

fn audit_trim_params(value: &Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.iter().map(audit_trim_value).collect()),
        other => audit_trim_value(other),
    }
}

fn audit_trim_value(value: &Value) -> Value {
    match value {
        Value::String(text) => Value::String(trim_string(text)),
        Value::Object(object) => {
            let mut out = Map::<String, Value>::new();
            for (key, child) in object {
                if is_secret_key(key.to_lowercase().as_str()) {
                    out.insert(key.clone(), Value::String(REDACTED.to_string()));
                } else {
                    out.insert(key.clone(), audit_trim_value(child));
                }
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(audit_trim_value).collect()),
        _ => value.clone(),
    }
}

fn trim_string(value: &str) -> String {
    if value.len() <= 24 {
        return value.to_string();
    }
    let head = &value[..12];
    format!("{head}…(len={})", value.len())
}

fn is_secret_key(key: &str) -> bool {
    key.contains("private_key")
        || key == "mnemonic"
        || key == "seed"
        || key == "seed_phrase"
        || key.contains("secret")
        || key == "signature"
        || key == "raw_tx"
        || key == "signed_tx"
}

fn looks_like_secret_string(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains("private key")
        || lower.contains("seed phrase")
        || lower.contains("mnemonic")
        || lower.starts_with("0x")
            && text.len() > 120
}

#[cfg(test)]
#[path = "redact_test.rs"]
mod tests;
