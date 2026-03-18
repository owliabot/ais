use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use thiserror::Error;

use crate::{
    maintenance_journal::{MaintenanceOperationKind, MaintenanceOperationStatus},
    maintenance_state::{
        sample_store_storage, StoreMaintenanceState, StoreStorageSample,
        STORE_METADATA_SCHEMA_VERSION,
    },
    SqliteStore,
};

pub const STORE_RETENTION_SCHEMA_VERSION: i64 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StorePurgeTable {
    Runs,
    RunInputs,
    RunEvents,
    RunAudits,
    RunCheckpoints,
    RunWaitStates,
    RunClaimHistory,
    MaintenanceJournal,
    StoreMaintenanceState,
}

impl StorePurgeTable {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Runs => "runs",
            Self::RunInputs => "run_inputs",
            Self::RunEvents => "run_events",
            Self::RunAudits => "run_audits",
            Self::RunCheckpoints => "run_checkpoints",
            Self::RunWaitStates => "run_wait_states",
            Self::RunClaimHistory => "run_claim_history",
            Self::MaintenanceJournal => "maintenance_journal",
            Self::StoreMaintenanceState => "store_maintenance_state",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorePruneRequest {
    pub started_at_ms: i64,
    pub finished_at_ms: i64,
    pub terminal_before_ms: i64,
    pub wait_state_orphan_before_ms: i64,
    pub vacuum_freelist_threshold_pages: u32,
    pub schema_retention_version: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorePruneResult {
    pub scanned_terminal_runs: u64,
    pub deleted_checkpoints: u64,
    pub deleted_wait_states: u64,
    pub vacuum: Option<StoreVacuumResult>,
    pub started_at_ms: i64,
    pub finished_at_ms: i64,
    pub terminal_before_ms: i64,
    pub wait_state_orphan_before_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "target")]
pub enum StorePurgeTarget {
    RunId { run_id: String },
    TerminalBefore { terminal_before_ms: i64 },
    Table { table: StorePurgeTable },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorePurgeRequest {
    pub started_at_ms: i64,
    pub finished_at_ms: i64,
    pub target: StorePurgeTarget,
    pub vacuum_freelist_threshold_pages: u32,
    pub schema_retention_version: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorePurgeResult {
    pub deleted_runs: u64,
    pub deleted_run_inputs: u64,
    pub deleted_events: u64,
    pub deleted_audits: u64,
    pub deleted_checkpoints: u64,
    pub deleted_wait_states: u64,
    pub deleted_claim_history: u64,
    pub deleted_table_rows: u64,
    pub vacuum: Option<StoreVacuumResult>,
    pub started_at_ms: i64,
    pub finished_at_ms: i64,
    pub target: StorePurgeTarget,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreVacuumRequest {
    pub started_at_ms: i64,
    pub finished_at_ms: i64,
    pub freelist_threshold_pages: u32,
    pub force: bool,
    pub schema_retention_version: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreVacuumResult {
    pub executed: bool,
    pub freelist_pages_before: u32,
    pub freelist_pages_after: u32,
    pub page_count_before: u32,
    pub page_count_after: u32,
    pub page_size_bytes: u32,
    pub reclaimed_bytes: i64,
    pub started_at_ms: i64,
    pub finished_at_ms: i64,
}

#[derive(Debug, Error)]
pub enum StoreMaintenanceError {
    #[error("sqlite maintenance storage error: {message}")]
    Storage { message: String },
    #[error("sqlite maintenance invalid request: {message}")]
    Invalid { message: String },
}

impl SqliteStore {
    pub fn prune_retention(
        &mut self,
        request: &StorePruneRequest,
    ) -> Result<StorePruneResult, StoreMaintenanceError> {
        validate_prune_request(request)?;
        let storage_before = sample_store_storage(self.connection(), request.started_at_ms)
            .map_err(|error| storage_error(error.to_string()))?;
        let previous_state = self
            .load_store_maintenance_state()
            .map_err(|error| storage_error(error.to_string()))?;
        let tx = self.connection_mut().transaction().map_err(storage_error)?;

        let scanned_terminal_runs = count_rows_with_param(
            &tx,
            "SELECT COUNT(*) FROM runs WHERE terminal_at_ms IS NOT NULL AND terminal_at_ms < ?1",
            request.terminal_before_ms,
        )?;
        let deleted_checkpoints = tx
            .execute(
                r#"
                DELETE FROM run_checkpoints
                WHERE retention_tier = 'terminal_intermediate'
                  AND run_id IN (
                    SELECT run_id
                    FROM runs
                    WHERE terminal_at_ms IS NOT NULL AND terminal_at_ms < ?1
                  )
                "#,
                [request.terminal_before_ms],
            )
            .map_err(storage_error)? as u64;
        let deleted_wait_states = tx
            .execute(
                r#"
                DELETE FROM run_wait_states AS ws
                WHERE NOT EXISTS (
                    SELECT 1
                    FROM runs AS r
                    WHERE r.run_id = ws.run_id
                )
                   OR EXISTS (
                    SELECT 1
                    FROM runs AS r
                    WHERE r.run_id = ws.run_id
                      AND (
                        (r.terminal_at_ms IS NOT NULL AND r.terminal_at_ms < ?1)
                        OR (
                            (r.active_wait_kind IS NULL OR r.active_wait_kind != ws.wait_kind)
                            AND COALESCE(r.updated_at_ms, r.created_at_ms, ws.entered_at_ms) < ?1
                        )
                      )
                )
                "#,
                [request.wait_state_orphan_before_ms],
            )
            .map_err(storage_error)? as u64;

        let result = StorePruneResult {
            scanned_terminal_runs,
            deleted_checkpoints,
            deleted_wait_states,
            vacuum: None,
            started_at_ms: request.started_at_ms,
            finished_at_ms: request.finished_at_ms,
            terminal_before_ms: request.terminal_before_ms,
            wait_state_orphan_before_ms: request.wait_state_orphan_before_ms,
        };
        let storage_after = sample_tx_storage(&tx, request.finished_at_ms)?;
        let summary = json!({
            "operation": "prune",
            "scanned_terminal_runs": result.scanned_terminal_runs,
            "deleted_checkpoints": result.deleted_checkpoints,
            "deleted_wait_states": result.deleted_wait_states,
            "terminal_before_ms": result.terminal_before_ms,
            "wait_state_orphan_before_ms": result.wait_state_orphan_before_ms,
            "storage_before": storage_sample_json(storage_before),
            "storage_after": storage_sample_json(storage_after),
            "storage_delta": storage_delta_json(storage_before, storage_after),
            "schema_retention_version": request.schema_retention_version,
        });

        insert_maintenance_journal(
            &tx,
            MaintenanceOperationKind::Prune,
            request.started_at_ms,
            request.finished_at_ms,
            MaintenanceOperationStatus::Succeeded,
            &summary,
        )?;
        upsert_maintenance_state(&tx, prune_state(previous_state.clone(), request, &result))?;

        tx.commit().map_err(storage_error)?;
        let vacuum = self.maybe_vacuum_retention(&StoreVacuumRequest {
            started_at_ms: request.finished_at_ms,
            finished_at_ms: request.finished_at_ms,
            freelist_threshold_pages: request.vacuum_freelist_threshold_pages,
            force: false,
            schema_retention_version: request.schema_retention_version,
        })?;
        if !vacuum.executed {
            refresh_storage_sample(self, request.finished_at_ms)?;
        }
        Ok(StorePruneResult {
            vacuum: Some(vacuum),
            ..result
        })
    }

    pub fn purge_retention(
        &mut self,
        request: &StorePurgeRequest,
    ) -> Result<StorePurgeResult, StoreMaintenanceError> {
        validate_purge_request(request)?;
        let storage_before = sample_store_storage(self.connection(), request.started_at_ms)
            .map_err(|error| storage_error(error.to_string()))?;
        let previous_state = self
            .load_store_maintenance_state()
            .map_err(|error| storage_error(error.to_string()))?;
        let tx = self.connection_mut().transaction().map_err(storage_error)?;

        let mut result = StorePurgeResult {
            deleted_runs: 0,
            deleted_run_inputs: 0,
            deleted_events: 0,
            deleted_audits: 0,
            deleted_checkpoints: 0,
            deleted_wait_states: 0,
            deleted_claim_history: 0,
            deleted_table_rows: 0,
            vacuum: None,
            started_at_ms: request.started_at_ms,
            finished_at_ms: request.finished_at_ms,
            target: request.target.clone(),
        };

        match &request.target {
            StorePurgeTarget::RunId { run_id } => {
                result.deleted_run_inputs = tx
                    .execute("DELETE FROM run_inputs WHERE run_id = ?1", [run_id])
                    .map_err(storage_error)? as u64;
                result.deleted_events = tx
                    .execute("DELETE FROM run_events WHERE run_id = ?1", [run_id])
                    .map_err(storage_error)? as u64;
                result.deleted_audits = tx
                    .execute("DELETE FROM run_audits WHERE run_id = ?1", [run_id])
                    .map_err(storage_error)? as u64;
                result.deleted_checkpoints = tx
                    .execute("DELETE FROM run_checkpoints WHERE run_id = ?1", [run_id])
                    .map_err(storage_error)? as u64;
                result.deleted_wait_states = tx
                    .execute("DELETE FROM run_wait_states WHERE run_id = ?1", [run_id])
                    .map_err(storage_error)? as u64;
                result.deleted_claim_history = tx
                    .execute("DELETE FROM run_claim_history WHERE run_id = ?1", [run_id])
                    .map_err(storage_error)? as u64;
                result.deleted_runs = tx
                    .execute("DELETE FROM runs WHERE run_id = ?1", [run_id])
                    .map_err(storage_error)? as u64;
            }
            StorePurgeTarget::TerminalBefore { terminal_before_ms } => {
                result.deleted_run_inputs = tx
                    .execute(
                        r#"
                        DELETE FROM run_inputs
                        WHERE run_id IN (
                            SELECT run_id FROM runs
                            WHERE terminal_at_ms IS NOT NULL AND terminal_at_ms < ?1
                        )
                        "#,
                        [terminal_before_ms],
                    )
                    .map_err(storage_error)? as u64;
                result.deleted_events = tx
                    .execute(
                        r#"
                        DELETE FROM run_events
                        WHERE run_id IN (
                            SELECT run_id FROM runs
                            WHERE terminal_at_ms IS NOT NULL AND terminal_at_ms < ?1
                        )
                        "#,
                        [terminal_before_ms],
                    )
                    .map_err(storage_error)? as u64;
                result.deleted_audits = tx
                    .execute(
                        r#"
                        DELETE FROM run_audits
                        WHERE run_id IN (
                            SELECT run_id FROM runs
                            WHERE terminal_at_ms IS NOT NULL AND terminal_at_ms < ?1
                        )
                        "#,
                        [terminal_before_ms],
                    )
                    .map_err(storage_error)? as u64;
                result.deleted_checkpoints = tx
                    .execute(
                        r#"
                        DELETE FROM run_checkpoints
                        WHERE run_id IN (
                            SELECT run_id FROM runs
                            WHERE terminal_at_ms IS NOT NULL AND terminal_at_ms < ?1
                        )
                        "#,
                        [terminal_before_ms],
                    )
                    .map_err(storage_error)? as u64;
                result.deleted_wait_states = tx
                    .execute(
                        r#"
                        DELETE FROM run_wait_states
                        WHERE run_id IN (
                            SELECT run_id FROM runs
                            WHERE terminal_at_ms IS NOT NULL AND terminal_at_ms < ?1
                        )
                        "#,
                        [terminal_before_ms],
                    )
                    .map_err(storage_error)? as u64;
                result.deleted_claim_history = tx
                    .execute(
                        r#"
                        DELETE FROM run_claim_history
                        WHERE run_id IN (
                            SELECT run_id FROM runs
                            WHERE terminal_at_ms IS NOT NULL AND terminal_at_ms < ?1
                        )
                        "#,
                        [terminal_before_ms],
                    )
                    .map_err(storage_error)? as u64;
                result.deleted_runs = tx
                    .execute(
                        r#"
                        DELETE FROM runs
                        WHERE terminal_at_ms IS NOT NULL AND terminal_at_ms < ?1
                        "#,
                        [terminal_before_ms],
                    )
                    .map_err(storage_error)? as u64;
            }
            StorePurgeTarget::Table { table } => {
                let sql = format!("DELETE FROM {}", table.as_str());
                result.deleted_table_rows = tx.execute(&sql, []).map_err(storage_error)? as u64;
            }
        }

        let storage_after = sample_tx_storage(&tx, request.finished_at_ms)?;
        let summary = json!({
            "operation": "purge",
            "target": &result.target,
            "deleted_runs": result.deleted_runs,
            "deleted_run_inputs": result.deleted_run_inputs,
            "deleted_events": result.deleted_events,
            "deleted_audits": result.deleted_audits,
            "deleted_checkpoints": result.deleted_checkpoints,
            "deleted_wait_states": result.deleted_wait_states,
            "deleted_claim_history": result.deleted_claim_history,
            "deleted_table_rows": result.deleted_table_rows,
            "storage_before": storage_sample_json(storage_before),
            "storage_after": storage_sample_json(storage_after),
            "storage_delta": storage_delta_json(storage_before, storage_after),
            "schema_retention_version": request.schema_retention_version,
        });
        if should_write_maintenance_journal(&request.target) {
            insert_maintenance_journal(
                &tx,
                MaintenanceOperationKind::Purge,
                request.started_at_ms,
                request.finished_at_ms,
                MaintenanceOperationStatus::Succeeded,
                &summary,
            )?;
        }
        if should_write_maintenance_state(&request.target) {
            upsert_maintenance_state(&tx, purge_state(previous_state.clone(), request, &result))?;
        }

        tx.commit().map_err(storage_error)?;
        let vacuum = if should_follow_up_vacuum(&request.target) {
            Some(self.maybe_vacuum_retention(&StoreVacuumRequest {
                started_at_ms: request.finished_at_ms,
                finished_at_ms: request.finished_at_ms,
                freelist_threshold_pages: request.vacuum_freelist_threshold_pages,
                force: false,
                schema_retention_version: request.schema_retention_version,
            })?)
        } else {
            None
        };
        if vacuum.as_ref().is_none_or(|value| !value.executed)
            && should_write_maintenance_state(&request.target)
        {
            refresh_storage_sample(self, request.finished_at_ms)?;
        }
        Ok(StorePurgeResult { vacuum, ..result })
    }

    pub fn maybe_vacuum_retention(
        &mut self,
        request: &StoreVacuumRequest,
    ) -> Result<StoreVacuumResult, StoreMaintenanceError> {
        validate_vacuum_request(request)?;
        let previous_state = self
            .load_store_maintenance_state()
            .map_err(|error| storage_error(error.to_string()))?;
        let page_size_bytes = pragma_u32(self.connection(), "page_size")?;
        let page_count_before = pragma_u32(self.connection(), "page_count")?;
        let freelist_pages_before = pragma_u32(self.connection(), "freelist_count")?;
        let should_execute =
            request.force || freelist_pages_before >= request.freelist_threshold_pages;

        if should_execute {
            let _ = self
                .connection()
                .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
            self.connection()
                .execute_batch("VACUUM")
                .map_err(storage_error)?;
        }

        let page_count_after = pragma_u32(self.connection(), "page_count")?;
        let freelist_pages_after = pragma_u32(self.connection(), "freelist_count")?;
        let reclaimed_pages = page_count_before.saturating_sub(page_count_after);
        let result = StoreVacuumResult {
            executed: should_execute,
            freelist_pages_before,
            freelist_pages_after,
            page_count_before,
            page_count_after,
            page_size_bytes,
            reclaimed_bytes: i64::from(reclaimed_pages) * i64::from(page_size_bytes),
            started_at_ms: request.started_at_ms,
            finished_at_ms: request.finished_at_ms,
        };

        if !result.executed {
            return Ok(result);
        }

        let tx = self.connection_mut().transaction().map_err(storage_error)?;
        let summary = json!({
            "operation": "vacuum",
            "executed": result.executed,
            "freelist_pages_before": result.freelist_pages_before,
            "freelist_pages_after": result.freelist_pages_after,
            "page_count_before": result.page_count_before,
            "page_count_after": result.page_count_after,
            "page_size_bytes": result.page_size_bytes,
            "reclaimed_bytes": result.reclaimed_bytes,
            "storage_before": {
                "page_count": result.page_count_before,
                "freelist_count": result.freelist_pages_before,
                "db_bytes": i64::from(result.page_count_before) * i64::from(result.page_size_bytes),
                "sampled_at_ms": request.started_at_ms,
            },
            "storage_after": {
                "page_count": result.page_count_after,
                "freelist_count": result.freelist_pages_after,
                "db_bytes": i64::from(result.page_count_after) * i64::from(result.page_size_bytes),
                "sampled_at_ms": request.finished_at_ms,
            },
            "storage_delta": {
                "page_count": i64::from(result.page_count_after) - i64::from(result.page_count_before),
                "freelist_count": i64::from(result.freelist_pages_after) - i64::from(result.freelist_pages_before),
                "db_bytes": i64::from(result.page_count_after.saturating_mul(result.page_size_bytes))
                    - i64::from(result.page_count_before.saturating_mul(result.page_size_bytes)),
            },
            "freelist_threshold_pages": request.freelist_threshold_pages,
            "force": request.force,
            "schema_retention_version": request.schema_retention_version,
        });
        insert_maintenance_journal(
            &tx,
            MaintenanceOperationKind::Vacuum,
            request.started_at_ms,
            request.finished_at_ms,
            MaintenanceOperationStatus::Succeeded,
            &summary,
        )?;
        upsert_maintenance_state(
            &tx,
            vacuum_state(
                previous_state,
                request,
                &result,
                i64::from(page_count_after),
                i64::from(freelist_pages_after),
                i64::from(page_count_after) * i64::from(page_size_bytes),
            ),
        )?;
        tx.commit().map_err(storage_error)?;
        Ok(result)
    }
}

fn validate_prune_request(request: &StorePruneRequest) -> Result<(), StoreMaintenanceError> {
    if request.finished_at_ms < request.started_at_ms {
        return Err(StoreMaintenanceError::Invalid {
            message: "prune finished_at_ms must be greater than or equal to started_at_ms"
                .to_owned(),
        });
    }
    Ok(())
}

fn validate_purge_request(request: &StorePurgeRequest) -> Result<(), StoreMaintenanceError> {
    if request.finished_at_ms < request.started_at_ms {
        return Err(StoreMaintenanceError::Invalid {
            message: "purge finished_at_ms must be greater than or equal to started_at_ms"
                .to_owned(),
        });
    }
    Ok(())
}

fn validate_vacuum_request(request: &StoreVacuumRequest) -> Result<(), StoreMaintenanceError> {
    if request.finished_at_ms < request.started_at_ms {
        return Err(StoreMaintenanceError::Invalid {
            message: "vacuum finished_at_ms must be greater than or equal to started_at_ms"
                .to_owned(),
        });
    }
    if request.freelist_threshold_pages == 0 && !request.force {
        return Err(StoreMaintenanceError::Invalid {
            message: "vacuum freelist_threshold_pages must be greater than zero unless force=true"
                .to_owned(),
        });
    }
    Ok(())
}

fn count_rows_with_param(
    conn: &rusqlite::Transaction<'_>,
    sql: &str,
    value: i64,
) -> Result<u64, StoreMaintenanceError> {
    conn.query_row(sql, [value], |row| row.get::<_, i64>(0))
        .map(|value| value as u64)
        .map_err(storage_error)
}

fn sample_tx_storage(
    tx: &rusqlite::Transaction<'_>,
    sampled_at_ms: i64,
) -> Result<StoreStorageSample, StoreMaintenanceError> {
    let page_size = tx
        .pragma_query_value(None, "page_size", |row| row.get::<_, i64>(0))
        .map_err(storage_error)?;
    let page_count = tx
        .pragma_query_value(None, "page_count", |row| row.get::<_, i64>(0))
        .map_err(storage_error)?;
    let freelist_count = tx
        .pragma_query_value(None, "freelist_count", |row| row.get::<_, i64>(0))
        .map_err(storage_error)?;

    Ok(StoreStorageSample {
        page_count,
        freelist_count,
        db_bytes: page_count.saturating_mul(page_size),
        sampled_at_ms,
    })
}

fn pragma_u32(conn: &rusqlite::Connection, name: &str) -> Result<u32, StoreMaintenanceError> {
    conn.pragma_query_value(None, name, |row| row.get::<_, u32>(0))
        .map_err(storage_error)
}

fn insert_maintenance_journal(
    conn: &rusqlite::Transaction<'_>,
    operation_kind: MaintenanceOperationKind,
    started_at_ms: i64,
    finished_at_ms: i64,
    status: MaintenanceOperationStatus,
    summary: &Value,
) -> Result<(), StoreMaintenanceError> {
    let summary_json = serde_json::to_string(summary).map_err(storage_error)?;
    conn.execute(
        r#"
        INSERT INTO maintenance_journal (
            operation_kind,
            started_at_ms,
            finished_at_ms,
            status,
            summary_json
        ) VALUES (?1, ?2, ?3, ?4, ?5)
        "#,
        rusqlite::params![
            maintenance_operation_kind_as_str(&operation_kind),
            started_at_ms,
            finished_at_ms,
            maintenance_operation_status_as_str(&status),
            summary_json,
        ],
    )
    .map_err(storage_error)?;
    Ok(())
}

fn upsert_maintenance_state(
    conn: &rusqlite::Transaction<'_>,
    state: StoreMaintenanceState,
) -> Result<(), StoreMaintenanceError> {
    conn.execute(
        r#"
        INSERT INTO store_maintenance_state (
            singleton_key,
            last_operation_kind,
            last_operation_status,
            last_store_opened_at_ms,
            last_prune_started_at_ms,
            last_prune_finished_at_ms,
            last_pruned_terminal_before_ms,
            last_prune_deleted_rows,
            last_purge_deleted_rows,
            last_vacuum_started_at_ms,
            last_vacuum_finished_at_ms,
            last_vacuum_at_ms,
            last_wal_checkpoint_at_ms,
            last_known_page_count,
            last_known_freelist_count,
            last_known_db_bytes,
            last_growth_sampled_at_ms,
            schema_retention_version,
            metadata_schema_version
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)
        ON CONFLICT(singleton_key) DO UPDATE SET
            last_operation_kind = excluded.last_operation_kind,
            last_operation_status = excluded.last_operation_status,
            last_store_opened_at_ms = excluded.last_store_opened_at_ms,
            last_prune_started_at_ms = excluded.last_prune_started_at_ms,
            last_prune_finished_at_ms = excluded.last_prune_finished_at_ms,
            last_pruned_terminal_before_ms = excluded.last_pruned_terminal_before_ms,
            last_prune_deleted_rows = excluded.last_prune_deleted_rows,
            last_purge_deleted_rows = excluded.last_purge_deleted_rows,
            last_vacuum_started_at_ms = excluded.last_vacuum_started_at_ms,
            last_vacuum_finished_at_ms = excluded.last_vacuum_finished_at_ms,
            last_vacuum_at_ms = excluded.last_vacuum_at_ms,
            last_wal_checkpoint_at_ms = excluded.last_wal_checkpoint_at_ms,
            last_known_page_count = excluded.last_known_page_count,
            last_known_freelist_count = excluded.last_known_freelist_count,
            last_known_db_bytes = excluded.last_known_db_bytes,
            last_growth_sampled_at_ms = excluded.last_growth_sampled_at_ms,
            schema_retention_version = excluded.schema_retention_version,
            metadata_schema_version = excluded.metadata_schema_version
        "#,
        rusqlite::params![
            "default",
            state
                .last_operation_kind
                .as_ref()
                .map(maintenance_operation_kind_as_str),
            state
                .last_operation_status
                .as_ref()
                .map(maintenance_operation_status_as_str),
            state.last_store_opened_at_ms,
            state.last_prune_started_at_ms,
            state.last_prune_finished_at_ms,
            state.last_pruned_terminal_before_ms,
            state.last_prune_deleted_rows,
            state.last_purge_deleted_rows,
            state.last_vacuum_started_at_ms,
            state.last_vacuum_finished_at_ms,
            state.last_vacuum_at_ms,
            state.last_wal_checkpoint_at_ms,
            state.last_known_page_count,
            state.last_known_freelist_count,
            state.last_known_db_bytes,
            state.last_growth_sampled_at_ms,
            state.schema_retention_version,
            state.metadata_schema_version,
        ],
    )
    .map_err(storage_error)?;
    Ok(())
}

fn prune_state(
    previous_state: Option<StoreMaintenanceState>,
    request: &StorePruneRequest,
    result: &StorePruneResult,
) -> StoreMaintenanceState {
    let mut state = previous_state.unwrap_or_else(StoreMaintenanceState::baseline);
    state.last_operation_kind = Some(MaintenanceOperationKind::Prune);
    state.last_operation_status = Some(MaintenanceOperationStatus::Succeeded);
    state.last_prune_started_at_ms = Some(request.started_at_ms);
    state.last_prune_finished_at_ms = Some(request.finished_at_ms);
    state.last_pruned_terminal_before_ms = Some(request.terminal_before_ms);
    state.last_prune_deleted_rows =
        Some((result.deleted_checkpoints + result.deleted_wait_states) as i64);
    state.schema_retention_version = request.schema_retention_version;
    state.metadata_schema_version = STORE_METADATA_SCHEMA_VERSION;
    state
}

fn purge_state(
    previous_state: Option<StoreMaintenanceState>,
    request: &StorePurgeRequest,
    result: &StorePurgeResult,
) -> StoreMaintenanceState {
    let mut state = previous_state.unwrap_or_else(StoreMaintenanceState::baseline);
    state.last_operation_kind = Some(MaintenanceOperationKind::Purge);
    state.last_operation_status = Some(MaintenanceOperationStatus::Succeeded);
    state.last_purge_deleted_rows = Some(
        (result.deleted_runs
            + result.deleted_run_inputs
            + result.deleted_events
            + result.deleted_audits
            + result.deleted_checkpoints
            + result.deleted_wait_states
            + result.deleted_claim_history
            + result.deleted_table_rows) as i64,
    );
    state.schema_retention_version = request.schema_retention_version;
    state.metadata_schema_version = STORE_METADATA_SCHEMA_VERSION;
    state
}

fn vacuum_state(
    previous_state: Option<StoreMaintenanceState>,
    request: &StoreVacuumRequest,
    _result: &StoreVacuumResult,
    page_count_after: i64,
    freelist_count_after: i64,
    db_bytes_after: i64,
) -> StoreMaintenanceState {
    let mut state = previous_state.unwrap_or_else(StoreMaintenanceState::baseline);
    state.last_operation_kind = Some(MaintenanceOperationKind::Vacuum);
    state.last_operation_status = Some(MaintenanceOperationStatus::Succeeded);
    state.last_vacuum_started_at_ms = Some(request.started_at_ms);
    state.last_vacuum_finished_at_ms = Some(request.finished_at_ms);
    state.last_vacuum_at_ms = Some(request.finished_at_ms);
    state.last_wal_checkpoint_at_ms = Some(request.started_at_ms);
    state.last_known_page_count = Some(page_count_after);
    state.last_known_freelist_count = Some(freelist_count_after);
    state.last_known_db_bytes = Some(db_bytes_after);
    state.last_growth_sampled_at_ms = Some(request.finished_at_ms);
    state.schema_retention_version = request.schema_retention_version;
    state.metadata_schema_version = STORE_METADATA_SCHEMA_VERSION;
    state
}

fn refresh_storage_sample(
    store: &mut SqliteStore,
    sampled_at_ms: i64,
) -> Result<(), StoreMaintenanceError> {
    let mut state = store
        .load_store_maintenance_state()
        .map_err(|error| storage_error(error.to_string()))?
        .unwrap_or_else(StoreMaintenanceState::baseline);
    state.apply_storage_sample(
        sample_store_storage(store.connection(), sampled_at_ms)
            .map_err(|error| storage_error(error.to_string()))?,
    );
    state.metadata_schema_version = STORE_METADATA_SCHEMA_VERSION;
    store
        .upsert_store_maintenance_state(&state)
        .map_err(|error| storage_error(error.to_string()))
}

fn should_write_maintenance_journal(target: &StorePurgeTarget) -> bool {
    !matches!(
        target,
        StorePurgeTarget::Table {
            table: StorePurgeTable::MaintenanceJournal
        }
    )
}

fn should_write_maintenance_state(target: &StorePurgeTarget) -> bool {
    !matches!(
        target,
        StorePurgeTarget::Table {
            table: StorePurgeTable::StoreMaintenanceState
        }
    )
}

fn should_follow_up_vacuum(target: &StorePurgeTarget) -> bool {
    should_write_maintenance_journal(target) && should_write_maintenance_state(target)
}

fn storage_sample_json(sample: StoreStorageSample) -> Value {
    json!({
        "page_count": sample.page_count,
        "freelist_count": sample.freelist_count,
        "db_bytes": sample.db_bytes,
        "sampled_at_ms": sample.sampled_at_ms,
    })
}

fn storage_delta_json(before: StoreStorageSample, after: StoreStorageSample) -> Value {
    json!({
        "page_count": after.page_count - before.page_count,
        "freelist_count": after.freelist_count - before.freelist_count,
        "db_bytes": after.db_bytes - before.db_bytes,
    })
}

fn maintenance_operation_kind_as_str(kind: &MaintenanceOperationKind) -> &'static str {
    match kind {
        MaintenanceOperationKind::Prune => "prune",
        MaintenanceOperationKind::Purge => "purge",
        MaintenanceOperationKind::Vacuum => "vacuum",
    }
}

fn maintenance_operation_status_as_str(status: &MaintenanceOperationStatus) -> &'static str {
    match status {
        MaintenanceOperationStatus::Started => "started",
        MaintenanceOperationStatus::Succeeded => "succeeded",
        MaintenanceOperationStatus::Failed => "failed",
    }
}

fn storage_error(error: impl ToString) -> StoreMaintenanceError {
    StoreMaintenanceError::Storage {
        message: error.to_string(),
    }
}
