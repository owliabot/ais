use super::ref_model::RefPath;
use serde_json::{Map, Value};
use std::collections::BTreeSet;

const PROHIBITED_INPUT_PREFIXES: [&str; 6] =
    ["nodes", "facts", "agent", "runtime", "workspace", "input"];
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

const INPUT_SLOT_ALIASES: &[(&str, &str)] = &[
    ("token_address", "token.address"),
    ("token_decimals", "token.decimals"),
    ("recipient_address", "recipient"),
];

pub(super) fn normalize_grounding_input_key(raw_key: &str) -> String {
    let trimmed = raw_key.trim();
    let stripped = if let Some(suffix) = trimmed.strip_prefix("runtime.inputs.") {
        suffix.trim().to_string()
    } else if let Some(suffix) = trimmed.strip_prefix("inputs.") {
        suffix.trim().to_string()
    } else {
        trimmed.to_string()
    };
    for &(alias, canonical) in INPUT_SLOT_ALIASES {
        if stripped == alias {
            return canonical.to_string();
        }
    }
    stripped
}

pub(super) fn parse_missing_ref_path(raw: &str) -> Option<RefPath> {
    let normalized = normalized_missing_ref_source(raw)?;
    if normalized.starts_with("input.") {
        return None;
    }
    RefPath::parse(normalized.as_str())
}

pub(super) fn canonical_missing_ref_path(raw: &str) -> Option<RefPath> {
    let parsed = parse_missing_ref_path(raw)?;
    match parsed {
        RefPath::Input { slot } => {
            normalize_input_slot_key(slot.as_str()).map(|slot| RefPath::Input { slot })
        }
        RefPath::Fact { key } => Some(RefPath::Fact { key }),
        RefPath::NodeOutput {
            step_id,
            field_path,
        } => Some(RefPath::NodeOutput {
            step_id,
            field_path,
        }),
    }
}

pub(super) fn canonical_missing_ref(raw: &str) -> Option<String> {
    canonical_missing_ref_path(raw).map(|reference| reference.as_canonical_str())
}

pub(super) fn normalize_missing_input_ref(raw: &str) -> Option<String> {
    let RefPath::Input { slot } = canonical_missing_ref_path(raw)? else {
        return None;
    };
    Some(slot)
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

fn normalized_missing_ref_source(raw: &str) -> Option<String> {
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
    let normalized = normalized
        .strip_suffix(".value")
        .unwrap_or(normalized)
        .trim_matches('.');
    if normalized.is_empty() {
        return None;
    }
    Some(normalized.to_string())
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

/// Sentinel key used when a flat InputStore slot (e.g. `owner = "0x..."`) coexists
/// with a nested sub-slot (e.g. `owner.balance.erc20 = "999..."`).  The leaf value is
/// preserved under `_value` so that ref resolution (`inputs.owner`) can still return
/// the original primitive while the subtree remains accessible.
pub(super) const LEAF_VALUE_KEY: &str = "_value";

fn set_nested_object_value(root: &mut Map<String, Value>, path: &[&str], value: Value) {
    if path.is_empty() {
        return;
    }
    if path.len() == 1 {
        let key = path[0].to_string();
        if let Some(existing) = root.get_mut(&key) {
            // Setting a leaf on a key that is already an object (subtree exists) —
            // store the leaf as `_value` inside the existing subtree.
            if existing.is_object() {
                if let Some(obj) = existing.as_object_mut() {
                    obj.insert(LEAF_VALUE_KEY.to_string(), value);
                }
                return;
            }
        }
        root.insert(key, value);
        return;
    }
    let entry = root
        .entry(path[0].to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if !entry.is_object() {
        // Existing value is a primitive leaf but we need to create a subtree under it.
        // Preserve the leaf as `_value`.
        let previous = std::mem::replace(entry, Value::Object(Map::new()));
        if let Some(obj) = entry.as_object_mut() {
            obj.insert(LEAF_VALUE_KEY.to_string(), previous);
        }
    }
    if let Some(child) = entry.as_object_mut() {
        set_nested_object_value(child, &path[1..], value);
    }
}
