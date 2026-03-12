//! Repository contract for durable checkpoint archive state.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use ais_agent_core::checkpoint::CheckpointSnapshot;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CheckpointArchiveError {
    #[error("checkpoint not found for run `{run_id}`")]
    NotFound { run_id: String },
    #[error("checkpoint archive rejected invalid recovery contract: {message}")]
    InvalidRecoveryContract { message: String },
    #[error("checkpoint archive storage error: {message}")]
    Storage { message: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointArchiveKind {
    Boundary,
    Progress,
    SideEffect,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointArchiveEntry {
    pub snapshot: CheckpointSnapshot,
    pub kind: CheckpointArchiveKind,
}

pub trait CheckpointArchive {
    fn latest(&self, run_id: &str) -> Result<CheckpointSnapshot, CheckpointArchiveError>;

    fn append(&mut self, entry: CheckpointArchiveEntry) -> Result<(), CheckpointArchiveError>;

    fn history(&self, run_id: &str) -> Result<Vec<CheckpointArchiveEntry>, CheckpointArchiveError>;
}

impl<T> CheckpointArchive for &mut T
where
    T: CheckpointArchive + ?Sized,
{
    fn latest(&self, run_id: &str) -> Result<CheckpointSnapshot, CheckpointArchiveError> {
        (**self).latest(run_id)
    }

    fn append(&mut self, entry: CheckpointArchiveEntry) -> Result<(), CheckpointArchiveError> {
        (**self).append(entry)
    }

    fn history(&self, run_id: &str) -> Result<Vec<CheckpointArchiveEntry>, CheckpointArchiveError> {
        (**self).history(run_id)
    }
}

pub use CheckpointArchive as CheckpointRepository;
pub use CheckpointArchiveError as CheckpointRepositoryError;
