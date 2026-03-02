use serde_json::Value;

use super::decode::ToolArgsNormalization;

pub(crate) fn guide_get_requires_full_schema(tool_name: &str, arguments: &Value) -> bool {
    if tool_name != "guide.get" {
        return false;
    }
    let full_requested = arguments
        .get("full")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !full_requested {
        return false;
    }
    arguments
        .get("schema")
        .and_then(Value::as_str)
        .map(str::trim)
        .is_some_and(|schema| !schema.is_empty())
}

pub(crate) fn guide_get_payload_contains_full_schema(content: &str) -> bool {
    serde_json::from_str::<Value>(content)
        .ok()
        .and_then(|payload| payload.pointer("/schema/json").cloned())
        .is_some()
}

pub(crate) fn normalize_guide_get_tool_args(arguments: &Value) -> ToolArgsNormalization {
    let Some(object) = arguments.as_object() else {
        return ToolArgsNormalization {
            arguments: arguments.clone(),
            normalized_fields: vec![],
        };
    };
    let mut normalized = object.clone();
    let mut normalized_fields = Vec::new();
    if let Some(value) = normalized.get("full").and_then(Value::as_str) {
        if let Some(parsed_bool) = parse_ascii_bool(value) {
            normalized.insert("full".to_string(), Value::Bool(parsed_bool));
            normalized_fields.push("full:string->bool");
        }
    }
    ToolArgsNormalization {
        arguments: Value::Object(normalized),
        normalized_fields,
    }
}

fn parse_ascii_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}
