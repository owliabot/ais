use crate::runtime_patch::{split_path, RuntimePatch};
use regex::Regex;
use serde::{Deserialize, Serialize};

pub const DEFAULT_RUNTIME_PATCH_ALLOW_ROOTS: &[&str] = &["inputs", "ctx", "contracts", "policy"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimePatchGuardPolicy {
    pub enabled: bool,
    pub allow_roots: Vec<String>,
    #[serde(default)]
    pub allow_path_patterns: Vec<String>,
    #[serde(default)]
    pub allow_nodes_paths: Vec<String>,
}

impl Default for RuntimePatchGuardPolicy {
    fn default() -> Self {
        build_runtime_patch_guard_policy()
    }
}

pub fn build_runtime_patch_guard_policy() -> RuntimePatchGuardPolicy {
    RuntimePatchGuardPolicy {
        enabled: true,
        allow_roots: DEFAULT_RUNTIME_PATCH_ALLOW_ROOTS
            .iter()
            .map(|value| value.to_string())
            .collect(),
        allow_path_patterns: Vec::new(),
        allow_nodes_paths: Vec::new(),
    }
}

pub fn check_runtime_patch_path_allowed(
    patch: &RuntimePatch,
    policy: &RuntimePatchGuardPolicy,
) -> Result<(), String> {
    if !policy.enabled {
        return Ok(());
    }

    let parts = split_path(&patch.path).ok_or_else(|| "invalid_path".to_string())?;
    let root = parts.first().copied().ok_or_else(|| "invalid_path".to_string())?;
    if root == "nodes" {
        if path_matches_any_pattern(&patch.path, &policy.allow_nodes_paths)? {
            return Ok(());
        }
        return Err("nodes_paths_forbidden".to_string());
    }

    if policy.allow_roots.iter().any(|allowed| allowed == root) {
        return Ok(());
    }

    if path_matches_any_pattern(&patch.path, &policy.allow_path_patterns)? {
        return Ok(());
    }

    Err(format!("root_not_allowed:{root}"))
}

fn path_matches_any_pattern(path: &str, patterns: &[String]) -> Result<bool, String> {
    for pattern in patterns {
        let regex = Regex::new(pattern).map_err(|err| format!("invalid_regex:{err}"))?;
        if regex.is_match(path) {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
#[path = "guard_test.rs"]
mod tests;
