use crate::cli::ApprovalsMode;
use crate::error::RunnerError;
use ais_engine::{PolicyEnforcementOptions, PolicyPackAllowlist, PolicyThresholdRules};
use ais_sdk::{
    parse_document_with_options, AisDocument, DocumentFormat, PackDocument, ParseDocumentOptions,
};
use serde_json::{Map, Value};
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum PackPolicyError {
    #[error("pack file must be AIS pack document")]
    NotPackDocument,
    #[error("pack policy must be an object")]
    PolicyNotObject,
    #[error("invalid approvals thresholds: auto_execute_max_risk_level ({auto_execute_max_risk_level}) must be < require_approval_min_risk_level ({require_approval_min_risk_level})")]
    InvalidApprovalThresholds {
        auto_execute_max_risk_level: u8,
        require_approval_min_risk_level: u8,
    },
}

pub fn load_pack_document(path: &Path) -> Result<PackDocument, RunnerError> {
    let text = fs::read_to_string(path).map_err(|source| RunnerError::ReadFile {
        path: path.display().to_string(),
        source,
    })?;
    let parsed = parse_document_with_options(
        text.as_str(),
        ParseDocumentOptions {
            format: DocumentFormat::Auto,
            validate_schema: true,
        },
    )
    .map_err(|issues| RunnerError::WorkspaceValidate(format!("{issues:?}")))?;

    match parsed {
        AisDocument::Pack(pack) => Ok(pack),
        _ => Err(RunnerError::WorkspaceValidate(
            PackPolicyError::NotPackDocument.to_string(),
        )),
    }
}

pub fn approvals_mode_from_pack(pack: &PackDocument) -> Option<ApprovalsMode> {
    let policy = pack.policy.as_ref()?.as_object()?;
    let approvals = policy.get("approvals")?.as_object()?;
    let mode = approvals.get("mode")?.as_str()?;
    match mode {
        "safe" => Some(ApprovalsMode::Safe),
        "assist" => Some(ApprovalsMode::Assist),
        "yolo" => Some(ApprovalsMode::Yolo),
        _ => None,
    }
}

pub fn llm_may_approve_max_risk_level_from_pack(pack: &PackDocument) -> Option<u8> {
    let policy = pack.policy.as_ref()?.as_object()?;
    let approvals = policy.get("approvals")?.as_object()?;
    approvals
        .get("llm_may_approve_max_risk_level")
        .and_then(Value::as_u64)
        .and_then(|value| u8::try_from(value).ok())
}

pub fn policy_from_pack(pack: &PackDocument) -> Result<PolicyEnforcementOptions, PackPolicyError> {
    let policy_object = match &pack.policy {
        Some(Value::Object(map)) => Some(map),
        Some(_) => return Err(PackPolicyError::PolicyNotObject),
        None => None,
    };

    let thresholds = thresholds_from_policy(policy_object)?;
    let allowlist = allowlist_from_pack(pack);

    Ok(PolicyEnforcementOptions {
        strict_allowlist: false,
        hard_block_on_missing: false,
        enforce_plugin_execution_allowlist: true,
        allowlist,
        thresholds,
    })
}

fn thresholds_from_policy(
    policy: Option<&Map<String, Value>>,
) -> Result<PolicyThresholdRules, PackPolicyError> {
    let mut thresholds = PolicyThresholdRules::default();

    let approvals = policy
        .and_then(|policy| policy.get("approvals"))
        .and_then(Value::as_object);
    let mut auto_execute_max_risk_level: Option<u8> = None;
    if let Some(approvals) = approvals {
        auto_execute_max_risk_level = approvals
            .get("auto_execute_max_risk_level")
            .and_then(Value::as_u64)
            .and_then(|value| u8::try_from(value).ok());
        let require_approval_min_risk_level = approvals
            .get("require_approval_min_risk_level")
            .and_then(Value::as_u64)
            .and_then(|value| u8::try_from(value).ok());
        if let (Some(a), Some(r)) = (auto_execute_max_risk_level, require_approval_min_risk_level) {
            if a >= r {
                return Err(PackPolicyError::InvalidApprovalThresholds {
                    auto_execute_max_risk_level: a,
                    require_approval_min_risk_level: r,
                });
            }
        }
    }
    thresholds.max_risk_level = auto_execute_max_risk_level.or(Some(0));

    Ok(thresholds)
}

fn allowlist_from_pack(pack: &PackDocument) -> PolicyPackAllowlist {
    let mut chains = BTreeSet::<String>::new();
    for include in &pack.includes {
        let Some(obj) = include.as_object() else {
            continue;
        };
        let Some(scope) = obj.get("chain_scope").and_then(Value::as_array) else {
            continue;
        };
        for chain in scope.iter().filter_map(Value::as_str) {
            if !chain.trim().is_empty() {
                chains.insert(chain.to_string());
            }
        }
    }

    let (plugin_types, plugin_chains) = plugin_execution_allowlist(pack);
    for chain in plugin_chains {
        chains.insert(chain);
    }

    PolicyPackAllowlist {
        chains: chains.into_iter().collect(),
        execution_types: plugin_types,
        action_refs: Vec::new(),
    }
}

fn plugin_execution_allowlist(pack: &PackDocument) -> (Vec<String>, Vec<String>) {
    let enabled = pack
        .plugins
        .as_ref()
        .and_then(Value::as_object)
        .and_then(|plugins| plugins.get("execution"))
        .and_then(Value::as_object)
        .and_then(|execution| execution.get("enabled"))
        .and_then(Value::as_array);
    let Some(enabled) = enabled else {
        return (Vec::new(), Vec::new());
    };

    let mut types = BTreeSet::<String>::new();
    let mut chains = BTreeSet::<String>::new();
    for entry in enabled {
        let Some(obj) = entry.as_object() else {
            continue;
        };
        let Some(exec_type) = obj.get("type").and_then(Value::as_str) else {
            continue;
        };
        if !exec_type.trim().is_empty() {
            types.insert(exec_type.to_string());
        }
        if let Some(scope) = obj.get("chains").and_then(Value::as_array) {
            for chain in scope.iter().filter_map(Value::as_str) {
                if !chain.trim().is_empty() {
                    chains.insert(chain.to_string());
                }
            }
        }
    }

    (types.into_iter().collect(), chains.into_iter().collect())
}

#[cfg(test)]
#[path = "pack_test.rs"]
mod tests;
