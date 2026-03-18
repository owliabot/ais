use std::collections::BTreeMap;

use ais_agent_control::{
    execution_artifact::{
        BranchStage, BranchTarget, EvmTransactionCandidate, ExecutionArtifactLaunchSpec,
        ExecutionChainFamily, ExecutionStage, ExecutionTransactionCandidate, OutputExportSpec,
        PredicateSpec, TransactionStage, ValueRef,
    },
    ids::{RunId, SignerRequestId},
    recovery::RunFailureCode,
};
use ais_agent_core::{
    action::{
        kinds::{
            actuate::{ActuateAction, ActuateLiveBinding, ActuateMode, EvmActuateLiveBinding},
            derive::{DeriveAction, DeriveKind},
            observe::{ObserveAction, ObserveSourceKind},
            recover::{RecoverAction, RecoverKind},
            simulate::{EvmSimulateLiveBinding, SimulateAction, SimulateKind, SimulateLiveBinding},
            verify::{VerifyAction, VerifyKind},
        },
        ActionGraph, ActionNode, ActionNodeKind, ActionNodeStatus, ActionOrigin, ActionPayload,
    },
    binding::evm::{EvmActuateBinding, EvmCallRequest, EvmSimulateBinding},
    checkpoint::{CheckpointSnapshot, ExecutionArtifactRuntimeSnapshot, PendingRequestsSnapshot},
    evidence::{
        EvidenceFreshness, EvidenceGraph, EvidenceKind, EvidenceProvenance, EvidenceRecord,
        EvidenceRequirement,
    },
    mission::{Mission, MissionBudget, MissionPolicy},
    runtime::{RunLifecycleState, RunPhase, RunStatus, SignerRequestState, SignerRequestStatus},
};
use serde_json::json;

use crate::{
    runtime::ActiveRun,
    stepper::{StepOnce, StepTransitionKind},
};
use alloy::primitives::Address;

#[tokio::test]
async fn step_once_applies_only_one_local_transition() {
    let mission = sample_mission();
    let checkpoint = checkpoint_with_nodes(vec![
        derive_node("derive-amount", vec![]),
        simulate_node("simulate-swap", vec![]),
    ]);
    let mut runtime = ActiveRun::new(mission, checkpoint);

    let first = StepOnce::apply(&mut runtime).await;

    assert_eq!(
        first.applied_transition.as_ref().map(|t| t.kind),
        Some(StepTransitionKind::Derive)
    );
    assert_eq!(
        runtime
            .checkpoint
            .action_graph
            .nodes
            .get("derive-amount")
            .map(|node| node.status.clone()),
        Some(ActionNodeStatus::Succeeded)
    );
    assert_eq!(
        runtime
            .checkpoint
            .action_graph
            .nodes
            .get("simulate-swap")
            .map(|node| node.status.clone()),
        Some(ActionNodeStatus::Pending)
    );
}

#[tokio::test]
async fn step_once_ingests_evidence_and_clears_awaiting_evidence_when_requirements_are_satisfied() {
    let mission = sample_mission();
    let mut checkpoint = checkpoint_with_nodes(vec![actuate_node("swap", vec!["simulate-swap"])]);
    checkpoint
        .evidence_graph
        .records
        .insert("quote".to_owned(), sample_evidence("quote"));
    checkpoint
        .evidence_graph
        .requirements
        .push(EvidenceRequirement {
            requirement_id: "req-1".to_owned(),
            reference: "evidence.quote".to_owned(),
            reason: "need fresh quote".to_owned(),
            required_by_node_id: Some("swap".to_owned()),
            satisfied_by_evidence_id: None,
        });
    checkpoint.pending_requests.pending_evidence_refs = vec!["evidence.quote".to_owned()];
    checkpoint
        .lifecycle
        .await_evidence("need quote", vec!["evidence.quote".to_owned()]);

    let mut runtime = ActiveRun::new(mission, checkpoint);
    let result = StepOnce::apply(&mut runtime).await;

    assert_eq!(
        result.applied_transition.as_ref().map(|t| t.kind),
        Some(StepTransitionKind::Ingest)
    );
    assert!(runtime
        .checkpoint
        .pending_requests
        .pending_evidence_refs
        .is_empty());
    assert_eq!(runtime.checkpoint.lifecycle.status, RunStatus::Running);
    assert_eq!(runtime.checkpoint.lifecycle.phase, RunPhase::Planning);
    assert_eq!(
        runtime.checkpoint.evidence_graph.requirements[0]
            .satisfied_by_evidence_id
            .as_deref(),
        Some("quote")
    );
}

#[tokio::test]
async fn step_once_enters_signer_boundary_after_governor_allows_write_with_signer() {
    let mission = sample_mission();
    let checkpoint = checkpoint_with_nodes(vec![
        simulate_succeeded_node("simulate-swap"),
        actuate_node("swap", vec!["simulate-swap"]),
    ]);
    let mut runtime = ActiveRun::new(mission, checkpoint);

    let result = StepOnce::apply(&mut runtime).await;

    assert_eq!(
        result.applied_transition.as_ref().map(|t| t.kind),
        Some(StepTransitionKind::Govern)
    );
    assert_eq!(
        runtime.checkpoint.lifecycle.status,
        RunStatus::AwaitingSigner
    );
    assert_eq!(
        runtime
            .checkpoint
            .pending_requests
            .pending_signer_request_id
            .is_some(),
        true
    );
    assert_eq!(
        runtime
            .pending_signer_state
            .as_ref()
            .map(|state| state.status.clone()),
        Some(SignerRequestStatus::Pending)
    );
    assert_eq!(
        runtime
            .checkpoint
            .action_graph
            .nodes
            .get("swap")
            .map(|node| node.status.clone()),
        Some(ActionNodeStatus::Blocked)
    );
}

#[tokio::test]
async fn step_once_governor_requires_missing_evidence_with_recovery_context() {
    let mission = sample_mission();
    let mut checkpoint = checkpoint_with_nodes(vec![
        simulate_succeeded_node("simulate-swap"),
        actuate_node("swap", vec!["simulate-swap"]),
    ]);
    checkpoint
        .action_graph
        .nodes
        .get_mut("swap")
        .unwrap()
        .evidence_refs = vec!["evidence.quote".to_owned()];
    checkpoint
        .evidence_graph
        .requirements
        .push(EvidenceRequirement {
            requirement_id: "req-quote".to_owned(),
            reference: "evidence.quote".to_owned(),
            reason: "need fresh quote".to_owned(),
            required_by_node_id: Some("swap".to_owned()),
            satisfied_by_evidence_id: None,
        });
    let mut runtime = ActiveRun::new(mission, checkpoint);

    let result = StepOnce::apply(&mut runtime).await;

    assert_eq!(
        result.applied_transition.as_ref().map(|t| t.kind),
        Some(StepTransitionKind::Govern)
    );
    assert_eq!(
        runtime.checkpoint.lifecycle.status,
        RunStatus::AwaitingEvidence
    );
    assert_eq!(
        runtime
            .checkpoint
            .lifecycle
            .failure
            .as_ref()
            .map(|failure| &failure.code),
        Some(&RunFailureCode::MissingEvidence)
    );
    assert_eq!(
        runtime.checkpoint.pending_requests.pending_evidence_refs,
        vec!["evidence.quote".to_owned()]
    );
}

#[tokio::test]
async fn step_once_governor_requires_stale_evidence_with_recovery_context() {
    let mission = sample_mission();
    let mut checkpoint = checkpoint_with_nodes(vec![
        simulate_succeeded_node("simulate-swap"),
        actuate_node("swap", vec!["simulate-swap"]),
    ]);
    checkpoint
        .action_graph
        .nodes
        .get_mut("swap")
        .unwrap()
        .evidence_refs = vec!["evidence.quote".to_owned()];
    checkpoint
        .evidence_graph
        .requirements
        .push(EvidenceRequirement {
            requirement_id: "req-quote".to_owned(),
            reference: "evidence.quote".to_owned(),
            reason: "need fresh quote".to_owned(),
            required_by_node_id: Some("swap".to_owned()),
            satisfied_by_evidence_id: Some("quote".to_owned()),
        });
    checkpoint
        .evidence_graph
        .records
        .insert("quote".to_owned(), stale_evidence("quote"));
    let mut runtime = ActiveRun::new(mission, checkpoint);

    let result = StepOnce::apply(&mut runtime).await;

    assert_eq!(
        result.applied_transition.as_ref().map(|t| t.kind),
        Some(StepTransitionKind::Govern)
    );
    assert_eq!(
        runtime.checkpoint.lifecycle.status,
        RunStatus::AwaitingEvidence
    );
    assert_eq!(
        runtime
            .checkpoint
            .lifecycle
            .failure
            .as_ref()
            .map(|failure| &failure.code),
        Some(&RunFailureCode::StaleEvidence)
    );
}

#[tokio::test]
async fn step_once_simulation_rejection_pauses_for_patch() {
    let mission = sample_mission();
    let checkpoint = checkpoint_with_nodes(vec![simulate_live_node_without_connection(
        "simulate-swap",
        vec![],
    )]);
    let mut runtime = ActiveRun::new(mission, checkpoint);

    let result = StepOnce::apply(&mut runtime).await;

    assert_eq!(
        result.applied_transition.as_ref().map(|t| t.kind),
        Some(StepTransitionKind::Simulate)
    );
    assert_eq!(runtime.checkpoint.lifecycle.status, RunStatus::Paused);
    assert_eq!(
        runtime
            .checkpoint
            .lifecycle
            .failure
            .as_ref()
            .map(|failure| &failure.code),
        Some(&RunFailureCode::SimulationRejected)
    );
    assert_eq!(
        runtime
            .checkpoint
            .action_graph
            .nodes
            .get("simulate-swap")
            .map(|node| node.status.clone()),
        Some(ActionNodeStatus::Failed)
    );
}

#[tokio::test]
async fn step_once_resolves_submitted_signer_state_into_verifying_progress() {
    let mission = sample_mission();
    let checkpoint =
        checkpoint_with_nodes(vec![actuate_blocked_node("swap", vec!["simulate-swap"])]);
    let mut runtime = ActiveRun::new(mission, checkpoint);
    runtime.pending_signer_state = Some(
        SignerRequestState::new_pending(
            SignerRequestId("signer-1".to_owned()),
            RunId("run-1".to_owned()),
            "eip155:1",
            "sign swap",
        )
        .with_node_id("swap")
        .with_timeout(10, Some(20)),
    );
    runtime
        .pending_signer_state
        .as_mut()
        .expect("signer state")
        .status = SignerRequestStatus::Submitted;
    runtime
        .pending_signer_state
        .as_mut()
        .expect("signer state")
        .submitted_tx_hash = Some("0xabc".to_owned());
    runtime
        .checkpoint
        .pending_requests
        .pending_signer_request_id = Some("signer-1".to_owned());
    runtime
        .checkpoint
        .lifecycle
        .await_signer("await signer", SignerRequestId("signer-1".to_owned()));

    let result = StepOnce::apply(&mut runtime).await;

    assert_eq!(
        result.applied_transition.as_ref().map(|t| t.kind),
        Some(StepTransitionKind::Signer)
    );
    assert!(runtime.pending_signer_state.is_none());
    assert!(runtime
        .checkpoint
        .pending_requests
        .pending_signer_request_id
        .is_none());
    assert_eq!(
        runtime.checkpoint.lifecycle.status,
        RunStatus::AwaitingConfirmation
    );
    assert_eq!(runtime.checkpoint.lifecycle.phase, RunPhase::AwaitingHost);
    assert_eq!(
        runtime
            .checkpoint
            .pending_requests
            .pending_confirmation_id
            .as_deref(),
        Some("0xabc")
    );
    assert_eq!(
        runtime
            .checkpoint
            .action_graph
            .nodes
            .get("swap")
            .map(|node| node.status.clone()),
        Some(ActionNodeStatus::Succeeded)
    );
    assert_eq!(runtime.checkpoint.actuation_records.len(), 1);
}

#[tokio::test]
async fn step_once_resolves_signed_signer_state_into_broadcasting_progress() {
    let mission = sample_mission();
    let checkpoint =
        checkpoint_with_nodes(vec![actuate_blocked_node("swap", vec!["simulate-swap"])]);
    let mut runtime = ActiveRun::new(mission, checkpoint);
    runtime.pending_signer_state = Some(
        SignerRequestState::new_pending(
            SignerRequestId("signer-1".to_owned()),
            RunId("run-1".to_owned()),
            "eip155:1",
            "sign swap",
        )
        .with_node_id("swap")
        .with_timeout(10, Some(20)),
    );
    runtime
        .pending_signer_state
        .as_mut()
        .expect("signer state")
        .status = SignerRequestStatus::Signed;
    runtime
        .pending_signer_state
        .as_mut()
        .expect("signer state")
        .signed_payload = Some(json!({
        "kind": "evm_signed_transaction",
        "raw_tx": "0x0102"
    }));
    runtime
        .checkpoint
        .pending_requests
        .pending_signer_request_id = Some("signer-1".to_owned());
    runtime
        .checkpoint
        .lifecycle
        .await_signer("await signer", SignerRequestId("signer-1".to_owned()));

    let result = StepOnce::apply(&mut runtime).await;

    assert_eq!(
        result.applied_transition.as_ref().map(|t| t.kind),
        Some(StepTransitionKind::Signer)
    );
    assert_eq!(
        runtime
            .pending_signer_state
            .as_ref()
            .map(|state| state.status.clone()),
        Some(SignerRequestStatus::Signed)
    );
    assert_eq!(
        runtime
            .checkpoint
            .pending_requests
            .pending_signer_request_id
            .as_deref(),
        Some("signer-1")
    );
    assert_eq!(runtime.checkpoint.lifecycle.status, RunStatus::Running);
    assert_eq!(runtime.checkpoint.lifecycle.phase, RunPhase::Broadcasting);
    assert_eq!(
        runtime
            .checkpoint
            .action_graph
            .nodes
            .get("swap")
            .map(|node| node.status.clone()),
        Some(ActionNodeStatus::Ready)
    );
}

#[tokio::test]
async fn step_once_signer_denial_pauses_for_patch() {
    let mission = sample_mission();
    let checkpoint =
        checkpoint_with_nodes(vec![actuate_blocked_node("swap", vec!["simulate-swap"])]);
    let mut runtime = ActiveRun::new(mission, checkpoint);
    runtime.pending_signer_state = Some(
        SignerRequestState::new_pending(
            SignerRequestId("signer-1".to_owned()),
            RunId("run-1".to_owned()),
            "eip155:1",
            "sign swap",
        )
        .with_node_id("swap")
        .with_timeout(10, Some(20)),
    );
    runtime
        .pending_signer_state
        .as_mut()
        .expect("signer state")
        .status = SignerRequestStatus::Denied;
    runtime
        .checkpoint
        .pending_requests
        .pending_signer_request_id = Some("signer-1".to_owned());
    runtime
        .checkpoint
        .lifecycle
        .await_signer("await signer", SignerRequestId("signer-1".to_owned()));

    let result = StepOnce::apply(&mut runtime).await;

    assert_eq!(
        result.applied_transition.as_ref().map(|t| t.kind),
        Some(StepTransitionKind::Signer)
    );
    assert_eq!(runtime.checkpoint.lifecycle.status, RunStatus::Paused);
    assert_eq!(
        runtime
            .checkpoint
            .lifecycle
            .failure
            .as_ref()
            .map(|failure| &failure.code),
        Some(&RunFailureCode::SignerDenied)
    );
    assert_eq!(
        runtime
            .checkpoint
            .action_graph
            .nodes
            .get("swap")
            .map(|node| node.status.clone()),
        Some(ActionNodeStatus::Failed)
    );
}

#[tokio::test]
async fn step_once_envelope_invalid_pauses_for_replacement_envelope() {
    let mission = sample_mission();
    let checkpoint = checkpoint_with_nodes(vec![actuate_evm_broadcast_node("swap", vec![])]);
    let mut runtime = ActiveRun::new(mission, checkpoint);

    let result = StepOnce::apply(&mut runtime).await;

    assert_eq!(
        result.applied_transition.as_ref().map(|t| t.kind),
        Some(StepTransitionKind::Broadcast)
    );
    assert_eq!(runtime.checkpoint.lifecycle.status, RunStatus::Paused);
    assert_eq!(
        runtime
            .checkpoint
            .lifecycle
            .failure
            .as_ref()
            .map(|failure| &failure.code),
        Some(&RunFailureCode::EnvelopeInvalid)
    );
    assert_eq!(
        runtime.checkpoint.pending_requests.pending_envelope_refs,
        vec!["env.swap".to_owned()]
    );
    assert_eq!(
        runtime
            .checkpoint
            .action_graph
            .nodes
            .get("swap")
            .map(|node| node.status.clone()),
        Some(ActionNodeStatus::Blocked)
    );
}

#[tokio::test]
async fn step_once_completes_when_terminal_verify_node_succeeds() {
    let mission = sample_mission();
    let checkpoint = checkpoint_with_terminal_nodes(
        vec![verify_node("verify-swap", vec![])],
        vec!["verify-swap".to_owned()],
    );
    let mut runtime = ActiveRun::new(mission, checkpoint);

    let first = StepOnce::apply(&mut runtime).await;
    let second = StepOnce::apply(&mut runtime).await;

    assert_eq!(
        first.applied_transition.as_ref().map(|t| t.kind),
        Some(StepTransitionKind::Verify)
    );
    assert_eq!(
        second.applied_transition.as_ref().map(|t| t.kind),
        Some(StepTransitionKind::Complete)
    );
    assert_eq!(runtime.checkpoint.lifecycle.status, RunStatus::Completed);
    assert_eq!(runtime.checkpoint.lifecycle.phase, RunPhase::Finalized);
}

#[tokio::test]
async fn step_once_execution_artifact_branch_activates_transaction_stage() {
    let mission = sample_mission();
    let mut checkpoint = checkpoint_with_nodes(Vec::new());
    checkpoint.execution_artifact = Some(ExecutionArtifactRuntimeSnapshot {
        launch_spec: ExecutionArtifactLaunchSpec {
            protocol_package_id: "owliabot.uniswap_v3".to_owned(),
            action_key: "swap".to_owned(),
            chain_family: ExecutionChainFamily::Evm,
            allowed_chains: vec!["eip155:1".to_owned()],
            entry_stage_id: "stage.branch".into(),
            actor: None,
            transactions: vec![ExecutionTransactionCandidate::EvmTransaction(
                EvmTransactionCandidate {
                    candidate_id: "tx.swap".into(),
                    to: "0x1111111111111111111111111111111111111111".to_owned(),
                    value: Some("0".to_owned()),
                    calldata: Some("0xdeadbeef".to_owned()),
                },
            )],
            stages: vec![
                ExecutionStage::Branch(BranchStage {
                    stage_id: "stage.branch".into(),
                    predicate: PredicateSpec::Cel {
                        expression: "refs.evidence.quote.quoted_at_ms > 0".to_owned(),
                    },
                    if_true: BranchTarget::GotoStage {
                        stage_id: "stage.swap".into(),
                    },
                    if_false: BranchTarget::Assert {
                        failure_code: "missing_quote".to_owned(),
                        message: "quote missing".to_owned(),
                    },
                }),
                ExecutionStage::Transaction(TransactionStage {
                    stage_id: "stage.swap".into(),
                    candidate_ref: "tx.swap".into(),
                    exports: Vec::new(),
                    next_stage_id: None,
                }),
            ],
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
            evidence: json!({
                "quote": {
                    "quoted_at_ms": 1
                }
            }),
            metadata: BTreeMap::new(),
        },
        active_stage_id: Some("stage.branch".into()),
        planned_stage_graphs: BTreeMap::from([(
            "stage.swap".into(),
            ActionGraph {
                graph_id: Some("artifact.owliabot.uniswap_v3.swap".to_owned()),
                roots: vec!["artifact.stage.swap.simulate".to_owned()],
                terminals: vec!["artifact.stage.swap.simulate".to_owned()],
                nodes: BTreeMap::from([(
                    "artifact.stage.swap.simulate".to_owned(),
                    simulate_node("artifact.stage.swap.simulate", vec![]),
                )]),
            },
        )]),
        exported_outputs: BTreeMap::new(),
        branch_trace: Vec::new(),
        awaiting_continuation: None,
    });

    let mut runtime = ActiveRun::new(mission, checkpoint);
    let result = StepOnce::apply(&mut runtime).await;

    assert_eq!(
        result.applied_transition.as_ref().map(|t| t.kind),
        Some(StepTransitionKind::Artifact)
    );
    assert_eq!(
        runtime
            .checkpoint
            .execution_artifact
            .as_ref()
            .and_then(|snapshot| snapshot.active_stage_id.as_ref())
            .map(|value| value.as_str()),
        Some("stage.swap")
    );
    assert_eq!(
        runtime
            .checkpoint
            .execution_artifact
            .as_ref()
            .map(|snapshot| snapshot.branch_trace.clone())
            .unwrap_or_default(),
        vec![ais_agent_core::checkpoint::ArtifactBranchTraceSnapshot {
            branch_stage_id: "stage.branch".into(),
            available_targets: vec!["stage.swap".to_owned(), "assert:missing_quote".to_owned(),],
            selected_target: "stage.swap".to_owned(),
            predicate_value: true,
        }]
    );
    assert!(runtime
        .checkpoint
        .action_graph
        .nodes
        .contains_key("artifact.stage.swap.simulate"));
}

#[tokio::test]
async fn step_once_execution_artifact_exports_and_pauses_for_continuation() {
    let mission = sample_mission();
    let mut checkpoint = checkpoint_with_terminal_nodes(
        vec![{
            let mut node = verify_node("artifact.stage.swap.verify", vec![]);
            node.status = ActionNodeStatus::Succeeded;
            node
        }],
        vec!["artifact.stage.swap.verify".to_owned()],
    );
    checkpoint.evidence_graph.records.insert(
        "receipt.artifact.stage.swap.verify".to_owned(),
        EvidenceRecord {
            evidence_id: "receipt.artifact.stage.swap.verify".to_owned(),
            kind: EvidenceKind::ExternalObservation,
            provenance: EvidenceProvenance {
                source: "evm.alloy.receipt".to_owned(),
                chain_scope: Some("eip155:1".to_owned()),
                trace_hint: Some("artifact.stage.swap.verify".to_owned()),
            },
            freshness: EvidenceFreshness {
                observed_at_ms: Some(1),
                expires_at_ms: None,
                max_age_ms: None,
            },
            confidence_ppm: Some(1_000_000),
            payload: json!({
                "tx_hash": "0xabc",
                "status": true
            }),
        },
    );
    checkpoint.execution_artifact = Some(ExecutionArtifactRuntimeSnapshot {
        launch_spec: ExecutionArtifactLaunchSpec {
            protocol_package_id: "owliabot.uniswap_v3".to_owned(),
            action_key: "swap".to_owned(),
            chain_family: ExecutionChainFamily::Evm,
            allowed_chains: vec!["eip155:1".to_owned()],
            entry_stage_id: "stage.swap".into(),
            actor: None,
            transactions: vec![ExecutionTransactionCandidate::EvmTransaction(
                EvmTransactionCandidate {
                    candidate_id: "tx.swap".into(),
                    to: "0x1111111111111111111111111111111111111111".to_owned(),
                    value: Some("0".to_owned()),
                    calldata: Some("0xdeadbeef".to_owned()),
                },
            )],
            stages: vec![
                ExecutionStage::Transaction(TransactionStage {
                    stage_id: "stage.swap".into(),
                    candidate_ref: "tx.swap".into(),
                    exports: vec![OutputExportSpec {
                        output_key: "swap.tx_hash".into(),
                        source: ValueRef::Ref {
                            reference: "refs.receipts.stage.swap.tx_hash".to_owned(),
                        },
                    }],
                    next_stage_id: Some("stage.continue".into()),
                }),
                ExecutionStage::Continuation(
                    ais_agent_control::execution_artifact::ContinuationStage {
                        stage_id: "stage.continue".into(),
                        required_outputs: vec!["swap.tx_hash".into()],
                        package_entry: "build_aave_supply_from_swap_output".into(),
                        next_stage_id: None,
                    },
                ),
            ],
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
        active_stage_id: Some("stage.swap".into()),
        planned_stage_graphs: BTreeMap::new(),
        exported_outputs: BTreeMap::new(),
        branch_trace: Vec::new(),
        awaiting_continuation: None,
    });

    let mut runtime = ActiveRun::new(mission, checkpoint);
    let first = StepOnce::apply(&mut runtime).await;
    let second = StepOnce::apply(&mut runtime).await;

    assert_eq!(
        first.applied_transition.as_ref().map(|t| t.kind),
        Some(StepTransitionKind::Artifact)
    );
    assert_eq!(
        second.applied_transition.as_ref().map(|t| t.kind),
        Some(StepTransitionKind::Artifact)
    );
    assert_eq!(
        runtime.checkpoint.lifecycle.status,
        RunStatus::AwaitingArtifactContinuation
    );
    assert_eq!(
        runtime
            .checkpoint
            .execution_artifact
            .as_ref()
            .and_then(|snapshot| snapshot.awaiting_continuation.as_ref())
            .map(|continuation| continuation.package_entry.as_str()),
        Some("build_aave_supply_from_swap_output")
    );
    assert_eq!(
        runtime
            .checkpoint
            .execution_artifact
            .as_ref()
            .and_then(|snapshot| { snapshot.exported_outputs.get(&"swap.tx_hash".into()) })
            .and_then(|value| value.as_str()),
        Some("0xabc")
    );
    assert_eq!(
        runtime
            .checkpoint
            .lifecycle
            .active_boundary
            .as_ref()
            .map(|boundary| boundary.blocking_refs.clone()),
        Some(vec!["swap.tx_hash".to_owned()])
    );
}

fn sample_mission() -> Mission {
    Mission {
        mission_id: "mission-1".to_owned(),
        goal: "swap usdc to eth".to_owned(),
        allowed_chains: vec!["eip155:1".to_owned()],
        budget: MissionBudget {
            max_steps: Some(16),
            max_signer_requests: Some(2),
            max_wall_clock_ms: Some(60_000),
        },
        policy: MissionPolicy {
            policy_mode: Some("guarded".to_owned()),
            allow_raw_envelopes: true,
            require_effect_contract_for_writes: true,
        },
        constraints: BTreeMap::new(),
        metadata: BTreeMap::new(),
    }
}

fn checkpoint_with_nodes(nodes: Vec<ActionNode>) -> CheckpointSnapshot {
    checkpoint_with_terminal_nodes(nodes, Vec::new())
}

fn checkpoint_with_terminal_nodes(
    nodes: Vec<ActionNode>,
    terminals: Vec<String>,
) -> CheckpointSnapshot {
    let mut lifecycle = RunLifecycleState::new(RunId("run-1".to_owned()), "mission-1");
    lifecycle.mark_running(RunPhase::Planning);

    CheckpointSnapshot {
        run_id: "run-1".to_owned(),
        mission_id: "mission-1".to_owned(),
        checkpoint_seq: 0,
        plan_epoch: 0,
        lifecycle,
        action_graph: ActionGraph {
            graph_id: Some("graph-1".to_owned()),
            roots: Vec::new(),
            terminals,
            nodes: nodes
                .into_iter()
                .map(|node| (node.node_id.clone(), node))
                .collect(),
        },
        evidence_graph: EvidenceGraph::default(),
        effect_contracts: Default::default(),
        pending_requests: PendingRequestsSnapshot::default(),
        last_completed_node_id: None,
        actuation_records: Vec::new(),
        execution_artifact: None,
    }
}

fn sample_evidence(evidence_id: &str) -> EvidenceRecord {
    EvidenceRecord {
        evidence_id: evidence_id.to_owned(),
        kind: EvidenceKind::RouteOrQuote,
        provenance: EvidenceProvenance {
            source: "quote-api".to_owned(),
            chain_scope: Some("eip155:1".to_owned()),
            trace_hint: Some("test".to_owned()),
        },
        freshness: EvidenceFreshness {
            observed_at_ms: Some(10),
            expires_at_ms: Some(100),
            max_age_ms: Some(90),
        },
        confidence_ppm: Some(900_000),
        payload: json!({"amount_out": "1000"}),
    }
}

fn stale_evidence(evidence_id: &str) -> EvidenceRecord {
    EvidenceRecord {
        evidence_id: evidence_id.to_owned(),
        kind: EvidenceKind::RouteOrQuote,
        provenance: EvidenceProvenance {
            source: "quote-api".to_owned(),
            chain_scope: Some("eip155:1".to_owned()),
            trace_hint: Some("test".to_owned()),
        },
        freshness: EvidenceFreshness {
            observed_at_ms: Some(0),
            expires_at_ms: Some(1),
            max_age_ms: Some(1),
        },
        confidence_ppm: Some(900_000),
        payload: json!({"amount_out": "1000"}),
    }
}

fn derive_node(node_id: &str, depends_on: Vec<&str>) -> ActionNode {
    ActionNode {
        node_id: node_id.to_owned(),
        kind: ActionNodeKind::Derive,
        origin: ActionOrigin::DriverFragment,
        status: ActionNodeStatus::Pending,
        depends_on: depends_on.into_iter().map(str::to_owned).collect(),
        inputs: Vec::new(),
        evidence_refs: Vec::new(),
        payload: ActionPayload::Derive(DeriveAction {
            derive_kind: DeriveKind::Parameter,
            derivation_hint: "derive amount".to_owned(),
            output_key: Some("amount".to_owned()),
        }),
        implementation_hint: None,
        expected_effect_ref: None,
    }
}

fn simulate_node(node_id: &str, depends_on: Vec<&str>) -> ActionNode {
    ActionNode {
        node_id: node_id.to_owned(),
        kind: ActionNodeKind::Simulate,
        origin: ActionOrigin::DriverFragment,
        status: ActionNodeStatus::Pending,
        depends_on: depends_on.into_iter().map(str::to_owned).collect(),
        inputs: Vec::new(),
        evidence_refs: Vec::new(),
        payload: ActionPayload::Simulate(SimulateAction {
            simulate_kind: SimulateKind::Call,
            simulator_hint: "rpc simulation".to_owned(),
            live: None,
        }),
        implementation_hint: None,
        expected_effect_ref: None,
    }
}

fn simulate_live_node_without_connection(node_id: &str, depends_on: Vec<&str>) -> ActionNode {
    ActionNode {
        node_id: node_id.to_owned(),
        kind: ActionNodeKind::Simulate,
        origin: ActionOrigin::DriverFragment,
        status: ActionNodeStatus::Pending,
        depends_on: depends_on.into_iter().map(str::to_owned).collect(),
        inputs: Vec::new(),
        evidence_refs: Vec::new(),
        payload: ActionPayload::Simulate(SimulateAction {
            simulate_kind: SimulateKind::Call,
            simulator_hint: "rpc simulation".to_owned(),
            live: Some(SimulateLiveBinding::Evm(EvmSimulateLiveBinding {
                connection: None,
                binding: EvmSimulateBinding::EthCall,
                request: EvmCallRequest {
                    from: None,
                    to: Address::ZERO,
                    data: Default::default(),
                    value: None,
                },
            })),
        }),
        implementation_hint: None,
        expected_effect_ref: None,
    }
}

fn simulate_succeeded_node(node_id: &str) -> ActionNode {
    let mut node = simulate_node(node_id, vec![]);
    node.status = ActionNodeStatus::Succeeded;
    node
}

fn actuate_node(node_id: &str, depends_on: Vec<&str>) -> ActionNode {
    ActionNode {
        node_id: node_id.to_owned(),
        kind: ActionNodeKind::Actuate,
        origin: ActionOrigin::DriverFragment,
        status: ActionNodeStatus::Pending,
        depends_on: depends_on.into_iter().map(str::to_owned).collect(),
        inputs: Vec::new(),
        evidence_refs: Vec::new(),
        payload: ActionPayload::Actuate(ActuateAction {
            mode: ActuateMode::DriverCall,
            actuator_hint: "swap".to_owned(),
            chain: Some("eip155:1".to_owned()),
            envelope_ref: Some("env.swap".to_owned()),
            requires_effect_contract: true,
            live: None,
        }),
        implementation_hint: None,
        expected_effect_ref: Some("effect.swap".to_owned()),
    }
}

fn actuate_blocked_node(node_id: &str, depends_on: Vec<&str>) -> ActionNode {
    let mut node = actuate_node(node_id, depends_on);
    node.status = ActionNodeStatus::Blocked;
    node
}

fn actuate_evm_broadcast_node(node_id: &str, depends_on: Vec<&str>) -> ActionNode {
    ActionNode {
        node_id: node_id.to_owned(),
        kind: ActionNodeKind::Actuate,
        origin: ActionOrigin::DriverFragment,
        status: ActionNodeStatus::Ready,
        depends_on: depends_on.into_iter().map(str::to_owned).collect(),
        inputs: Vec::new(),
        evidence_refs: Vec::new(),
        payload: ActionPayload::Actuate(ActuateAction {
            mode: ActuateMode::RawEnvelope,
            actuator_hint: "swap".to_owned(),
            chain: Some("eip155:1".to_owned()),
            envelope_ref: Some("env.swap".to_owned()),
            requires_effect_contract: true,
            live: Some(ActuateLiveBinding::Evm(EvmActuateLiveBinding {
                connection: Some(ais_agent_core::binding::evm::EvmConnectionSpec {
                    http_url: "http://localhost:8545".to_owned(),
                    ws_url: None,
                }),
                binding: EvmActuateBinding::BroadcastRawTransaction,
            })),
        }),
        implementation_hint: None,
        expected_effect_ref: Some("effect.swap".to_owned()),
    }
}

fn verify_node(node_id: &str, depends_on: Vec<&str>) -> ActionNode {
    ActionNode {
        node_id: node_id.to_owned(),
        kind: ActionNodeKind::Verify,
        origin: ActionOrigin::DriverFragment,
        status: ActionNodeStatus::Pending,
        depends_on: depends_on.into_iter().map(str::to_owned).collect(),
        inputs: Vec::new(),
        evidence_refs: Vec::new(),
        payload: ActionPayload::Verify(VerifyAction {
            verify_kind: VerifyKind::EffectContract,
            verifier_hint: "effect verifier".to_owned(),
            pre_observation_ref: None,
            post_observation_ref: None,
            live: None,
        }),
        implementation_hint: None,
        expected_effect_ref: Some("effect.swap".to_owned()),
    }
}

#[allow(dead_code)]
fn observe_node(node_id: &str, depends_on: Vec<&str>) -> ActionNode {
    ActionNode {
        node_id: node_id.to_owned(),
        kind: ActionNodeKind::Observe,
        origin: ActionOrigin::DriverFragment,
        status: ActionNodeStatus::Pending,
        depends_on: depends_on.into_iter().map(str::to_owned).collect(),
        inputs: Vec::new(),
        evidence_refs: Vec::new(),
        payload: ActionPayload::Observe(ObserveAction {
            source_kind: ObserveSourceKind::ChainRead,
            source_hint: "read quote".to_owned(),
            output_key: Some("quote".to_owned()),
            live: None,
        }),
        implementation_hint: None,
        expected_effect_ref: None,
    }
}

#[allow(dead_code)]
fn recover_node(node_id: &str, depends_on: Vec<&str>) -> ActionNode {
    ActionNode {
        node_id: node_id.to_owned(),
        kind: ActionNodeKind::Recover,
        origin: ActionOrigin::RecoveryRuntime,
        status: ActionNodeStatus::Pending,
        depends_on: depends_on.into_iter().map(str::to_owned).collect(),
        inputs: Vec::new(),
        evidence_refs: Vec::new(),
        payload: ActionPayload::Recover(RecoverAction {
            recover_kind: RecoverKind::Retry,
            recovery_hint: "retry swap".to_owned(),
        }),
        implementation_hint: None,
        expected_effect_ref: None,
    }
}
