//! Helpers for persisting normalized runtime checkpoints.

use ais_agent_core::checkpoint::CheckpointSnapshot;

use crate::{
    persistence::{
        CheckpointArchive, CheckpointArchiveEntry, CheckpointArchiveError, CheckpointArchiveKind,
    },
    runtime::{
        validate_checkpoint_recovery_contract, ActiveRun, CheckpointPersistenceMode,
        RuntimeStateMachine,
    },
};

pub fn persist_boundary_checkpoint(
    repository: &mut impl CheckpointArchive,
    runtime: &ActiveRun,
) -> Result<CheckpointSnapshot, CheckpointArchiveError> {
    let snapshot = RuntimeStateMachine::checkpoint_for_persistence(
        runtime,
        CheckpointPersistenceMode::Boundary,
    );
    validate_checkpoint_recovery_contract(&snapshot)
        .map_err(|message| CheckpointArchiveError::InvalidRecoveryContract { message })?;
    repository.append(CheckpointArchiveEntry {
        snapshot: snapshot.clone(),
        kind: CheckpointArchiveKind::Boundary,
    })?;
    Ok(snapshot)
}

pub fn persist_progress_checkpoint(
    repository: &mut impl CheckpointArchive,
    runtime: &ActiveRun,
) -> Result<CheckpointSnapshot, CheckpointArchiveError> {
    let snapshot = RuntimeStateMachine::checkpoint_for_persistence(
        runtime,
        CheckpointPersistenceMode::Progress,
    );
    validate_checkpoint_recovery_contract(&snapshot)
        .map_err(|message| CheckpointArchiveError::InvalidRecoveryContract { message })?;
    repository.append(CheckpointArchiveEntry {
        snapshot: snapshot.clone(),
        kind: CheckpointArchiveKind::Progress,
    })?;
    Ok(snapshot)
}

pub fn persist_side_effect_checkpoint(
    repository: &mut impl CheckpointArchive,
    runtime: &ActiveRun,
) -> Result<CheckpointSnapshot, CheckpointArchiveError> {
    let snapshot = RuntimeStateMachine::checkpoint_for_persistence(
        runtime,
        CheckpointPersistenceMode::SideEffect,
    );
    validate_checkpoint_recovery_contract(&snapshot)
        .map_err(|message| CheckpointArchiveError::InvalidRecoveryContract { message })?;
    repository.append(CheckpointArchiveEntry {
        snapshot: snapshot.clone(),
        kind: CheckpointArchiveKind::SideEffect,
    })?;
    Ok(snapshot)
}
