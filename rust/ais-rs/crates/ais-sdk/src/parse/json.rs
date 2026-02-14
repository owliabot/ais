use ais_core::{IssueSeverity, StructuredIssue};
use serde_json::Value;

pub fn parse_json(input: &str) -> Result<Value, Vec<StructuredIssue>> {
    serde_json::from_str::<Value>(input).map_err(|err| {
        vec![StructuredIssue {
            kind: "parse_error".to_string(),
            severity: IssueSeverity::Error,
            node_id: None,
            field_path: "$".parse().expect("field path must parse"),
            message: format!("json parse failed: {err}"),
            reference: Some("json.parse_error".to_string()),
            related: None,
        }]
    })
}
