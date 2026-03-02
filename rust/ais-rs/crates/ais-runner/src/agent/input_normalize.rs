use serde_json::{Map, Value};
use std::collections::BTreeSet;

const PROHIBITED_INPUT_PREFIXES: [&str; 6] =
    ["nodes", "facts", "agent", "runtime", "workspace", "input"];
const NON_INPUT_MISSING_REF_ROOTS: [&str; 14] = [
    "agent",
    "calculated",
    "contracts",
    "ctx",
    "facts",
    "nodes",
    "params",
    "policy",
    "query",
    "runtime",
    "session",
    "state",
    "todo",
    "workspace",
];
const MISSING_REF_HINT_POINTERS: [&str; 14] = [
    "/missing_ref_fields",
    "/missing_ref_expansions",
    "/required_fields",
    "/required_object_fields",
    "/missing_fields",
    "/missing_object_fields",
    "/object_fields",
    "/metadata/missing_ref_fields",
    "/metadata/missing_ref_expansions",
    "/metadata/required_fields",
    "/metadata/required_object_fields",
    "/metadata/missing_fields",
    "/metadata/missing_object_fields",
    "/metadata/object_fields",
];

pub(super) fn canonical_input_slot_key(key: &str) -> String {
    normalize_grounding_input_key(key)
}

pub(super) fn normalize_input_slot_key(raw_key: &str) -> Option<String> {
    let canonical = canonical_input_slot_key(raw_key);
    if canonical.is_empty() {
        return None;
    }

    let segments = canonical
        .split('.')
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();

    if segments.is_empty() {
        return None;
    }

    if PROHIBITED_INPUT_PREFIXES.contains(&segments[0]) {
        return None;
    }

    Some(segments.join("."))
}

pub(super) fn normalize_grounding_input_key(raw_key: &str) -> String {
    let trimmed = raw_key.trim();
    if let Some(suffix) = trimmed.strip_prefix("runtime.inputs.") {
        return suffix.trim().to_string();
    }
    if let Some(suffix) = trimmed.strip_prefix("inputs.") {
        return suffix.trim().to_string();
    }
    trimmed.to_string()
}

pub(super) fn normalize_missing_input_ref(raw: &str) -> Option<String> {
    let trimmed = raw.trim_matches(|ch: char| {
        ch.is_whitespace() || matches!(ch, '`' | '"' | '\'' | ',' | ';' | '.' | ')' | '(')
    });
    if trimmed.is_empty() {
        return None;
    }
    let right_of_equals = trimmed
        .rsplit_once('=')
        .map(|(_, value)| value)
        .unwrap_or(trimmed);
    let normalized = right_of_equals
        .strip_prefix("runtime.")
        .unwrap_or(right_of_equals);
    let key = normalized.strip_prefix("inputs.").unwrap_or(normalized);
    let key = key.strip_suffix(".value").unwrap_or(key).trim_matches('.');
    if key.is_empty() {
        return None;
    }

    let root = key
        .split('.')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if NON_INPUT_MISSING_REF_ROOTS.contains(&root.as_str()) {
        return None;
    }

    normalize_input_slot_key(key)
}

pub(super) fn expand_missing_input_slot(slot: &str, metadata: Option<&Value>) -> Vec<String> {
    let Some(canonical_slot) = normalize_input_slot_key(slot) else {
        return Vec::new();
    };
    let mut expanded = BTreeSet::<String>::new();
    expanded.insert(canonical_slot.clone());
    let Some(metadata) = metadata else {
        return expanded.into_iter().collect::<Vec<_>>();
    };

    for hint in missing_ref_hints_from_metadata(metadata) {
        if let Some(candidate) = normalize_missing_ref_hint(canonical_slot.as_str(), hint.as_str())
        {
            expanded.insert(candidate);
        }
    }
    expanded.into_iter().collect::<Vec<_>>()
}

fn missing_ref_hints_from_metadata(metadata: &Value) -> Vec<String> {
    let mut out = BTreeSet::<String>::new();
    for pointer in MISSING_REF_HINT_POINTERS {
        for value in metadata
            .pointer(pointer)
            .and_then(Value::as_array)
            .into_iter()
            .flat_map(|items| items.iter())
            .filter_map(Value::as_str)
        {
            let normalized = value.trim();
            if !normalized.is_empty() {
                out.insert(normalized.to_string());
            }
        }
    }
    for key in metadata
        .pointer("/schema/properties")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|map| map.keys())
        .chain(
            metadata
                .pointer("/metadata/schema/properties")
                .and_then(Value::as_object)
                .into_iter()
                .flat_map(|map| map.keys()),
        )
    {
        let normalized = key.trim();
        if !normalized.is_empty() {
            out.insert(normalized.to_string());
        }
    }
    for value in metadata
        .pointer("/schema/required")
        .and_then(Value::as_array)
        .into_iter()
        .flat_map(|items| items.iter())
        .filter_map(Value::as_str)
        .chain(
            metadata
                .pointer("/metadata/schema/required")
                .and_then(Value::as_array)
                .into_iter()
                .flat_map(|items| items.iter())
                .filter_map(Value::as_str),
        )
    {
        let normalized = value.trim();
        if !normalized.is_empty() {
            out.insert(normalized.to_string());
        }
    }
    out.into_iter().collect::<Vec<_>>()
}

fn normalize_missing_ref_hint(slot: &str, hint: &str) -> Option<String> {
    if hint.trim().is_empty() {
        return None;
    }
    if let Some(normalized_hint) = normalize_missing_input_ref(hint) {
        if normalized_hint == slot
            || normalized_hint
                .strip_prefix(slot)
                .is_some_and(|suffix| suffix.starts_with('.'))
        {
            return Some(normalized_hint);
        }
        return None;
    }

    let relative = hint
        .trim_matches(|ch: char| {
            ch.is_whitespace() || matches!(ch, '`' | '"' | '\'' | ',' | ';' | '.' | ')' | '(')
        })
        .split('.')
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    if relative.is_empty() {
        return None;
    }
    let joined = relative.join(".");
    normalize_input_slot_key(format!("{slot}.{joined}").as_str())
}

#[cfg(test)]
#[path = "tests/input_normalize.rs"]
mod tests;

pub(super) fn set_runtime_input_value(runtime: &mut Value, key: &str, value: Value) {
    if !runtime.is_object() {
        *runtime = Value::Object(Map::new());
    }
    let Some(root) = runtime.as_object_mut() else {
        return;
    };
    let inputs = root
        .entry("inputs".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if !inputs.is_object() {
        *inputs = Value::Object(Map::new());
    }
    let Some(inputs_obj) = inputs.as_object_mut() else {
        return;
    };
    let canonical_key = canonical_input_slot_key(key);
    if canonical_key.is_empty() {
        return;
    }
    let segments = canonical_key
        .split('.')
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    if segments.is_empty() {
        return;
    }
    set_nested_object_value(inputs_obj, segments.as_slice(), value);
}

fn set_nested_object_value(root: &mut Map<String, Value>, path: &[&str], value: Value) {
    if path.is_empty() {
        return;
    }
    if path.len() == 1 {
        root.insert(path[0].to_string(), value);
        return;
    }
    let entry = root
        .entry(path[0].to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if !entry.is_object() {
        *entry = Value::Object(Map::new());
    }
    if let Some(child) = entry.as_object_mut() {
        set_nested_object_value(child, &path[1..], value);
    }
}
