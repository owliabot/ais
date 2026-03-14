use ais_agent_control::{
    ids::RunId,
    recovery::{
        CancelState, InterruptionClass, RecoveryActionKind, RecoveryDisposition, RunFailureCode,
        RunFailureContext, RunFailureStage, SideEffectPhase, StableBoundaryKind,
    },
};
use ais_agent_core::{
    action::ActionGraph,
    checkpoint::{CheckpointSnapshot, PendingRequestsSnapshot},
    evidence::{EvidenceGraph, EvidenceRequirement},
    runtime::{RunLifecycleState, RunPhase},
};

use crate::runtime::classify_recovery_view;

#[test]
fn awaiting_evidence_checkpoint_projects_runtime_owned_recovery_view() {
    let checkpoint = awaiting_evidence_checkpoint();

    let recovery = classify_recovery_view(&checkpoint);

    assert_eq!(
        recovery.recovery_disposition,
        Some(RecoveryDisposition::AwaitEvidence)
    );
    assert_eq!(
        recovery.allowed_recovery_actions,
        vec![
            RecoveryActionKind::SubmitEvidence,
            RecoveryActionKind::CancelRun,
        ]
    );
    assert_eq!(recovery.recovery_suggestions.len(), 1);
    assert_eq!(
        recovery.recovery_suggestions[0].action_kind,
        RecoveryActionKind::SubmitEvidence
    );
    assert_eq!(
        recovery.recovery_suggestions[0].required_inputs[0]
            .value
            .as_deref(),
        Some("evidence.quote")
    );
}

#[test]
fn running_budget_interruption_projects_continue_wait_recovery_view() {
    let checkpoint = running_budget_interrupted_checkpoint();

    let recovery = classify_recovery_view(&checkpoint);

    assert_eq!(
        recovery.recovery_disposition,
        Some(RecoveryDisposition::ContinueWait)
    );
    assert_eq!(
        recovery.allowed_recovery_actions,
        vec![RecoveryActionKind::RetryStep, RecoveryActionKind::CancelRun]
    );
    assert_eq!(
        recovery.interruption_class,
        Some(InterruptionClass::StepBudgetExhausted)
    );
    assert_eq!(recovery.cancel_state, None);
    assert_eq!(recovery.side_effect_phase, None);
    assert_eq!(recovery.recovery_suggestions.len(), 1);
    assert_eq!(
        recovery.recovery_suggestions[0].retry_intent,
        Some(ais_agent_control::commands::RetryIntent::ResumeExecution)
    );
}

#[test]
fn paused_governor_failure_projects_patch_recovery_view() {
    let checkpoint = paused_patch_checkpoint();

    let recovery = classify_recovery_view(&checkpoint);

    assert_eq!(
        recovery.recovery_disposition,
        Some(RecoveryDisposition::AwaitPatch)
    );
    assert_eq!(
        recovery
            .failure_context
            .as_ref()
            .map(|failure| &failure.code),
        Some(&RunFailureCode::GovernorDenied)
    );
    assert!(recovery
        .allowed_recovery_actions
        .contains(&RecoveryActionKind::SubmitPlanPatch));
    assert_eq!(recovery.recovery_suggestions.len(), 1);
    assert_eq!(
        recovery.recovery_suggestions[0].action_kind,
        RecoveryActionKind::SubmitPlanPatch
    );
}

#[test]
fn paused_broadcast_uncertainty_projects_user_review_recovery_view() {
    let checkpoint = paused_broadcast_uncertain_checkpoint();

    let recovery = classify_recovery_view(&checkpoint);

    assert_eq!(
        recovery.recovery_disposition,
        Some(RecoveryDisposition::AwaitUserInput)
    );
    assert_eq!(
        recovery.allowed_recovery_actions,
        vec![
            RecoveryActionKind::EscalateUserReview,
            RecoveryActionKind::CancelRun,
        ]
    );
    assert_eq!(recovery.recovery_suggestions.len(), 1);
    assert_eq!(
        recovery.recovery_suggestions[0].action_kind,
        RecoveryActionKind::EscalateUserReview
    );
    assert_eq!(
        recovery.recovery_suggestions[0].reason_code,
        RunFailureCode::BroadcastUncertain
    );
    assert_eq!(
        recovery.interruption_class,
        Some(InterruptionClass::BroadcastOutcomeUncertain)
    );
    assert_eq!(
        recovery.side_effect_phase,
        Some(SideEffectPhase::BroadcastSubmitted)
    );
    assert_eq!(recovery.cancel_state, None);
}

#[test]
fn paused_provider_failure_projects_retry_ready_recovery_view() {
    let checkpoint = paused_provider_failure_checkpoint();

    let recovery = classify_recovery_view(&checkpoint);

    assert_eq!(
        recovery.recovery_disposition,
        Some(RecoveryDisposition::RetryReady)
    );
    assert_eq!(
        recovery.allowed_recovery_actions,
        vec![RecoveryActionKind::RetryStep, RecoveryActionKind::CancelRun]
    );
    assert_eq!(recovery.recovery_suggestions.len(), 1);
    assert_eq!(
        recovery.recovery_suggestions[0].reason_code,
        RunFailureCode::ProviderUnavailable
    );
    assert_eq!(
        recovery.interruption_class,
        Some(InterruptionClass::ProviderUnavailable)
    );
    assert_eq!(
        recovery.side_effect_phase,
        Some(SideEffectPhase::AwaitingConfirmation)
    );
}

#[test]
fn paused_confirmation_timeout_projects_confirmation_wait_interruption() {
    let checkpoint = paused_confirmation_timeout_checkpoint();

    let recovery = classify_recovery_view(&checkpoint);

    assert_eq!(
        recovery.recovery_disposition,
        Some(RecoveryDisposition::RetryReady)
    );
    assert_eq!(
        recovery.interruption_class,
        Some(InterruptionClass::ConfirmationWaitTimeout)
    );
    assert_eq!(
        recovery.side_effect_phase,
        Some(SideEffectPhase::AwaitingConfirmation)
    );
}

#[test]
fn paused_observe_timeout_projects_retryable_provider_timeout() {
    let checkpoint = paused_observe_timeout_checkpoint();

    let recovery = classify_recovery_view(&checkpoint);

    assert_eq!(
        recovery.recovery_disposition,
        Some(RecoveryDisposition::RetryReady)
    );
    assert_eq!(
        recovery.interruption_class,
        Some(InterruptionClass::ProviderTimeout)
    );
    assert_eq!(recovery.side_effect_phase, None);
}

#[test]
fn cancelled_checkpoint_projects_terminal_cancel_state() {
    let checkpoint = cancelled_checkpoint();

    let recovery = classify_recovery_view(&checkpoint);

    assert_eq!(recovery.cancel_state, Some(CancelState::Cancelled));
    assert_eq!(
        recovery.recovery_disposition,
        Some(RecoveryDisposition::AbortOnly)
    );
    assert!(recovery.allowed_recovery_actions.is_empty());
    assert!(recovery.recovery_suggestions.is_empty());
}

#[test]
fn cancel_pending_confirmation_wait_projects_without_cancel_action() {
    let checkpoint = cancel_pending_confirmation_checkpoint();

    let recovery = classify_recovery_view(&checkpoint);

    assert_eq!(recovery.cancel_state, Some(CancelState::Pending));
    assert_eq!(
        recovery.interruption_class,
        Some(InterruptionClass::HostCancelRequested)
    );
    assert_eq!(
        recovery.recovery_disposition,
        Some(RecoveryDisposition::ContinueWait)
    );
    assert_eq!(
        recovery.allowed_recovery_actions,
        vec![
            RecoveryActionKind::RetryStep,
            RecoveryActionKind::AwaitConfirmation,
        ]
    );
}

#[test]
fn paused_envelope_failure_projects_await_envelope_recovery_view() {
    let checkpoint = paused_envelope_checkpoint();

    let recovery = classify_recovery_view(&checkpoint);

    assert_eq!(
        recovery.recovery_disposition,
        Some(RecoveryDisposition::AwaitEnvelope)
    );
    assert_eq!(
        recovery.allowed_recovery_actions,
        vec![
            RecoveryActionKind::SubmitEnvelope,
            RecoveryActionKind::CancelRun,
        ]
    );
    assert_eq!(recovery.recovery_suggestions.len(), 1);
    assert_eq!(
        recovery.recovery_suggestions[0].action_kind,
        RecoveryActionKind::SubmitEnvelope
    );
    assert_eq!(
        recovery.recovery_suggestions[0].required_inputs[0]
            .value
            .as_deref(),
        Some("env.swap")
    );
}

#[test]
fn terminal_failure_projects_failed_closed_recovery_view() {
    let checkpoint = failed_checkpoint();

    let recovery = classify_recovery_view(&checkpoint);

    assert_eq!(
        recovery.recovery_disposition,
        Some(RecoveryDisposition::FailedClosed)
    );
    assert_eq!(
        recovery
            .failure_context
            .as_ref()
            .map(|failure| &failure.code),
        Some(&RunFailureCode::VerifyMismatch)
    );
    assert_eq!(
        recovery.allowed_recovery_actions,
        vec![RecoveryActionKind::CancelRun]
    );
    assert_eq!(recovery.recovery_suggestions.len(), 1);
    assert_eq!(
        recovery.recovery_suggestions[0].reason_code,
        RunFailureCode::VerifyMismatch
    );
}

#[test]
fn verify_mismatch_projects_evidence_patch_and_user_review_suggestions() {
    let checkpoint = paused_verify_mismatch_checkpoint();

    let recovery = classify_recovery_view(&checkpoint);

    assert_eq!(
        recovery.recovery_disposition,
        Some(RecoveryDisposition::AwaitPatch)
    );
    assert_eq!(
        recovery.allowed_recovery_actions,
        vec![
            RecoveryActionKind::SubmitEvidence,
            RecoveryActionKind::SubmitPlanPatch,
            RecoveryActionKind::EscalateUserReview,
            RecoveryActionKind::CancelRun,
        ]
    );
    assert_eq!(recovery.recovery_suggestions.len(), 3);
    assert!(recovery
        .recovery_suggestions
        .iter()
        .any(|suggestion| suggestion.action_kind == RecoveryActionKind::SubmitEvidence));
    assert!(recovery
        .recovery_suggestions
        .iter()
        .any(|suggestion| suggestion.action_kind == RecoveryActionKind::SubmitPlanPatch));
    assert!(recovery
        .recovery_suggestions
        .iter()
        .any(|suggestion| suggestion.action_kind == RecoveryActionKind::EscalateUserReview));
}

fn awaiting_evidence_checkpoint() -> CheckpointSnapshot {
    let mut lifecycle = RunLifecycleState::new(RunId("run-recovery-1".to_owned()), "mission-1");
    lifecycle.mark_running(RunPhase::Planning);
    lifecycle.bump_checkpoint();
    lifecycle.bump_plan_epoch();
    lifecycle.await_evidence(
        "quote is required before simulation",
        vec!["evidence.quote".to_owned()],
    );

    CheckpointSnapshot {
        run_id: "run-recovery-1".to_owned(),
        mission_id: "mission-1".to_owned(),
        checkpoint_seq: lifecycle.checkpoint_seq,
        plan_epoch: lifecycle.plan_epoch,
        lifecycle,
        action_graph: ActionGraph::default(),
        evidence_graph: EvidenceGraph {
            requirements: vec![EvidenceRequirement {
                requirement_id: "req-quote".to_owned(),
                reference: "evidence.quote".to_owned(),
                reason: "quote needed to continue guarded execution".to_owned(),
                required_by_node_id: Some("simulate-swap".to_owned()),
                satisfied_by_evidence_id: None,
            }],
            ..EvidenceGraph::default()
        },
        effect_contracts: Default::default(),
        pending_requests: PendingRequestsSnapshot {
            pending_evidence_refs: vec!["evidence.quote".to_owned()],
            ..PendingRequestsSnapshot::default()
        },
        last_completed_node_id: None,
        actuation_records: Vec::new(),
        execution_artifact: None,
    }
}

fn paused_patch_checkpoint() -> CheckpointSnapshot {
    let mut lifecycle = RunLifecycleState::new(RunId("run-recovery-2".to_owned()), "mission-2");
    lifecycle.mark_running(RunPhase::Governing);
    lifecycle.bump_checkpoint();
    lifecycle.bump_plan_epoch();
    lifecycle.pause("governor requested host plan patch");
    let mut failure = RunFailureContext::new(
        RunFailureCode::GovernorDenied,
        RunFailureStage::Govern,
        lifecycle.checkpoint_seq,
        lifecycle.plan_epoch,
        Some(StableBoundaryKind::Pause),
        "governor requested host plan patch",
    );
    failure.node_refs.push("govern.swap".to_owned());
    lifecycle.failure = Some(failure);

    CheckpointSnapshot {
        run_id: "run-recovery-2".to_owned(),
        mission_id: "mission-2".to_owned(),
        checkpoint_seq: lifecycle.checkpoint_seq,
        plan_epoch: lifecycle.plan_epoch,
        lifecycle,
        action_graph: ActionGraph::default(),
        evidence_graph: EvidenceGraph::default(),
        effect_contracts: Default::default(),
        pending_requests: PendingRequestsSnapshot::default(),
        last_completed_node_id: None,
        actuation_records: Vec::new(),
        execution_artifact: None,
    }
}

fn running_budget_interrupted_checkpoint() -> CheckpointSnapshot {
    let mut lifecycle = RunLifecycleState::new(RunId("run-recovery-0".to_owned()), "mission-0");
    lifecycle.mark_running(RunPhase::Planning);
    lifecycle.bump_checkpoint();
    lifecycle.record_interruption(
        InterruptionClass::StepBudgetExhausted,
        Some(RunFailureStage::Derive),
        None,
        "step budget exhausted after 1 transitions",
    );

    CheckpointSnapshot {
        run_id: "run-recovery-0".to_owned(),
        mission_id: "mission-0".to_owned(),
        checkpoint_seq: lifecycle.checkpoint_seq,
        plan_epoch: lifecycle.plan_epoch,
        lifecycle,
        action_graph: ActionGraph::default(),
        evidence_graph: EvidenceGraph::default(),
        effect_contracts: Default::default(),
        pending_requests: PendingRequestsSnapshot::default(),
        last_completed_node_id: None,
        actuation_records: Vec::new(),
        execution_artifact: None,
    }
}

fn paused_broadcast_uncertain_checkpoint() -> CheckpointSnapshot {
    let mut lifecycle = RunLifecycleState::new(RunId("run-recovery-5".to_owned()), "mission-5");
    lifecycle.mark_running(RunPhase::Broadcasting);
    lifecycle.bump_checkpoint();
    lifecycle.bump_plan_epoch();
    lifecycle.pause_with_failure(
        RunFailureStage::Broadcast,
        RunFailureCode::BroadcastUncertain,
        "rpc timed out after signed submission; chain status unknown",
    );
    if let Some(failure) = lifecycle.failure.as_mut() {
        failure.node_refs.push("broadcast.swap".to_owned());
        failure.confirmation_refs = vec!["broadcast-uncertain:broadcast.swap".to_owned()];
    }

    CheckpointSnapshot {
        run_id: "run-recovery-5".to_owned(),
        mission_id: "mission-5".to_owned(),
        checkpoint_seq: lifecycle.checkpoint_seq,
        plan_epoch: lifecycle.plan_epoch,
        lifecycle,
        action_graph: ActionGraph::default(),
        evidence_graph: EvidenceGraph::default(),
        effect_contracts: Default::default(),
        pending_requests: PendingRequestsSnapshot::default(),
        last_completed_node_id: None,
        actuation_records: Vec::new(),
        execution_artifact: None,
    }
}

fn paused_provider_failure_checkpoint() -> CheckpointSnapshot {
    let mut lifecycle = RunLifecycleState::new(RunId("run-recovery-6".to_owned()), "mission-6");
    lifecycle.mark_running(RunPhase::Verifying);
    lifecycle.bump_checkpoint();
    lifecycle.bump_plan_epoch();
    lifecycle.pause_with_failure(
        RunFailureStage::Confirm,
        RunFailureCode::ProviderUnavailable,
        "rpc provider returned 429 during confirmation lookup",
    );
    if let Some(failure) = lifecycle.failure.as_mut() {
        failure.node_refs.push("verify.swap".to_owned());
        failure.confirmation_refs = vec!["0xabc".to_owned()];
        failure.provider_error = Some(ais_agent_control::recovery::ProviderFailureInfo {
            provider: "evm.rpc".to_owned(),
            operation: "eth_getTransactionReceipt".to_owned(),
            code: Some("429".to_owned()),
            message: "rate limited".to_owned(),
            retryable: true,
        });
    }

    CheckpointSnapshot {
        run_id: "run-recovery-6".to_owned(),
        mission_id: "mission-6".to_owned(),
        checkpoint_seq: lifecycle.checkpoint_seq,
        plan_epoch: lifecycle.plan_epoch,
        lifecycle,
        action_graph: ActionGraph::default(),
        evidence_graph: EvidenceGraph::default(),
        effect_contracts: Default::default(),
        pending_requests: PendingRequestsSnapshot {
            pending_confirmation_id: Some("0xabc".to_owned()),
            ..PendingRequestsSnapshot::default()
        },
        last_completed_node_id: None,
        actuation_records: Vec::new(),
        execution_artifact: None,
    }
}

fn paused_confirmation_timeout_checkpoint() -> CheckpointSnapshot {
    let mut lifecycle = RunLifecycleState::new(RunId("run-recovery-6c".to_owned()), "mission-6c");
    lifecycle.mark_running(RunPhase::Verifying);
    lifecycle.bump_checkpoint();
    lifecycle.bump_plan_epoch();
    lifecycle.await_confirmation("waiting for chain receipt 0xabc");
    lifecycle.failure = Some(RunFailureContext::new(
        RunFailureCode::ConfirmationTimeout,
        RunFailureStage::Confirm,
        lifecycle.checkpoint_seq,
        lifecycle.plan_epoch,
        Some(StableBoundaryKind::Confirmation),
        "confirmation lookup timed out",
    ));
    lifecycle.record_interruption(
        InterruptionClass::ConfirmationWaitTimeout,
        Some(RunFailureStage::Confirm),
        Some(SideEffectPhase::AwaitingConfirmation),
        "confirmation lookup timed out",
    );
    if let Some(failure) = lifecycle.failure.as_mut() {
        failure.node_refs.push("verify.swap".to_owned());
        failure.confirmation_refs = vec!["0xabc".to_owned()];
    }

    CheckpointSnapshot {
        run_id: "run-recovery-6c".to_owned(),
        mission_id: "mission-6c".to_owned(),
        checkpoint_seq: lifecycle.checkpoint_seq,
        plan_epoch: lifecycle.plan_epoch,
        lifecycle,
        action_graph: ActionGraph::default(),
        evidence_graph: EvidenceGraph::default(),
        effect_contracts: Default::default(),
        pending_requests: PendingRequestsSnapshot {
            pending_confirmation_id: Some("0xabc".to_owned()),
            ..PendingRequestsSnapshot::default()
        },
        last_completed_node_id: None,
        actuation_records: Vec::new(),
        execution_artifact: None,
    }
}

fn paused_observe_timeout_checkpoint() -> CheckpointSnapshot {
    let mut lifecycle = RunLifecycleState::new(RunId("run-recovery-6b".to_owned()), "mission-6b");
    lifecycle.mark_running(RunPhase::Planning);
    lifecycle.bump_checkpoint();
    lifecycle.pause_with_failure(
        RunFailureStage::Observe,
        RunFailureCode::ProviderUnavailable,
        "rpc provider timed out during eth_call",
    );
    lifecycle.record_interruption(
        InterruptionClass::ProviderTimeout,
        Some(RunFailureStage::Observe),
        None,
        "rpc provider timed out during eth_call",
    );
    if let Some(failure) = lifecycle.failure.as_mut() {
        failure.node_refs.push("observe.swap".to_owned());
        failure.provider_error = Some(ais_agent_control::recovery::ProviderFailureInfo {
            provider: "evm.rpc".to_owned(),
            operation: "eth_call".to_owned(),
            code: None,
            message: "timed out".to_owned(),
            retryable: true,
        });
    }

    CheckpointSnapshot {
        run_id: "run-recovery-6b".to_owned(),
        mission_id: "mission-6b".to_owned(),
        checkpoint_seq: lifecycle.checkpoint_seq,
        plan_epoch: lifecycle.plan_epoch,
        lifecycle,
        action_graph: ActionGraph::default(),
        evidence_graph: EvidenceGraph::default(),
        effect_contracts: Default::default(),
        pending_requests: PendingRequestsSnapshot::default(),
        last_completed_node_id: None,
        actuation_records: Vec::new(),
        execution_artifact: None,
    }
}

fn failed_checkpoint() -> CheckpointSnapshot {
    let mut lifecycle = RunLifecycleState::new(RunId("run-recovery-3".to_owned()), "mission-3");
    lifecycle.mark_running(RunPhase::Verifying);
    lifecycle.bump_checkpoint();
    lifecycle.bump_plan_epoch();
    lifecycle.fail(
        RunFailureStage::Verify,
        RunFailureCode::VerifyMismatch,
        "post-state balance mismatch",
    );
    if let Some(failure) = lifecycle.failure.as_mut() {
        failure.effect_refs = vec!["effects.swap".to_owned()];
    }

    CheckpointSnapshot {
        run_id: "run-recovery-3".to_owned(),
        mission_id: "mission-3".to_owned(),
        checkpoint_seq: lifecycle.checkpoint_seq,
        plan_epoch: lifecycle.plan_epoch,
        lifecycle,
        action_graph: ActionGraph::default(),
        evidence_graph: EvidenceGraph::default(),
        effect_contracts: Default::default(),
        pending_requests: PendingRequestsSnapshot::default(),
        last_completed_node_id: None,
        actuation_records: Vec::new(),
        execution_artifact: None,
    }
}

fn paused_verify_mismatch_checkpoint() -> CheckpointSnapshot {
    let mut lifecycle = RunLifecycleState::new(RunId("run-recovery-7".to_owned()), "mission-7");
    lifecycle.mark_running(RunPhase::Verifying);
    lifecycle.bump_checkpoint();
    lifecycle.bump_plan_epoch();
    lifecycle.pause_with_failure(
        RunFailureStage::Verify,
        RunFailureCode::VerifyMismatch,
        "post-state balance mismatch",
    );
    if let Some(failure) = lifecycle.failure.as_mut() {
        failure.node_refs.push("verify.swap".to_owned());
        failure.effect_refs.push("effect.swap".to_owned());
        failure
            .actuation_refs
            .push("broadcast.swap:broadcast_submitted:1".to_owned());
        failure.confirmation_refs.push("0xdef".to_owned());
        failure.evidence_refs = vec![
            "receipt.verify.swap".to_owned(),
            "post.verify.swap".to_owned(),
        ];
    }

    CheckpointSnapshot {
        run_id: "run-recovery-7".to_owned(),
        mission_id: "mission-7".to_owned(),
        checkpoint_seq: lifecycle.checkpoint_seq,
        plan_epoch: lifecycle.plan_epoch,
        lifecycle,
        action_graph: ActionGraph::default(),
        evidence_graph: EvidenceGraph::default(),
        effect_contracts: Default::default(),
        pending_requests: PendingRequestsSnapshot {
            pending_confirmation_id: Some("0xdef".to_owned()),
            ..PendingRequestsSnapshot::default()
        },
        last_completed_node_id: None,
        actuation_records: Vec::new(),
        execution_artifact: None,
    }
}

fn paused_envelope_checkpoint() -> CheckpointSnapshot {
    let mut lifecycle = RunLifecycleState::new(RunId("run-recovery-4".to_owned()), "mission-4");
    lifecycle.mark_running(RunPhase::Broadcasting);
    lifecycle.bump_checkpoint();
    lifecycle.bump_plan_epoch();
    lifecycle.pause_with_failure(
        RunFailureStage::Broadcast,
        RunFailureCode::EnvelopeInvalid,
        "replacement envelope required",
    );
    if let Some(failure) = lifecycle.failure.as_mut() {
        failure.node_refs.push("swap".to_owned());
    }
    if let Some(boundary) = lifecycle.active_boundary.as_mut() {
        boundary.blocking_refs = vec!["env.swap".to_owned()];
    }

    CheckpointSnapshot {
        run_id: "run-recovery-4".to_owned(),
        mission_id: "mission-4".to_owned(),
        checkpoint_seq: lifecycle.checkpoint_seq,
        plan_epoch: lifecycle.plan_epoch,
        lifecycle,
        action_graph: ActionGraph::default(),
        evidence_graph: EvidenceGraph::default(),
        effect_contracts: Default::default(),
        pending_requests: PendingRequestsSnapshot {
            pending_envelope_refs: vec!["env.swap".to_owned()],
            ..PendingRequestsSnapshot::default()
        },
        last_completed_node_id: None,
        actuation_records: Vec::new(),
        execution_artifact: None,
    }
}

fn cancelled_checkpoint() -> CheckpointSnapshot {
    let mut lifecycle = RunLifecycleState::new(RunId("run-recovery-8".to_owned()), "mission-8");
    lifecycle.mark_running(RunPhase::Verifying);
    lifecycle.bump_checkpoint();
    lifecycle.bump_plan_epoch();
    lifecycle.cancel("cancelled by host");

    CheckpointSnapshot {
        run_id: "run-recovery-8".to_owned(),
        mission_id: "mission-8".to_owned(),
        checkpoint_seq: lifecycle.checkpoint_seq,
        plan_epoch: lifecycle.plan_epoch,
        lifecycle,
        action_graph: ActionGraph::default(),
        evidence_graph: EvidenceGraph::default(),
        effect_contracts: Default::default(),
        pending_requests: PendingRequestsSnapshot::default(),
        last_completed_node_id: None,
        actuation_records: Vec::new(),
        execution_artifact: None,
    }
}

fn cancel_pending_confirmation_checkpoint() -> CheckpointSnapshot {
    let mut lifecycle = RunLifecycleState::new(RunId("run-recovery-8b".to_owned()), "mission-8b");
    lifecycle.mark_running(RunPhase::Verifying);
    lifecycle.bump_checkpoint();
    lifecycle.bump_plan_epoch();
    lifecycle.await_confirmation("waiting for chain receipt 0xdef");
    lifecycle.request_cancel_pending(
        "cancel after submission",
        Some(SideEffectPhase::AwaitingConfirmation),
    );

    CheckpointSnapshot {
        run_id: "run-recovery-8b".to_owned(),
        mission_id: "mission-8b".to_owned(),
        checkpoint_seq: lifecycle.checkpoint_seq,
        plan_epoch: lifecycle.plan_epoch,
        lifecycle,
        action_graph: ActionGraph::default(),
        evidence_graph: EvidenceGraph::default(),
        effect_contracts: Default::default(),
        pending_requests: PendingRequestsSnapshot {
            pending_confirmation_id: Some("0xdef".to_owned()),
            ..PendingRequestsSnapshot::default()
        },
        last_completed_node_id: None,
        actuation_records: Vec::new(),
        execution_artifact: None,
    }
}
