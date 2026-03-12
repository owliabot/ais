//! Repository contracts for hot runtime state.

use std::collections::BTreeMap;

use thiserror::Error;

use ais_agent_control::ids::RunId;

use crate::runtime::ActiveRun;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RunRepositoryError {
    #[error("runtime not found for run `{run_id}`")]
    NotFound { run_id: String },
    #[error("runtime already exists for run `{run_id}`")]
    AlreadyExists { run_id: String },
    #[error(
        "runtime version conflict for run `{run_id}`: expected revision {expected}, found {actual}"
    )]
    VersionConflict {
        run_id: String,
        expected: u64,
        actual: u64,
    },
}

pub trait RunRepository {
    fn insert(&mut self, runtime: ActiveRun) -> Result<(), RunRepositoryError>;
    fn load(&self, run_id: &RunId) -> Result<ActiveRun, RunRepositoryError>;
    fn save(
        &mut self,
        runtime: ActiveRun,
        expected_revision: Option<u64>,
    ) -> Result<(), RunRepositoryError>;
    fn delete(&mut self, run_id: &RunId) -> Result<(), RunRepositoryError>;
}

#[derive(Debug, Default)]
pub struct InMemoryRunRepository {
    runtimes: BTreeMap<String, ActiveRun>,
}

impl RunRepository for InMemoryRunRepository {
    fn insert(&mut self, runtime: ActiveRun) -> Result<(), RunRepositoryError> {
        if self.runtimes.contains_key(&runtime.run_id.0) {
            return Err(RunRepositoryError::AlreadyExists {
                run_id: runtime.run_id.0,
            });
        }

        self.runtimes.insert(runtime.run_id.0.clone(), runtime);
        Ok(())
    }

    fn load(&self, run_id: &RunId) -> Result<ActiveRun, RunRepositoryError> {
        self.runtimes
            .get(&run_id.0)
            .cloned()
            .ok_or_else(|| RunRepositoryError::NotFound {
                run_id: run_id.0.clone(),
            })
    }

    fn save(
        &mut self,
        runtime: ActiveRun,
        expected_revision: Option<u64>,
    ) -> Result<(), RunRepositoryError> {
        let Some(existing) = self.runtimes.get(&runtime.run_id.0) else {
            return Err(RunRepositoryError::NotFound {
                run_id: runtime.run_id.0,
            });
        };

        if let Some(expected_revision) = expected_revision {
            if existing.revision != expected_revision {
                return Err(RunRepositoryError::VersionConflict {
                    run_id: runtime.run_id.0,
                    expected: expected_revision,
                    actual: existing.revision,
                });
            }
        }

        self.runtimes.insert(runtime.run_id.0.clone(), runtime);
        Ok(())
    }

    fn delete(&mut self, run_id: &RunId) -> Result<(), RunRepositoryError> {
        self.runtimes
            .remove(&run_id.0)
            .map(|_| ())
            .ok_or_else(|| RunRepositoryError::NotFound {
                run_id: run_id.0.clone(),
            })
    }
}
