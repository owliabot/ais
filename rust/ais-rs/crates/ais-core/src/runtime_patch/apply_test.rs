use super::apply_runtime_patches;
use crate::runtime_patch::{
    build_runtime_patch_guard_policy, RuntimePatch, RuntimePatchGuardPolicy, RuntimePatchOp,
};
use serde_json::json;

#[test]
fn apply_set_and_merge_patches() {
    let mut runtime = json!({});
    let patches = vec![
        RuntimePatch {
            op: RuntimePatchOp::Set,
            path: "inputs.amount".to_string(),
            value: json!("100"),
            extensions: None,
        },
        RuntimePatch {
            op: RuntimePatchOp::Merge,
            path: "ctx.wallet".to_string(),
            value: json!({"address": "0xabc"}),
            extensions: None,
        },
    ];
    let policy = build_runtime_patch_guard_policy();

    let result = apply_runtime_patches(&mut runtime, &patches, &policy);

    assert_eq!(result.audit.applied_count, 2);
    assert_eq!(result.audit.rejected_count, 0);
    assert!(!result.audit.hash.is_empty());
    assert_eq!(runtime["inputs"]["amount"], "100");
    assert_eq!(runtime["ctx"]["wallet"]["address"], "0xabc");
}

#[test]
fn guard_rejection_produces_partial_success_audit() {
    let mut runtime = json!({});
    let patches = vec![
        RuntimePatch {
            op: RuntimePatchOp::Set,
            path: "inputs.amount".to_string(),
            value: json!("100"),
            extensions: None,
        },
        RuntimePatch {
            op: RuntimePatchOp::Set,
            path: "nodes.n1.outputs".to_string(),
            value: json!({"x": 1}),
            extensions: None,
        },
    ];
    let policy = build_runtime_patch_guard_policy();

    let result = apply_runtime_patches(&mut runtime, &patches, &policy);

    assert_eq!(result.audit.applied_count, 1);
    assert_eq!(result.audit.rejected_count, 1);
    assert!(result.audit.partial_success);
    assert_eq!(result.rejected[0].path, "nodes.n1.outputs");
}

#[test]
fn allow_nodes_paths_can_enable_specific_target() {
    let mut runtime = json!({});
    let patch = RuntimePatch {
        op: RuntimePatchOp::Set,
        path: "nodes.allowed.outputs".to_string(),
        value: json!({"ok": true}),
        extensions: None,
    };
    let mut policy = RuntimePatchGuardPolicy::default();
    policy.allow_nodes_paths.push(r"^nodes\.allowed\.outputs$".to_string());

    let result = apply_runtime_patches(&mut runtime, &[patch], &policy);

    assert_eq!(result.audit.rejected_count, 0);
    assert_eq!(runtime["nodes"]["allowed"]["outputs"]["ok"], true);
}
