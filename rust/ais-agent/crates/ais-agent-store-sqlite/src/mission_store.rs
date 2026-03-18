use ais_agent_control::ids::RunId;
use ais_agent_core::mission::Mission;
use ais_agent_runtime::persistence::{MissionRepository, MissionRepositoryError};

use crate::{RunStoreError, SqliteStore};

impl MissionRepository for SqliteStore {
    fn insert(&mut self, run_id: RunId, mission: Mission) -> Result<(), MissionRepositoryError> {
        let mission_json =
            serde_json::to_string(&mission).map_err(|error| MissionRepositoryError::Storage {
                message: error.to_string(),
            })?;
        let changed = self
            .connection()
            .execute(
                "INSERT OR IGNORE INTO run_inputs (run_id, mission_json, launch_input_json, created_at_ms) VALUES (?1, ?2, NULL, NULL)",
                (&run_id.0, &mission_json),
            )
            .map_err(|error| MissionRepositoryError::Storage {
                message: error.to_string(),
            })?;

        if changed == 0 {
            return Err(MissionRepositoryError::AlreadyExists { run_id: run_id.0 });
        }

        Ok(())
    }

    fn upsert(&mut self, run_id: RunId, mission: Mission) -> Result<(), MissionRepositoryError> {
        let mission_json =
            serde_json::to_string(&mission).map_err(|error| MissionRepositoryError::Storage {
                message: error.to_string(),
            })?;
        self.connection()
            .execute(
                "INSERT INTO run_inputs (run_id, mission_json, launch_input_json, created_at_ms) VALUES (?1, ?2, NULL, NULL)
                 ON CONFLICT(run_id) DO UPDATE SET mission_json = excluded.mission_json",
                (&run_id.0, &mission_json),
            )
            .map_err(|error| MissionRepositoryError::Storage {
                message: error.to_string(),
            })?;
        Ok(())
    }

    fn load(&self, run_id: &RunId) -> Result<Mission, MissionRepositoryError> {
        let input = self
            .load_run_input(&run_id.0)
            .map_err(|error| match error {
                RunStoreError::NotFound { .. } => MissionRepositoryError::NotFound {
                    run_id: run_id.0.clone(),
                },
                other => MissionRepositoryError::Storage {
                    message: other.to_string(),
                },
            })?;

        serde_json::from_value(input.mission).map_err(|error| MissionRepositoryError::Storage {
            message: error.to_string(),
        })
    }
}
