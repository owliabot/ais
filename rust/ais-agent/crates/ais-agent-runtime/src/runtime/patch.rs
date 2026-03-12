use std::collections::{BTreeSet, VecDeque};

use ais_agent_control::patch::{PlanPatchOperation, PlanPatchSubmission, PlanPatchTarget};
use ais_agent_core::{
    action::{ActionNodeKind, ActionNodeStatus},
    driver::{ActionGraphFragment, DriverBuildOutput},
    effect::EffectContract,
    runtime::RunPhase,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::runtime::{
    ActiveRun, DriverBindingContext, RuntimeDriverBinder, RuntimeDriverBindingError,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Error)]
pub enum RuntimePatchError {
    #[error("plan patch requires a stable boundary")]
    NotAtStableBoundary,
    #[error("run is terminal and cannot be patched")]
    TerminalRun,
    #[error("patch target `{target}` is not legal in the current runtime state: {reason}")]
    IllegalTarget { target: String, reason: String },
    #[error("patch target resolved to no scope")]
    EmptyScope,
    #[error("plan patch fragment payload is invalid: {0}")]
    InvalidFragment(String),
    #[error("plan patch effect-contract payload is invalid: {0}")]
    InvalidEffectContract(String),
    #[error("plan patch references unknown effect contract `{effect_ref}`")]
    MissingEffectContract { effect_ref: String },
    #[error("plan patch contract effect_ref mismatch: expected `{expected}`, got `{actual}`")]
    EffectContractRefMismatch { expected: String, actual: String },
    #[error("plan patch driver binding failed: {0}")]
    DriverBinding(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuntimePatchOutcome {
    pub patched_node_refs: Vec<String>,
    pub updated_effect_refs: Vec<String>,
    pub mission_constraints_updated: bool,
}

pub fn apply_plan_patch(
    runtime: &mut ActiveRun,
    patch: &PlanPatchSubmission,
) -> Result<RuntimePatchOutcome, RuntimePatchError> {
    ensure_patchable_runtime(runtime)?;

    let scope = compute_scope(runtime, &patch.target)?;
    validate_scope(runtime, &patch.target, &scope)?;
    let inherited_dependencies = external_predecessors(runtime, &scope);

    let mut outcome = RuntimePatchOutcome {
        patched_node_refs: scope.iter().cloned().collect(),
        ..RuntimePatchOutcome::default()
    };

    for node_id in &scope {
        if let Some(node) = runtime.checkpoint.action_graph.nodes.get_mut(node_id) {
            node.status = ActionNodeStatus::Skipped;
        }
    }

    for operation in &patch.operations {
        match operation {
            PlanPatchOperation::ReplaceFragment {
                fragment,
                preserved_effect_refs,
            }
            | PlanPatchOperation::AppendFragment {
                fragment,
                preserved_effect_refs,
            } => {
                for effect_ref in preserved_effect_refs {
                    if !runtime.checkpoint.effect_contracts.contains_key(effect_ref) {
                        return Err(RuntimePatchError::MissingEffectContract {
                            effect_ref: effect_ref.clone(),
                        });
                    }
                }

                let mut fragment: ActionGraphFragment = serde_json::from_value(fragment.clone())
                    .map_err(|error| RuntimePatchError::InvalidFragment(error.to_string()))?;
                graft_fragment_dependencies(&mut fragment, &inherited_dependencies);
                let inserted_node_refs = fragment.nodes.keys().cloned().collect::<Vec<_>>();

                RuntimeDriverBinder::bind_output(
                    runtime,
                    DriverBuildOutput {
                        fragment,
                        evidence_requirements: Vec::new(),
                        effect_contracts: Vec::new(),
                    },
                    &DriverBindingContext::default(),
                )
                .map_err(map_driver_binding_error)?;
                outcome.patched_node_refs.extend(inserted_node_refs);
            }
            PlanPatchOperation::DropBranch { node_ids } => {
                for node_id in node_ids {
                    let Some(node) = runtime.checkpoint.action_graph.nodes.get_mut(node_id) else {
                        return Err(RuntimePatchError::IllegalTarget {
                            target: "drop_branch".to_owned(),
                            reason: format!("node `{node_id}` not found"),
                        });
                    };
                    node.status = ActionNodeStatus::Skipped;
                }
            }
            PlanPatchOperation::TightenConstraints { constraints } => {
                runtime.mission.constraints.extend(constraints.clone());
                outcome.mission_constraints_updated = true;
            }
            PlanPatchOperation::ReplaceEffectContract {
                effect_ref,
                contract,
            } => {
                let parsed: EffectContract = serde_json::from_value(contract.clone())
                    .map_err(|error| RuntimePatchError::InvalidEffectContract(error.to_string()))?;
                if parsed.effect_id != *effect_ref {
                    return Err(RuntimePatchError::EffectContractRefMismatch {
                        expected: effect_ref.clone(),
                        actual: parsed.effect_id,
                    });
                }
                runtime
                    .checkpoint
                    .effect_contracts
                    .insert(effect_ref.clone(), parsed);
                outcome.updated_effect_refs.push(effect_ref.clone());
            }
        }
    }

    runtime.pending_signer_state = None;
    runtime
        .checkpoint
        .pending_requests
        .pending_evidence_refs
        .clear();
    runtime
        .checkpoint
        .pending_requests
        .pending_envelope_refs
        .clear();
    runtime
        .checkpoint
        .pending_requests
        .pending_signer_request_id = None;
    runtime.checkpoint.lifecycle.bump_plan_epoch();
    runtime
        .checkpoint
        .lifecycle
        .mark_running(RunPhase::Recovering);
    runtime.touch_transition();

    dedup(&mut outcome.patched_node_refs);
    dedup(&mut outcome.updated_effect_refs);

    Ok(outcome)
}

fn ensure_patchable_runtime(runtime: &ActiveRun) -> Result<(), RuntimePatchError> {
    use ais_agent_core::runtime::RunStatus;

    match runtime.checkpoint.lifecycle.status {
        RunStatus::Completed | RunStatus::Cancelled => Err(RuntimePatchError::TerminalRun),
        _ if runtime.checkpoint.lifecycle.active_boundary.is_none() => {
            Err(RuntimePatchError::NotAtStableBoundary)
        }
        _ => Ok(()),
    }
}

fn compute_scope(
    runtime: &ActiveRun,
    target: &PlanPatchTarget,
) -> Result<BTreeSet<String>, RuntimePatchError> {
    let graph = &runtime.checkpoint.action_graph;
    let scope = match target {
        PlanPatchTarget::ActiveFrontier => graph
            .nodes
            .iter()
            .filter(|(_, node)| {
                !matches!(
                    node.status,
                    ActionNodeStatus::Succeeded | ActionNodeStatus::Skipped
                )
            })
            .map(|(node_id, _)| node_id.clone())
            .collect::<BTreeSet<_>>(),
        PlanPatchTarget::NodeSet { node_ids } | PlanPatchTarget::FailedFragment { node_ids } => {
            descendants_for(graph, node_ids.iter().cloned())?
        }
        PlanPatchTarget::PendingVerifyBranch { effect_refs } => {
            let seeds = graph
                .nodes
                .iter()
                .filter(|(_, node)| {
                    matches!(node.kind, ActionNodeKind::Verify | ActionNodeKind::Recover)
                        && node
                            .expected_effect_ref
                            .as_ref()
                            .is_some_and(|effect_ref| effect_refs.contains(effect_ref))
                })
                .map(|(node_id, _)| node_id.clone())
                .collect::<Vec<_>>();
            descendants_for(graph, seeds.into_iter())?
        }
    };

    if scope.is_empty() {
        return Err(RuntimePatchError::EmptyScope);
    }

    Ok(scope)
}

fn validate_scope(
    runtime: &ActiveRun,
    target: &PlanPatchTarget,
    scope: &BTreeSet<String>,
) -> Result<(), RuntimePatchError> {
    for node_id in scope {
        let node = runtime
            .checkpoint
            .action_graph
            .nodes
            .get(node_id)
            .ok_or_else(|| RuntimePatchError::IllegalTarget {
                target: target_name(target),
                reason: format!("node `{node_id}` not found"),
            })?;

        if matches!(
            node.status,
            ActionNodeStatus::Succeeded | ActionNodeStatus::Running
        ) {
            return Err(RuntimePatchError::IllegalTarget {
                target: target_name(target),
                reason: format!("node `{node_id}` is already executed history"),
            });
        }

        if !matches!(target, PlanPatchTarget::PendingVerifyBranch { .. })
            && runtime
                .checkpoint
                .actuation_records
                .iter()
                .any(|record| record.node_id == *node_id)
        {
            return Err(RuntimePatchError::IllegalTarget {
                target: target_name(target),
                reason: format!(
                    "node `{node_id}` is in confirmed or broadcast side-effect ancestry"
                ),
            });
        }
    }

    Ok(())
}

fn descendants_for(
    graph: &ais_agent_core::action::ActionGraph,
    seeds: impl IntoIterator<Item = String>,
) -> Result<BTreeSet<String>, RuntimePatchError> {
    let mut scope = BTreeSet::new();
    let mut queue = VecDeque::new();

    for seed in seeds {
        if !graph.nodes.contains_key(&seed) {
            return Err(RuntimePatchError::IllegalTarget {
                target: "node_scope".to_owned(),
                reason: format!("node `{seed}` not found"),
            });
        }
        queue.push_back(seed);
    }

    while let Some(node_id) = queue.pop_front() {
        if !scope.insert(node_id.clone()) {
            continue;
        }
        for (candidate_id, candidate) in &graph.nodes {
            if candidate
                .depends_on
                .iter()
                .any(|dependency| dependency == &node_id)
            {
                queue.push_back(candidate_id.clone());
            }
        }
    }

    Ok(scope)
}

fn external_predecessors(runtime: &ActiveRun, scope: &BTreeSet<String>) -> Vec<String> {
    let mut predecessors = Vec::new();
    for node_id in scope {
        let Some(node) = runtime.checkpoint.action_graph.nodes.get(node_id) else {
            continue;
        };
        if node
            .depends_on
            .iter()
            .any(|dependency| scope.contains(dependency))
        {
            continue;
        }
        predecessors.extend(
            node.depends_on
                .iter()
                .filter(|dependency| !scope.contains(*dependency))
                .cloned(),
        );
    }
    dedup(&mut predecessors);
    predecessors
}

fn graft_fragment_dependencies(
    fragment: &mut ActionGraphFragment,
    inherited_dependencies: &[String],
) {
    for root_id in fragment.roots.clone() {
        if let Some(root) = fragment.nodes.get_mut(&root_id) {
            root.depends_on
                .extend(inherited_dependencies.iter().cloned());
            dedup(&mut root.depends_on);
        }
    }
}

fn dedup(values: &mut Vec<String>) {
    let mut seen = BTreeSet::new();
    values.retain(|value| seen.insert(value.clone()));
}

fn target_name(target: &PlanPatchTarget) -> String {
    match target {
        PlanPatchTarget::ActiveFrontier => "active_frontier".to_owned(),
        PlanPatchTarget::NodeSet { .. } => "node_set".to_owned(),
        PlanPatchTarget::FailedFragment { .. } => "failed_fragment".to_owned(),
        PlanPatchTarget::PendingVerifyBranch { .. } => "pending_verify_branch".to_owned(),
    }
}

fn map_driver_binding_error(error: RuntimeDriverBindingError) -> RuntimePatchError {
    RuntimePatchError::DriverBinding(error.to_string())
}
