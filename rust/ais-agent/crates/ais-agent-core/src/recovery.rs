use std::collections::BTreeSet;

use ais_agent_control::{
    commands::RetryIntent,
    recovery::{
        CancelState, InterruptionClass, RecoveryActionKind, RecoveryDisposition,
        RecoveryInputRequirement, RecoveryPriority, RecoverySuggestion, RunFailureCode,
        RunFailureContext, RunFailureStage, SideEffectPhase,
    },
};

use crate::{actuation::ActuationKind, checkpoint::CheckpointSnapshot, runtime::RunStatus};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RecoveryProjection {
    pub recovery_disposition: Option<RecoveryDisposition>,
    pub failure_context: Option<RunFailureContext>,
    pub recovery_suggestions: Vec<RecoverySuggestion>,
    pub allowed_recovery_actions: Vec<RecoveryActionKind>,
    pub interruption_class: Option<InterruptionClass>,
    pub cancel_state: Option<CancelState>,
    pub side_effect_phase: Option<SideEffectPhase>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CancelRequestResolution {
    CancelImmediately,
    CancelPending,
    Reject(String),
}

pub fn classify_recovery_view(checkpoint: &CheckpointSnapshot) -> RecoveryProjection {
    RecoveryProjection {
        recovery_disposition: classify_recovery_disposition(checkpoint),
        failure_context: checkpoint.lifecycle.failure.clone(),
        recovery_suggestions: classify_recovery_suggestions(checkpoint),
        allowed_recovery_actions: classify_allowed_recovery_actions(checkpoint),
        interruption_class: classify_interruption_class(checkpoint),
        cancel_state: classify_cancel_state(checkpoint),
        side_effect_phase: classify_side_effect_phase(checkpoint),
    }
}

pub fn classify_interruption_class(checkpoint: &CheckpointSnapshot) -> Option<InterruptionClass> {
    if let Some(interruption) = checkpoint.lifecycle.interruption.as_ref() {
        return Some(interruption.class.clone());
    }
    let failure = checkpoint.lifecycle.failure.as_ref()?;
    match (&failure.code, &failure.stage) {
        (RunFailureCode::BudgetExhausted, _) => Some(InterruptionClass::StepBudgetExhausted),
        (RunFailureCode::ProviderUnavailable, _) => {
            Some(provider_interruption_class(&failure.summary))
        }
        (RunFailureCode::ConfirmationTimeout, RunFailureStage::Verify) => {
            Some(InterruptionClass::VerifyWaitTimeout)
        }
        (RunFailureCode::ConfirmationTimeout, _) => {
            Some(InterruptionClass::ConfirmationWaitTimeout)
        }
        (RunFailureCode::BroadcastUncertain, _) => {
            Some(InterruptionClass::BroadcastOutcomeUncertain)
        }
        (RunFailureCode::CancelRequested, _) => Some(InterruptionClass::HostCancelRequested),
        (RunFailureCode::RuntimeInvariantViolation, RunFailureStage::Recover)
            if failure.summary.contains("stall") =>
        {
            Some(InterruptionClass::RuntimeStallDetected)
        }
        _ => None,
    }
}

pub fn classify_cancel_state(checkpoint: &CheckpointSnapshot) -> Option<CancelState> {
    if checkpoint.lifecycle.cancel_state.is_some() {
        return checkpoint.lifecycle.cancel_state.clone();
    }
    match checkpoint.lifecycle.status {
        RunStatus::Cancelled => Some(CancelState::Cancelled),
        _ if matches!(
            checkpoint
                .lifecycle
                .failure
                .as_ref()
                .map(|failure| &failure.code),
            Some(RunFailureCode::CancelRequested)
        ) =>
        {
            Some(CancelState::Requested)
        }
        _ => None,
    }
}

pub fn classify_cancel_request(checkpoint: &CheckpointSnapshot) -> CancelRequestResolution {
    match checkpoint.lifecycle.status {
        RunStatus::Completed => {
            return CancelRequestResolution::Reject("run is already completed".to_owned())
        }
        RunStatus::Failed => {
            return CancelRequestResolution::Reject("run is already failed".to_owned())
        }
        RunStatus::Cancelled => {
            return CancelRequestResolution::Reject("run is already cancelled".to_owned())
        }
        RunStatus::Created
        | RunStatus::Running
        | RunStatus::Paused
        | RunStatus::AwaitingEvidence
        | RunStatus::AwaitingSigner
        | RunStatus::AwaitingConfirmation
        | RunStatus::AwaitingArtifactContinuation => {}
    }

    if matches!(
        checkpoint.lifecycle.cancel_state,
        Some(CancelState::Pending | CancelState::Requested)
    ) {
        return CancelRequestResolution::CancelPending;
    }
    if matches!(
        checkpoint.lifecycle.cancel_state,
        Some(CancelState::Rejected)
    ) {
        return CancelRequestResolution::Reject(
            "cancel request is not allowed for this run".to_owned(),
        );
    }

    let side_effect_phase = classify_side_effect_phase(checkpoint);
    if checkpoint
        .pending_requests
        .pending_confirmation_id
        .is_some()
        || matches!(
            side_effect_phase,
            Some(SideEffectPhase::BroadcastSubmitted | SideEffectPhase::AwaitingConfirmation)
        )
    {
        return CancelRequestResolution::CancelPending;
    }

    CancelRequestResolution::CancelImmediately
}

pub fn classify_side_effect_phase(checkpoint: &CheckpointSnapshot) -> Option<SideEffectPhase> {
    if let Some(interruption) = checkpoint.lifecycle.interruption.as_ref() {
        if interruption.side_effect_phase.is_some() {
            return interruption.side_effect_phase.clone();
        }
    }
    if matches!(
        checkpoint
            .lifecycle
            .failure
            .as_ref()
            .map(|failure| &failure.code),
        Some(RunFailureCode::BroadcastUncertain)
    ) {
        return Some(SideEffectPhase::BroadcastSubmitted);
    }
    if checkpoint.lifecycle.status == RunStatus::Completed
        && !checkpoint.actuation_records.is_empty()
    {
        return Some(SideEffectPhase::Verified);
    }
    if checkpoint
        .pending_requests
        .pending_confirmation_id
        .is_some()
    {
        return Some(SideEffectPhase::AwaitingConfirmation);
    }
    if checkpoint
        .pending_requests
        .pending_signer_request_id
        .is_some()
    {
        return Some(SideEffectPhase::AwaitingSigner);
    }
    checkpoint
        .actuation_records
        .iter()
        .rev()
        .find_map(|record| match record.kind {
            ActuationKind::ReceiptObserved => Some(SideEffectPhase::ReceiptObserved),
            ActuationKind::BroadcastSubmitted | ActuationKind::ExternalJobSubmitted => {
                Some(SideEffectPhase::BroadcastSubmitted)
            }
            ActuationKind::SignerRequested => Some(SideEffectPhase::AwaitingSigner),
            ActuationKind::EnvelopeBuilt => Some(SideEffectPhase::EnvelopePrepared),
        })
}

pub fn classify_recovery_disposition(
    checkpoint: &CheckpointSnapshot,
) -> Option<RecoveryDisposition> {
    let lifecycle = &checkpoint.lifecycle;
    let failure = lifecycle.failure.as_ref();
    let interruption = lifecycle.interruption.as_ref();

    if matches!(
        interruption.map(|interruption| &interruption.class),
        Some(InterruptionClass::StepBudgetExhausted | InterruptionClass::WallClockBudgetExhausted)
    ) && lifecycle.status == RunStatus::Running
    {
        return Some(RecoveryDisposition::ContinueWait);
    }

    match lifecycle.status {
        RunStatus::AwaitingEvidence => Some(RecoveryDisposition::AwaitEvidence),
        RunStatus::AwaitingSigner => Some(RecoveryDisposition::AwaitSigner),
        RunStatus::AwaitingConfirmation => Some(match failure.map(|failure| &failure.code) {
            Some(RunFailureCode::ConfirmationTimeout | RunFailureCode::ProviderUnavailable) => {
                RecoveryDisposition::RetryReady
            }
            _ => RecoveryDisposition::ContinueWait,
        }),
        RunStatus::AwaitingArtifactContinuation => Some(RecoveryDisposition::AwaitContinuation),
        RunStatus::Paused => Some(
            if matches!(
                interruption.map(|interruption| &interruption.class),
                Some(InterruptionClass::RecoveryRetryReady)
            ) && failure.is_none()
            {
                RecoveryDisposition::RetryReady
            } else {
                match failure.map(|failure| &failure.code) {
                    Some(
                        RunFailureCode::SimulationRejected
                        | RunFailureCode::GovernorDenied
                        | RunFailureCode::SignerDenied
                        | RunFailureCode::SignerExpired
                        | RunFailureCode::VerifyMismatch,
                    ) => RecoveryDisposition::AwaitPatch,
                    Some(RunFailureCode::BroadcastUncertain) => RecoveryDisposition::AwaitUserInput,
                    Some(RunFailureCode::EnvelopeInvalid)
                        if !checkpoint.pending_requests.pending_envelope_refs.is_empty() =>
                    {
                        RecoveryDisposition::AwaitEnvelope
                    }
                    Some(RunFailureCode::EnvelopeInvalid) => RecoveryDisposition::AwaitPatch,
                    Some(
                        RunFailureCode::ConfirmationTimeout
                        | RunFailureCode::ProviderUnavailable
                        | RunFailureCode::BudgetExhausted,
                    ) => RecoveryDisposition::RetryReady,
                    Some(RunFailureCode::CancelRequested) => RecoveryDisposition::AbortOnly,
                    Some(
                        RunFailureCode::RuntimeInvariantViolation
                        | RunFailureCode::BroadcastRejected,
                    ) => RecoveryDisposition::FailedClosed,
                    Some(RunFailureCode::MissingEvidence | RunFailureCode::StaleEvidence) => {
                        RecoveryDisposition::AwaitEvidence
                    }
                    None => RecoveryDisposition::AwaitUserInput,
                }
            },
        ),
        RunStatus::Failed => Some(RecoveryDisposition::FailedClosed),
        RunStatus::Cancelled => Some(RecoveryDisposition::AbortOnly),
        RunStatus::Created | RunStatus::Running | RunStatus::Completed => None,
    }
}

pub fn classify_allowed_recovery_actions(
    checkpoint: &CheckpointSnapshot,
) -> Vec<RecoveryActionKind> {
    let cancel_state = classify_cancel_state(checkpoint);
    let mut actions = match classify_recovery_disposition(checkpoint) {
        Some(RecoveryDisposition::AwaitEvidence) => vec![
            RecoveryActionKind::SubmitEvidence,
            RecoveryActionKind::CancelRun,
        ],
        Some(RecoveryDisposition::AwaitEnvelope) => vec![
            RecoveryActionKind::SubmitEnvelope,
            RecoveryActionKind::CancelRun,
        ],
        Some(RecoveryDisposition::AwaitSigner) => vec![
            RecoveryActionKind::SubmitSignerResolution,
            RecoveryActionKind::CancelRun,
        ],
        Some(RecoveryDisposition::AwaitContinuation) => vec![
            RecoveryActionKind::SubmitExecutionArtifactContinuation,
            RecoveryActionKind::CancelRun,
        ],
        Some(RecoveryDisposition::ContinueWait) => {
            let mut actions = vec![RecoveryActionKind::RetryStep];
            if checkpoint
                .pending_requests
                .pending_confirmation_id
                .is_some()
            {
                actions.push(RecoveryActionKind::AwaitConfirmation);
            }
            actions.push(RecoveryActionKind::CancelRun);
            actions
        }
        Some(RecoveryDisposition::AwaitPatch)
            if matches!(
                checkpoint
                    .lifecycle
                    .failure
                    .as_ref()
                    .map(|failure| &failure.code),
                Some(RunFailureCode::VerifyMismatch)
            ) =>
        {
            vec![
                RecoveryActionKind::SubmitEvidence,
                RecoveryActionKind::SubmitPlanPatch,
                RecoveryActionKind::EscalateUserReview,
                RecoveryActionKind::CancelRun,
            ]
        }
        Some(RecoveryDisposition::AwaitPatch) => {
            vec![
                RecoveryActionKind::SubmitPlanPatch,
                RecoveryActionKind::CancelRun,
            ]
        }
        Some(RecoveryDisposition::AwaitUserInput) => vec![
            RecoveryActionKind::EscalateUserReview,
            RecoveryActionKind::CancelRun,
        ],
        Some(RecoveryDisposition::RetryReady) => {
            vec![RecoveryActionKind::RetryStep, RecoveryActionKind::CancelRun]
        }
        Some(RecoveryDisposition::AbortOnly) => vec![RecoveryActionKind::CancelRun],
        Some(RecoveryDisposition::FailedClosed) => vec![RecoveryActionKind::CancelRun],
        None => Vec::new(),
    };

    match cancel_state {
        Some(CancelState::Cancelled) => Vec::new(),
        Some(CancelState::Pending | CancelState::Requested) => {
            actions.retain(|action| action != &RecoveryActionKind::CancelRun);
            actions
        }
        Some(CancelState::Rejected) | None => actions,
    }
}

pub fn provider_interruption_class(reason: &str) -> InterruptionClass {
    let reason = reason.to_ascii_lowercase();
    if [
        "timeout",
        "timed out",
        "deadline exceeded",
        "context deadline exceeded",
    ]
    .iter()
    .any(|needle| reason.contains(needle))
    {
        return InterruptionClass::ProviderTimeout;
    }
    InterruptionClass::ProviderUnavailable
}

pub fn classify_recovery_suggestions(checkpoint: &CheckpointSnapshot) -> Vec<RecoverySuggestion> {
    let disposition = classify_recovery_disposition(checkpoint);
    let cancel_state = classify_cancel_state(checkpoint);
    let failure = checkpoint.lifecycle.failure.as_ref();
    let basis_checkpoint_seq = checkpoint.checkpoint_seq;
    let basis_plan_epoch = checkpoint.plan_epoch;

    if matches!(cancel_state, Some(CancelState::Cancelled)) {
        return Vec::new();
    }

    match disposition {
        Some(RecoveryDisposition::AwaitEvidence) => {
            let evidence_refs = checkpoint.pending_requests.pending_evidence_refs.clone();
            let required_inputs = checkpoint
                .evidence_graph
                .requirements
                .iter()
                .filter(|requirement| {
                    requirement.satisfied_by_evidence_id.is_none()
                        || evidence_refs.contains(&requirement.reference)
                })
                .map(|requirement| RecoveryInputRequirement {
                    key: "evidence_ref".to_owned(),
                    value: Some(requirement.reference.clone()),
                    description: requirement.reason.clone(),
                })
                .collect();
            vec![RecoverySuggestion {
                suggestion_id: format!(
                    "{}:recovery:{}:submit_evidence",
                    checkpoint.run_id, basis_checkpoint_seq
                ),
                action_kind: RecoveryActionKind::SubmitEvidence,
                reason_code: failure
                    .map(|failure| failure.code.clone())
                    .unwrap_or(RunFailureCode::MissingEvidence),
                priority: RecoveryPriority::Automatic,
                basis_checkpoint_seq,
                basis_plan_epoch,
                retry_intent: None,
                target_refs: evidence_refs,
                required_inputs,
                constraints: Vec::new(),
            }]
        }
        Some(RecoveryDisposition::AwaitEnvelope) => checkpoint
            .pending_requests
            .pending_envelope_refs
            .iter()
            .cloned()
            .map(|envelope_ref| RecoverySuggestion {
                suggestion_id: format!(
                    "{}:recovery:{}:submit_envelope",
                    checkpoint.run_id, basis_checkpoint_seq
                ),
                action_kind: RecoveryActionKind::SubmitEnvelope,
                reason_code: failure
                    .map(|failure| failure.code.clone())
                    .unwrap_or(RunFailureCode::EnvelopeInvalid),
                priority: RecoveryPriority::HostReview,
                basis_checkpoint_seq,
                basis_plan_epoch,
                retry_intent: None,
                target_refs: vec![envelope_ref.clone()],
                required_inputs: vec![RecoveryInputRequirement {
                    key: "envelope_ref".to_owned(),
                    value: Some(envelope_ref),
                    description: "submit a replacement envelope for the blocked actuation"
                        .to_owned(),
                }],
                constraints: Vec::new(),
            })
            .collect(),
        Some(RecoveryDisposition::AwaitSigner) => checkpoint
            .pending_requests
            .pending_signer_request_id
            .clone()
            .into_iter()
            .map(|request_id| RecoverySuggestion {
                suggestion_id: format!(
                    "{}:recovery:{}:submit_signer_resolution",
                    checkpoint.run_id, basis_checkpoint_seq
                ),
                action_kind: RecoveryActionKind::SubmitSignerResolution,
                reason_code: failure
                    .map(|failure| failure.code.clone())
                    .unwrap_or(RunFailureCode::SignerDenied),
                priority: RecoveryPriority::HostReview,
                basis_checkpoint_seq,
                basis_plan_epoch,
                retry_intent: None,
                target_refs: vec![request_id],
                required_inputs: vec![RecoveryInputRequirement {
                    key: "signer_request_id".to_owned(),
                    value: checkpoint
                        .pending_requests
                        .pending_signer_request_id
                        .clone(),
                    description: "resolve the pending signer request".to_owned(),
                }],
                constraints: Vec::new(),
            })
            .collect(),
        Some(RecoveryDisposition::AwaitContinuation) => Vec::new(),
        Some(RecoveryDisposition::ContinueWait) => {
            let pending_confirmation_refs = checkpoint
                .pending_requests
                .pending_confirmation_id
                .clone()
                .into_iter()
                .collect::<Vec<_>>();
            let retry_reason_code =
                failure
                    .map(|failure| failure.code.clone())
                    .unwrap_or_else(|| match classify_interruption_class(checkpoint) {
                        Some(
                            InterruptionClass::StepBudgetExhausted
                            | InterruptionClass::WallClockBudgetExhausted,
                        ) => RunFailureCode::BudgetExhausted,
                        _ => RunFailureCode::ConfirmationTimeout,
                    });
            let mut suggestions = vec![RecoverySuggestion {
                suggestion_id: format!(
                    "{}:recovery:{}:retry_step",
                    checkpoint.run_id, basis_checkpoint_seq
                ),
                action_kind: RecoveryActionKind::RetryStep,
                reason_code: retry_reason_code,
                priority: RecoveryPriority::Automatic,
                basis_checkpoint_seq,
                basis_plan_epoch,
                retry_intent: Some(RetryIntent::ResumeExecution),
                target_refs: pending_confirmation_refs.clone(),
                required_inputs: Vec::new(),
                constraints: Vec::new(),
            }];
            if !pending_confirmation_refs.is_empty() {
                suggestions.push(RecoverySuggestion {
                    suggestion_id: format!(
                        "{}:recovery:{}:await_confirmation",
                        checkpoint.run_id, basis_checkpoint_seq
                    ),
                    action_kind: RecoveryActionKind::AwaitConfirmation,
                    reason_code: failure
                        .map(|failure| failure.code.clone())
                        .unwrap_or(RunFailureCode::ConfirmationTimeout),
                    priority: RecoveryPriority::Automatic,
                    basis_checkpoint_seq,
                    basis_plan_epoch,
                    retry_intent: Some(RetryIntent::PollConfirmation),
                    target_refs: pending_confirmation_refs,
                    required_inputs: Vec::new(),
                    constraints: Vec::new(),
                });
            }
            suggestions
        }
        Some(RecoveryDisposition::AwaitPatch)
            if matches!(
                failure.map(|failure| &failure.code),
                Some(RunFailureCode::VerifyMismatch)
            ) =>
        {
            let failure = failure.expect("verified by match guard");
            let evidence_targets = if failure.evidence_refs.is_empty() {
                failure.confirmation_refs.clone()
            } else {
                failure.evidence_refs.clone()
            };
            let mut suggestions = vec![RecoverySuggestion {
                suggestion_id: format!(
                    "{}:recovery:{}:submit_plan_patch",
                    checkpoint.run_id, basis_checkpoint_seq
                ),
                action_kind: RecoveryActionKind::SubmitPlanPatch,
                reason_code: failure.code.clone(),
                priority: RecoveryPriority::HostReview,
                basis_checkpoint_seq,
                basis_plan_epoch,
                retry_intent: None,
                target_refs: if failure.node_refs.is_empty() {
                    failure.effect_refs.clone()
                } else {
                    failure.node_refs.clone()
                },
                required_inputs: Vec::new(),
                constraints: Vec::new(),
            }];
            if !evidence_targets.is_empty() {
                suggestions.push(RecoverySuggestion {
                    suggestion_id: format!(
                        "{}:recovery:{}:submit_evidence",
                        checkpoint.run_id, basis_checkpoint_seq
                    ),
                    action_kind: RecoveryActionKind::SubmitEvidence,
                    reason_code: failure.code.clone(),
                    priority: RecoveryPriority::HostReview,
                    basis_checkpoint_seq,
                    basis_plan_epoch,
                    retry_intent: None,
                    target_refs: evidence_targets.clone(),
                    required_inputs: evidence_targets
                        .iter()
                        .cloned()
                        .map(|reference| RecoveryInputRequirement {
                            key: "evidence_ref".to_owned(),
                            value: Some(reference),
                            description:
                                "supply fresh verification evidence or post-state observations"
                                    .to_owned(),
                        })
                        .collect(),
                    constraints: Vec::new(),
                });
            }
            suggestions.push(RecoverySuggestion {
                suggestion_id: format!(
                    "{}:recovery:{}:escalate_user_review",
                    checkpoint.run_id, basis_checkpoint_seq
                ),
                action_kind: RecoveryActionKind::EscalateUserReview,
                reason_code: failure.code.clone(),
                priority: RecoveryPriority::UserReview,
                basis_checkpoint_seq,
                basis_plan_epoch,
                retry_intent: None,
                target_refs: failure.confirmation_refs.clone(),
                required_inputs: Vec::new(),
                constraints: Vec::new(),
            });
            suggestions
        }
        Some(RecoveryDisposition::AwaitPatch) => vec![RecoverySuggestion {
            suggestion_id: format!(
                "{}:recovery:{}:submit_plan_patch",
                checkpoint.run_id, basis_checkpoint_seq
            ),
            action_kind: RecoveryActionKind::SubmitPlanPatch,
            reason_code: failure
                .map(|failure| failure.code.clone())
                .unwrap_or(RunFailureCode::GovernorDenied),
            priority: RecoveryPriority::HostReview,
            basis_checkpoint_seq,
            basis_plan_epoch,
            retry_intent: None,
            target_refs: failure_target_refs(failure),
            required_inputs: Vec::new(),
            constraints: Vec::new(),
        }],
        Some(RecoveryDisposition::AwaitUserInput) => vec![RecoverySuggestion {
            suggestion_id: format!(
                "{}:recovery:{}:escalate_user_review",
                checkpoint.run_id, basis_checkpoint_seq
            ),
            action_kind: RecoveryActionKind::EscalateUserReview,
            reason_code: failure
                .map(|failure| failure.code.clone())
                .unwrap_or(RunFailureCode::GovernorDenied),
            priority: RecoveryPriority::UserReview,
            basis_checkpoint_seq,
            basis_plan_epoch,
            retry_intent: None,
            target_refs: failure_target_refs(failure),
            required_inputs: Vec::new(),
            constraints: Vec::new(),
        }],
        Some(RecoveryDisposition::RetryReady) => vec![RecoverySuggestion {
            suggestion_id: format!(
                "{}:recovery:{}:retry_step",
                checkpoint.run_id, basis_checkpoint_seq
            ),
            action_kind: RecoveryActionKind::RetryStep,
            reason_code: failure
                .map(|failure| failure.code.clone())
                .unwrap_or(RunFailureCode::ProviderUnavailable),
            priority: RecoveryPriority::Automatic,
            basis_checkpoint_seq,
            basis_plan_epoch,
            retry_intent: Some(RetryIntent::ResumeExecution),
            target_refs: confirmation_or_failure_refs(checkpoint, failure),
            required_inputs: Vec::new(),
            constraints: Vec::new(),
        }],
        Some(RecoveryDisposition::AbortOnly) => vec![RecoverySuggestion {
            suggestion_id: format!(
                "{}:recovery:{}:cancel_run",
                checkpoint.run_id, basis_checkpoint_seq
            ),
            action_kind: RecoveryActionKind::CancelRun,
            reason_code: failure
                .map(|failure| failure.code.clone())
                .unwrap_or(RunFailureCode::CancelRequested),
            priority: RecoveryPriority::HostReview,
            basis_checkpoint_seq,
            basis_plan_epoch,
            retry_intent: None,
            target_refs: Vec::new(),
            required_inputs: Vec::new(),
            constraints: Vec::new(),
        }],
        Some(RecoveryDisposition::FailedClosed) => vec![RecoverySuggestion {
            suggestion_id: format!(
                "{}:recovery:{}:cancel_run",
                checkpoint.run_id, basis_checkpoint_seq
            ),
            action_kind: RecoveryActionKind::CancelRun,
            reason_code: failure
                .map(|failure| failure.code.clone())
                .unwrap_or(RunFailureCode::RuntimeInvariantViolation),
            priority: RecoveryPriority::HostReview,
            basis_checkpoint_seq,
            basis_plan_epoch,
            retry_intent: None,
            target_refs: Vec::new(),
            required_inputs: Vec::new(),
            constraints: Vec::new(),
        }],
        None => Vec::new(),
    }
}

fn confirmation_or_failure_refs(
    checkpoint: &CheckpointSnapshot,
    failure: Option<&RunFailureContext>,
) -> Vec<String> {
    let refs = failure_target_refs(failure);
    if !refs.is_empty() {
        return refs;
    }

    checkpoint
        .pending_requests
        .pending_confirmation_id
        .clone()
        .into_iter()
        .collect()
}

fn failure_target_refs(failure: Option<&RunFailureContext>) -> Vec<String> {
    let Some(failure) = failure else {
        return Vec::new();
    };

    let mut refs = BTreeSet::new();
    refs.extend(failure.node_refs.iter().cloned());
    refs.extend(failure.evidence_refs.iter().cloned());
    refs.extend(failure.effect_refs.iter().cloned());
    refs.extend(failure.actuation_refs.iter().cloned());
    refs.extend(failure.confirmation_refs.iter().cloned());
    refs.into_iter().collect()
}
