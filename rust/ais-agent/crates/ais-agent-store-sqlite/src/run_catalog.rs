use ais_agent_control::ids::RunId;
use ais_agent_core::runtime::{BoundaryKind, RunPhase, RunStatus};
use ais_agent_runtime::persistence::{
    RunCatalogEntry, RunCatalogRepository, RunCatalogRepositoryError,
};

use crate::{run_projection, SqliteStore};

impl RunCatalogRepository for SqliteStore {
    fn upsert(&mut self, entry: RunCatalogEntry) -> Result<(), RunCatalogRepositoryError> {
        run_projection::upsert_run_head(self.connection(), &entry, None, None)
            .map_err(storage_error)?;

        Ok(())
    }

    fn load(&self, run_id: &RunId) -> Result<RunCatalogEntry, RunCatalogRepositoryError> {
        let head = self.load_run_head(&run_id.0).map_err(|error| match error {
            crate::RunStoreError::NotFound { .. } => RunCatalogRepositoryError::NotFound {
                run_id: run_id.0.clone(),
            },
            other => storage_error(other),
        })?;

        Ok(RunCatalogEntry {
            run_id: run_id.clone(),
            mission_id: head.mission_id,
            status: parse_run_status(&head.status)?,
            phase: parse_run_phase(head.phase.as_deref())?,
            active_boundary_kind: head
                .active_boundary_kind
                .as_deref()
                .map(parse_boundary_kind)
                .transpose()?,
            latest_checkpoint_seq: head.latest_checkpoint_seq.unwrap_or_default() as u64,
            latest_event_seq: head.latest_event_seq.map(|value| value as u64),
            latest_revision: head.latest_checkpoint_seq.unwrap_or_default() as u64,
            created_at_ms: head.created_at_ms.map(|value| value as u64),
            updated_at_ms: head.updated_at_ms.map(|value| value as u64),
            terminal_at_ms: head.terminal_at_ms.map(|value| value as u64),
        })
    }
}

fn parse_run_status(value: &str) -> Result<RunStatus, RunCatalogRepositoryError> {
    serde_json::from_value(serde_json::Value::String(value.to_owned())).map_err(storage_error)
}

fn parse_run_phase(value: Option<&str>) -> Result<RunPhase, RunCatalogRepositoryError> {
    let value = value.ok_or_else(|| RunCatalogRepositoryError::Storage {
        message: "run head missing phase".to_owned(),
    })?;
    serde_json::from_value(serde_json::Value::String(value.to_owned())).map_err(storage_error)
}

fn parse_boundary_kind(value: &str) -> Result<BoundaryKind, RunCatalogRepositoryError> {
    serde_json::from_value(serde_json::Value::String(value.to_owned())).map_err(storage_error)
}

fn storage_error(error: impl ToString) -> RunCatalogRepositoryError {
    RunCatalogRepositoryError::Storage {
        message: error.to_string(),
    }
}
