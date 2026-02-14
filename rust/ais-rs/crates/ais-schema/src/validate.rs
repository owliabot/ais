use ais_core::{FieldPath, FieldPathSegment, IssueSeverity, StructuredIssue};
use jsonschema::JSONSchema;
use serde_json::Value;

use crate::registry::get_json_schema;

pub fn validate_schema_instance(schema_id: &str, instance: &Value) -> Vec<StructuredIssue> {
    let Some(schema) = get_json_schema(schema_id) else {
        return vec![StructuredIssue {
            kind: "schema_error".to_string(),
            severity: IssueSeverity::Error,
            node_id: None,
            field_path: FieldPath::root(),
            message: format!("unknown schema id: {schema_id}"),
            reference: Some("schema_registry.unknown_schema".to_string()),
            related: None,
        }];
    };

    let schema_json: Value = match serde_json::from_str(schema.json) {
        Ok(value) => value,
        Err(err) => {
            return vec![StructuredIssue {
                kind: "schema_error".to_string(),
                severity: IssueSeverity::Error,
                node_id: None,
                field_path: FieldPath::root(),
                message: format!("embedded schema json parse failed: {err}"),
                reference: Some("schema_registry.invalid_embedded_schema".to_string()),
                related: None,
            }];
        }
    };

    let compiled = match JSONSchema::options().compile(&schema_json) {
        Ok(compiled) => compiled,
        Err(err) => {
            return vec![StructuredIssue {
                kind: "schema_error".to_string(),
                severity: IssueSeverity::Error,
                node_id: None,
                field_path: FieldPath::root(),
                message: format!("schema compile failed for {schema_id}: {err}"),
                reference: Some("schema_registry.compile_failed".to_string()),
                related: None,
            }];
        }
    };

    let mut issues = Vec::new();
    if let Err(errors) = compiled.validate(instance) {
        for error in errors {
            issues.push(StructuredIssue {
                kind: "schema_error".to_string(),
                severity: IssueSeverity::Error,
                node_id: None,
                field_path: json_pointer_to_field_path(error.instance_path.to_string().as_str()),
                message: error.to_string(),
                reference: Some("json_schema.validation".to_string()),
                related: None,
            });
        }
    }
    StructuredIssue::sort_stable(&mut issues);
    issues
}

fn json_pointer_to_field_path(pointer: &str) -> FieldPath {
    if pointer.is_empty() || pointer == "/" {
        return FieldPath::root();
    }

    let mut segments = Vec::new();
    for raw_segment in pointer.trim_start_matches('/').split('/') {
        if raw_segment.is_empty() {
            continue;
        }
        let decoded = raw_segment.replace("~1", "/").replace("~0", "~");
        if let Ok(index) = decoded.parse::<usize>() {
            segments.push(FieldPathSegment::Index(index));
        } else {
            segments.push(FieldPathSegment::Key(decoded));
        }
    }
    FieldPath::from_segments(segments)
}

#[cfg(test)]
#[path = "validate_test.rs"]
mod tests;
