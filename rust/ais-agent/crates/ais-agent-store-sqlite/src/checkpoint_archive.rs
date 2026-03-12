use ais_agent_core::checkpoint::CheckpointSnapshot;
use ais_agent_runtime::persistence::{
    CheckpointArchive, CheckpointArchiveEntry, CheckpointArchiveError, CheckpointArchiveKind,
};

use crate::SqliteStore;

impl CheckpointArchive for SqliteStore {
    fn latest(&self, run_id: &str) -> Result<CheckpointSnapshot, CheckpointArchiveError> {
        self.connection()
            .query_row(
                r#"
                SELECT snapshot_json
                FROM checkpoint_archive
                WHERE run_id = ?1
                ORDER BY checkpoint_seq DESC, plan_epoch DESC, archive_id DESC
                LIMIT 1
                "#,
                [run_id],
                |row| {
                    let json = row.get::<_, String>(0)?;
                    serde_json::from_str::<CheckpointSnapshot>(&json).map_err(deser_error)
                },
            )
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => CheckpointArchiveError::NotFound {
                    run_id: run_id.to_owned(),
                },
                other => storage_error(other),
            })
    }

    fn append(&mut self, entry: CheckpointArchiveEntry) -> Result<(), CheckpointArchiveError> {
        let snapshot_json = serde_json::to_string(&entry.snapshot).map_err(storage_error)?;
        let kind_json = serde_json::to_string(&entry.kind).map_err(storage_error)?;
        self.connection()
            .execute(
                r#"
                INSERT INTO checkpoint_archive (
                    run_id,
                    checkpoint_seq,
                    plan_epoch,
                    archive_kind_json,
                    snapshot_json
                ) VALUES (?1, ?2, ?3, ?4, ?5)
                "#,
                rusqlite::params![
                    entry.snapshot.run_id,
                    entry.snapshot.checkpoint_seq,
                    entry.snapshot.plan_epoch,
                    kind_json,
                    snapshot_json,
                ],
            )
            .map_err(storage_error)?;
        Ok(())
    }

    fn history(&self, run_id: &str) -> Result<Vec<CheckpointArchiveEntry>, CheckpointArchiveError> {
        let mut stmt = self
            .connection()
            .prepare(
                r#"
                SELECT archive_kind_json, snapshot_json
                FROM checkpoint_archive
                WHERE run_id = ?1
                ORDER BY archive_id ASC
                "#,
            )
            .map_err(storage_error)?;
        let rows = stmt
            .query_map([run_id], |row| {
                let kind_json = row.get::<_, String>(0)?;
                let snapshot_json = row.get::<_, String>(1)?;
                Ok(CheckpointArchiveEntry {
                    kind: serde_json::from_str::<CheckpointArchiveKind>(&kind_json)
                        .map_err(deser_error)?,
                    snapshot: serde_json::from_str::<CheckpointSnapshot>(&snapshot_json)
                        .map_err(deser_error)?,
                })
            })
            .map_err(storage_error)?;

        let history = rows.collect::<Result<Vec<_>, _>>().map_err(storage_error)?;
        if history.is_empty() {
            return Err(CheckpointArchiveError::NotFound {
                run_id: run_id.to_owned(),
            });
        }
        Ok(history)
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
