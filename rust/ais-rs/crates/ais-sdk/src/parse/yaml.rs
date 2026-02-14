use super::detect_duplicate_keys::detect_yaml_duplicate_keys;
use ais_core::{IssueSeverity, StructuredIssue};
use serde_json::Value;

pub fn parse_yaml(input: &str) -> Result<Value, Vec<StructuredIssue>> {
    let mut issues = detect_yaml_duplicate_keys(input);
    if !issues.is_empty() {
        StructuredIssue::sort_stable(&mut issues);
        return Err(issues);
    }

    let yaml_value: serde_yaml::Value = serde_yaml::from_str(input).map_err(|err| {
        let message = err.to_string();
        let reference = if message.to_ascii_lowercase().contains("duplicate") {
            "yaml.duplicate_key"
        } else {
            "yaml.parse_error"
        };
        vec![StructuredIssue {
            kind: "parse_error".to_string(),
            severity: IssueSeverity::Error,
            node_id: None,
            field_path: "$".parse().expect("field path must parse"),
            message: format!("yaml parse failed: {message}"),
            reference: Some(reference.to_string()),
            related: None,
        }]
    })?;

    serde_json::to_value(yaml_value).map_err(|err| {
        vec![StructuredIssue {
            kind: "parse_error".to_string(),
            severity: IssueSeverity::Error,
            node_id: None,
            field_path: "$".parse().expect("field path must parse"),
            message: format!("yaml-to-json conversion failed: {err}"),
            reference: Some("yaml.to_json_error".to_string()),
            related: None,
        }]
    })
}
