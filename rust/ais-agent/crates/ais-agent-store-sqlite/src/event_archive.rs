use ais_agent_control::events::RunEventEnvelope;
use ais_agent_runtime::persistence::{
    EventArchive, EventArchiveError, EventArchiveQuery, EventArchiveSlice,
};

use crate::SqliteStore;

impl EventArchive for SqliteStore {
    fn append(&mut self, event: RunEventEnvelope) -> Result<(), EventArchiveError> {
        let event_json = serde_json::to_string(&event).map_err(storage_error)?;
        self.connection()
            .execute(
                r#"
                INSERT INTO event_archive (
                    run_id,
                    event_seq,
                    checkpoint_seq,
                    plan_epoch,
                    event_json
                ) VALUES (?1, ?2, ?3, ?4, ?5)
                "#,
                rusqlite::params![
                    event.run_id.0,
                    event.event_seq,
                    event.checkpoint_seq,
                    event.plan_epoch,
                    event_json,
                ],
            )
            .map_err(storage_error)?;
        Ok(())
    }

    fn read(&self, query: EventArchiveQuery) -> Result<EventArchiveSlice, EventArchiveError> {
        let latest_event_seq = self
            .connection()
            .query_row(
                "SELECT MAX(event_seq) FROM event_archive WHERE run_id = ?1",
                [&query.run_id.0],
                |row| row.get::<_, Option<u64>>(0),
            )
            .map_err(storage_error)?;

        if latest_event_seq.is_none() {
            return Err(EventArchiveError::NotFound {
                run_id: query.run_id.0,
            });
        }

        let mut events = if let Some(fetch_limit) = event_fetch_limit(query.limit) {
            let sql = r#"
                SELECT event_json
                FROM event_archive
                WHERE run_id = ?1
                  AND (?2 IS NULL OR event_seq > ?2)
                ORDER BY event_seq ASC
                LIMIT ?3
            "#;
            let mut stmt = self.connection().prepare(sql).map_err(storage_error)?;
            let rows = stmt
                .query_map(
                    rusqlite::params![query.run_id.0.clone(), query.after_event_seq, fetch_limit],
                    event_from_row,
                )
                .map_err(storage_error)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(storage_error)?
        } else {
            let sql = r#"
                SELECT event_json
                FROM event_archive
                WHERE run_id = ?1
                  AND (?2 IS NULL OR event_seq > ?2)
                ORDER BY event_seq ASC
            "#;
            let mut stmt = self.connection().prepare(sql).map_err(storage_error)?;
            let rows = stmt
                .query_map(
                    rusqlite::params![query.run_id.0.clone(), query.after_event_seq],
                    event_from_row,
                )
                .map_err(storage_error)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(storage_error)?
        };
        let truncated = query
            .limit
            .map(|limit| events.len() > limit)
            .unwrap_or(false);
        if let Some(limit) = query.limit.filter(|_| truncated) {
            events.truncate(limit);
        }
        let next_after_event_seq = events.last().map(|event| event.event_seq);

        Ok(EventArchiveSlice {
            run_id: query.run_id,
            after_event_seq: query.after_event_seq,
            latest_event_seq,
            next_after_event_seq,
            truncated,
            events,
        })
    }
}

fn event_fetch_limit(limit: Option<usize>) -> Option<u64> {
    let limit = limit?;
    let fetch_limit = limit.checked_add(1)?;
    u64::try_from(fetch_limit).ok()
}

fn event_from_row(row: &rusqlite::Row<'_>) -> Result<RunEventEnvelope, rusqlite::Error> {
    let json = row.get::<_, String>(0)?;
    serde_json::from_str::<RunEventEnvelope>(&json).map_err(deser_error)
}

fn storage_error(error: impl ToString) -> EventArchiveError {
    EventArchiveError::Storage {
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
