use crate::runtime_patch::{
    check_runtime_patch_path_allowed, split_path, validate_runtime_patch, RuntimePatch,
    RuntimePatchGuardPolicy, RuntimePatchOp,
};
use crate::{stable_hash_hex, StableJsonOptions};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimePatchRejection {
    pub index: usize,
    pub path: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimePatchAudit {
    pub patch_count: usize,
    pub applied_count: usize,
    pub rejected_count: usize,
    pub affected_paths: Vec<String>,
    pub partial_success: bool,
    pub hash: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimePatchApplyResult {
    pub rejected: Vec<RuntimePatchRejection>,
    pub audit: RuntimePatchAudit,
}

pub fn apply_runtime_patches(
    runtime: &mut Value,
    patches: &[RuntimePatch],
    guard_policy: &RuntimePatchGuardPolicy,
) -> RuntimePatchApplyResult {
    let mut applied_count = 0usize;
    let mut rejected = Vec::new();
    let mut affected_paths = BTreeSet::new();

    if !runtime.is_object() {
        *runtime = Value::Object(Map::new());
    }

    for (index, patch) in patches.iter().enumerate() {
        let validation_issues = validate_runtime_patch(patch);
        if !validation_issues.is_empty() {
            rejected.push(RuntimePatchRejection {
                index,
                path: patch.path.clone(),
                reason: validation_issues[0].message.clone(),
            });
            continue;
        }

        if let Err(reason) = check_runtime_patch_path_allowed(patch, guard_policy) {
            rejected.push(RuntimePatchRejection {
                index,
                path: patch.path.clone(),
                reason,
            });
            continue;
        }

        let Some(parts) = split_path(&patch.path) else {
            rejected.push(RuntimePatchRejection {
                index,
                path: patch.path.clone(),
                reason: "invalid_path".to_string(),
            });
            continue;
        };

        match apply_one_patch(runtime, &parts, patch) {
            Ok(()) => {
                applied_count += 1;
                affected_paths.insert(patch.path.clone());
            }
            Err(reason) => rejected.push(RuntimePatchRejection {
                index,
                path: patch.path.clone(),
                reason,
            }),
        }
    }

    let affected_paths = affected_paths.into_iter().collect::<Vec<_>>();
    let rejected_count = rejected.len();
    let partial_success = applied_count > 0 && rejected_count > 0;
    let hash_input = json!({
        "patch_count": patches.len(),
        "applied_count": applied_count,
        "rejected_count": rejected_count,
        "affected_paths": affected_paths,
        "partial_success": partial_success
    });
    let hash = stable_hash_hex(&hash_input, &StableJsonOptions::default())
        .expect("stable hash for audit input must always succeed");

    RuntimePatchApplyResult {
        rejected,
        audit: RuntimePatchAudit {
            patch_count: patches.len(),
            applied_count,
            rejected_count,
            affected_paths,
            partial_success,
            hash,
        },
    }
}

fn apply_one_patch(runtime: &mut Value, path_parts: &[&str], patch: &RuntimePatch) -> Result<(), String> {
    let Some(last_key) = path_parts.last().copied() else {
        return Err("invalid_path".to_string());
    };

    let mut current = runtime;
    for segment in &path_parts[..path_parts.len() - 1] {
        let Some(object) = current.as_object_mut() else {
            return Err("non_object_intermediate".to_string());
        };

        current = object
            .entry((*segment).to_string())
            .or_insert_with(|| Value::Object(Map::new()));

        if !current.is_object() {
            return Err(format!("non_object_intermediate:{segment}"));
        }
    }

    let Some(object) = current.as_object_mut() else {
        return Err("non_object_target_parent".to_string());
    };

    match patch.op {
        RuntimePatchOp::Set => {
            object.insert(last_key.to_string(), patch.value.clone());
            Ok(())
        }
        RuntimePatchOp::Merge => {
            let patch_object = patch
                .value
                .as_object()
                .ok_or_else(|| "merge_value_must_be_object".to_string())?;
            let target = object
                .entry(last_key.to_string())
                .or_insert_with(|| Value::Object(Map::new()));
            let target_object = target
                .as_object_mut()
                .ok_or_else(|| "merge_target_not_object".to_string())?;
            for (key, value) in patch_object {
                target_object.insert(key.clone(), value.clone());
            }
            Ok(())
        }
    }
}

#[cfg(test)]
#[path = "apply_test.rs"]
mod tests;
