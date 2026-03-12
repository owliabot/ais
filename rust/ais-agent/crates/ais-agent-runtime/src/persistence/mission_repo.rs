//! Repository contract for durable mission archive state.

use thiserror::Error;

use ais_agent_control::ids::RunId;
use ais_agent_core::mission::Mission;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MissionRepositoryError {
    #[error("mission already exists for run `{run_id}`")]
    AlreadyExists { run_id: String },
    #[error("mission not found for run `{run_id}`")]
    NotFound { run_id: String },
    #[error("mission repository storage error: {message}")]
    Storage { message: String },
}

pub trait MissionRepository {
    fn insert(&mut self, run_id: RunId, mission: Mission) -> Result<(), MissionRepositoryError>;

    fn upsert(&mut self, run_id: RunId, mission: Mission) -> Result<(), MissionRepositoryError>;

    fn load(&self, run_id: &RunId) -> Result<Mission, MissionRepositoryError>;
}

impl<T> MissionRepository for &mut T
where
    T: MissionRepository + ?Sized,
{
    fn insert(&mut self, run_id: RunId, mission: Mission) -> Result<(), MissionRepositoryError> {
        (**self).insert(run_id, mission)
    }

    fn upsert(&mut self, run_id: RunId, mission: Mission) -> Result<(), MissionRepositoryError> {
        (**self).upsert(run_id, mission)
    }

    fn load(&self, run_id: &RunId) -> Result<Mission, MissionRepositoryError> {
        (**self).load(run_id)
    }
}
