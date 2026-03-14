use ais_agent_control::{
    execution_artifact::{BranchTarget, ExecutionStage},
    recovery::{RunFailureCode, RunFailureStage},
};
use ais_agent_core::{
    action::ActionNodeStatus, checkpoint::ArtifactContinuationSnapshot, runtime::RunPhase,
};

use crate::{
    runtime::ActiveRun,
    runtime_branch::evaluate_predicate,
    runtime_exports::export_transaction_outputs,
    service::host_service::artifact_planner::activate_execution_artifact_stage,
    stepper::{StepTransition, StepTransitionKind},
};

pub(crate) fn apply_execution_artifact_transition(
    runtime: &mut ActiveRun,
) -> Option<StepTransition> {
    let snapshot = runtime.checkpoint.execution_artifact.clone()?;
    let active_stage_id = snapshot.active_stage_id?;
    let stage = snapshot
        .launch_spec
        .stage(active_stage_id.as_str())?
        .clone();

    match stage {
        ExecutionStage::Branch(stage) => {
            let predicate = match evaluate_predicate(runtime, &stage.predicate) {
                Ok(value) => value,
                Err(error) => {
                    return fail_artifact(runtime, active_stage_id.as_str(), error);
                }
            };
            let target = if predicate {
                &stage.if_true
            } else {
                &stage.if_false
            };
            match target {
                BranchTarget::GotoStage { stage_id } => {
                    if let Some(snapshot) = runtime.checkpoint.execution_artifact.as_mut() {
                        snapshot.active_stage_id = Some(stage_id.clone());
                        snapshot.awaiting_continuation = None;
                    }
                    if let Err(error) = activate_execution_artifact_stage(&mut runtime.checkpoint) {
                        return fail_artifact(runtime, active_stage_id.as_str(), error);
                    }
                    runtime
                        .checkpoint
                        .lifecycle
                        .mark_running(RunPhase::Planning);
                    runtime.touch_transition();
                    Some(StepTransition {
                        kind: StepTransitionKind::Artifact,
                        node_id: None,
                        summary: format!(
                            "execution artifact branch `{}` selected stage `{}`",
                            active_stage_id, stage_id
                        ),
                    })
                }
                BranchTarget::Assert {
                    failure_code,
                    message,
                } => {
                    runtime.checkpoint.lifecycle.pause_with_failure(
                        RunFailureStage::Verify,
                        RunFailureCode::VerifyMismatch,
                        format!("artifact assertion `{failure_code}` failed: {message}"),
                    );
                    runtime.touch_transition();
                    Some(StepTransition {
                        kind: StepTransitionKind::Artifact,
                        node_id: None,
                        summary: format!(
                            "execution artifact assertion `{failure_code}` failed at stage `{active_stage_id}`"
                        ),
                    })
                }
            }
        }
        ExecutionStage::Transaction(stage) => {
            if !transaction_stage_complete(runtime) {
                return None;
            }
            let exported = match export_transaction_outputs(runtime, &stage) {
                Ok(exported) => exported,
                Err(error) => {
                    return fail_artifact(runtime, active_stage_id.as_str(), error);
                }
            };
            if let Some(snapshot) = runtime.checkpoint.execution_artifact.as_mut() {
                snapshot.active_stage_id = stage.next_stage_id.clone();
                snapshot.awaiting_continuation = None;
            }
            if let Some(next_stage_id) = stage.next_stage_id.as_ref() {
                if let Err(error) = activate_execution_artifact_stage(&mut runtime.checkpoint) {
                    return fail_artifact(runtime, active_stage_id.as_str(), error);
                }
                runtime
                    .checkpoint
                    .lifecycle
                    .mark_running(RunPhase::Planning);
                runtime.touch_transition();
                Some(StepTransition {
                    kind: StepTransitionKind::Artifact,
                    node_id: None,
                    summary: if exported.is_empty() {
                        format!(
                            "execution artifact advanced from `{}` to `{}`",
                            active_stage_id, next_stage_id
                        )
                    } else {
                        format!(
                            "execution artifact exported {} output(s) and advanced to `{}`",
                            exported.len(),
                            next_stage_id
                        )
                    },
                })
            } else {
                runtime.touch_transition();
                Some(StepTransition {
                    kind: StepTransitionKind::Artifact,
                    node_id: None,
                    summary: if exported.is_empty() {
                        format!("execution artifact stage `{active_stage_id}` completed")
                    } else {
                        format!(
                            "execution artifact exported {} output(s) from `{}`",
                            exported.len(),
                            active_stage_id
                        )
                    },
                })
            }
        }
        ExecutionStage::Continuation(stage) => {
            let blocking_refs = stage
                .required_outputs
                .iter()
                .map(ToString::to_string)
                .collect();
            if let Some(snapshot) = runtime.checkpoint.execution_artifact.as_mut() {
                snapshot.awaiting_continuation = Some(ArtifactContinuationSnapshot {
                    stage_id: stage.stage_id.clone(),
                    required_outputs: stage.required_outputs.clone(),
                    package_entry: stage.package_entry.clone(),
                    next_stage_id: stage.next_stage_id.clone(),
                });
            }
            runtime.checkpoint.lifecycle.await_artifact_continuation(
                format!(
                    "awaiting artifact continuation `{}` for stage `{}`",
                    stage.package_entry, stage.stage_id
                ),
                blocking_refs,
            );
            runtime.touch_transition();
            Some(StepTransition {
                kind: StepTransitionKind::Artifact,
                node_id: None,
                summary: format!(
                    "execution artifact paused for continuation `{}`",
                    stage.package_entry
                ),
            })
        }
    }
}

fn transaction_stage_complete(runtime: &ActiveRun) -> bool {
    !runtime.checkpoint.action_graph.terminals.is_empty()
        && runtime
            .checkpoint
            .action_graph
            .terminals
            .iter()
            .all(|node_id| {
                runtime
                    .checkpoint
                    .action_graph
                    .nodes
                    .get(node_id)
                    .is_some_and(|node| {
                        matches!(
                            node.status,
                            ActionNodeStatus::Succeeded | ActionNodeStatus::Skipped
                        )
                    })
            })
}

fn fail_artifact(
    runtime: &mut ActiveRun,
    stage_id: &str,
    reason: impl Into<String>,
) -> Option<StepTransition> {
    let reason = reason.into();
    runtime.checkpoint.lifecycle.fail(
        RunFailureStage::Recover,
        RunFailureCode::RuntimeInvariantViolation,
        format!("execution artifact stage `{stage_id}` failed: {reason}"),
    );
    runtime.touch_transition();
    Some(StepTransition {
        kind: StepTransitionKind::Artifact,
        node_id: None,
        summary: format!("execution artifact stage `{stage_id}` failed: {reason}"),
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use ais_agent_control::{
        execution_artifact::{
            EvmTransactionCandidate, ExecutionArtifactLaunchSpec, ExecutionChainFamily,
            ExecutionStage, ExecutionTransactionCandidate, OutputExportSpec, TransactionStage,
            ValueRef,
        },
        ids::RunId,
    };
    use ais_agent_core::{
        action::{
            kinds::observe::{ObserveAction, ObserveSourceKind},
            ActionGraph, ActionNode, ActionNodeKind, ActionNodeStatus, ActionOrigin, ActionPayload,
        },
        checkpoint::{
            CheckpointSnapshot, ExecutionArtifactRuntimeSnapshot, PendingRequestsSnapshot,
        },
        evidence::{
            EvidenceFreshness, EvidenceGraph, EvidenceKind, EvidenceProvenance, EvidenceRecord,
        },
        mission::{Mission, MissionBudget, MissionPolicy},
        runtime::RunLifecycleState,
    };
    use serde_json::json;

    use crate::runtime::ActiveRun;

    use super::apply_execution_artifact_transition;

    #[test]
    fn transaction_stage_exports_outputs_from_observation_evidence() {
        let post_observe_id = "artifact.stage.swap.post_observe.state.post.received_balance";
        let stage = TransactionStage {
            stage_id: "stage.swap".into(),
            candidate_ref: "swap.call".into(),
            exports: vec![OutputExportSpec {
                output_key: "swap.received_atomic".into(),
                source: ValueRef::Ref {
                    reference: "refs.evidence.state.post.received_balance.balance".to_owned(),
                },
            }],
            next_stage_id: None,
        };
        let mut runtime = ActiveRun::new(
            sample_mission(),
            CheckpointSnapshot {
                run_id: "run-1".to_owned(),
                mission_id: "mission-1".to_owned(),
                checkpoint_seq: 0,
                plan_epoch: 0,
                lifecycle: RunLifecycleState::new(RunId("run-1".to_owned()), "mission-1"),
                action_graph: ActionGraph {
                    graph_id: Some("artifact.owliabot.uniswap_v3.swap".to_owned()),
                    roots: vec![post_observe_id.to_owned()],
                    terminals: vec![post_observe_id.to_owned()],
                    nodes: BTreeMap::from([(
                        post_observe_id.to_owned(),
                        ActionNode {
                            node_id: post_observe_id.to_owned(),
                            kind: ActionNodeKind::Observe,
                            origin: ActionOrigin::DriverFragment,
                            status: ActionNodeStatus::Succeeded,
                            depends_on: vec!["artifact.stage.swap.verify".to_owned()],
                            inputs: Vec::new(),
                            evidence_refs: Vec::new(),
                            payload: ActionPayload::Observe(ObserveAction {
                                source_kind: ObserveSourceKind::ChainRead,
                                source_hint: "observe post balance".to_owned(),
                                output_key: Some("state.post.received_balance".to_owned()),
                                live: None,
                            }),
                            implementation_hint: Some("execution_artifact".to_owned()),
                            expected_effect_ref: None,
                        },
                    )]),
                },
                evidence_graph: EvidenceGraph {
                    records: BTreeMap::from([(
                        "state.post.received_balance".to_owned(),
                        EvidenceRecord {
                            evidence_id: "state.post.received_balance".to_owned(),
                            kind: EvidenceKind::ExternalObservation,
                            provenance: EvidenceProvenance {
                                source: "evm_rpc".to_owned(),
                                chain_scope: Some("eip155:8453".to_owned()),
                                trace_hint: Some(post_observe_id.to_owned()),
                            },
                            freshness: EvidenceFreshness {
                                observed_at_ms: Some(10),
                                expires_at_ms: None,
                                max_age_ms: None,
                            },
                            confidence_ppm: Some(1_000_000),
                            payload: json!({ "balance": "123" }),
                        },
                    )]),
                    requirements: Vec::new(),
                    usages: Vec::new(),
                },
                effect_contracts: BTreeMap::new(),
                pending_requests: PendingRequestsSnapshot::default(),
                last_completed_node_id: Some(post_observe_id.to_owned()),
                actuation_records: Vec::new(),
                execution_artifact: Some(ExecutionArtifactRuntimeSnapshot {
                    launch_spec: ExecutionArtifactLaunchSpec {
                        protocol_package_id: "owliabot.uniswap_v3".to_owned(),
                        action_key: "swap".to_owned(),
                        chain_family: ExecutionChainFamily::Evm,
                        allowed_chains: vec!["8453".to_owned()],
                        entry_stage_id: stage.stage_id.clone(),
                        actor: None,
                        transactions: vec![ExecutionTransactionCandidate::EvmTransaction(
                            EvmTransactionCandidate {
                                candidate_id: "swap.call".into(),
                                to: "0x1111111111111111111111111111111111111111".to_owned(),
                                value: Some("0".to_owned()),
                                calldata: Some("0xdeadbeef".to_owned()),
                            },
                        )],
                        stages: vec![ExecutionStage::Transaction(stage.clone())],
                        preconditions: Vec::new(),
                        postconditions: Vec::new(),
                        expected_effects: Vec::new(),
                        execution_policy: None,
                        evidence: json!({}),
                        metadata: BTreeMap::new(),
                    },
                    active_stage_id: Some(stage.stage_id.clone()),
                    planned_stage_graphs: BTreeMap::new(),
                    exported_outputs: BTreeMap::new(),
                    awaiting_continuation: None,
                }),
            },
        );

        let transition =
            apply_execution_artifact_transition(&mut runtime).expect("artifact transition");

        assert_eq!(
            transition.summary,
            "execution artifact exported 1 output(s) from `stage.swap`"
        );
        assert_eq!(
            runtime
                .checkpoint
                .execution_artifact
                .as_ref()
                .expect("artifact snapshot")
                .exported_outputs
                .get(&"swap.received_atomic".into()),
            Some(&json!("123"))
        );
        assert!(runtime
            .checkpoint
            .execution_artifact
            .as_ref()
            .expect("artifact snapshot")
            .active_stage_id
            .is_none());
    }

    fn sample_mission() -> Mission {
        Mission {
            mission_id: "mission-1".to_owned(),
            goal: "artifact".to_owned(),
            allowed_chains: vec!["8453".to_owned()],
            budget: MissionBudget::default(),
            policy: MissionPolicy::default(),
            constraints: BTreeMap::new(),
            metadata: BTreeMap::new(),
        }
    }
}
