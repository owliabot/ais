use ais_agent_control::{
    ids::{ClaimId, RunId},
    ownership::OwnershipErrorCode,
};
use ais_agent_host::control::{
    HostCommandError, HostCommandOutcome, HostCommandResponse, HostErrorClass,
    HostErrorCorrelation, HostErrorRecoveryHints, HostProviderBindingErrorContext,
};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderBindingFailure {
    NotConfigured {
        chain_scope: String,
        expected_family: String,
        provider_lookup_scope: String,
    },
    FamilyMismatch {
        chain_scope: String,
        expected_family: String,
        actual_family: String,
        provider_lookup_scope: String,
    },
}

impl ProviderBindingFailure {
    fn context(&self) -> HostProviderBindingErrorContext {
        match self {
            Self::NotConfigured {
                chain_scope,
                expected_family,
                provider_lookup_scope,
            } => HostProviderBindingErrorContext {
                chain_scope: chain_scope.clone(),
                expected_family: expected_family.clone(),
                actual_family: None,
                provider_lookup_scope: provider_lookup_scope.clone(),
            },
            Self::FamilyMismatch {
                chain_scope,
                expected_family,
                actual_family,
                provider_lookup_scope,
            } => HostProviderBindingErrorContext {
                chain_scope: chain_scope.clone(),
                expected_family: expected_family.clone(),
                actual_family: Some(actual_family.clone()),
                provider_lookup_scope: provider_lookup_scope.clone(),
            },
        }
    }
}

impl std::fmt::Display for ProviderBindingFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotConfigured {
                chain_scope,
                expected_family,
                ..
            } => write!(
                f,
                "no provider entry resolved for chain `{chain_scope}` (family `{expected_family}`)"
            ),
            Self::FamilyMismatch {
                chain_scope,
                expected_family,
                actual_family,
                ..
            } => write!(
                f,
                "chain `{chain_scope}` expected family `{expected_family}` but found `{actual_family}`"
            ),
        }
    }
}

#[derive(Debug, Error)]
pub enum RuntimeHostServiceError {
    #[error("run `{run_id}` not found")]
    RunNotFound { run_id: String },
    #[error("idempotency conflict for begin_run")]
    IdempotencyConflict,
    #[error("idempotency replay has no cached outcome")]
    IdempotencyReplayIncomplete,
    #[error("host session `{host_session_id}` is not linked to run `{run_id}`")]
    SessionRunMismatch {
        host_session_id: String,
        run_id: String,
    },
    #[error(
        "host session `{host_session_id}` must inspect run `{run_id}` before mutating it after restart"
    )]
    SessionRelinkRequired {
        host_session_id: String,
        run_id: String,
    },
    #[error("provider binding failed: {0}")]
    ProviderBinding(ProviderBindingFailure),
    #[error("checkpoint persistence failed: {0}")]
    Checkpoint(#[from] crate::persistence::CheckpointRepositoryError),
    #[error("mission persistence failed: {0}")]
    Mission(#[from] crate::persistence::MissionRepositoryError),
    #[error("event archive failed: {0}")]
    EventArchive(#[from] crate::persistence::EventArchiveError),
    #[error("run catalog persistence failed: {0}")]
    RunCatalog(#[from] crate::persistence::RunCatalogRepositoryError),
    #[error("signer state store failed: {0}")]
    SignerStateStore(#[from] crate::persistence::SignerStateStoreError),
    #[error("runtime audit archive failed: {0}")]
    RuntimeAuditArchive(#[from] crate::persistence::RuntimeAuditArchiveError),
    #[error("durable grouped commit failed: {0}")]
    DurableCommit(#[from] crate::persistence::DurableCommitError),
    #[error("runtime repository failed: {0}")]
    Repository(#[from] crate::runtime::RunRepositoryError),
    #[error("runtime restore failed: {0}")]
    Restore(#[from] crate::persistence::RestoreRuntimeError),
    #[error("stepper failed: {0}")]
    Stepper(#[from] crate::stepper::StepSchedulerError),
    #[error("signer resolution does not match pending signer state")]
    SignerResolutionMismatch,
    #[error("envelope submission rejected: {0}")]
    EnvelopeRejected(String),
    #[error("execution artifact continuation rejected: {0}")]
    ContinuationRejected(String),
    #[error("invalid command: {0}")]
    InvalidCommand(String),
    #[error("plan patch rejected: {0}")]
    PlanPatchLegality(String),
    #[error("cancel request rejected: {0}")]
    CancelRejected(String),
    #[error("ownership violation `{code:?}` for run `{run_id}`: {message}")]
    OwnershipViolation {
        code: OwnershipErrorCode,
        run_id: String,
        claim_id: Option<ClaimId>,
        message: String,
    },
    #[error("ownership command `{command}` is not implemented yet")]
    OwnershipCommandNotImplemented { command: &'static str },
    #[error("invalid recovery contract: {0}")]
    InvalidRecoveryContract(String),
    #[error("{0:?}")]
    VersionConflict(crate::concurrency::CommandVersionConflict),
}

#[derive(Debug, Clone)]
struct HostErrorMetadata {
    code: String,
    error_class: HostErrorClass,
    retryable: bool,
    recovery_hints: HostErrorRecoveryHints,
    correlation: Option<HostErrorCorrelation>,
    provider_binding: Option<HostProviderBindingErrorContext>,
}

impl RuntimeHostServiceError {
    pub fn invalid_command(message: impl Into<String>) -> Self {
        Self::InvalidCommand(message.into())
    }

    pub fn provider_not_configured(
        chain_scope: impl Into<String>,
        expected_family: impl Into<String>,
        provider_lookup_scope: impl Into<String>,
    ) -> Self {
        Self::ProviderBinding(ProviderBindingFailure::NotConfigured {
            chain_scope: chain_scope.into(),
            expected_family: expected_family.into(),
            provider_lookup_scope: provider_lookup_scope.into(),
        })
    }

    pub fn provider_family_mismatch(
        chain_scope: impl Into<String>,
        expected_family: impl Into<String>,
        actual_family: impl Into<String>,
        provider_lookup_scope: impl Into<String>,
    ) -> Self {
        Self::ProviderBinding(ProviderBindingFailure::FamilyMismatch {
            chain_scope: chain_scope.into(),
            expected_family: expected_family.into(),
            actual_family: actual_family.into(),
            provider_lookup_scope: provider_lookup_scope.into(),
        })
    }

    pub fn into_outcome(self) -> HostCommandOutcome {
        let metadata = error_metadata(&self);
        HostCommandOutcome {
            response: HostCommandResponse::Error(HostCommandError {
                code: metadata.code,
                message: self.to_string(),
                error_class: metadata.error_class,
                retryable: metadata.retryable,
                recovery_hints: metadata.recovery_hints,
                correlation: metadata.correlation,
                provider_binding: metadata.provider_binding,
            }),
            events: Vec::new(),
        }
    }
}

fn error_metadata(error: &RuntimeHostServiceError) -> HostErrorMetadata {
    match error {
        RuntimeHostServiceError::RunNotFound { run_id } => HostErrorMetadata {
            code: "run_not_found".to_owned(),
            error_class: HostErrorClass::NotFound,
            retryable: false,
            recovery_hints: HostErrorRecoveryHints::default(),
            correlation: Some(run_correlation(run_id)),
            provider_binding: None,
        },
        RuntimeHostServiceError::IdempotencyConflict => HostErrorMetadata {
            code: "idempotency_conflict".to_owned(),
            error_class: HostErrorClass::Conflict,
            retryable: true,
            recovery_hints: HostErrorRecoveryHints::default(),
            correlation: None,
            provider_binding: None,
        },
        RuntimeHostServiceError::IdempotencyReplayIncomplete => HostErrorMetadata {
            code: "idempotency_replay_incomplete".to_owned(),
            error_class: HostErrorClass::Precondition,
            retryable: true,
            recovery_hints: HostErrorRecoveryHints {
                operator_action_recommended: true,
                ..HostErrorRecoveryHints::default()
            },
            correlation: None,
            provider_binding: None,
        },
        RuntimeHostServiceError::SessionRunMismatch { run_id, .. } => HostErrorMetadata {
            code: "session_run_mismatch".to_owned(),
            error_class: HostErrorClass::Conflict,
            retryable: false,
            recovery_hints: HostErrorRecoveryHints::default(),
            correlation: Some(run_correlation(run_id)),
            provider_binding: None,
        },
        RuntimeHostServiceError::SessionRelinkRequired { run_id, .. } => HostErrorMetadata {
            code: "session_relink_required".to_owned(),
            error_class: HostErrorClass::Precondition,
            retryable: false,
            recovery_hints: HostErrorRecoveryHints {
                requires_relink: true,
                operator_action_recommended: true,
                ..HostErrorRecoveryHints::default()
            },
            correlation: Some(run_correlation(run_id)),
            provider_binding: None,
        },
        RuntimeHostServiceError::ProviderBinding(failure) => HostErrorMetadata {
            code: match failure {
                ProviderBindingFailure::NotConfigured { .. } => "provider_not_configured",
                ProviderBindingFailure::FamilyMismatch { .. } => "provider_family_mismatch",
            }
            .to_owned(),
            error_class: HostErrorClass::ProviderBinding,
            retryable: false,
            recovery_hints: HostErrorRecoveryHints {
                operator_action_recommended: true,
                ..HostErrorRecoveryHints::default()
            },
            correlation: None,
            provider_binding: Some(failure.context()),
        },
        RuntimeHostServiceError::Checkpoint(_)
        | RuntimeHostServiceError::Mission(_)
        | RuntimeHostServiceError::EventArchive(_)
        | RuntimeHostServiceError::RunCatalog(_)
        | RuntimeHostServiceError::SignerStateStore(_)
        | RuntimeHostServiceError::RuntimeAuditArchive(_)
        | RuntimeHostServiceError::DurableCommit(_)
        | RuntimeHostServiceError::Repository(_) => HostErrorMetadata {
            code: error_code(error),
            error_class: HostErrorClass::Persistence,
            retryable: true,
            recovery_hints: HostErrorRecoveryHints {
                operator_action_recommended: true,
                ..HostErrorRecoveryHints::default()
            },
            correlation: None,
            provider_binding: None,
        },
        RuntimeHostServiceError::Restore(restore_error) => restore_error_metadata(restore_error),
        RuntimeHostServiceError::Stepper(_) => HostErrorMetadata {
            code: "stepper_error".to_owned(),
            error_class: HostErrorClass::Internal,
            retryable: false,
            recovery_hints: HostErrorRecoveryHints {
                operator_action_recommended: true,
                ..HostErrorRecoveryHints::default()
            },
            correlation: None,
            provider_binding: None,
        },
        RuntimeHostServiceError::SignerResolutionMismatch => HostErrorMetadata {
            code: "signer_resolution_mismatch".to_owned(),
            error_class: HostErrorClass::Precondition,
            retryable: false,
            recovery_hints: HostErrorRecoveryHints::default(),
            correlation: None,
            provider_binding: None,
        },
        RuntimeHostServiceError::EnvelopeRejected(_) => HostErrorMetadata {
            code: "envelope_invalid".to_owned(),
            error_class: HostErrorClass::InvalidCommand,
            retryable: false,
            recovery_hints: HostErrorRecoveryHints {
                requires_envelope: true,
                ..HostErrorRecoveryHints::default()
            },
            correlation: None,
            provider_binding: None,
        },
        RuntimeHostServiceError::ContinuationRejected(_) => HostErrorMetadata {
            code: "artifact_continuation_invalid".to_owned(),
            error_class: HostErrorClass::InvalidCommand,
            retryable: false,
            recovery_hints: HostErrorRecoveryHints::default(),
            correlation: None,
            provider_binding: None,
        },
        RuntimeHostServiceError::InvalidCommand(_) => HostErrorMetadata {
            code: "invalid_command".to_owned(),
            error_class: HostErrorClass::InvalidCommand,
            retryable: false,
            recovery_hints: HostErrorRecoveryHints::default(),
            correlation: None,
            provider_binding: None,
        },
        RuntimeHostServiceError::PlanPatchLegality(_) => HostErrorMetadata {
            code: "plan_patch_illegal".to_owned(),
            error_class: HostErrorClass::RecoveryContract,
            retryable: false,
            recovery_hints: HostErrorRecoveryHints {
                requires_patch: true,
                operator_action_recommended: true,
                ..HostErrorRecoveryHints::default()
            },
            correlation: None,
            provider_binding: None,
        },
        RuntimeHostServiceError::CancelRejected(_) => HostErrorMetadata {
            code: "cancel_rejected".to_owned(),
            error_class: HostErrorClass::Precondition,
            retryable: false,
            recovery_hints: HostErrorRecoveryHints::default(),
            correlation: None,
            provider_binding: None,
        },
        RuntimeHostServiceError::OwnershipViolation {
            code,
            run_id,
            claim_id,
            ..
        } => HostErrorMetadata {
            code: ownership_error_code(code.clone()),
            error_class: HostErrorClass::Ownership,
            retryable: false,
            recovery_hints: HostErrorRecoveryHints::default(),
            correlation: Some(HostErrorCorrelation {
                run_id: Some(RunId(run_id.clone())),
                claim_id: claim_id.clone(),
                checkpoint_seq: None,
            }),
            provider_binding: None,
        },
        RuntimeHostServiceError::OwnershipCommandNotImplemented { .. } => HostErrorMetadata {
            code: "ownership_command_not_implemented".to_owned(),
            error_class: HostErrorClass::Unavailable,
            retryable: false,
            recovery_hints: HostErrorRecoveryHints {
                operator_action_recommended: true,
                ..HostErrorRecoveryHints::default()
            },
            correlation: None,
            provider_binding: None,
        },
        RuntimeHostServiceError::InvalidRecoveryContract(_) => HostErrorMetadata {
            code: "recovery_contract_invalid".to_owned(),
            error_class: HostErrorClass::RecoveryContract,
            retryable: false,
            recovery_hints: HostErrorRecoveryHints {
                requires_patch: true,
                operator_action_recommended: true,
                ..HostErrorRecoveryHints::default()
            },
            correlation: None,
            provider_binding: None,
        },
        RuntimeHostServiceError::VersionConflict(conflict) => HostErrorMetadata {
            code: conflict.code.clone(),
            error_class: HostErrorClass::Conflict,
            retryable: true,
            recovery_hints: HostErrorRecoveryHints::default(),
            correlation: Some(HostErrorCorrelation {
                run_id: Some(conflict.run_id.clone()),
                claim_id: None,
                checkpoint_seq: Some(conflict.current.checkpoint_seq),
            }),
            provider_binding: None,
        },
    }
}

fn restore_error_metadata(error: &crate::persistence::RestoreRuntimeError) -> HostErrorMetadata {
    use crate::persistence::RestoreRuntimeError;

    match error {
        RestoreRuntimeError::MissionRepository(_)
        | RestoreRuntimeError::CheckpointRepository(_)
        | RestoreRuntimeError::WaitStateStore(_) => HostErrorMetadata {
            code: "restore_error".to_owned(),
            error_class: HostErrorClass::Persistence,
            retryable: true,
            recovery_hints: HostErrorRecoveryHints {
                operator_action_recommended: true,
                ..HostErrorRecoveryHints::default()
            },
            correlation: None,
            provider_binding: None,
        },
        RestoreRuntimeError::MissionMismatch { .. } => HostErrorMetadata {
            code: "restore_mission_mismatch".to_owned(),
            error_class: HostErrorClass::RecoveryContract,
            retryable: false,
            recovery_hints: HostErrorRecoveryHints {
                operator_action_recommended: true,
                ..HostErrorRecoveryHints::default()
            },
            correlation: None,
            provider_binding: None,
        },
        RestoreRuntimeError::SignerRunMismatch {
            checkpoint_run_id, ..
        } => HostErrorMetadata {
            code: "restore_signer_run_mismatch".to_owned(),
            error_class: HostErrorClass::RecoveryContract,
            retryable: false,
            recovery_hints: HostErrorRecoveryHints {
                operator_action_recommended: true,
                ..HostErrorRecoveryHints::default()
            },
            correlation: Some(run_correlation(checkpoint_run_id)),
            provider_binding: None,
        },
        RestoreRuntimeError::SignerRequestMismatch { .. } => HostErrorMetadata {
            code: "restore_signer_request_mismatch".to_owned(),
            error_class: HostErrorClass::RecoveryContract,
            retryable: false,
            recovery_hints: HostErrorRecoveryHints {
                operator_action_recommended: true,
                ..HostErrorRecoveryHints::default()
            },
            correlation: None,
            provider_binding: None,
        },
        RestoreRuntimeError::MissingPendingSignerState {
            expected_request_id: _,
        } => HostErrorMetadata {
            code: "restore_missing_pending_signer_state".to_owned(),
            error_class: HostErrorClass::RecoveryContract,
            retryable: false,
            recovery_hints: HostErrorRecoveryHints {
                operator_action_recommended: true,
                ..HostErrorRecoveryHints::default()
            },
            correlation: None,
            provider_binding: None,
        },
        RestoreRuntimeError::MissingPendingConfirmationId { run_id } => HostErrorMetadata {
            code: "restore_missing_pending_submission_id".to_owned(),
            error_class: HostErrorClass::RecoveryContract,
            retryable: false,
            recovery_hints: HostErrorRecoveryHints {
                operator_action_recommended: true,
                ..HostErrorRecoveryHints::default()
            },
            correlation: Some(run_correlation(run_id)),
            provider_binding: None,
        },
        RestoreRuntimeError::MissingEffectContractForConfirmationResume { run_id, .. } => {
            HostErrorMetadata {
                code: "restore_missing_effect_contract".to_owned(),
                error_class: HostErrorClass::RecoveryContract,
                retryable: false,
                recovery_hints: HostErrorRecoveryHints {
                    requires_patch: true,
                    operator_action_recommended: true,
                    ..HostErrorRecoveryHints::default()
                },
                correlation: Some(run_correlation(run_id)),
                provider_binding: None,
            }
        }
    }
}

fn run_correlation(run_id: &str) -> HostErrorCorrelation {
    HostErrorCorrelation {
        run_id: Some(RunId(run_id.to_owned())),
        claim_id: None,
        checkpoint_seq: None,
    }
}

fn ownership_error_code(code: OwnershipErrorCode) -> String {
    match code {
        OwnershipErrorCode::ClaimRequired => "claim_required".to_owned(),
        OwnershipErrorCode::ClaimConflict => "claim_conflict".to_owned(),
        OwnershipErrorCode::ClaimExpired => "claim_expired".to_owned(),
        OwnershipErrorCode::ClaimNotOwner => "claim_not_owner".to_owned(),
        OwnershipErrorCode::ClaimEpochStale => "claim_epoch_stale".to_owned(),
        OwnershipErrorCode::ClaimTransferRequired => "claim_transfer_required".to_owned(),
        OwnershipErrorCode::ObserverOnly => "observer_only".to_owned(),
    }
}

fn error_code(error: &RuntimeHostServiceError) -> String {
    match error {
        RuntimeHostServiceError::RunNotFound { .. } => "run_not_found".to_owned(),
        RuntimeHostServiceError::IdempotencyConflict => "idempotency_conflict".to_owned(),
        RuntimeHostServiceError::IdempotencyReplayIncomplete => {
            "idempotency_replay_incomplete".to_owned()
        }
        RuntimeHostServiceError::SessionRunMismatch { .. } => "session_run_mismatch".to_owned(),
        RuntimeHostServiceError::SessionRelinkRequired { .. } => {
            "session_relink_required".to_owned()
        }
        RuntimeHostServiceError::ProviderBinding(failure) => match failure {
            ProviderBindingFailure::NotConfigured { .. } => "provider_not_configured".to_owned(),
            ProviderBindingFailure::FamilyMismatch { .. } => "provider_family_mismatch".to_owned(),
        },
        RuntimeHostServiceError::Checkpoint(_) => "checkpoint_error".to_owned(),
        RuntimeHostServiceError::Mission(_) => "mission_error".to_owned(),
        RuntimeHostServiceError::EventArchive(_) => "event_archive_error".to_owned(),
        RuntimeHostServiceError::RunCatalog(_) => "run_catalog_error".to_owned(),
        RuntimeHostServiceError::SignerStateStore(_) => "signer_state_store_error".to_owned(),
        RuntimeHostServiceError::RuntimeAuditArchive(_) => "runtime_audit_archive_error".to_owned(),
        RuntimeHostServiceError::DurableCommit(error) => match error {
            crate::persistence::DurableCommitError::InvalidUnit(_) => {
                "durable_commit_error".to_owned()
            }
            crate::persistence::DurableCommitError::Transaction { .. } => {
                "durable_commit_error".to_owned()
            }
            crate::persistence::DurableCommitError::MemberWrite { member, .. } => match member {
                crate::persistence::DurableMutationMember::Mission => "mission_error".to_owned(),
                crate::persistence::DurableMutationMember::Checkpoint => {
                    "checkpoint_error".to_owned()
                }
                crate::persistence::DurableMutationMember::Event => {
                    "event_archive_error".to_owned()
                }
                crate::persistence::DurableMutationMember::Catalog => {
                    "run_catalog_error".to_owned()
                }
                crate::persistence::DurableMutationMember::WaitState => {
                    "wait_state_store_error".to_owned()
                }
                crate::persistence::DurableMutationMember::Audit => {
                    "runtime_audit_archive_error".to_owned()
                }
            },
        },
        RuntimeHostServiceError::Repository(_) => "repository_error".to_owned(),
        RuntimeHostServiceError::Restore(_) => "restore_error".to_owned(),
        RuntimeHostServiceError::Stepper(_) => "stepper_error".to_owned(),
        RuntimeHostServiceError::SignerResolutionMismatch => {
            "signer_resolution_mismatch".to_owned()
        }
        RuntimeHostServiceError::EnvelopeRejected(_) => "envelope_invalid".to_owned(),
        RuntimeHostServiceError::ContinuationRejected(_) => {
            "artifact_continuation_invalid".to_owned()
        }
        RuntimeHostServiceError::InvalidCommand(_) => "invalid_command".to_owned(),
        RuntimeHostServiceError::PlanPatchLegality(_) => "plan_patch_illegal".to_owned(),
        RuntimeHostServiceError::CancelRejected(_) => "cancel_rejected".to_owned(),
        RuntimeHostServiceError::OwnershipViolation { code, .. } => {
            ownership_error_code(code.clone())
        }
        RuntimeHostServiceError::OwnershipCommandNotImplemented { .. } => {
            "ownership_command_not_implemented".to_owned()
        }
        RuntimeHostServiceError::InvalidRecoveryContract(_) => {
            "recovery_contract_invalid".to_owned()
        }
        RuntimeHostServiceError::VersionConflict(conflict) => conflict.code.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_binding_error_into_outcome_preserves_machine_readable_context() {
        let outcome = RuntimeHostServiceError::provider_not_configured(
            "eip155:8453",
            "evm",
            "runtime_execution_wiring.chains",
        )
        .into_outcome();

        let HostCommandResponse::Error(error) = outcome.response else {
            panic!("expected error response");
        };
        assert_eq!(error.code, "provider_not_configured");
        assert!(matches!(error.error_class, HostErrorClass::ProviderBinding));
        assert!(!error.retryable);
        let provider_binding = error.provider_binding.expect("provider binding context");
        assert_eq!(provider_binding.chain_scope, "eip155:8453");
        assert_eq!(provider_binding.expected_family, "evm");
        assert_eq!(
            provider_binding.provider_lookup_scope,
            "runtime_execution_wiring.chains"
        );
    }

    #[test]
    fn restore_contract_error_into_outcome_emits_patch_hint_and_correlation() {
        let outcome = RuntimeHostServiceError::Restore(
            crate::persistence::RestoreRuntimeError::MissingEffectContractForConfirmationResume {
                run_id: "run-9".to_owned(),
                node_id: "verify.swap".to_owned(),
                effect_id: "effect.swap".to_owned(),
            },
        )
        .into_outcome();

        let HostCommandResponse::Error(error) = outcome.response else {
            panic!("expected error response");
        };
        assert_eq!(error.code, "restore_missing_effect_contract");
        assert!(matches!(
            error.error_class,
            HostErrorClass::RecoveryContract
        ));
        assert!(error.recovery_hints.requires_patch);
        assert_eq!(
            error
                .correlation
                .as_ref()
                .and_then(|correlation| correlation.run_id.as_ref())
                .map(|run_id| run_id.0.as_str()),
            Some("run-9")
        );
    }
}
