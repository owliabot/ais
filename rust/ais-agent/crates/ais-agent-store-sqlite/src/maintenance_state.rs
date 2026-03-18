use thiserror::Error;

use crate::{
    maintenance::STORE_RETENTION_SCHEMA_VERSION,
    maintenance_journal::{MaintenanceOperationKind, MaintenanceOperationStatus},
    SqliteStore,
};

const DEFAULT_STATE_KEY: &str = "default";
pub const STORE_METADATA_SCHEMA_VERSION: i64 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreMaintenanceState {
    pub last_operation_kind: Option<MaintenanceOperationKind>,
    pub last_operation_status: Option<MaintenanceOperationStatus>,
    pub last_store_opened_at_ms: Option<i64>,
    pub last_prune_started_at_ms: Option<i64>,
    pub last_prune_finished_at_ms: Option<i64>,
    pub last_pruned_terminal_before_ms: Option<i64>,
    pub last_prune_deleted_rows: Option<i64>,
    pub last_purge_deleted_rows: Option<i64>,
    pub last_vacuum_started_at_ms: Option<i64>,
    pub last_vacuum_finished_at_ms: Option<i64>,
    pub last_vacuum_at_ms: Option<i64>,
    pub last_wal_checkpoint_at_ms: Option<i64>,
    pub last_known_page_count: Option<i64>,
    pub last_known_freelist_count: Option<i64>,
    pub last_known_db_bytes: Option<i64>,
    pub last_growth_sampled_at_ms: Option<i64>,
    pub schema_retention_version: i64,
    pub metadata_schema_version: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StoreStorageSample {
    pub page_count: i64,
    pub freelist_count: i64,
    pub db_bytes: i64,
    pub sampled_at_ms: i64,
}

impl StoreMaintenanceState {
    pub(crate) fn baseline() -> Self {
        Self {
            last_operation_kind: None,
            last_operation_status: None,
            last_store_opened_at_ms: None,
            last_prune_started_at_ms: None,
            last_prune_finished_at_ms: None,
            last_pruned_terminal_before_ms: None,
            last_prune_deleted_rows: None,
            last_purge_deleted_rows: None,
            last_vacuum_started_at_ms: None,
            last_vacuum_finished_at_ms: None,
            last_vacuum_at_ms: None,
            last_wal_checkpoint_at_ms: None,
            last_known_page_count: None,
            last_known_freelist_count: None,
            last_known_db_bytes: None,
            last_growth_sampled_at_ms: None,
            schema_retention_version: STORE_RETENTION_SCHEMA_VERSION,
            metadata_schema_version: STORE_METADATA_SCHEMA_VERSION,
        }
    }

    pub(crate) fn apply_storage_sample(&mut self, sample: StoreStorageSample) {
        self.last_known_page_count = Some(sample.page_count);
        self.last_known_freelist_count = Some(sample.freelist_count);
        self.last_known_db_bytes = Some(sample.db_bytes);
        self.last_growth_sampled_at_ms = Some(sample.sampled_at_ms);
    }
}

#[derive(Debug, Error)]
pub enum StoreMaintenanceStateError {
    #[error("store maintenance state storage error: {message}")]
    Storage { message: String },
    #[error("store maintenance state data integrity error: {message}")]
    DataIntegrity { message: String },
}

impl SqliteStore {
    pub fn touch_store_opened_metadata(
        &mut self,
        opened_at_ms: i64,
    ) -> Result<(), StoreMaintenanceStateError> {
        let mut state = self
            .load_store_maintenance_state()?
            .unwrap_or_else(StoreMaintenanceState::baseline);
        state.last_store_opened_at_ms = Some(opened_at_ms);
        state.metadata_schema_version = STORE_METADATA_SCHEMA_VERSION;
        state.apply_storage_sample(sample_store_storage(self.connection(), opened_at_ms)?);
        self.upsert_store_maintenance_state(&state)
    }

    pub fn load_store_maintenance_state(
        &self,
    ) -> Result<Option<StoreMaintenanceState>, StoreMaintenanceStateError> {
        match self.connection().query_row(
            r#"
            SELECT
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
            FROM store_maintenance_state
            WHERE singleton_key = ?1
            "#,
            [DEFAULT_STATE_KEY],
            |row| {
                let last_operation_kind = row
                    .get::<_, Option<String>>(0)?
                    .map(|value| parse_operation_kind(&value))
                    .transpose()
                    .map_err(data_integrity_to_sql_error)?;
                let last_operation_status = row
                    .get::<_, Option<String>>(1)?
                    .map(|value| parse_operation_status(&value))
                    .transpose()
                    .map_err(data_integrity_to_sql_error)?;
                Ok(StoreMaintenanceState {
                    last_operation_kind,
                    last_operation_status,
                    last_store_opened_at_ms: row.get(2)?,
                    last_prune_started_at_ms: row.get(3)?,
                    last_prune_finished_at_ms: row.get(4)?,
                    last_pruned_terminal_before_ms: row.get(5)?,
                    last_prune_deleted_rows: row.get(6)?,
                    last_purge_deleted_rows: row.get(7)?,
                    last_vacuum_started_at_ms: row.get(8)?,
                    last_vacuum_finished_at_ms: row.get(9)?,
                    last_vacuum_at_ms: row.get(10)?,
                    last_wal_checkpoint_at_ms: row.get(11)?,
                    last_known_page_count: row.get(12)?,
                    last_known_freelist_count: row.get(13)?,
                    last_known_db_bytes: row.get(14)?,
                    last_growth_sampled_at_ms: row.get(15)?,
                    schema_retention_version: row.get(16)?,
                    metadata_schema_version: row.get(17)?,
                })
            },
        ) {
            Ok(state) => Ok(Some(state)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(other) => Err(storage_error(other)),
        }
    }

    pub fn upsert_store_maintenance_state(
        &mut self,
        state: &StoreMaintenanceState,
    ) -> Result<(), StoreMaintenanceStateError> {
        self.connection()
            .execute(
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
                    DEFAULT_STATE_KEY,
                    state
                        .last_operation_kind
                        .as_ref()
                        .map(operation_kind_as_str),
                    state
                        .last_operation_status
                        .as_ref()
                        .map(operation_status_as_str),
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
}

pub(crate) fn sample_store_storage(
    conn: &rusqlite::Connection,
    sampled_at_ms: i64,
) -> Result<StoreStorageSample, StoreMaintenanceStateError> {
    let page_size = conn
        .pragma_query_value(None, "page_size", |row| row.get::<_, i64>(0))
        .map_err(storage_error)?;
    let page_count = conn
        .pragma_query_value(None, "page_count", |row| row.get::<_, i64>(0))
        .map_err(storage_error)?;
    let freelist_count = conn
        .pragma_query_value(None, "freelist_count", |row| row.get::<_, i64>(0))
        .map_err(storage_error)?;

    Ok(StoreStorageSample {
        page_count,
        freelist_count,
        db_bytes: page_count.saturating_mul(page_size),
        sampled_at_ms,
    })
}

fn operation_kind_as_str(kind: &MaintenanceOperationKind) -> &'static str {
    match kind {
        MaintenanceOperationKind::Prune => "prune",
        MaintenanceOperationKind::Purge => "purge",
        MaintenanceOperationKind::Vacuum => "vacuum",
    }
}

fn operation_status_as_str(status: &MaintenanceOperationStatus) -> &'static str {
    match status {
        MaintenanceOperationStatus::Started => "started",
        MaintenanceOperationStatus::Succeeded => "succeeded",
        MaintenanceOperationStatus::Failed => "failed",
    }
}

fn parse_operation_kind(
    value: &str,
) -> Result<MaintenanceOperationKind, StoreMaintenanceStateError> {
    match value {
        "prune" => Ok(MaintenanceOperationKind::Prune),
        "purge" => Ok(MaintenanceOperationKind::Purge),
        "vacuum" => Ok(MaintenanceOperationKind::Vacuum),
        other => Err(StoreMaintenanceStateError::DataIntegrity {
            message: format!("unknown maintenance operation kind: {other}"),
        }),
    }
}

fn parse_operation_status(
    value: &str,
) -> Result<MaintenanceOperationStatus, StoreMaintenanceStateError> {
    match value {
        "started" => Ok(MaintenanceOperationStatus::Started),
        "succeeded" => Ok(MaintenanceOperationStatus::Succeeded),
        "failed" => Ok(MaintenanceOperationStatus::Failed),
        other => Err(StoreMaintenanceStateError::DataIntegrity {
            message: format!("unknown maintenance operation status: {other}"),
        }),
    }
}

fn storage_error(error: impl ToString) -> StoreMaintenanceStateError {
    StoreMaintenanceStateError::Storage {
        message: error.to_string(),
    }
}

fn data_integrity_to_sql_error(error: StoreMaintenanceStateError) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        0,
        rusqlite::types::Type::Text,
        Box::<dyn std::error::Error + Send + Sync>::from(error.to_string()),
    )
}
