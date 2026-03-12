//! Repository contract for durable run catalog summary state.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use ais_agent_control::ids::RunId;
use ais_agent_core::runtime::{BoundaryKind, RunPhase, RunStatus};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunCatalogEntry {
    pub run_id: RunId,
    pub mission_id: String,
    pub status: RunStatus,
    pub phase: RunPhase,
    pub active_boundary_kind: Option<BoundaryKind>,
    pub latest_checkpoint_seq: u64,
    pub latest_event_seq: Option<u64>,
    pub latest_revision: u64,
    pub created_at_ms: Option<u64>,
    pub updated_at_ms: Option<u64>,
    pub terminal_at_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RunCatalogRepositoryError {
    #[error("run catalog entry not found for run `{run_id}`")]
    NotFound { run_id: String },
    #[error("run catalog repository storage error: {message}")]
    Storage { message: String },
}

pub trait RunCatalogRepository {
    fn upsert(&mut self, entry: RunCatalogEntry) -> Result<(), RunCatalogRepositoryError>;

    fn load(&self, run_id: &RunId) -> Result<RunCatalogEntry, RunCatalogRepositoryError>;
}

impl<T> RunCatalogRepository for &mut T
where
    T: RunCatalogRepository + ?Sized,
{
    fn upsert(&mut self, entry: RunCatalogEntry) -> Result<(), RunCatalogRepositoryError> {
        (**self).upsert(entry)
    }

    fn load(&self, run_id: &RunId) -> Result<RunCatalogEntry, RunCatalogRepositoryError> {
        (**self).load(run_id)
    }
}
