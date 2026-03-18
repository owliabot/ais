use ais_agent_control::events::RunEventEnvelope;
use ais_agent_runtime::persistence::{
    EventArchive, EventArchiveError, EventArchiveQuery, EventArchiveSlice,
};

use crate::{run_projection, SqliteStore};

impl EventArchive for SqliteStore {
    fn append(&mut self, event: RunEventEnvelope) -> Result<(), EventArchiveError> {
        run_projection::append_event(self.connection(), &event).map_err(storage_error)?;
        Ok(())
    }

    fn read(&self, query: EventArchiveQuery) -> Result<EventArchiveSlice, EventArchiveError> {
        let not_found_run_id = query.run_id.0.clone();
        let slice = self
            .read_run_events(crate::StoredRunEventQuery {
                run_id: query.run_id.0.clone(),
                after_event_seq: query.after_event_seq.map(|value| value as i64),
                limit: query.limit,
            })
            .map_err(|error| match error {
                crate::RunStoreError::NotFound { .. } => EventArchiveError::NotFound {
                    run_id: not_found_run_id.clone(),
                },
                other => storage_error(other),
            })?;
        let events = slice
            .records
            .into_iter()
            .map(|record| {
                serde_json::from_value::<RunEventEnvelope>(record.payload).map_err(deser_error)
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage_error)?;

        Ok(EventArchiveSlice {
            run_id: query.run_id,
            after_event_seq: query.after_event_seq,
            latest_event_seq: slice.latest_event_seq.map(|value| value as u64),
            next_after_event_seq: slice.next_after_event_seq.map(|value| value as u64),
            truncated: slice.truncated,
            events,
        })
    }
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
