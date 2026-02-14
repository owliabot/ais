use super::{validate_runtime_patch, RuntimePatch, RuntimePatchOp};
use serde_json::json;

#[test]
fn validate_rejects_empty_path() {
    let patch = RuntimePatch {
        op: RuntimePatchOp::Set,
        path: String::new(),
        value: json!(1),
        extensions: None,
    };
    let issues = validate_runtime_patch(&patch);
    assert!(!issues.is_empty());
}

#[test]
fn validate_accepts_basic_path() {
    let patch = RuntimePatch {
        op: RuntimePatchOp::Merge,
        path: "inputs.amount".to_string(),
        value: json!({"x":1}),
        extensions: None,
    };
    let issues = validate_runtime_patch(&patch);
    assert!(issues.is_empty());
}
