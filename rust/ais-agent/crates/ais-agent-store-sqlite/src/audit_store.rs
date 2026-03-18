use ais_agent_control::audit::RuntimeAuditRecord;
use ais_agent_runtime::persistence::{
    RuntimeAuditArchive, RuntimeAuditArchiveError, RuntimeAuditQuery, RuntimeAuditSlice,
};

use crate::{run_projection, SqliteStore};

impl RuntimeAuditArchive for SqliteStore {
    fn append(&mut self, record: RuntimeAuditRecord) -> Result<(), RuntimeAuditArchiveError> {
        run_projection::append_audit(self.connection(), &record).map_err(storage_error)?;
        run_projection::update_run_latest_audit_seq(
            self.connection(),
            &record.run_id.0,
            i64::try_from(record.audit_seq).map_err(storage_error)?,
        )
        .map_err(storage_error)?;
        Ok(())
    }

    fn read(
        &self,
        query: RuntimeAuditQuery,
    ) -> Result<RuntimeAuditSlice, RuntimeAuditArchiveError> {
        let not_found_run_id = query.run_id.0.clone();
        let slice = self
            .read_run_audits(crate::StoredRunAuditQuery {
                run_id: query.run_id.0.clone(),
                after_audit_seq: query.after_audit_seq.map(|value| value as i64),
                limit: query.limit,
            })
            .map_err(|error| match error {
                crate::RunStoreError::NotFound { .. } => RuntimeAuditArchiveError::NotFound {
                    run_id: not_found_run_id.clone(),
                },
                other => storage_error(other),
            })?;
        let records = slice
            .records
            .into_iter()
            .map(|record| {
                serde_json::from_value::<RuntimeAuditRecord>(record.payload).map_err(deser_error)
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage_error)?;

        Ok(RuntimeAuditSlice {
            run_id: query.run_id,
            after_audit_seq: query.after_audit_seq,
            latest_audit_seq: slice.latest_audit_seq.map(|value| value as u64),
            next_after_audit_seq: slice.next_after_audit_seq.map(|value| value as u64),
            truncated: slice.truncated,
            records,
        })
    }
}

fn storage_error(error: impl ToString) -> RuntimeAuditArchiveError {
    RuntimeAuditArchiveError::Storage {
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
