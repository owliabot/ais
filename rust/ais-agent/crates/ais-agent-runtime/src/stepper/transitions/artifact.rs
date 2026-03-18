use ais_agent_control::{
    execution_artifact::{BranchTarget, ExecutionStage},
    recovery::{RunFailureCode, RunFailureStage},
};
use ais_agent_core::{
    action::ActionNodeStatus, checkpoint::ArtifactContinuationSnapshot, runtime::RunPhase,
};
use tracing::{info, warn};

use crate::{
    runtime::ActiveRun,
    runtime_branch::evaluate_predicate,
    runtime_exports::{export_observe_outputs, export_transaction_outputs},
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
            let available_targets = vec![
                describe_branch_target(&stage.if_true),
                describe_branch_target(&stage.if_false),
            ];
            let selected_target = describe_branch_target(target);
            match target {
                BranchTarget::GotoStage { stage_id } => {
                    if let Some(snapshot) = runtime.checkpoint.execution_artifact.as_mut() {
                        snapshot.branch_trace.push(
                            ais_agent_core::checkpoint::ArtifactBranchTraceSnapshot {
                                branch_stage_id: active_stage_id.clone(),
                                available_targets,
                                selected_target,
                                predicate_value: predicate,
                            },
                        );
                        snapshot.active_stage_id = Some(stage_id.clone());
                        snapshot.awaiting_continuation = None;
                    }
                    info!(
                        parent: None,
                        run_id = %runtime.run_id.0,
                        stage = %active_stage_id,
                        next_stage = %stage_id,
                        predicate = predicate,
                        "run.artifact.branch_selected"
                    );
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
                    if let Some(snapshot) = runtime.checkpoint.execution_artifact.as_mut() {
                        snapshot.branch_trace.push(
                            ais_agent_core::checkpoint::ArtifactBranchTraceSnapshot {
                                branch_stage_id: active_stage_id.clone(),
                                available_targets,
                                selected_target,
                                predicate_value: predicate,
                            },
                        );
                    }
                    runtime.checkpoint.lifecycle.pause_with_failure(
                        RunFailureStage::Verify,
                        RunFailureCode::VerifyMismatch,
                        format!("artifact assertion `{failure_code}` failed: {message}"),
                    );
                    warn!(
                        parent: None,
                        run_id = %runtime.run_id.0,
                        stage = %active_stage_id,
                        failure_code = %failure_code,
                        message = %message,
                        predicate_value = predicate,
                        "run.artifact.assert_failed"
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
            if !active_stage_graph_complete(runtime) {
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
                info!(
                    parent: None,
                    run_id = %runtime.run_id.0,
                    stage = %active_stage_id,
                    next_stage = %next_stage_id,
                    exported_output_count = exported.len(),
                    "run.artifact.stage_advanced"
                );
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
                info!(
                    parent: None,
                    run_id = %runtime.run_id.0,
                    stage = %active_stage_id,
                    exported_output_count = exported.len(),
                    "run.artifact.stage_completed"
                );
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
        ExecutionStage::Observe(stage) => {
            if !active_stage_graph_complete(runtime) {
                return None;
            }
            let exported = match export_observe_outputs(runtime, &stage) {
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
                info!(
                    parent: None,
                    run_id = %runtime.run_id.0,
                    stage = %active_stage_id,
                    next_stage = %next_stage_id,
                    exported_output_count = exported.len(),
                    "run.artifact.stage_advanced"
                );
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
                info!(
                    parent: None,
                    run_id = %runtime.run_id.0,
                    stage = %active_stage_id,
                    exported_output_count = exported.len(),
                    "run.artifact.stage_completed"
                );
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
            info!(
                run_id = %runtime.run_id.0,
                stage = %stage.stage_id,
                package_entry = %stage.package_entry,
                required_output_count = stage.required_outputs.len(),
                "run.artifact.awaiting_continuation"
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

fn describe_branch_target(target: &BranchTarget) -> String {
    match target {
        BranchTarget::GotoStage { stage_id } => stage_id.to_string(),
        BranchTarget::Assert { failure_code, .. } => format!("assert:{failure_code}"),
    }
}

fn active_stage_graph_complete(runtime: &ActiveRun) -> bool {
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
            ComparisonOperator, EvmTransactionCandidate, ExecutionArtifactLaunchSpec,
            ExecutionChainFamily, ExecutionStage, ExecutionTransactionCandidate, ObserveStage,
            OutputExportSpec, PredicateSpec, TransactionStage, ValueRef,
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

    use crate::{runtime::ActiveRun, tests::tracing_capture::capture_tracing_output_at_level};

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
                        observations: Vec::new(),
                        preconditions: Vec::new(),
                        postconditions: Vec::new(),
                        expected_effects: Vec::new(),
                        execution_policy: None,
                        risk_class: None,
                        risk_tags: Vec::new(),
                        decoded_intent: None,
                        candidate_envelopes: Vec::new(),
                        decode_spec: None,
                        validation_plan: None,
                        evidence: json!({}),
                        metadata: BTreeMap::new(),
                    },
                    active_stage_id: Some(stage.stage_id.clone()),
                    planned_stage_graphs: BTreeMap::new(),
                    exported_outputs: BTreeMap::new(),
                    branch_trace: Vec::new(),
                    awaiting_continuation: None,
                }),
            },
        );

        let (output, transition) = capture_tracing_output_at_level(tracing::Level::INFO, || {
            apply_execution_artifact_transition(&mut runtime).expect("artifact transition")
        });

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
        assert!(output.contains("run.artifact.stage_completed"));
        assert!(output.contains("stage=stage.swap"));
        assert!(output.contains("exported_output_count=1"));
    }

    #[test]
    fn observe_stage_exports_outputs_from_observation_evidence() {
        let stage = ObserveStage {
            stage_id: "stage.quote".into(),
            observation_ref: "query.quote".to_owned(),
            exports: vec![OutputExportSpec {
                output_key: "quote.amount_out_atomic".into(),
                source: ValueRef::Ref {
                    reference: "refs.evidence.query.quote.amount_out_atomic".to_owned(),
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
                    graph_id: Some("artifact.owliabot.uniswap_v3.quote".to_owned()),
                    roots: vec!["artifact.stage.quote.observe".to_owned()],
                    terminals: vec!["artifact.stage.quote.observe".to_owned()],
                    nodes: BTreeMap::from([(
                        "artifact.stage.quote.observe".to_owned(),
                        ActionNode {
                            node_id: "artifact.stage.quote.observe".to_owned(),
                            kind: ActionNodeKind::Observe,
                            origin: ActionOrigin::DriverFragment,
                            status: ActionNodeStatus::Succeeded,
                            depends_on: Vec::new(),
                            inputs: Vec::new(),
                            evidence_refs: Vec::new(),
                            payload: ActionPayload::Observe(ObserveAction {
                                source_kind: ObserveSourceKind::ChainRead,
                                source_hint: "query quote".to_owned(),
                                output_key: Some("query.quote".to_owned()),
                                live: None,
                            }),
                            implementation_hint: Some("execution_artifact".to_owned()),
                            expected_effect_ref: None,
                        },
                    )]),
                },
                evidence_graph: EvidenceGraph::default(),
                effect_contracts: BTreeMap::new(),
                pending_requests: PendingRequestsSnapshot::default(),
                last_completed_node_id: None,
                actuation_records: Vec::new(),
                execution_artifact: None,
            },
        );
        runtime.checkpoint.evidence_graph.records.insert(
            "query.quote".to_owned(),
            EvidenceRecord {
                evidence_id: "query.quote".to_owned(),
                kind: EvidenceKind::ExternalObservation,
                provenance: EvidenceProvenance {
                    source: "test".to_owned(),
                    chain_scope: Some("eip155:1".to_owned()),
                    trace_hint: None,
                },
                freshness: EvidenceFreshness {
                    observed_at_ms: Some(1),
                    expires_at_ms: None,
                    max_age_ms: None,
                },
                confidence_ppm: Some(1_000_000),
                payload: json!({
                    "amount_out_atomic": "1000"
                }),
            },
        );
        runtime.checkpoint.execution_artifact = Some(ExecutionArtifactRuntimeSnapshot {
            launch_spec: ExecutionArtifactLaunchSpec {
                protocol_package_id: "owliabot.uniswap_v3".to_owned(),
                action_key: "quote_exact_in_single".to_owned(),
                chain_family: ExecutionChainFamily::Evm,
                allowed_chains: vec!["eip155:1".to_owned()],
                entry_stage_id: "stage.quote".into(),
                actor: None,
                transactions: Vec::new(),
                stages: vec![ExecutionStage::Observe(stage.clone())],
                observations: vec![ais_agent_control::execution_artifact::ObservationSpec {
                    observation_id: "query.quote".to_owned(),
                    kind: "evm.contract_state_read".to_owned(),
                    params: BTreeMap::new(),
                }],
                preconditions: Vec::new(),
                postconditions: Vec::new(),
                expected_effects: Vec::new(),
                execution_policy: None,
                risk_class: None,
                risk_tags: Vec::new(),
                decoded_intent: None,
                candidate_envelopes: Vec::new(),
                decode_spec: None,
                validation_plan: None,
                evidence: serde_json::Value::Null,
                metadata: BTreeMap::new(),
            },
            active_stage_id: Some(stage.stage_id.clone()),
            planned_stage_graphs: BTreeMap::new(),
            exported_outputs: BTreeMap::new(),
            branch_trace: Vec::new(),
            awaiting_continuation: None,
        });

        let (output, transition) = capture_tracing_output_at_level(tracing::Level::INFO, || {
            apply_execution_artifact_transition(&mut runtime).expect("artifact transition")
        });

        assert!(transition.summary.contains("exported 1 output(s)"));
        assert_eq!(
            runtime
                .checkpoint
                .execution_artifact
                .as_ref()
                .expect("artifact state")
                .exported_outputs
                .get(&"quote.amount_out_atomic".into())
                .expect("exported amount"),
            &json!("1000")
        );
        assert!(runtime
            .checkpoint
            .execution_artifact
            .as_ref()
            .expect("artifact state")
            .active_stage_id
            .is_none());
        assert!(output.contains("run.artifact.stage_completed"));
        assert!(output.contains("stage=stage.quote"));
        assert!(output.contains("exported_output_count=1"));
    }

    #[test]
    fn branch_stage_logs_selected_target() {
        let branch_stage = ais_agent_control::execution_artifact::BranchStage {
            stage_id: "stage.branch".into(),
            predicate: PredicateSpec::Comparison {
                left: ValueRef::Literal { value: json!(1) },
                op: ComparisonOperator::Eq,
                right: ValueRef::Literal { value: json!(1) },
            },
            if_true: ais_agent_control::execution_artifact::BranchTarget::GotoStage {
                stage_id: "stage.next".into(),
            },
            if_false: ais_agent_control::execution_artifact::BranchTarget::Assert {
                failure_code: "unexpected_false".to_owned(),
                message: "predicate was false".to_owned(),
            },
        };
        let next_stage = ObserveStage {
            stage_id: "stage.next".into(),
            observation_ref: "query.quote".to_owned(),
            exports: Vec::new(),
            next_stage_id: None,
        };
        let mut runtime = ActiveRun::new(
            sample_mission(),
            CheckpointSnapshot {
                run_id: "run-branch".to_owned(),
                mission_id: "mission-1".to_owned(),
                checkpoint_seq: 0,
                plan_epoch: 0,
                lifecycle: RunLifecycleState::new(RunId("run-branch".to_owned()), "mission-1"),
                action_graph: ActionGraph::default(),
                evidence_graph: EvidenceGraph::default(),
                effect_contracts: BTreeMap::new(),
                pending_requests: PendingRequestsSnapshot::default(),
                last_completed_node_id: None,
                actuation_records: Vec::new(),
                execution_artifact: Some(ExecutionArtifactRuntimeSnapshot {
                    launch_spec: ExecutionArtifactLaunchSpec {
                        protocol_package_id: "owliabot.uniswap_v3".to_owned(),
                        action_key: "quote_exact_in_single".to_owned(),
                        chain_family: ExecutionChainFamily::Evm,
                        allowed_chains: vec!["eip155:1".to_owned()],
                        entry_stage_id: "stage.branch".into(),
                        actor: None,
                        transactions: Vec::new(),
                        stages: vec![
                            ExecutionStage::Branch(branch_stage),
                            ExecutionStage::Observe(next_stage),
                        ],
                        observations: vec![
                            ais_agent_control::execution_artifact::ObservationSpec {
                                observation_id: "query.quote".to_owned(),
                                kind: "evm.contract_state_read".to_owned(),
                                params: BTreeMap::new(),
                            },
                        ],
                        preconditions: Vec::new(),
                        postconditions: Vec::new(),
                        expected_effects: Vec::new(),
                        execution_policy: None,
                        risk_class: None,
                        risk_tags: Vec::new(),
                        decoded_intent: None,
                        candidate_envelopes: Vec::new(),
                        decode_spec: None,
                        validation_plan: None,
                        evidence: serde_json::Value::Null,
                        metadata: BTreeMap::new(),
                    },
                    active_stage_id: Some("stage.branch".into()),
                    planned_stage_graphs: BTreeMap::new(),
                    exported_outputs: BTreeMap::new(),
                    branch_trace: Vec::new(),
                    awaiting_continuation: None,
                }),
            },
        );

        let (output, transition) = capture_tracing_output_at_level(tracing::Level::INFO, || {
            apply_execution_artifact_transition(&mut runtime).expect("artifact transition")
        });

        assert!(transition.summary.contains("stage `stage.branch` failed"));
        assert!(output.contains("run.artifact.branch_selected"));
        assert!(output.contains("stage=stage.branch"));
        assert!(output.contains("next_stage=stage.next"));
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
