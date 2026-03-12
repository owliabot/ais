use ais_agent_control::ids::RunId;
use ais_agent_core::runtime::{BoundaryKind, RunPhase, RunStatus};
use ais_agent_runtime::persistence::{
    RunCatalogEntry, RunCatalogRepository, RunCatalogRepositoryError,
};

use crate::SqliteStore;

impl RunCatalogRepository for SqliteStore {
    fn upsert(&mut self, entry: RunCatalogEntry) -> Result<(), RunCatalogRepositoryError> {
        let status_json = serde_json::to_string(&entry.status).map_err(storage_error)?;
        let phase_json = serde_json::to_string(&entry.phase).map_err(storage_error)?;
        let boundary_json = entry
            .active_boundary_kind
            .as_ref()
            .map(|kind| serde_json::to_string(kind).map_err(storage_error))
            .transpose()?;

        self.connection()
            .execute(
                r#"
                INSERT INTO run_catalog (
                    run_id,
                    mission_id,
                    status_json,
                    phase_json,
                    active_boundary_kind_json,
                    latest_checkpoint_seq,
                    latest_event_seq,
                    latest_revision,
                    created_at_ms,
                    updated_at_ms,
                    terminal_at_ms
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                ON CONFLICT(run_id) DO UPDATE SET
                    mission_id = excluded.mission_id,
                    status_json = excluded.status_json,
                    phase_json = excluded.phase_json,
                    active_boundary_kind_json = excluded.active_boundary_kind_json,
                    latest_checkpoint_seq = excluded.latest_checkpoint_seq,
                    latest_event_seq = excluded.latest_event_seq,
                    latest_revision = excluded.latest_revision,
                    created_at_ms = excluded.created_at_ms,
                    updated_at_ms = excluded.updated_at_ms,
                    terminal_at_ms = excluded.terminal_at_ms
                "#,
                rusqlite::params![
                    entry.run_id.0,
                    entry.mission_id,
                    status_json,
                    phase_json,
                    boundary_json,
                    entry.latest_checkpoint_seq,
                    entry.latest_event_seq,
                    entry.latest_revision,
                    entry.created_at_ms,
                    entry.updated_at_ms,
                    entry.terminal_at_ms,
                ],
            )
            .map_err(storage_error)?;

        Ok(())
    }

    fn load(&self, run_id: &RunId) -> Result<RunCatalogEntry, RunCatalogRepositoryError> {
        self.connection()
            .query_row(
                r#"
                SELECT
                    mission_id,
                    status_json,
                    phase_json,
                    active_boundary_kind_json,
                    latest_checkpoint_seq,
                    latest_event_seq,
                    latest_revision,
                    created_at_ms,
                    updated_at_ms,
                    terminal_at_ms
                FROM run_catalog
                WHERE run_id = ?1
                "#,
                [&run_id.0],
                |row| {
                    let status_json = row.get::<_, String>(1)?;
                    let phase_json = row.get::<_, String>(2)?;
                    let boundary_json = row.get::<_, Option<String>>(3)?;

                    Ok(RunCatalogEntry {
                        run_id: run_id.clone(),
                        mission_id: row.get(0)?,
                        status: serde_json::from_str::<RunStatus>(&status_json)
                            .map_err(deser_error)?,
                        phase: serde_json::from_str::<RunPhase>(&phase_json)
                            .map_err(deser_error)?,
                        active_boundary_kind: boundary_json
                            .map(|json| {
                                serde_json::from_str::<BoundaryKind>(&json).map_err(deser_error)
                            })
                            .transpose()?,
                        latest_checkpoint_seq: row.get(4)?,
                        latest_event_seq: row.get(5)?,
                        latest_revision: row.get(6)?,
                        created_at_ms: row.get(7)?,
                        updated_at_ms: row.get(8)?,
                        terminal_at_ms: row.get(9)?,
                    })
                },
            )
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => RunCatalogRepositoryError::NotFound {
                    run_id: run_id.0.clone(),
                },
                other => storage_error(other),
            })
    }
}

fn storage_error(error: impl ToString) -> RunCatalogRepositoryError {
    RunCatalogRepositoryError::Storage {
        message: error.to_string(),
    }
}

fn deser_error(error: impl ToString) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        0,
        rusqlite::types::Type::Text,
        Box::<dyn std::error::Error + Send + Sync>::from(error.to_string()),
    )
}
