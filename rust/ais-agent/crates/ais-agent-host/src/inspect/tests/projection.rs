use std::collections::BTreeMap;

use ais_agent_control::{
    ids::{RunId, SignerRequestId},
    recovery::{
        RecoveryActionKind, RecoveryDisposition, RunFailureCode, RunFailureContext,
        RunFailureStage, StableBoundaryKind,
    },
};
use ais_agent_core::{
    action::{
        kinds::{
            actuate::{ActuateAction, ActuateMode},
            derive::{DeriveAction, DeriveKind},
        },
        ActionGraph, ActionInputRef, ActionNode, ActionNodeKind, ActionNodeStatus, ActionOrigin,
        ActionPayload,
    },
    actuation::{ActuationKind, ActuationRecord, ActuationStatus},
    checkpoint::{
        CheckpointSnapshot, CheckpointStore, InMemoryCheckpointStore, PendingRequestsSnapshot,
    },
    evidence::{
        EvidenceFreshness, EvidenceGraph, EvidenceKind, EvidenceProvenance, EvidenceRecord,
        EvidenceRequirement,
    },
    mission::{Mission, MissionBudget, MissionPolicy},
    runtime::{
        BoundaryKind, RunLifecycleState, RunPhase, RunStatus as CoreRunStatus, StableBoundary,
    },
};
use serde_json::json;

use crate::inspect::{
    project_inspect_snapshot, project_pause_bundle, EffectStatusView, PauseKind, RunStatus,
};

#[test]
fn inspect_snapshot_projects_required_inputs_progress_and_side_effects() {
    let mission = sample_mission();
    let checkpoint = sample_checkpoint();

    let snapshot = project_inspect_snapshot(&mission, &checkpoint);

    assert_eq!(snapshot.status, RunStatus::AwaitingEvidence);
    assert_eq!(
        snapshot.recovery_disposition,
        Some(RecoveryDisposition::AwaitEvidence)
    );
    assert_eq!(snapshot.mission_summary.goal, "swap usdc to eth safely");
    assert_eq!(snapshot.required_inputs.len(), 1);
    assert_eq!(snapshot.required_inputs[0].reference, "evidence.quote");
    assert_eq!(snapshot.recovery_suggestions.len(), 1);
    assert_eq!(
        snapshot.recovery_suggestions[0].action_kind,
        RecoveryActionKind::SubmitEvidence
    );
    assert!(snapshot
        .allowed_recovery_actions
        .contains(&RecoveryActionKind::SubmitEvidence));
    assert_eq!(snapshot.pending_signer_requests.len(), 1);
    assert_eq!(snapshot.progress.total_nodes, 2);
    assert_eq!(snapshot.progress.status_counts.succeeded, 1);
    assert_eq!(snapshot.progress.status_counts.blocked, 1);
    assert_eq!(snapshot.recent_side_effects.len(), 1);
    assert_eq!(snapshot.effect_status, Some(EffectStatusView::Unknown));
    assert!(snapshot.ownership.claim_required_for_mutation);
    assert!(snapshot.ownership.current_claim.is_none());
    assert!(snapshot.run_result.is_none());
}

#[test]
fn pause_bundle_projects_pause_kind_and_required_actions() {
    let checkpoint = sample_checkpoint();

    let pause = project_pause_bundle(&checkpoint).expect("pause bundle");

    assert_eq!(pause.kind, PauseKind::NeedEvidence);
    assert_eq!(
        pause.recovery_disposition,
        RecoveryDisposition::AwaitEvidence
    );
    assert_eq!(pause.blocking_refs, vec!["evidence.quote".to_owned()]);
    assert_eq!(pause.required_actions.len(), 2);
    assert_eq!(
        pause.required_actions[0].action_kind,
        RecoveryActionKind::SubmitEvidence
    );
    assert_eq!(pause.required_actions[0].action, "submit_evidence");
    assert_eq!(pause.recovery_suggestions.len(), 1);
    assert_eq!(
        pause.allowed_recovery_actions,
        vec![
            RecoveryActionKind::SubmitEvidence,
            RecoveryActionKind::CancelRun
        ]
    );
    assert!(pause.ownership.claim_required_for_mutation);
    assert!(pause.required_actions[0].requires_mutation_claim);
}

#[test]
fn confirmation_pause_bundle_disambiguates_step_actions() {
    let checkpoint = awaiting_confirmation_checkpoint();

    let pause = project_pause_bundle(&checkpoint).expect("pause bundle");

    assert_eq!(pause.kind, PauseKind::NeedConfirmation);
    assert_eq!(
        pause.side_effect_phase,
        Some(ais_agent_control::recovery::SideEffectPhase::AwaitingConfirmation)
    );
    assert_eq!(
        pause.required_actions,
        vec![
            crate::inspect::PauseActionView {
                action_kind: RecoveryActionKind::RetryStep,
                action: "step_run".to_owned(),
                description: "Run the stepper again when retry or confirmation polling is allowed."
                    .to_owned(),
                requires_mutation_claim: true,
                retry_intent: Some(ais_agent_control::commands::RetryIntent::ResumeExecution),
            },
            crate::inspect::PauseActionView {
                action_kind: RecoveryActionKind::AwaitConfirmation,
                action: "step_run".to_owned(),
                description:
                    "Wait for more chain confirmation information before making a new decision."
                        .to_owned(),
                requires_mutation_claim: true,
                retry_intent: Some(ais_agent_control::commands::RetryIntent::PollConfirmation),
            },
            crate::inspect::PauseActionView {
                action_kind: RecoveryActionKind::CancelRun,
                action: "cancel_run".to_owned(),
                description: "Abort the run instead of attempting further recovery.".to_owned(),
                requires_mutation_claim: true,
                retry_intent: None,
            },
        ]
    );
    assert!(pause.ownership.claim_required_for_mutation);
}

#[test]
fn restored_checkpoint_keeps_signer_wait_visible_in_inspect_projection() {
    let mission = sample_mission();
    let checkpoint = sample_checkpoint();
    let mut store = InMemoryCheckpointStore::default();

    store.save(checkpoint).expect("save checkpoint");
    let restored = store.latest("run-1").expect("latest checkpoint");

    let snapshot = project_inspect_snapshot(&mission, &restored);
    let pause = project_pause_bundle(&restored).expect("pause bundle");

    assert_eq!(snapshot.status, RunStatus::AwaitingEvidence);
    assert_eq!(snapshot.pending_signer_requests.len(), 1);
    assert_eq!(snapshot.pending_signer_requests[0].request_id.0, "signer-1");
    assert_eq!(pause.kind, PauseKind::NeedEvidence);
    assert_eq!(pause.pending_signer_requests.len(), 1);
    assert_eq!(pause.pending_signer_requests[0].request_id.0, "signer-1");
}

#[test]
fn paused_checkpoint_with_failure_projects_patch_required_recovery() {
    let checkpoint = paused_patch_checkpoint();

    let snapshot = project_inspect_snapshot(&sample_mission(), &checkpoint);
    let pause = project_pause_bundle(&checkpoint).expect("pause bundle");

    assert_eq!(snapshot.status, RunStatus::Paused);
    assert_eq!(snapshot.interruption_class, None);
    assert_eq!(snapshot.cancel_state, None);
    assert_eq!(
        snapshot.recovery_disposition,
        Some(RecoveryDisposition::AwaitPatch)
    );
    assert_eq!(
        snapshot
            .failure_context
            .as_ref()
            .map(|failure| &failure.code),
        Some(&RunFailureCode::GovernorDenied)
    );
    assert!(snapshot
        .allowed_recovery_actions
        .contains(&RecoveryActionKind::SubmitPlanPatch));
    assert_eq!(pause.kind, PauseKind::NeedUserInput);
    assert_eq!(pause.recovery_disposition, RecoveryDisposition::AwaitPatch);
    assert_eq!(
        pause.failure_context.as_ref().map(|failure| &failure.code),
        Some(&RunFailureCode::GovernorDenied)
    );
    assert!(pause
        .required_actions
        .iter()
        .any(|action| action.action == "submit_plan_patch"));
    assert!(pause.ownership.claim_required_for_mutation);
}

#[test]
fn failed_checkpoint_projects_terminal_run_result_with_recovery_context() {
    let checkpoint = failed_checkpoint();

    let snapshot = project_inspect_snapshot(&sample_mission(), &checkpoint);
    let pause = project_pause_bundle(&checkpoint).expect("pause bundle");

    assert_eq!(snapshot.status, RunStatus::Failed);
    assert_eq!(
        snapshot.recovery_disposition,
        Some(RecoveryDisposition::FailedClosed)
    );
    assert_eq!(
        snapshot
            .run_result
            .as_ref()
            .map(|result| result.summary.as_str()),
        Some("post-state balance mismatch")
    );
    assert_eq!(
        snapshot
            .run_result
            .as_ref()
            .and_then(|result| result.terminal_failure_context.as_ref())
            .map(|failure| &failure.code),
        Some(&RunFailureCode::VerifyMismatch)
    );
    assert_eq!(
        snapshot
            .run_result
            .as_ref()
            .map(|result| result.ownership.claim_required_for_mutation),
        Some(false)
    );
    assert_eq!(pause.kind, PauseKind::RuntimeFailure);
    assert_eq!(
        pause.recovery_disposition,
        RecoveryDisposition::FailedClosed
    );
}

#[test]
fn envelope_invalid_checkpoint_projects_await_envelope_recovery() {
    let checkpoint = envelope_wait_checkpoint();

    let snapshot = project_inspect_snapshot(&sample_mission(), &checkpoint);
    let pause = project_pause_bundle(&checkpoint).expect("pause bundle");

    assert_eq!(snapshot.status, RunStatus::Paused);
    assert_eq!(
        snapshot.recovery_disposition,
        Some(RecoveryDisposition::AwaitEnvelope)
    );
    assert_eq!(
        snapshot.allowed_recovery_actions,
        vec![
            RecoveryActionKind::SubmitEnvelope,
            RecoveryActionKind::CancelRun
        ]
    );
    assert_eq!(snapshot.recovery_suggestions.len(), 1);
    assert_eq!(
        snapshot.recovery_suggestions[0].action_kind,
        RecoveryActionKind::SubmitEnvelope
    );
    assert_eq!(
        pause.recovery_disposition,
        RecoveryDisposition::AwaitEnvelope
    );
    assert!(pause
        .required_actions
        .iter()
        .any(|action| action.action == "submit_envelope"));
}

#[test]
fn verify_mismatch_checkpoint_projects_richer_recovery_contract() {
    let checkpoint = paused_verify_mismatch_checkpoint();

    let snapshot = project_inspect_snapshot(&sample_mission(), &checkpoint);
    let pause = project_pause_bundle(&checkpoint).expect("pause bundle");

    assert_eq!(snapshot.status, RunStatus::Paused);
    assert_eq!(
        snapshot.recovery_disposition,
        Some(RecoveryDisposition::AwaitPatch)
    );
    assert!(snapshot
        .allowed_recovery_actions
        .contains(&RecoveryActionKind::SubmitEvidence));
    assert!(snapshot
        .allowed_recovery_actions
        .contains(&RecoveryActionKind::SubmitPlanPatch));
    assert!(snapshot
        .allowed_recovery_actions
        .contains(&RecoveryActionKind::EscalateUserReview));
    assert!(snapshot
        .recovery_suggestions
        .iter()
        .any(|suggestion| suggestion.action_kind == RecoveryActionKind::SubmitEvidence));
    assert!(snapshot
        .recovery_suggestions
        .iter()
        .any(|suggestion| suggestion.action_kind == RecoveryActionKind::SubmitPlanPatch));
    assert!(snapshot
        .recovery_suggestions
        .iter()
        .any(|suggestion| suggestion.action_kind == RecoveryActionKind::EscalateUserReview));
    assert_eq!(pause.recovery_disposition, RecoveryDisposition::AwaitPatch);
    assert!(pause
        .required_actions
        .iter()
        .any(|action| action.action == "submit_plan_patch"));
}

fn sample_mission() -> Mission {
    Mission {
        mission_id: "mission-1".to_owned(),
        goal: "swap usdc to eth safely".to_owned(),
        allowed_chains: vec!["eip155:1".to_owned()],
        budget: MissionBudget {
            max_steps: Some(8),
            max_signer_requests: Some(1),
            max_wall_clock_ms: Some(120_000),
        },
        policy: MissionPolicy {
            policy_mode: Some("guarded".to_owned()),
            allow_raw_envelopes: false,
            require_effect_contract_for_writes: true,
        },
        constraints: BTreeMap::new(),
        metadata: BTreeMap::new(),
    }
}

fn sample_checkpoint() -> CheckpointSnapshot {
    let mut lifecycle = RunLifecycleState::new(RunId("run-1".to_owned()), "mission-1");
    lifecycle.mark_running(RunPhase::Planning);
    lifecycle.bump_checkpoint();
    lifecycle.bump_plan_epoch();
    lifecycle.await_evidence(
        "quote is required before simulation",
        vec!["evidence.quote".into()],
    );

    let mut nodes = BTreeMap::new();
    nodes.insert(
        "derive-min-out".to_owned(),
        ActionNode {
            node_id: "derive-min-out".to_owned(),
            kind: ActionNodeKind::Derive,
            origin: ActionOrigin::DriverFragment,
            status: ActionNodeStatus::Succeeded,
            depends_on: Vec::new(),
            inputs: vec![ActionInputRef {
                reference: "evidence.quote".to_owned(),
                optional: false,
            }],
            evidence_refs: vec!["quote-1".to_owned()],
            payload: ActionPayload::Derive(DeriveAction {
                derive_kind: DeriveKind::SlippageBound,
                derivation_hint: "derive minimum output from quote".to_owned(),
                output_key: Some("derived.min_out".to_owned()),
            }),
            implementation_hint: None,
            expected_effect_ref: None,
        },
    );
    nodes.insert(
        "broadcast-swap".to_owned(),
        ActionNode {
            node_id: "broadcast-swap".to_owned(),
            kind: ActionNodeKind::Actuate,
            origin: ActionOrigin::RawEnvelopePath,
            status: ActionNodeStatus::Blocked,
            depends_on: vec!["derive-min-out".to_owned()],
            inputs: vec![ActionInputRef {
                reference: "derived.min_out".to_owned(),
                optional: false,
            }],
            evidence_refs: vec!["quote-1".to_owned()],
            payload: ActionPayload::Actuate(ActuateAction {
                mode: ActuateMode::RawEnvelope,
                actuator_hint: "broadcast raw swap envelope".to_owned(),
                chain: Some("eip155:1".to_owned()),
                envelope_ref: Some("envelopes.swap".to_owned()),
                requires_effect_contract: true,
                live: None,
            }),
            implementation_hint: Some("swap envelope pending".to_owned()),
            expected_effect_ref: Some("effects.swap".to_owned()),
        },
    );

    let mut records = BTreeMap::new();
    records.insert(
        "quote-1".to_owned(),
        EvidenceRecord {
            evidence_id: "quote-1".to_owned(),
            kind: EvidenceKind::RouteOrQuote,
            provenance: EvidenceProvenance {
                source: "host.quote_api".to_owned(),
                chain_scope: Some("eip155:1".to_owned()),
                trace_hint: Some("req-1".to_owned()),
            },
            freshness: EvidenceFreshness {
                observed_at_ms: Some(1_000),
                expires_at_ms: Some(31_000),
                max_age_ms: Some(30_000),
            },
            confidence_ppm: Some(900_000),
            payload: json!({"amount_out":"1000000"}),
        },
    );

    CheckpointSnapshot {
        run_id: "run-1".to_owned(),
        mission_id: "mission-1".to_owned(),
        checkpoint_seq: lifecycle.checkpoint_seq,
        plan_epoch: lifecycle.plan_epoch,
        lifecycle,
        action_graph: ActionGraph {
            graph_id: Some("graph-1".to_owned()),
            roots: vec!["derive-min-out".to_owned()],
            terminals: vec!["broadcast-swap".to_owned()],
            nodes,
        },
        evidence_graph: EvidenceGraph {
            records,
            requirements: vec![EvidenceRequirement {
                requirement_id: "req-quote".to_owned(),
                reference: "evidence.quote".to_owned(),
                reason: "best route quote is required".to_owned(),
                required_by_node_id: Some("broadcast-swap".to_owned()),
                satisfied_by_evidence_id: None,
            }],
            usages: Vec::new(),
        },
        effect_contracts: Default::default(),
        pending_requests: PendingRequestsSnapshot {
            pending_evidence_refs: vec!["evidence.quote".to_owned()],
            pending_envelope_refs: Vec::new(),
            pending_signer_request_id: Some(SignerRequestId("signer-1".to_owned()).0),
            pending_confirmation_id: None,
        },
        last_completed_node_id: Some("derive-min-out".to_owned()),
        actuation_records: vec![ActuationRecord {
            record_id: "act-1".to_owned(),
            node_id: "broadcast-swap".to_owned(),
            kind: ActuationKind::EnvelopeBuilt,
            status: ActuationStatus::Pending,
            chain: Some("eip155:1".to_owned()),
            tx_hash: None,
            summary: "swap envelope prepared".to_owned(),
        }],
    }
}

fn paused_patch_checkpoint() -> CheckpointSnapshot {
    let mut checkpoint = sample_checkpoint();
    checkpoint.lifecycle.status = CoreRunStatus::Paused;
    checkpoint.lifecycle.failure = Some(RunFailureContext::new(
        RunFailureCode::GovernorDenied,
        RunFailureStage::Govern,
        checkpoint.checkpoint_seq,
        checkpoint.plan_epoch,
        Some(StableBoundaryKind::Pause),
        "governor denied the active route",
    ));
    checkpoint.lifecycle.active_boundary = Some(StableBoundary {
        kind: BoundaryKind::Pause,
        summary: "governor denied the active route".to_owned(),
        blocking_refs: vec!["node.broadcast-swap".to_owned()],
        signer_request_id: None,
    });
    checkpoint
}

fn failed_checkpoint() -> CheckpointSnapshot {
    let mut checkpoint = sample_checkpoint();
    checkpoint.lifecycle.fail(
        RunFailureStage::Verify,
        RunFailureCode::VerifyMismatch,
        "post-state balance mismatch",
    );
    checkpoint
}

fn envelope_wait_checkpoint() -> CheckpointSnapshot {
    let mut checkpoint = sample_checkpoint();
    checkpoint.lifecycle.status = CoreRunStatus::Paused;
    checkpoint.pending_requests.pending_envelope_refs = vec!["env.swap".to_owned()];
    checkpoint.lifecycle.failure = Some(RunFailureContext::new(
        RunFailureCode::EnvelopeInvalid,
        RunFailureStage::Broadcast,
        checkpoint.checkpoint_seq,
        checkpoint.plan_epoch,
        Some(StableBoundaryKind::Pause),
        "replacement envelope required",
    ));
    checkpoint.lifecycle.active_boundary = Some(StableBoundary {
        kind: BoundaryKind::Pause,
        summary: "replacement envelope required".to_owned(),
        blocking_refs: vec!["env.swap".to_owned()],
        signer_request_id: None,
    });
    checkpoint
}

fn paused_verify_mismatch_checkpoint() -> CheckpointSnapshot {
    let mut checkpoint = sample_checkpoint();
    checkpoint.lifecycle.status = CoreRunStatus::Paused;
    let mut failure = RunFailureContext::new(
        RunFailureCode::VerifyMismatch,
        RunFailureStage::Verify,
        checkpoint.checkpoint_seq,
        checkpoint.plan_epoch,
        Some(StableBoundaryKind::Pause),
        "post-state balance mismatch",
    );
    failure.node_refs.push("node.verify-swap".to_owned());
    failure.effect_refs.push("effect.swap".to_owned());
    failure.confirmation_refs.push("0xabc".to_owned());
    failure.evidence_refs.push("state.post.balance".to_owned());
    checkpoint.lifecycle.failure = Some(failure);
    checkpoint.lifecycle.active_boundary = Some(StableBoundary {
        kind: BoundaryKind::Pause,
        summary: "post-state balance mismatch".to_owned(),
        blocking_refs: vec!["node.verify-swap".to_owned()],
        signer_request_id: None,
    });
    checkpoint.pending_requests.pending_confirmation_id = Some("0xabc".to_owned());
    checkpoint
}

fn awaiting_confirmation_checkpoint() -> CheckpointSnapshot {
    let mut checkpoint = sample_checkpoint();
    checkpoint.lifecycle.status = CoreRunStatus::AwaitingConfirmation;
    checkpoint.pending_requests.pending_confirmation_id = Some("0xconfirm".to_owned());
    checkpoint.lifecycle.active_boundary = Some(StableBoundary {
        kind: BoundaryKind::Confirmation,
        summary: "waiting for chain receipt".to_owned(),
        blocking_refs: vec!["0xconfirm".to_owned()],
        signer_request_id: None,
    });
    checkpoint
}
