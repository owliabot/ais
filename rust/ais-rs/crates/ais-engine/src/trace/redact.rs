use crate::events::EngineEventRecord;
use serde::{Deserialize, Serialize};
use serde_json::Value;

const REDACTED: &str = "[REDACTED]";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum TraceRedactMode {
    #[default]
    Default,
    Audit,
    Off,
}


#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceRedactOptions {
    pub mode: TraceRedactMode,
    #[serde(default)]
    pub allow_path_patterns: Vec<String>,
}

pub fn redact_engine_event_record(
    record: &EngineEventRecord,
    options: &TraceRedactOptions,
) -> EngineEventRecord {
    let mut value = serde_json::to_value(record).unwrap_or(Value::Null);
    redact_value(&mut value, options);
    serde_json::from_value(value).unwrap_or_else(|_| record.clone())
}

pub fn redact_value(value: &mut Value, options: &TraceRedactOptions) {
    if options.mode == TraceRedactMode::Off {
        return;
    }
    walk_and_redact(value, &mut Vec::new(), options);
}

fn walk_and_redact(value: &mut Value, path: &mut Vec<String>, options: &TraceRedactOptions) {
    if is_allowed_path(path, &options.allow_path_patterns) {
        return;
    }

    match value {
        Value::Object(object) => {
            if should_redact_full_object(path, object, options.mode) {
                *value = Value::String(REDACTED.to_string());
                return;
            }
            let keys = object.keys().cloned().collect::<Vec<_>>();
            for key in keys {
                path.push(key.clone());
                if is_allowed_path(path, &options.allow_path_patterns) {
                    path.pop();
                    continue;
                }
                if should_redact_key(path, &key, options.mode) {
                    object.insert(key, Value::String(REDACTED.to_string()));
                    path.pop();
                    continue;
                }
                if let Some(child) = object.get_mut(&key) {
                    walk_and_redact(child, path, options);
                }
                path.pop();
            }
        }
        Value::Array(items) => {
            for (index, item) in items.iter_mut().enumerate() {
                path.push(index.to_string());
                walk_and_redact(item, path, options);
                path.pop();
            }
        }
        Value::String(text) => {
            if should_redact_string(path, text, options.mode) {
                *text = REDACTED.to_string();
            }
        }
        _ => {}
    }
}

fn should_redact_full_object(
    path: &[String],
    object: &serde_json::Map<String, Value>,
    mode: TraceRedactMode,
) -> bool {
    let full_path = path.join(".");
    let lower_path = full_path.to_lowercase();
    let has_rpc_shape =
        object.contains_key("method") && (object.contains_key("params") || object.contains_key("result"));
    if has_rpc_shape {
        return mode == TraceRedactMode::Default;
    }
    lower_path.ends_with("rpc_payload") && mode == TraceRedactMode::Default
}

fn should_redact_key(path: &[String], key: &str, mode: TraceRedactMode) -> bool {
    if is_secret_keyword(key) {
        return true;
    }
    let full_path = path.join(".");
    let lower_path = full_path.to_lowercase();
    if mode == TraceRedactMode::Default
        && (lower_path.contains("rpc.payload") || lower_path.contains("rpc_payload"))
    {
        return true;
    }
    false
}

fn should_redact_string(path: &[String], text: &str, _mode: TraceRedactMode) -> bool {
    if looks_like_secret_string(text) {
        return true;
    }
    let full_path = path.join(".").to_lowercase();
    full_path.contains("private_key")
        || full_path.contains("mnemonic")
        || full_path.contains("seed")
        || full_path.contains("signature")
}

fn is_secret_keyword(key: &str) -> bool {
    let lower = key.to_lowercase();
    lower.contains("private_key")
        || lower == "mnemonic"
        || lower == "seed"
        || lower == "seed_phrase"
        || lower == "secret"
        || lower == "signature"
        || lower == "raw_tx"
}

fn looks_like_secret_string(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains("private key")
        || lower.contains("seed phrase")
        || lower.contains("mnemonic")
        || lower.contains("-----begin")
}

fn is_allowed_path(path: &[String], patterns: &[String]) -> bool {
    if patterns.is_empty() {
        return false;
    }
    patterns.iter().any(|pattern| match_pattern(path, pattern))
}

fn match_pattern(path: &[String], pattern: &str) -> bool {
    let pattern_segments = pattern
        .split('.')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    match_segments(path, &pattern_segments)
}

fn match_segments(path: &[String], pattern: &[&str]) -> bool {
    if pattern.is_empty() {
        return path.is_empty();
    }
    match pattern[0] {
        "**" => {
            if pattern.len() == 1 {
                return true;
            }
            for i in 0..=path.len() {
                if match_segments(&path[i..], &pattern[1..]) {
                    return true;
                }
            }
            false
        }
        "*" => {
            if path.is_empty() {
                return false;
            }
            match_segments(&path[1..], &pattern[1..])
        }
        expected => {
            if path.is_empty() {
                return false;
            }
            if path[0] != expected {
                return false;
            }
            match_segments(&path[1..], &pattern[1..])
        }
    }
}

#[cfg(test)]
#[path = "redact_test.rs"]
mod tests;
