use crate::field_path::FieldPath;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IssueSeverity {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StructuredIssue {
    pub kind: String,
    pub severity: IssueSeverity,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    pub field_path: FieldPath,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub related: Option<Value>,
}

impl StructuredIssue {
    pub fn sort_stable(issues: &mut [Self]) {
        issues.sort_by(|left, right| {
            (
                left.severity,
                &left.kind,
                &left.field_path,
                &left.message,
                &left.node_id,
            )
                .cmp(&(
                    right.severity,
                    &right.kind,
                    &right.field_path,
                    &right.message,
                    &right.node_id,
                ))
        });
    }
}

#[cfg(test)]
#[path = "issues_test.rs"]
mod tests;
