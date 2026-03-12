//! Helpers for reconstructing hot runtime state from persisted snapshots.

use thiserror::Error;

use ais_agent_control::ids::RunId;
use ais_agent_core::{
    checkpoint::CheckpointSnapshot,
    mission::Mission,
    runtime::{RunStatus, SignerRequestState},
};

use crate::{
    persistence::{
        CheckpointRepository, CheckpointRepositoryError, MissionRepository, MissionRepositoryError,
        SignerStateArchive, SignerStateArchiveError,
    },
    runtime::{ActiveRun, RuntimeStateMachine},
};

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RestoreRuntimeError {
    #[error("mission archive error: {0}")]
    MissionRepository(#[from] MissionRepositoryError),
    #[error("checkpoint archive error: {0}")]
    CheckpointRepository(#[from] CheckpointRepositoryError),
    #[error("signer state archive error: {0}")]
    SignerStateArchive(#[from] SignerStateArchiveError),
    #[error("mission `{mission_id}` does not match checkpoint mission `{checkpoint_mission_id}`")]
    MissionMismatch {
        mission_id: String,
        checkpoint_mission_id: String,
    },
    #[error("checkpoint run `{checkpoint_run_id}` does not match signer run `{signer_run_id}`")]
    SignerRunMismatch {
        checkpoint_run_id: String,
        signer_run_id: String,
    },
    #[error(
        "checkpoint expects signer request `{expected_request_id}`, found `{actual_request_id}`"
    )]
    SignerRequestMismatch {
        expected_request_id: String,
        actual_request_id: String,
    },
    #[error(
        "checkpoint is awaiting signer `{expected_request_id}` but no signer state was available"
    )]
    MissingPendingSignerState { expected_request_id: String },
    #[error(
        "checkpoint for run `{run_id}` is awaiting confirmation but has no pending confirmation id"
    )]
    MissingPendingConfirmationId { run_id: String },
    #[error(
        "checkpoint for run `{run_id}` cannot resume verify node `{node_id}` because effect contract `{effect_id}` is missing"
    )]
    MissingEffectContractForConfirmationResume {
        run_id: String,
        node_id: String,
        effect_id: String,
    },
}

pub fn restore_active_run(
    run_id: &RunId,
    mission_repository: &impl MissionRepository,
    checkpoint_repository: &impl CheckpointRepository,
    signer_state_archive: &impl SignerStateArchive,
) -> Result<ActiveRun, RestoreRuntimeError> {
    let mission = mission_repository.load(run_id)?;
    let checkpoint = checkpoint_repository.latest(&run_id.0)?;
    let pending_signer_state = match signer_state_archive.load(run_id) {
        Ok(state) => Some(state),
        Err(SignerStateArchiveError::NotFound { .. }) => None,
        Err(error) => return Err(error.into()),
    };
    restore_active_run_from_parts(mission, checkpoint, pending_signer_state)
}

pub fn restore_active_run_from_parts(
    mission: Mission,
    checkpoint: CheckpointSnapshot,
    pending_signer_state: Option<SignerRequestState>,
) -> Result<ActiveRun, RestoreRuntimeError> {
    if mission.mission_id != checkpoint.mission_id {
        return Err(RestoreRuntimeError::MissionMismatch {
            mission_id: mission.mission_id,
            checkpoint_mission_id: checkpoint.mission_id,
        });
    }

    if let Some(state) = pending_signer_state.as_ref() {
        if state.run_id.0 != checkpoint.run_id {
            return Err(RestoreRuntimeError::SignerRunMismatch {
                checkpoint_run_id: checkpoint.run_id.clone(),
                signer_run_id: state.run_id.0.clone(),
            });
        }
    }

    let expected_signer_request_id = checkpoint
        .pending_requests
        .pending_signer_request_id
        .clone();
    let restored_pending_signer_state = RuntimeStateMachine::restored_pending_signer_state(
        &checkpoint,
        pending_signer_state.clone(),
    );

    if matches!(checkpoint.lifecycle.status, RunStatus::AwaitingSigner)
        || expected_signer_request_id.is_some()
    {
        let Some(expected_request_id) = expected_signer_request_id else {
            return Err(RestoreRuntimeError::MissingPendingSignerState {
                expected_request_id: "unknown".to_owned(),
            });
        };

        match pending_signer_state {
            Some(state) if state.request_id.0 == expected_request_id => {}
            Some(state) => {
                return Err(RestoreRuntimeError::SignerRequestMismatch {
                    expected_request_id,
                    actual_request_id: state.request_id.0,
                });
            }
            None => {
                return Err(RestoreRuntimeError::MissingPendingSignerState {
                    expected_request_id,
                });
            }
        }
    }

    if RuntimeStateMachine::requires_confirmation_resume(&checkpoint)
        && checkpoint
            .pending_requests
            .pending_confirmation_id
            .is_none()
    {
        return Err(RestoreRuntimeError::MissingPendingConfirmationId {
            run_id: checkpoint.run_id.clone(),
        });
    }

    if RuntimeStateMachine::requires_confirmation_resume(&checkpoint) {
        if let Some((node_id, effect_id)) =
            RuntimeStateMachine::missing_effect_contract_for_confirmation_resume(&checkpoint)
        {
            return Err(
                RestoreRuntimeError::MissingEffectContractForConfirmationResume {
                    run_id: checkpoint.run_id.clone(),
                    node_id,
                    effect_id,
                },
            );
        }
    }

    let mut runtime = ActiveRun::new(mission, checkpoint);
    runtime.set_pending_signer_state(restored_pending_signer_state);
    Ok(runtime)
}
