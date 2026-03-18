use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::SqliteStore;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaintenanceOperationKind {
    Prune,
    Purge,
    Vacuum,
}

impl MaintenanceOperationKind {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Prune => "prune",
            Self::Purge => "purge",
            Self::Vacuum => "vacuum",
        }
    }

    fn parse(value: &str) -> Result<Self, MaintenanceJournalError> {
        match value {
            "prune" => Ok(Self::Prune),
            "purge" => Ok(Self::Purge),
            "vacuum" => Ok(Self::Vacuum),
            other => Err(MaintenanceJournalError::DataIntegrity {
                message: format!("unknown maintenance operation kind: {other}"),
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaintenanceOperationStatus {
    Started,
    Succeeded,
    Failed,
}

impl MaintenanceOperationStatus {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Started => "started",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
        }
    }

    fn parse(value: &str) -> Result<Self, MaintenanceJournalError> {
        match value {
            "started" => Ok(Self::Started),
            "succeeded" => Ok(Self::Succeeded),
            "failed" => Ok(Self::Failed),
            other => Err(MaintenanceJournalError::DataIntegrity {
                message: format!("unknown maintenance operation status: {other}"),
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaintenanceJournalAppend {
    pub operation_kind: MaintenanceOperationKind,
    pub started_at_ms: i64,
    pub finished_at_ms: Option<i64>,
    pub status: MaintenanceOperationStatus,
    pub summary: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaintenanceJournalEntry {
    pub journal_id: i64,
    pub operation_kind: MaintenanceOperationKind,
    pub started_at_ms: i64,
    pub finished_at_ms: Option<i64>,
    pub status: MaintenanceOperationStatus,
    pub summary: Value,
}

#[derive(Debug, Error)]
pub enum MaintenanceJournalError {
    #[error("maintenance journal storage error: {message}")]
    Storage { message: String },
    #[error("maintenance journal serialization error: {message}")]
    Serialization { message: String },
    #[error("maintenance journal data integrity error: {message}")]
    DataIntegrity { message: String },
}

impl SqliteStore {
    pub fn append_maintenance_journal(
        &mut self,
        entry: MaintenanceJournalAppend,
    ) -> Result<MaintenanceJournalEntry, MaintenanceJournalError> {
        let summary_json = serde_json::to_string(&entry.summary).map_err(serialization_error)?;
        self.connection()
            .execute(
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
                    entry.operation_kind.as_str(),
                    entry.started_at_ms,
                    entry.finished_at_ms,
                    entry.status.as_str(),
                    summary_json,
                ],
            )
            .map_err(storage_error)?;

        let journal_id = self.connection().last_insert_rowid();
        Ok(MaintenanceJournalEntry {
            journal_id,
            operation_kind: entry.operation_kind,
            started_at_ms: entry.started_at_ms,
            finished_at_ms: entry.finished_at_ms,
            status: entry.status,
            summary: entry.summary,
        })
    }

    pub fn list_maintenance_journal(
        &self,
        limit: usize,
    ) -> Result<Vec<MaintenanceJournalEntry>, MaintenanceJournalError> {
        let mut stmt = self
            .connection()
            .prepare(
                r#"
                SELECT
                    journal_id,
                    operation_kind,
                    started_at_ms,
                    finished_at_ms,
                    status,
                    summary_json
                FROM maintenance_journal
                ORDER BY started_at_ms DESC, journal_id DESC
                LIMIT ?1
                "#,
            )
            .map_err(storage_error)?;

        let rows = stmt
            .query_map([limit as i64], |row| {
                let operation_kind = MaintenanceOperationKind::parse(&row.get::<_, String>(1)?)
                    .map_err(data_integrity_to_sql_error)?;
                let status = MaintenanceOperationStatus::parse(&row.get::<_, String>(4)?)
                    .map_err(data_integrity_to_sql_error)?;
                let summary = serde_json::from_str::<Value>(&row.get::<_, String>(5)?)
                    .map_err(serialization_to_sql_error)?;
                Ok(MaintenanceJournalEntry {
                    journal_id: row.get(0)?,
                    operation_kind,
                    started_at_ms: row.get(2)?,
                    finished_at_ms: row.get(3)?,
                    status,
                    summary,
                })
            })
            .map_err(storage_error)?;

        rows.collect::<Result<Vec<_>, _>>().map_err(storage_error)
    }
}

fn storage_error(error: impl ToString) -> MaintenanceJournalError {
    MaintenanceJournalError::Storage {
        message: error.to_string(),
    }
}

fn serialization_error(error: impl ToString) -> MaintenanceJournalError {
    MaintenanceJournalError::Serialization {
        message: error.to_string(),
    }
}

fn serialization_to_sql_error(error: impl ToString) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        0,
        rusqlite::types::Type::Text,
        Box::<dyn std::error::Error + Send + Sync>::from(error.to_string()),
    )
}

fn data_integrity_to_sql_error(error: MaintenanceJournalError) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        0,
        rusqlite::types::Type::Text,
        Box::<dyn std::error::Error + Send + Sync>::from(error.to_string()),
    )
}
