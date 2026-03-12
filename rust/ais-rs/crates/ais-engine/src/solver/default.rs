use crate::events::{EngineEvent, EngineEventType};
use ais_core::RuntimePatch;
use ais_sdk::{NodeReadinessResult, NodeRunState};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

pub trait Solver {
    fn solve(
        &self,
        node: &Value,
        readiness: &NodeReadinessResult,
        context: &SolverContext,
    ) -> SolverDecision;
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SolverContext {
    #[serde(default)]
    pub contract_candidates: BTreeMap<String, Vec<Value>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SolverDecision {
    Noop,
    ApplyPatches {
        patches: Vec<RuntimePatch>,
        summary: String,
    },
    NeedUserInput {
        reason: String,
        details: Map<String, Value>,
    },
    NeedUserConfirm {
        reason: String,
        details: Map<String, Value>,
    },
}

#[derive(Debug, Clone, Default)]
pub struct DefaultSolver;

impl Solver for DefaultSolver {
    fn solve(
        &self,
        _node: &Value,
        readiness: &NodeReadinessResult,
        _context: &SolverContext,
    ) -> SolverDecision {
        if readiness.state != NodeRunState::Blocked {
            return SolverDecision::Noop;
        }

        let recoverable_missing = readiness
            .missing_refs
            .iter()
            .filter(|path| normalize_contract_path(path).is_none())
            .cloned()
            .collect::<Vec<_>>();
        let unresolved_system_refs = readiness
            .missing_refs
            .iter()
            .filter_map(|path| normalize_contract_path(path.as_str()))
            .collect::<Vec<_>>();

        if !recoverable_missing.is_empty() {
            return SolverDecision::NeedUserInput {
                reason: "missing_inputs_or_runtime_refs".to_string(),
                details: map_from_entries(vec![(
                    "missing_refs",
                    Value::Array(recoverable_missing.into_iter().map(Value::String).collect()),
                )]),
            };
        }

        if !unresolved_system_refs.is_empty() {
            return SolverDecision::NeedUserConfirm {
                reason: "unresolved_system_refs".to_string(),
                details: map_from_entries(vec![(
                    "system_missing_refs",
                    Value::Array(
                        unresolved_system_refs
                            .into_iter()
                            .map(Value::String)
                            .collect(),
                    ),
                )]),
            };
        }

        if !readiness.errors.is_empty() {
            return SolverDecision::NeedUserConfirm {
                reason: "readiness_errors".to_string(),
                details: map_from_entries(vec![(
                    "errors",
                    Value::Array(
                        readiness
                            .errors
                            .iter()
                            .cloned()
                            .map(Value::String)
                            .collect(),
                    ),
                )]),
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
        SolverDecision::NeedUserInput { reason, details } => {
            let mut event = EngineEvent::new(EngineEventType::NeedUserInput);
            event.node_id = node_id.map(str::to_string);
            event
                .data
                .insert("reason".to_string(), Value::String(reason.clone()));
            event
                .data
                .insert("details".to_string(), Value::Object(details.clone()));
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
