use super::apply_patches_from_command;
use crate::commands::{EngineCommand, EngineCommandEnvelope, EngineCommandType};
use crate::events::{EngineEventStream, EngineEventType};
use ais_core::build_runtime_patch_guard_policy;
use serde_json::{json, Value};

fn apply_patches_command(patches: Value) -> EngineCommandEnvelope {
    EngineCommandEnvelope::new(EngineCommand {
        id: "cmd-patch-1".to_string(),
        command_type: EngineCommandType::ApplyPatches,
        data: serde_json::Map::from_iter([("patches".to_string(), patches)]),
    })
}

#[test]
fn apply_patches_emits_patch_applied_with_audit_hash() {
    let mut runtime = json!({});
    let command = apply_patches_command(json!([
        {"op": "set", "path": "inputs.amount", "value": "100"}
    ]));
    let guard = build_runtime_patch_guard_policy();
    let mut stream = EngineEventStream::new("run-1");

    let execution = apply_patches_from_command(
        &mut runtime,
        &command,
        &guard,
        &mut stream,
        "2026-02-13T00:00:00Z",
    )
    .expect("must apply");

    assert_eq!(runtime.get("inputs").and_then(|value| value.get("amount")), Some(&json!("100")));
    assert_eq!(execution.apply_result.audit.applied_count, 1);
    assert_eq!(execution.apply_result.audit.rejected_count, 0);
    assert_eq!(execution.events.len(), 1);
    assert_eq!(execution.events[0].event.event_type, EngineEventType::PatchApplied);
    assert_eq!(
        execution.events[0].event.data.get("audit_hash"),
        Some(&json!(execution.apply_result.audit.hash))
    );
}

#[test]
fn apply_patches_rejects_nodes_paths_and_emits_patch_rejected() {
    let mut runtime = json!({});
    let command = apply_patches_command(json!([
        {"op": "set", "path": "nodes.swap.outputs", "value": {"ok": true}}
    ]));
    let guard = build_runtime_patch_guard_policy();
    let mut stream = EngineEventStream::new("run-2");

    let execution = apply_patches_from_command(
        &mut runtime,
        &command,
        &guard,
        &mut stream,
        "2026-02-13T00:00:00Z",
    )
    .expect("must execute");

    assert_eq!(execution.apply_result.audit.applied_count, 0);
    assert_eq!(execution.apply_result.audit.rejected_count, 1);
    assert_eq!(execution.events.len(), 1);
    assert_eq!(execution.events[0].event.event_type, EngineEventType::PatchRejected);
    assert!(execution.events[0].event.data.get("rejections").is_some());
}

#[test]
fn apply_patches_partial_success_emits_both_events() {
    let mut runtime = json!({});
    let command = apply_patches_command(json!([
        {"op": "set", "path": "inputs.ok", "value": true},
        {"op": "set", "path": "nodes.swap.outputs", "value": {"ok": true}}
    ]));
    let guard = build_runtime_patch_guard_policy();
    let mut stream = EngineEventStream::new("run-3");

    let execution = apply_patches_from_command(
        &mut runtime,
        &command,
        &guard,
        &mut stream,
        "2026-02-13T00:00:00Z",
    )
    .expect("must execute");

    assert_eq!(execution.apply_result.audit.applied_count, 1);
    assert_eq!(execution.apply_result.audit.rejected_count, 1);
    assert!(execution.apply_result.audit.partial_success);
    assert_eq!(execution.events.len(), 2);
    assert_eq!(execution.events[0].event.event_type, EngineEventType::PatchApplied);
    assert_eq!(execution.events[1].event.event_type, EngineEventType::PatchRejected);
}

#[test]
fn apply_patches_enforces_guard_even_if_policy_disabled() {
    let mut runtime = json!({});
    let command = apply_patches_command(json!([
        {"op": "set", "path": "nodes.swap.outputs", "value": {"ok": true}}
    ]));
    let mut guard = build_runtime_patch_guard_policy();
    guard.enabled = false;
    let mut stream = EngineEventStream::new("run-4");

    let execution = apply_patches_from_command(
        &mut runtime,
        &command,
        &guard,
        &mut stream,
        "2026-02-13T00:00:00Z",
    )
    .expect("must execute");

    assert_eq!(execution.apply_result.audit.applied_count, 0);
    assert_eq!(execution.apply_result.audit.rejected_count, 1);
}
