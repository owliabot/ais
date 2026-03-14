use std::collections::BTreeMap;

use ais_agent_control::{
    ids::{RunId, SignerRequestId},
    recovery::{RunFailureCode, RunFailureContext, RunFailureStage, StableBoundaryKind},
};
use ais_agent_core::{
    action::{
        kinds::verify::{VerifyAction, VerifyKind},
        ActionGraph, ActionNode, ActionNodeKind, ActionNodeStatus, ActionOrigin, ActionPayload,
    },
    checkpoint::{CheckpointSnapshot, PendingRequestsSnapshot},
    effect::{EffectAssertion, EffectContract, EffectContractKind},
    evidence::{
        EvidenceFreshness, EvidenceGraph, EvidenceKind, EvidenceProvenance, EvidenceRecord,
    },
    mission::{Mission, MissionBudget, MissionPolicy},
    runtime::{RunLifecycleState, RunPhase, RunStatus, SignerRequestState},
};
use serde_json::json;

use crate::{
    persistence::{
        persist_boundary_checkpoint, persist_progress_checkpoint, persist_side_effect_checkpoint,
        restore_active_run_from_parts, CheckpointRepository, InMemoryCheckpointRepository,
        RestoreRuntimeError,
    },
    runtime::{ActiveRun, RuntimeStateMachine},
};

#[test]
fn restore_active_run_rehydrates_pending_signer_state_from_latest_checkpoint() {
    let mission = sample_mission();
    let signer_state = sample_signer_state();
    let checkpoint = sample_awaiting_signer_checkpoint(&signer_state);

    let restored = restore_active_run_from_parts(
        mission.clone(),
        checkpoint.clone(),
        Some(signer_state.clone()),
    )
    .expect("restore runtime");

    assert_eq!(restored.run_id.0, "run-1");
    assert_eq!(restored.mission.goal, mission.goal);
    assert_eq!(
        restored.checkpoint.checkpoint_seq,
        checkpoint.checkpoint_seq
    );
    assert_eq!(
        restored
            .pending_signer_state
            .as_ref()
            .map(|state| state.request_id.0.as_str()),
        Some("signer-1")
    );
    assert_eq!(
        restored
            .checkpoint
            .pending_requests
            .pending_signer_request_id
            .as_deref(),
        Some("signer-1")
    );
}

#[test]
fn restore_rejects_mission_mismatch_between_mission_and_checkpoint() {
    let mut mission = sample_mission();
    mission.mission_id = "mission-x".to_owned();
    let checkpoint = sample_running_checkpoint();

    let error =
        restore_active_run_from_parts(mission, checkpoint, None).expect_err("mismatch should fail");

    assert!(matches!(
        error,
        RestoreRuntimeError::MissionMismatch {
            mission_id,
            checkpoint_mission_id
        } if mission_id == "mission-x" && checkpoint_mission_id == "mission-1"
    ));
}

#[test]
fn restore_rejects_signer_request_mismatch() {
    let mission = sample_mission();
    let signer_state = SignerRequestState::new_pending(
        SignerRequestId("signer-x".to_owned()),
        RunId("run-1".to_owned()),
        "eip155:1",
        "sign swap",
    )
    .with_node_id("swap");
    let checkpoint = sample_awaiting_signer_checkpoint(&sample_signer_state());

    let error = restore_active_run_from_parts(mission, checkpoint, Some(signer_state))
        .expect_err("signer request mismatch should fail");

    assert!(matches!(
        error,
        RestoreRuntimeError::SignerRequestMismatch {
            expected_request_id,
            actual_request_id
        } if expected_request_id == "signer-1" && actual_request_id == "signer-x"
    ));
}

#[test]
fn boundary_progress_and_side_effect_checkpoint_persistence_have_distinct_wait_semantics() {
    let mission = sample_mission();
    let signer_state = sample_signer_state();
    let mut boundary_runtime = ActiveRun::new(
        mission.clone(),
        sample_awaiting_signer_checkpoint(&signer_state),
    );
    boundary_runtime.set_pending_signer_state(Some(signer_state.clone()));

    let mut checkpoint_repo = InMemoryCheckpointRepository::default();
    let boundary = persist_boundary_checkpoint(&mut checkpoint_repo, &boundary_runtime)
        .expect("persist boundary checkpoint");

    assert_eq!(
        boundary
            .pending_requests
            .pending_signer_request_id
            .as_deref(),
        Some("signer-1")
    );
    assert_eq!(checkpoint_repo.history_len("run-1"), 1);

    let mut progress_runtime = ActiveRun::new(mission, boundary.clone());
    progress_runtime
        .checkpoint
        .lifecycle
        .mark_running(RunPhase::Verifying);
    progress_runtime
        .checkpoint
        .pending_requests
        .pending_evidence_refs = vec!["evidence.quote".to_owned()];
    progress_runtime
        .checkpoint
        .pending_requests
        .pending_signer_request_id = Some("stale-signer".to_owned());
    progress_runtime
        .checkpoint
        .pending_requests
        .pending_confirmation_id = Some("confirm-1".to_owned());
    progress_runtime.set_pending_signer_state(None);
    progress_runtime.touch_transition();

    let progress = persist_progress_checkpoint(&mut checkpoint_repo, &progress_runtime)
        .expect("persist progress checkpoint");

    assert!(progress.pending_requests.pending_evidence_refs.is_empty());
    assert_eq!(progress.pending_requests.pending_signer_request_id, None);
    assert_eq!(progress.pending_requests.pending_confirmation_id, None);
    assert_eq!(checkpoint_repo.history_len("run-1"), 2);

    let latest = checkpoint_repo.latest("run-1").expect("latest checkpoint");
    assert!(latest.pending_requests.pending_evidence_refs.is_empty());
    assert_eq!(latest.pending_requests.pending_signer_request_id, None);

    let mut side_effect_runtime = ActiveRun::new(sample_mission(), sample_running_checkpoint());
    side_effect_runtime
        .checkpoint
        .pending_requests
        .pending_confirmation_id = Some("confirm-2".to_owned());
    side_effect_runtime
        .checkpoint
        .pending_requests
        .pending_evidence_refs = vec!["evidence.pre_state".to_owned()];
    side_effect_runtime.touch_transition();

    let side_effect = persist_side_effect_checkpoint(&mut checkpoint_repo, &side_effect_runtime)
        .expect("persist side-effect checkpoint");
    assert_eq!(
        side_effect
            .pending_requests
            .pending_confirmation_id
            .as_deref(),
        Some("confirm-2")
    );
    assert_eq!(
        side_effect.pending_requests.pending_evidence_refs,
        vec!["evidence.pre_state".to_owned()]
    );
    assert_eq!(checkpoint_repo.history_len("run-1"), 3);
}

#[test]
fn persist_boundary_checkpoint_rejects_invalid_recovery_contract() {
    let mission = sample_mission();
    let mut runtime = ActiveRun::new(mission, sample_running_checkpoint());
    runtime.checkpoint.lifecycle.status = RunStatus::AwaitingEvidence;
    runtime.checkpoint.lifecycle.failure = Some(RunFailureContext::new(
        RunFailureCode::MissingEvidence,
        RunFailureStage::Observe,
        runtime.checkpoint.checkpoint_seq,
        runtime.checkpoint.plan_epoch,
        Some(StableBoundaryKind::Evidence),
        "quote missing",
    ));

    let mut checkpoint_repo = InMemoryCheckpointRepository::default();
    let error = persist_boundary_checkpoint(&mut checkpoint_repo, &runtime)
        .expect_err("invalid recovery contract should be rejected");

    assert!(matches!(
        error,
        crate::persistence::CheckpointArchiveError::InvalidRecoveryContract { .. }
    ));
}

#[test]
fn runtime_state_machine_restores_none_when_checkpoint_has_no_pending_signer() {
    let checkpoint = sample_running_checkpoint();
    let restored = RuntimeStateMachine::restored_pending_signer_state(
        &checkpoint,
        Some(sample_signer_state()),
    );

    assert!(restored.is_none());
}

#[test]
fn side_effect_checkpoint_preserves_confirmation_and_verify_resume_truth() {
    let mission = sample_mission();
    let mut runtime = ActiveRun::new(mission, sample_running_checkpoint());
    runtime.checkpoint.lifecycle.status = RunStatus::AwaitingConfirmation;
    runtime
        .checkpoint
        .lifecycle
        .await_confirmation("waiting for receipt 0xabc");
    runtime.checkpoint.pending_requests.pending_confirmation_id = Some("0xabc".to_owned());
    runtime
        .checkpoint
        .effect_contracts
        .insert("effect.swap".to_owned(), sample_effect_contract());
    runtime.checkpoint.evidence_graph.records.insert(
        "state.pre.out".to_owned(),
        sample_pre_observation("state.pre.out"),
    );
    runtime.checkpoint.action_graph.nodes.insert(
        "verify-swap".to_owned(),
        verify_effect_node("verify-swap", Some("state.pre.out"), Some("state.post.out")),
    );
    runtime.touch_transition();

    let mut checkpoint_repo = InMemoryCheckpointRepository::default();
    let side_effect = persist_side_effect_checkpoint(&mut checkpoint_repo, &runtime)
        .expect("persist side-effect checkpoint");

    assert_eq!(
        side_effect
            .pending_requests
            .pending_confirmation_id
            .as_deref(),
        Some("0xabc")
    );
    assert!(side_effect.effect_contracts.contains_key("effect.swap"));
    assert!(side_effect
        .evidence_graph
        .records
        .contains_key("state.pre.out"));
    let verify = side_effect
        .action_graph
        .nodes
        .get("verify-swap")
        .expect("verify node");
    assert_eq!(verify.expected_effect_ref.as_deref(), Some("effect.swap"));
    match &verify.payload {
        ActionPayload::Verify(verify) => {
            assert_eq!(verify.pre_observation_ref.as_deref(), Some("state.pre.out"));
            assert_eq!(
                verify.post_observation_ref.as_deref(),
                Some("state.post.out")
            );
        }
        other => panic!("unexpected payload: {other:?}"),
    }

    let restored =
        restore_active_run_from_parts(sample_mission(), side_effect, None).expect("restore");
    assert_eq!(
        restored
            .checkpoint
            .pending_requests
            .pending_confirmation_id
            .as_deref(),
        Some("0xabc")
    );
    assert!(restored
        .checkpoint
        .effect_contracts
        .contains_key("effect.swap"));
    assert!(restored
        .checkpoint
        .evidence_graph
        .records
        .contains_key("state.pre.out"));
}

#[test]
fn restore_rejects_awaiting_confirmation_checkpoint_without_pending_confirmation_id() {
    let mut checkpoint = sample_running_checkpoint();
    checkpoint
        .lifecycle
        .await_confirmation("waiting for receipt");

    let error = restore_active_run_from_parts(sample_mission(), checkpoint, None)
        .expect_err("restore should reject missing confirmation id");

    assert_eq!(
        error,
        RestoreRuntimeError::MissingPendingConfirmationId {
            run_id: "run-1".to_owned(),
        }
    );
}

#[test]
fn restore_rejects_confirmation_resume_without_effect_contract() {
    let mut checkpoint = sample_running_checkpoint();
    checkpoint
        .lifecycle
        .await_confirmation("waiting for receipt 0xabc");
    checkpoint.pending_requests.pending_confirmation_id = Some("0xabc".to_owned());
    checkpoint.action_graph.nodes.insert(
        "verify-swap".to_owned(),
        verify_effect_node("verify-swap", Some("state.pre.out"), Some("state.post.out")),
    );

    let error = restore_active_run_from_parts(sample_mission(), checkpoint, None)
        .expect_err("restore should reject missing effect contract");

    assert_eq!(
        error,
        RestoreRuntimeError::MissingEffectContractForConfirmationResume {
            run_id: "run-1".to_owned(),
            node_id: "verify-swap".to_owned(),
            effect_id: "effect.swap".to_owned(),
        }
    );
}

fn sample_mission() -> Mission {
    Mission {
        mission_id: "mission-1".to_owned(),
        goal: "swap usdc to eth".to_owned(),
        allowed_chains: vec!["eip155:1".to_owned()],
        budget: MissionBudget {
            max_steps: Some(10),
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

fn sample_signer_state() -> SignerRequestState {
    SignerRequestState::new_pending(
        SignerRequestId("signer-1".to_owned()),
        RunId("run-1".to_owned()),
        "eip155:1",
        "sign swap",
    )
    .with_node_id("broadcast-swap")
}

fn sample_awaiting_signer_checkpoint(signer_state: &SignerRequestState) -> CheckpointSnapshot {
    let mut lifecycle = RunLifecycleState::new(RunId("run-1".to_owned()), "mission-1");
    lifecycle.mark_running(RunPhase::Broadcasting);
    lifecycle.await_signer_request(signer_state);
    lifecycle.bump_checkpoint();
    lifecycle.bump_plan_epoch();

    CheckpointSnapshot {
        run_id: "run-1".to_owned(),
        mission_id: "mission-1".to_owned(),
        checkpoint_seq: lifecycle.checkpoint_seq,
        plan_epoch: lifecycle.plan_epoch,
        lifecycle,
        action_graph: ActionGraph {
            graph_id: Some("graph-1".to_owned()),
            roots: Vec::new(),
            terminals: Vec::new(),
            nodes: BTreeMap::new(),
        },
        evidence_graph: EvidenceGraph::default(),
        effect_contracts: Default::default(),
        pending_requests: PendingRequestsSnapshot {
            pending_evidence_refs: Vec::new(),
            pending_envelope_refs: Vec::new(),
            pending_signer_request_id: Some(signer_state.request_id.0.clone()),
            pending_confirmation_id: None,
        },
        last_completed_node_id: Some("simulate-1".to_owned()),
        actuation_records: Vec::new(),
        execution_artifact: None,
    }
}

fn sample_running_checkpoint() -> CheckpointSnapshot {
    let mut lifecycle = RunLifecycleState::new(RunId("run-1".to_owned()), "mission-1");
    lifecycle.mark_running(RunPhase::Verifying);

    CheckpointSnapshot {
        run_id: "run-1".to_owned(),
        mission_id: "mission-1".to_owned(),
        checkpoint_seq: lifecycle.checkpoint_seq,
        plan_epoch: lifecycle.plan_epoch,
        lifecycle,
        action_graph: ActionGraph {
            graph_id: Some("graph-1".to_owned()),
            roots: Vec::new(),
            terminals: Vec::new(),
            nodes: BTreeMap::new(),
        },
        evidence_graph: EvidenceGraph::default(),
        effect_contracts: Default::default(),
        pending_requests: PendingRequestsSnapshot::default(),
        last_completed_node_id: Some("broadcast-1".to_owned()),
        actuation_records: Vec::new(),
        execution_artifact: None,
    }
}

fn verify_effect_node(
    node_id: &str,
    pre_observation_ref: Option<&str>,
    post_observation_ref: Option<&str>,
) -> ActionNode {
    ActionNode {
        node_id: node_id.to_owned(),
        kind: ActionNodeKind::Verify,
        origin: ActionOrigin::DriverFragment,
        status: ActionNodeStatus::Pending,
        depends_on: vec!["swap".to_owned()],
        inputs: Vec::new(),
        evidence_refs: Vec::new(),
        payload: ActionPayload::Verify(VerifyAction {
            verify_kind: VerifyKind::EffectContract,
            verifier_hint: "verify effect".to_owned(),
            pre_observation_ref: pre_observation_ref.map(str::to_owned),
            post_observation_ref: post_observation_ref.map(str::to_owned),
            live: None,
        }),
        implementation_hint: None,
        expected_effect_ref: Some("effect.swap".to_owned()),
    }
}

fn sample_effect_contract() -> EffectContract {
    EffectContract {
        effect_id: "effect.swap".to_owned(),
        kind: EffectContractKind::StateTransition,
        assertions: vec![EffectAssertion {
            expression: "post.decoded_u256 == \"120\"".to_owned(),
            description: "post-state output should be 120".to_owned(),
        }],
        tolerance_hint: Some("exact_output".to_owned()),
    }
}

fn sample_pre_observation(evidence_id: &str) -> EvidenceRecord {
    EvidenceRecord {
        evidence_id: evidence_id.to_owned(),
        kind: EvidenceKind::ExternalObservation,
        provenance: EvidenceProvenance {
            source: "test.pre_state".to_owned(),
            chain_scope: Some("eip155:1".to_owned()),
            trace_hint: Some("verify-swap".to_owned()),
        },
        freshness: EvidenceFreshness {
            observed_at_ms: Some(1),
            expires_at_ms: None,
            max_age_ms: None,
        },
        confidence_ppm: Some(1_000_000),
        payload: json!({"decoded_u256":"90"}),
    }
}
