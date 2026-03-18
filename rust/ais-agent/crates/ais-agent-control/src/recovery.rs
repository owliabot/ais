use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{commands::RetryIntent, ids::SignerRequestId};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StableBoundaryKind {
    Pause,
    Evidence,
    Signer,
    Confirmation,
    ArtifactContinuation,
    Completion,
    Failure,
    Cancellation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterruptionClass {
    StepBudgetExhausted,
    WallClockBudgetExhausted,
    RecoveryRetryReady,
    ProviderTimeout,
    ProviderUnavailable,
    ConfirmationWaitTimeout,
    VerifyWaitTimeout,
    BroadcastOutcomeUncertain,
    HostCancelRequested,
    RuntimeStallDetected,
    ProcessRestart,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CancelState {
    Requested,
    Pending,
    Rejected,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SideEffectPhase {
    EnvelopePrepared,
    AwaitingSigner,
    BroadcastSubmitted,
    AwaitingConfirmation,
    ReceiptObserved,
    Verified,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterruptionState {
    pub class: InterruptionClass,
    pub stage: Option<RunFailureStage>,
    pub side_effect_phase: Option<SideEffectPhase>,
    pub summary: String,
}

impl InterruptionState {
    pub fn validate(&self) -> Result<(), String> {
        if self.summary.trim().is_empty() {
            return Err("interruption_state.summary must not be empty".to_owned());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunFailureCode {
    MissingEvidence,
    StaleEvidence,
    SimulationRejected,
    GovernorDenied,
    SignerDenied,
    SignerExpired,
    EnvelopeInvalid,
    BroadcastRejected,
    BroadcastUncertain,
    ConfirmationTimeout,
    VerifyMismatch,
    ProviderUnavailable,
    BudgetExhausted,
    CancelRequested,
    RuntimeInvariantViolation,
}

impl RunFailureCode {
    pub fn default_severity(&self) -> RunFailureSeverity {
        match self {
            Self::MissingEvidence
            | Self::StaleEvidence
            | Self::SimulationRejected
            | Self::GovernorDenied
            | Self::SignerDenied
            | Self::SignerExpired
            | Self::EnvelopeInvalid
            | Self::VerifyMismatch => RunFailureSeverity::PatchRequired,
            Self::BroadcastUncertain
            | Self::ConfirmationTimeout
            | Self::ProviderUnavailable
            | Self::BudgetExhausted => RunFailureSeverity::Retryable,
            Self::CancelRequested => RunFailureSeverity::ManualReview,
            Self::BroadcastRejected | Self::RuntimeInvariantViolation => {
                RunFailureSeverity::TerminalClosed
            }
        }
    }

    pub fn default_blame_surface(&self) -> RunFailureBlameSurface {
        match self {
            Self::MissingEvidence | Self::StaleEvidence => RunFailureBlameSurface::Evidence,
            Self::SimulationRejected | Self::EnvelopeInvalid | Self::VerifyMismatch => {
                RunFailureBlameSurface::Fragment
            }
            Self::GovernorDenied | Self::BudgetExhausted => RunFailureBlameSurface::Policy,
            Self::SignerDenied | Self::SignerExpired => RunFailureBlameSurface::Signer,
            Self::BroadcastRejected
            | Self::BroadcastUncertain
            | Self::ConfirmationTimeout
            | Self::ProviderUnavailable => RunFailureBlameSurface::Provider,
            Self::CancelRequested => RunFailureBlameSurface::Chain,
            Self::RuntimeInvariantViolation => RunFailureBlameSurface::Runtime,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunFailureStage {
    Observe,
    Derive,
    Simulate,
    Govern,
    Signer,
    Broadcast,
    Confirm,
    Verify,
    Recover,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunFailureSeverity {
    Retryable,
    PatchRequired,
    ManualReview,
    TerminalClosed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunFailureBlameSurface {
    Evidence,
    Fragment,
    Policy,
    Signer,
    Provider,
    Chain,
    Runtime,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryDisposition {
    ContinueWait,
    AwaitEvidence,
    AwaitEnvelope,
    AwaitSigner,
    AwaitContinuation,
    AwaitPatch,
    AwaitUserInput,
    RetryReady,
    AbortOnly,
    FailedClosed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryActionKind {
    SubmitEvidence,
    SubmitEnvelope,
    SubmitSignerResolution,
    SubmitPlanPatch,
    SubmitExecutionArtifactContinuation,
    RetryStep,
    CancelRun,
    AwaitConfirmation,
    EscalateUserReview,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryPriority {
    Automatic,
    HostReview,
    UserReview,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderFailureInfo {
    pub provider: String,
    pub operation: String,
    pub code: Option<String>,
    pub message: String,
    pub retryable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct VersionConflictInfo {
    pub expected_checkpoint_seq: Option<u64>,
    pub actual_checkpoint_seq: Option<u64>,
    pub expected_plan_epoch: Option<u64>,
    pub actual_plan_epoch: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryInputRequirement {
    pub key: String,
    pub value: Option<String>,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecoveryConstraintHint {
    pub key: String,
    pub value: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunFailureContext {
    pub code: RunFailureCode,
    pub stage: RunFailureStage,
    pub severity: RunFailureSeverity,
    pub blame_surface: RunFailureBlameSurface,
    pub observed_at_checkpoint_seq: u64,
    pub observed_at_plan_epoch: u64,
    pub active_boundary: Option<StableBoundaryKind>,
    pub summary: String,
    #[serde(default)]
    pub node_refs: Vec<String>,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
    #[serde(default)]
    pub effect_refs: Vec<String>,
    #[serde(default)]
    pub actuation_refs: Vec<String>,
    #[serde(default)]
    pub confirmation_refs: Vec<String>,
    pub provider_error: Option<ProviderFailureInfo>,
    pub governor_decision_ref: Option<String>,
    pub signer_request_ref: Option<SignerRequestId>,
    pub stale_version_conflict: Option<VersionConflictInfo>,
}

impl RunFailureContext {
    pub fn new(
        code: RunFailureCode,
        stage: RunFailureStage,
        observed_at_checkpoint_seq: u64,
        observed_at_plan_epoch: u64,
        active_boundary: Option<StableBoundaryKind>,
        summary: impl Into<String>,
    ) -> Self {
        Self {
            severity: code.default_severity(),
            blame_surface: code.default_blame_surface(),
            code,
            stage,
            observed_at_checkpoint_seq,
            observed_at_plan_epoch,
            active_boundary,
            summary: summary.into(),
            node_refs: Vec::new(),
            evidence_refs: Vec::new(),
            effect_refs: Vec::new(),
            actuation_refs: Vec::new(),
            confirmation_refs: Vec::new(),
            provider_error: None,
            governor_decision_ref: None,
            signer_request_ref: None,
            stale_version_conflict: None,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.summary.trim().is_empty() {
            return Err("run_failure_context.summary must not be empty".to_owned());
        }

        match self.code {
            RunFailureCode::MissingEvidence | RunFailureCode::StaleEvidence => {
                if self.evidence_refs.is_empty() {
                    return Err(format!(
                        "run_failure_context.{:?} requires evidence_refs",
                        self.code
                    ));
                }
            }
            RunFailureCode::SignerDenied | RunFailureCode::SignerExpired => {
                if self.signer_request_ref.is_none() {
                    return Err(format!(
                        "run_failure_context.{:?} requires signer_request_ref",
                        self.code
                    ));
                }
            }
            RunFailureCode::BroadcastUncertain | RunFailureCode::ConfirmationTimeout => {
                if self.confirmation_refs.is_empty() {
                    return Err(format!(
                        "run_failure_context.{:?} requires confirmation_refs",
                        self.code
                    ));
                }
            }
            RunFailureCode::VerifyMismatch => {
                if self.effect_refs.is_empty()
                    && self.actuation_refs.is_empty()
                    && self.confirmation_refs.is_empty()
                {
                    return Err(
                        "run_failure_context.verify_mismatch requires effect_refs, actuation_refs, or confirmation_refs"
                            .to_owned(),
                    );
                }
            }
            RunFailureCode::GovernorDenied => {
                if self.node_refs.is_empty() && self.governor_decision_ref.is_none() {
                    return Err(
                        "run_failure_context.governor_denied requires node_refs or governor_decision_ref"
                            .to_owned(),
                    );
                }
            }
            RunFailureCode::ProviderUnavailable => {
                if self.provider_error.is_none() {
                    return Err(
                        "run_failure_context.provider_unavailable requires provider_error"
                            .to_owned(),
                    );
                }
            }
            RunFailureCode::RuntimeInvariantViolation => {
                if self.severity != RunFailureSeverity::TerminalClosed
                    || self.blame_surface != RunFailureBlameSurface::Runtime
                {
                    return Err(
                        "run_failure_context.runtime_invariant_violation must stay runtime/terminal_closed"
                            .to_owned(),
                    );
                }
            }
            RunFailureCode::SimulationRejected
            | RunFailureCode::EnvelopeInvalid
            | RunFailureCode::BroadcastRejected
            | RunFailureCode::BudgetExhausted
            | RunFailureCode::CancelRequested => {}
        }

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecoverySuggestion {
    pub suggestion_id: String,
    pub action_kind: RecoveryActionKind,
    pub reason_code: RunFailureCode,
    pub priority: RecoveryPriority,
    pub basis_checkpoint_seq: u64,
    pub basis_plan_epoch: u64,
    pub retry_intent: Option<RetryIntent>,
    #[serde(default)]
    pub target_refs: Vec<String>,
    #[serde(default)]
    pub required_inputs: Vec<RecoveryInputRequirement>,
    #[serde(default)]
    pub constraints: Vec<RecoveryConstraintHint>,
}

impl RecoverySuggestion {
    pub fn validate(&self) -> Result<(), String> {
        if self.suggestion_id.trim().is_empty() {
            return Err("recovery_suggestion.suggestion_id must not be empty".to_owned());
        }
        match self.action_kind {
            RecoveryActionKind::RetryStep | RecoveryActionKind::AwaitConfirmation => {
                if self.retry_intent.is_none() {
                    return Err(format!(
                        "recovery_suggestion.{} requires retry_intent for {:?}",
                        self.suggestion_id, self.action_kind
                    ));
                }
            }
            _ if self.retry_intent.is_some() => {
                return Err(format!(
                    "recovery_suggestion.{} only step-related actions may carry retry_intent",
                    self.suggestion_id
                ));
            }
            _ => {}
        }
        Ok(())
    }
}

pub fn validate_recovery_contract(
    recovery_disposition: Option<&RecoveryDisposition>,
    failure_context: Option<&RunFailureContext>,
    recovery_suggestions: &[RecoverySuggestion],
    allowed_recovery_actions: &[RecoveryActionKind],
    checkpoint_seq: u64,
    plan_epoch: u64,
) -> Result<(), String> {
    if recovery_disposition.is_none()
        && (failure_context.is_some()
            || !recovery_suggestions.is_empty()
            || !allowed_recovery_actions.is_empty())
    {
        return Err(
            "recovery contract with failure/actions/suggestions requires a recovery_disposition"
                .to_owned(),
        );
    }

    if let Some(failure) = failure_context {
        failure.validate()?;
    }

    for suggestion in recovery_suggestions {
        suggestion.validate()?;
        if suggestion.basis_checkpoint_seq != checkpoint_seq {
            return Err(format!(
                "recovery_suggestion.{} basis_checkpoint_seq {} does not match checkpoint {}",
                suggestion.suggestion_id, suggestion.basis_checkpoint_seq, checkpoint_seq
            ));
        }
        if suggestion.basis_plan_epoch != plan_epoch {
            return Err(format!(
                "recovery_suggestion.{} basis_plan_epoch {} does not match plan epoch {}",
                suggestion.suggestion_id, suggestion.basis_plan_epoch, plan_epoch
            ));
        }
        if !allowed_recovery_actions.contains(&suggestion.action_kind) {
            return Err(format!(
                "recovery_suggestion.{} action_kind {:?} is not allowed by the recovery contract",
                suggestion.suggestion_id, suggestion.action_kind
            ));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_evidence_failure_requires_evidence_refs() {
        let mut failure = RunFailureContext::new(
            RunFailureCode::MissingEvidence,
            RunFailureStage::Observe,
            7,
            3,
            Some(StableBoundaryKind::Evidence),
            "quote is required",
        );
        assert!(failure.validate().is_err());

        failure.evidence_refs.push("evidence.quote".to_owned());
        assert!(failure.validate().is_ok());
    }

    #[test]
    fn signer_denied_failure_requires_signer_request_ref() {
        let mut failure = RunFailureContext::new(
            RunFailureCode::SignerDenied,
            RunFailureStage::Signer,
            9,
            4,
            Some(StableBoundaryKind::Failure),
            "signer denied the request",
        );
        assert!(failure.validate().is_err());

        failure.signer_request_ref = Some(SignerRequestId("signer-1".to_owned()));
        assert!(failure.validate().is_ok());
    }

    #[test]
    fn provider_unavailable_failure_requires_provider_error() {
        let mut failure = RunFailureContext::new(
            RunFailureCode::ProviderUnavailable,
            RunFailureStage::Observe,
            4,
            1,
            None,
            "rpc request failed",
        );
        assert!(failure.validate().is_err());

        failure.provider_error = Some(ProviderFailureInfo {
            provider: "rpc".to_owned(),
            operation: "eth_call".to_owned(),
            code: Some("429".to_owned()),
            message: "rate limited".to_owned(),
            retryable: true,
        });
        assert!(failure.validate().is_ok());
    }

    #[test]
    fn recovery_contract_rejects_suggestion_outside_allowed_actions() {
        let suggestion = RecoverySuggestion {
            suggestion_id: "run-1:recovery:7:submit_plan_patch".to_owned(),
            action_kind: RecoveryActionKind::SubmitPlanPatch,
            reason_code: RunFailureCode::GovernorDenied,
            priority: RecoveryPriority::HostReview,
            basis_checkpoint_seq: 7,
            basis_plan_epoch: 3,
            retry_intent: None,
            target_refs: vec!["node.swap".to_owned()],
            required_inputs: Vec::new(),
            constraints: Vec::new(),
        };

        let error = validate_recovery_contract(
            Some(&RecoveryDisposition::AwaitPatch),
            None,
            &[suggestion],
            &[RecoveryActionKind::CancelRun],
            7,
            3,
        )
        .expect_err("invalid action kind should fail");

        assert!(error.contains("is not allowed"));
    }

    #[test]
    fn recovery_contract_rejects_failure_without_disposition() {
        let mut failure = RunFailureContext::new(
            RunFailureCode::MissingEvidence,
            RunFailureStage::Observe,
            5,
            2,
            Some(StableBoundaryKind::Evidence),
            "quote missing",
        );
        failure.evidence_refs.push("evidence.quote".to_owned());

        let error = validate_recovery_contract(None, Some(&failure), &[], &[], 5, 2)
            .expect_err("failure without disposition should fail");

        assert!(error.contains("requires a recovery_disposition"));
    }

    #[test]
    fn retry_step_suggestion_requires_retry_intent() {
        let suggestion = RecoverySuggestion {
            suggestion_id: "run-1:recovery:7:retry_step".to_owned(),
            action_kind: RecoveryActionKind::RetryStep,
            reason_code: RunFailureCode::ProviderUnavailable,
            priority: RecoveryPriority::Automatic,
            basis_checkpoint_seq: 7,
            basis_plan_epoch: 3,
            retry_intent: None,
            target_refs: vec!["confirm-1".to_owned()],
            required_inputs: Vec::new(),
            constraints: Vec::new(),
        };

        let error = suggestion
            .validate()
            .expect_err("retry intent should be required");
        assert!(error.contains("requires retry_intent"));
    }
}
