use ais_agent_core::checkpoint::CheckpointSnapshot;
use ais_agent_runtime::persistence::{
    CheckpointArchive, CheckpointArchiveEntry, CheckpointArchiveError, CheckpointArchiveKind,
};

use crate::{run_projection, RunStoreError, SqliteStore};

impl CheckpointArchive for SqliteStore {
    fn latest(&self, run_id: &str) -> Result<CheckpointSnapshot, CheckpointArchiveError> {
        let checkpoint = self
            .load_latest_run_checkpoint(run_id)
            .map_err(|error| match error {
                RunStoreError::NotFound { .. } => CheckpointArchiveError::NotFound {
                    run_id: run_id.to_owned(),
                },
                other => storage_error(other),
            })?;
        serde_json::from_value(checkpoint.snapshot).map_err(storage_error)
    }

    fn append(&mut self, entry: CheckpointArchiveEntry) -> Result<(), CheckpointArchiveError> {
        run_projection::append_checkpoint(self.connection(), &entry.snapshot, entry.kind)
            .map_err(storage_error)?;
        Ok(())
    }

    fn history(&self, run_id: &str) -> Result<Vec<CheckpointArchiveEntry>, CheckpointArchiveError> {
        let history = load_checkpoint_history(self.connection(), run_id).map_err(storage_error)?;
        if history.is_empty() {
            return Err(CheckpointArchiveError::NotFound {
                run_id: run_id.to_owned(),
            });
        }
        Ok(history)
    }
}

fn load_checkpoint_history(
    conn: &rusqlite::Connection,
    run_id: &str,
) -> Result<Vec<CheckpointArchiveEntry>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        r#"
        SELECT checkpoint_kind, snapshot_json
        FROM run_checkpoints
        WHERE run_id = ?1
        ORDER BY checkpoint_id ASC
        "#,
    )?;
    let rows = stmt.query_map([run_id], |row| {
        let kind = row.get::<_, String>(0)?;
        let snapshot_json = row.get::<_, String>(1)?;
        Ok(CheckpointArchiveEntry {
            kind: checkpoint_kind_from_str(&kind)?,
            snapshot: serde_json::from_str::<CheckpointSnapshot>(&snapshot_json)
                .map_err(deser_error)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>()
}

fn checkpoint_kind_from_str(kind: &str) -> Result<CheckpointArchiveKind, rusqlite::Error> {
    match kind {
        "boundary" => Ok(CheckpointArchiveKind::Boundary),
        "progress" => Ok(CheckpointArchiveKind::Progress),
        "side_effect" => Ok(CheckpointArchiveKind::SideEffect),
        other => Err(deser_error(format!("unknown checkpoint kind `{other}`"))),
    }
}

fn storage_error(error: impl ToString) -> CheckpointArchiveError {
    CheckpointArchiveError::Storage {
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
