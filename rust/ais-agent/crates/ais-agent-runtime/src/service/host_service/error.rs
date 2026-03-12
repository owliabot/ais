use ais_agent_host::control::{HostCommandError, HostCommandOutcome, HostCommandResponse};
use thiserror::Error;

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
    #[error("checkpoint persistence failed: {0}")]
    Checkpoint(#[from] crate::persistence::CheckpointRepositoryError),
    #[error("mission persistence failed: {0}")]
    Mission(#[from] crate::persistence::MissionRepositoryError),
    #[error("event archive failed: {0}")]
    EventArchive(#[from] crate::persistence::EventArchiveError),
    #[error("run catalog persistence failed: {0}")]
    RunCatalog(#[from] crate::persistence::RunCatalogRepositoryError),
    #[error("signer state archive failed: {0}")]
    SignerArchive(#[from] crate::persistence::SignerStateArchiveError),
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
    #[error("signer decision does not match pending signer state")]
    SignerDecisionMismatch,
    #[error("envelope submission rejected: {0}")]
    EnvelopeRejected(String),
    #[error("plan patch rejected: {0}")]
    PlanPatchLegality(String),
    #[error("cancel request rejected: {0}")]
    CancelRejected(String),
    #[error("invalid recovery contract: {0}")]
    InvalidRecoveryContract(String),
    #[error("{0:?}")]
    VersionConflict(crate::concurrency::CommandVersionConflict),
}

impl RuntimeHostServiceError {
    pub fn into_outcome(self) -> HostCommandOutcome {
        let code = error_code(&self);
        HostCommandOutcome {
            response: HostCommandResponse::Error(HostCommandError {
                code,
                message: self.to_string(),
            }),
            events: Vec::new(),
        }
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
        RuntimeHostServiceError::Checkpoint(_) => "checkpoint_error".to_owned(),
        RuntimeHostServiceError::Mission(_) => "mission_error".to_owned(),
        RuntimeHostServiceError::EventArchive(_) => "event_archive_error".to_owned(),
        RuntimeHostServiceError::RunCatalog(_) => "run_catalog_error".to_owned(),
        RuntimeHostServiceError::SignerArchive(_) => "signer_archive_error".to_owned(),
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
                crate::persistence::DurableMutationMember::Signer => {
                    "signer_archive_error".to_owned()
                }
                crate::persistence::DurableMutationMember::Audit => {
                    "runtime_audit_archive_error".to_owned()
                }
            },
        },
        RuntimeHostServiceError::Repository(_) => "repository_error".to_owned(),
        RuntimeHostServiceError::Restore(_) => "restore_error".to_owned(),
        RuntimeHostServiceError::Stepper(_) => "stepper_error".to_owned(),
        RuntimeHostServiceError::SignerDecisionMismatch => "signer_decision_mismatch".to_owned(),
        RuntimeHostServiceError::EnvelopeRejected(_) => "envelope_invalid".to_owned(),
        RuntimeHostServiceError::PlanPatchLegality(_) => "plan_patch_illegal".to_owned(),
        RuntimeHostServiceError::CancelRejected(_) => "cancel_rejected".to_owned(),
        RuntimeHostServiceError::InvalidRecoveryContract(_) => {
            "recovery_contract_invalid".to_owned()
        }
        RuntimeHostServiceError::VersionConflict(conflict) => conflict.code.clone(),
    }
}
