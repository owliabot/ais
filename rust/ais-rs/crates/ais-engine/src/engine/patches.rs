use crate::commands::{EngineCommandEnvelope, EngineCommandType};
use crate::events::{EngineEvent, EngineEventRecord, EngineEventStream, EngineEventType};
use ais_core::{
    apply_runtime_patches, build_runtime_patch_guard_policy, RuntimePatch, RuntimePatchApplyResult,
    RuntimePatchGuardPolicy,
};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub struct ApplyPatchesExecution {
    pub apply_result: RuntimePatchApplyResult,
    pub events: Vec<EngineEventRecord>,
}

#[derive(Debug, thiserror::Error)]
pub enum ApplyPatchesCommandError {
    #[error("command type must be apply_patches")]
    InvalidCommandType,
    #[error("apply_patches command must contain array field `patches`: {0}")]
    InvalidPayload(String),
}

pub fn apply_patches_from_command(
    runtime: &mut Value,
    envelope: &EngineCommandEnvelope,
    guard_policy: &RuntimePatchGuardPolicy,
    stream: &mut EngineEventStream,
    ts: impl Into<String>,
) -> Result<ApplyPatchesExecution, ApplyPatchesCommandError> {
    if envelope.command.command_type != EngineCommandType::ApplyPatches {
        return Err(ApplyPatchesCommandError::InvalidCommandType);
    }

    let patches = parse_patches_from_command(envelope)?;
    let enforced_guard_policy = enforce_guard_policy(guard_policy);
    let apply_result = apply_runtime_patches(runtime, &patches, &enforced_guard_policy);
    let events = build_patch_audit_events(&envelope.command.id, &apply_result, stream, ts.into());

    Ok(ApplyPatchesExecution {
        apply_result,
        events,
    })
}

fn parse_patches_from_command(
    envelope: &EngineCommandEnvelope,
) -> Result<Vec<RuntimePatch>, ApplyPatchesCommandError> {
    let Some(raw_patches) = envelope.command.data.get("patches") else {
        return Err(ApplyPatchesCommandError::InvalidPayload(
            "missing `patches` field".to_string(),
        ));
    };
    serde_json::from_value::<Vec<RuntimePatch>>(raw_patches.clone())
        .map_err(|error| ApplyPatchesCommandError::InvalidPayload(error.to_string()))
}

fn enforce_guard_policy(policy: &RuntimePatchGuardPolicy) -> RuntimePatchGuardPolicy {
    let mut out = policy.clone();
    if !out.enabled {
        let fallback = build_runtime_patch_guard_policy();
        out.enabled = true;
        if out.allow_roots.is_empty() {
            out.allow_roots = fallback.allow_roots;
        }
    }
    out
}

fn build_patch_audit_events(
    command_id: &str,
    apply_result: &RuntimePatchApplyResult,
    stream: &mut EngineEventStream,
    ts: String,
) -> Vec<EngineEventRecord> {
    let mut events = Vec::<EngineEventRecord>::new();

    if apply_result.audit.applied_count > 0 {
        events.push(stream.next_record(
            ts.clone(),
            patch_event(EngineEventType::PatchApplied, command_id, apply_result, false),
        ));
    }
    if apply_result.audit.rejected_count > 0 {
        events.push(stream.next_record(
            ts,
            patch_event(EngineEventType::PatchRejected, command_id, apply_result, true),
        ));
    }

    events
}

fn patch_event(
    event_type: EngineEventType,
    command_id: &str,
    apply_result: &RuntimePatchApplyResult,
    include_rejections: bool,
) -> EngineEvent {
    let mut event = EngineEvent::new(event_type);
    event
        .data
        .insert("command_id".to_string(), Value::String(command_id.to_string()));
    event.data.insert(
        "audit_hash".to_string(),
        Value::String(apply_result.audit.hash.clone()),
    );
    event.data.insert(
        "patch_count".to_string(),
        Value::Number(serde_json::Number::from(apply_result.audit.patch_count as u64)),
    );
    event.data.insert(
        "applied_count".to_string(),
        Value::Number(serde_json::Number::from(apply_result.audit.applied_count as u64)),
    );
    event.data.insert(
        "rejected_count".to_string(),
        Value::Number(serde_json::Number::from(apply_result.audit.rejected_count as u64)),
    );
    event.data.insert(
        "partial_success".to_string(),
        Value::Bool(apply_result.audit.partial_success),
    );
    event.data.insert(
        "affected_paths".to_string(),
        Value::Array(
            apply_result
                .audit
                .affected_paths
                .iter()
                .cloned()
                .map(Value::String)
                .collect(),
        ),
    );
    if include_rejections {
        event.data.insert(
            "rejections".to_string(),
            serde_json::to_value(&apply_result.rejected).unwrap_or(Value::Array(Vec::new())),
        );
    }
    event
}

#[cfg(test)]
#[path = "patches_test.rs"]
mod tests;
