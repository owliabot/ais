use ais_agent_control::audit::RuntimeAuditRecord;
use ais_agent_runtime::persistence::{
    RuntimeAuditArchive, RuntimeAuditArchiveError, RuntimeAuditQuery, RuntimeAuditSlice,
};

use crate::SqliteStore;

impl RuntimeAuditArchive for SqliteStore {
    fn append(&mut self, record: RuntimeAuditRecord) -> Result<(), RuntimeAuditArchiveError> {
        let audit_json = serde_json::to_string(&record).map_err(storage_error)?;
        self.connection()
            .execute(
                r#"
                INSERT INTO runtime_audit_archive (
                    run_id,
                    audit_seq,
                    checkpoint_seq,
                    plan_epoch,
                    audit_id,
                    audit_json
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                "#,
                rusqlite::params![
                    record.run_id.0,
                    record.audit_seq,
                    record.checkpoint_seq,
                    record.plan_epoch,
                    record.audit_id.0,
                    audit_json,
                ],
            )
            .map_err(storage_error)?;
        Ok(())
    }

    fn read(
        &self,
        query: RuntimeAuditQuery,
    ) -> Result<RuntimeAuditSlice, RuntimeAuditArchiveError> {
        let latest_audit_seq = self
            .connection()
            .query_row(
                "SELECT MAX(audit_seq) FROM runtime_audit_archive WHERE run_id = ?1",
                [&query.run_id.0],
                |row| row.get::<_, Option<u64>>(0),
            )
            .map_err(storage_error)?;

        if latest_audit_seq.is_none() {
            return Err(RuntimeAuditArchiveError::NotFound {
                run_id: query.run_id.0,
            });
        }

        let mut records = if let Some(fetch_limit) = fetch_limit(query.limit) {
            let sql = r#"
                SELECT audit_json
                FROM runtime_audit_archive
                WHERE run_id = ?1
                  AND (?2 IS NULL OR audit_seq > ?2)
                ORDER BY audit_seq ASC
                LIMIT ?3
            "#;
            let mut stmt = self.connection().prepare(sql).map_err(storage_error)?;
            let rows = stmt
                .query_map(
                    rusqlite::params![query.run_id.0.clone(), query.after_audit_seq, fetch_limit],
                    record_from_row,
                )
                .map_err(storage_error)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(storage_error)?
        } else {
            let sql = r#"
                SELECT audit_json
                FROM runtime_audit_archive
                WHERE run_id = ?1
                  AND (?2 IS NULL OR audit_seq > ?2)
                ORDER BY audit_seq ASC
            "#;
            let mut stmt = self.connection().prepare(sql).map_err(storage_error)?;
            let rows = stmt
                .query_map(
                    rusqlite::params![query.run_id.0.clone(), query.after_audit_seq],
                    record_from_row,
                )
                .map_err(storage_error)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(storage_error)?
        };
        let truncated = query
            .limit
            .map(|limit| records.len() > limit)
            .unwrap_or(false);
        if let Some(limit) = query.limit.filter(|_| truncated) {
            records.truncate(limit);
        }
        let next_after_audit_seq = records.last().map(|record| record.audit_seq);

        Ok(RuntimeAuditSlice {
            run_id: query.run_id,
            after_audit_seq: query.after_audit_seq,
            latest_audit_seq,
            next_after_audit_seq,
            truncated,
            records,
        })
    }
}

fn fetch_limit(limit: Option<usize>) -> Option<u64> {
    let limit = limit?;
    let fetch_limit = limit.checked_add(1)?;
    u64::try_from(fetch_limit).ok()
}

fn record_from_row(row: &rusqlite::Row<'_>) -> Result<RuntimeAuditRecord, rusqlite::Error> {
    let json = row.get::<_, String>(0)?;
    serde_json::from_str::<RuntimeAuditRecord>(&json).map_err(deser_error)
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
