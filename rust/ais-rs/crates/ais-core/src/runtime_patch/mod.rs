mod apply;
mod guard;

pub use apply::{apply_runtime_patches, RuntimePatchApplyResult, RuntimePatchAudit, RuntimePatchRejection};
pub use guard::{
    build_runtime_patch_guard_policy, check_runtime_patch_path_allowed, RuntimePatchGuardPolicy,
    DEFAULT_RUNTIME_PATCH_ALLOW_ROOTS,
};

use crate::{IssueSeverity, StructuredIssue};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimePatchOp {
    Set,
    Merge,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimePatch {
    pub op: RuntimePatchOp,
    pub path: String,
    pub value: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extensions: Option<Value>,
}

pub fn validate_runtime_patch(patch: &RuntimePatch) -> Vec<StructuredIssue> {
    let mut issues = Vec::new();

    if patch.path.trim().is_empty() {
        issues.push(StructuredIssue {
            kind: "patch_validation".to_string(),
            severity: IssueSeverity::Error,
            node_id: None,
            field_path: "$.path".parse().expect("field path must parse"),
            message: "runtime patch path must be non-empty".to_string(),
            reference: Some("runtime_patch.path".to_string()),
            related: None,
        });
    }

    if split_path(&patch.path).is_none() {
        issues.push(StructuredIssue {
            kind: "patch_validation".to_string(),
            severity: IssueSeverity::Error,
            node_id: None,
            field_path: "$.path".parse().expect("field path must parse"),
            message: "runtime patch path must be dot-separated identifiers".to_string(),
            reference: Some("runtime_patch.path_format".to_string()),
            related: None,
        });
    }

    issues
}

pub(crate) fn split_path(path: &str) -> Option<Vec<&str>> {
    let parts = path
        .split('.')
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    if parts.is_empty() {
        return None;
    }
    if parts.iter().any(|segment| segment.chars().any(char::is_whitespace)) {
        return None;
    }
    Some(parts)
}

#[cfg(test)]
#[path = "mod_test.rs"]
mod tests;
