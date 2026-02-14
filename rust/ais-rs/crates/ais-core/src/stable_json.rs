use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Default)]
pub struct StableJsonOptions {
    pub ignore_object_keys: BTreeSet<String>,
}

pub fn stable_json_bytes(value: &Value, options: &StableJsonOptions) -> serde_json::Result<Vec<u8>> {
    let normalized = normalize_value(value, options);
    serde_json::to_vec(&normalized)
}

fn normalize_value(value: &Value, options: &StableJsonOptions) -> Value {
    match value {
        Value::Object(object) => normalize_object(object, options),
        Value::Array(items) => Value::Array(items.iter().map(|item| normalize_value(item, options)).collect()),
        _ => value.clone(),
    }
}

fn normalize_object(object: &Map<String, Value>, options: &StableJsonOptions) -> Value {
    let mut ordered = BTreeMap::new();
    for (key, value) in object {
        if options.ignore_object_keys.contains(key) {
            continue;
        }
        ordered.insert(key.clone(), normalize_value(value, options));
    }

    let mut out = Map::new();
    for (key, value) in ordered {
        out.insert(key, value);
    }
    Value::Object(out)
}

#[cfg(test)]
#[path = "stable_json_test.rs"]
mod tests;
