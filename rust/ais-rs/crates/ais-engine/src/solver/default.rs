use crate::events::{EngineEvent, EngineEventType};
use ais_core::{RuntimePatch, RuntimePatchOp};
use ais_sdk::{NodeReadinessResult, NodeRunState};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};

pub trait Solver {
    fn solve(&self, node: &Value, readiness: &NodeReadinessResult, context: &SolverContext) -> SolverDecision;
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SolverContext {
    #[serde(default)]
    pub contract_candidates: BTreeMap<String, Vec<Value>>,
    #[serde(default)]
    pub detect_provider_candidates: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SolverDecision {
    Noop,
    ApplyPatches {
        patches: Vec<RuntimePatch>,
        summary: String,
    },
    SelectProvider {
        provider: String,
        summary: String,
    },
    NeedUserConfirm {
        reason: String,
        details: Map<String, Value>,
    },
}

#[derive(Debug, Clone, Default)]
pub struct DefaultSolver;

impl Solver for DefaultSolver {
    fn solve(&self, _node: &Value, readiness: &NodeReadinessResult, context: &SolverContext) -> SolverDecision {
        if readiness.state != NodeRunState::Blocked {
            return SolverDecision::Noop;
        }

        let mut proposed_patches = Vec::<RuntimePatch>::new();
        let mut resolved_contract_paths = BTreeSet::<String>::new();
        for missing_ref in &readiness.missing_refs {
            if let Some(contract_path) = normalize_contract_path(missing_ref) {
                if resolved_contract_paths.contains(&contract_path) {
                    continue;
                }
                let Some(candidates) = context.contract_candidates.get(&contract_path) else {
                    continue;
                };
                if candidates.len() == 1 {
                    proposed_patches.push(RuntimePatch {
                        op: RuntimePatchOp::Set,
                        path: contract_path.clone(),
                        value: candidates[0].clone(),
                        extensions: None,
                    });
                    resolved_contract_paths.insert(contract_path);
                }
            }
        }

        let remaining_missing = readiness
            .missing_refs
            .iter()
            .filter(|path| {
                normalize_contract_path(path)
                    .map(|contract_path| !resolved_contract_paths.contains(&contract_path))
                    .unwrap_or(true)
            })
            .cloned()
            .collect::<Vec<_>>();

        if !remaining_missing.is_empty() {
            return SolverDecision::NeedUserConfirm {
                reason: "missing_inputs_or_runtime_refs".to_string(),
                details: map_from_entries(vec![
                    ("missing_refs", Value::Array(remaining_missing.into_iter().map(Value::String).collect())),
                    ("proposed_patch_count", Value::Number((proposed_patches.len() as u64).into())),
                ]),
            };
        }

        if readiness.needs_detect {
            return match context.detect_provider_candidates.as_slice() {
                [provider] => SolverDecision::SelectProvider {
                    provider: provider.clone(),
                    summary: "detect provider selected by default solver".to_string(),
                },
                candidates => SolverDecision::NeedUserConfirm {
                    reason: "detect_provider_selection_required".to_string(),
                    details: map_from_entries(vec![(
                        "provider_candidates",
                        Value::Array(candidates.iter().cloned().map(Value::String).collect()),
                    )]),
                },
            };
        }

        if !readiness.errors.is_empty() {
            return SolverDecision::NeedUserConfirm {
                reason: "readiness_errors".to_string(),
                details: map_from_entries(vec![(
                    "errors",
                    Value::Array(readiness.errors.iter().cloned().map(Value::String).collect()),
                )]),
            };
        }

        if !proposed_patches.is_empty() {
            return SolverDecision::ApplyPatches {
                patches: proposed_patches,
                summary: "contracts auto-filled by default solver".to_string(),
            };
        }

        SolverDecision::NeedUserConfirm {
            reason: "blocked_no_safe_solver_action".to_string(),
            details: Map::new(),
        }
    }
}

pub fn build_solver_event(node_id: Option<&str>, decision: &SolverDecision) -> Option<EngineEvent> {
    match decision {
        SolverDecision::Noop => None,
        SolverDecision::ApplyPatches { patches, summary } => {
            let mut event = EngineEvent::new(EngineEventType::SolverApplied);
            event.node_id = node_id.map(str::to_string);
            event.data.insert(
                "patches".to_string(),
                serde_json::to_value(patches).unwrap_or(Value::Array(Vec::new())),
            );
            event
                .data
                .insert("summary".to_string(), Value::String(summary.clone()));
            Some(event)
        }
        SolverDecision::SelectProvider { provider, summary } => {
            let mut event = EngineEvent::new(EngineEventType::SolverApplied);
            event.node_id = node_id.map(str::to_string);
            event
                .data
                .insert("provider".to_string(), Value::String(provider.clone()));
            event
                .data
                .insert("summary".to_string(), Value::String(summary.clone()));
            Some(event)
        }
        SolverDecision::NeedUserConfirm { reason, details } => {
            let mut event = EngineEvent::new(EngineEventType::NeedUserConfirm);
            event.node_id = node_id.map(str::to_string);
            event
                .data
                .insert("reason".to_string(), Value::String(reason.clone()));
            event
                .data
                .insert("details".to_string(), Value::Object(details.clone()));
            Some(event)
        }
    }
}

fn normalize_contract_path(path: &str) -> Option<String> {
    let parts = path.split('.').collect::<Vec<_>>();
    if parts.len() < 2 || parts[0] != "contracts" {
        return None;
    }
    Some(format!("{}.{}", parts[0], parts[1]))
}

fn map_from_entries(entries: Vec<(&str, Value)>) -> Map<String, Value> {
    let mut out = Map::new();
    for (key, value) in entries {
        out.insert(key.to_string(), value);
    }
    out
}

#[cfg(test)]
#[path = "default_test.rs"]
mod tests;
