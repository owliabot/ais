use super::{build_runtime_patch_guard_policy, check_runtime_patch_path_allowed, RuntimePatchGuardPolicy};
use crate::runtime_patch::{RuntimePatch, RuntimePatchOp};
use serde_json::json;

fn patch(path: &str) -> RuntimePatch {
    RuntimePatch {
        op: RuntimePatchOp::Set,
        path: path.to_string(),
        value: json!(1),
        extensions: None,
    }
}

#[test]
fn default_policy_rejects_nodes_paths() {
    let policy = build_runtime_patch_guard_policy();
    let result = check_runtime_patch_path_allowed(&patch("nodes.n1.outputs"), &policy);
    assert!(result.is_err());
}

#[test]
fn default_policy_accepts_inputs() {
    let policy = build_runtime_patch_guard_policy();
    let result = check_runtime_patch_path_allowed(&patch("inputs.amount"), &policy);
    assert!(result.is_ok());
}

#[test]
fn allow_nodes_path_pattern_overrides_default_block() {
    let mut policy = RuntimePatchGuardPolicy::default();
    policy.allow_nodes_paths = vec![r"^nodes\.n1\.outputs$".to_string()];
    let result = check_runtime_patch_path_allowed(&patch("nodes.n1.outputs"), &policy);
    assert!(result.is_ok());
}
