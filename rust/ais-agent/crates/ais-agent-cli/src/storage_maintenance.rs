use std::{
    fs,
    io::{Error, ErrorKind},
    time::{SystemTime, UNIX_EPOCH},
};

use ais_agent_store_sqlite::{
    SqliteStore, StorePruneRequest, StorePruneResult, StoreVacuumRequest,
    STORE_RETENTION_SCHEMA_VERSION,
};

use crate::config::types::{AisAgentSqliteRetentionConfig, AisAgentSqliteStorageConfig};

pub(crate) fn prepare_sqlite_path(sqlite: &AisAgentSqliteStorageConfig) -> Result<(), Error> {
    if !sqlite.path.exists() && !sqlite.create_if_missing {
        return Err(invalid_input(&format!(
            "ais-agent SQLite store does not exist and create_if_missing=false: {}",
            sqlite.path.display()
        )));
    }
    if let Some(parent) = sqlite.path.parent() {
        if !parent.as_os_str().is_empty() && sqlite.create_if_missing {
            fs::create_dir_all(parent)?;
        }
    }
    Ok(())
}

pub(crate) fn build_prune_request(
    retention: &AisAgentSqliteRetentionConfig,
    now_ms: i64,
) -> StorePruneRequest {
    StorePruneRequest {
        started_at_ms: now_ms,
        finished_at_ms: now_ms,
        terminal_before_ms: now_ms - days_to_ms(retention.checkpoint_full_window_days),
        wait_state_orphan_before_ms: now_ms - days_to_ms(retention.wait_state_orphan_ttl_days),
        vacuum_freelist_threshold_pages: retention.vacuum_freelist_threshold_pages,
        schema_retention_version: STORE_RETENTION_SCHEMA_VERSION,
    }
}

pub(crate) fn build_vacuum_request(
    retention: &AisAgentSqliteRetentionConfig,
    now_ms: i64,
) -> StoreVacuumRequest {
    StoreVacuumRequest {
        started_at_ms: now_ms,
        finished_at_ms: now_ms,
        freelist_threshold_pages: retention.vacuum_freelist_threshold_pages,
        force: true,
        schema_retention_version: STORE_RETENTION_SCHEMA_VERSION,
    }
}

pub(crate) fn maybe_auto_prune_sqlite(
    sqlite: &AisAgentSqliteStorageConfig,
    now_ms: i64,
) -> Result<Option<StorePruneResult>, Box<dyn std::error::Error>> {
    prepare_sqlite_path(sqlite)?;
    if !sqlite.path.exists() {
        return Ok(None);
    }

    let mut store = SqliteStore::open_path(&sqlite.path)?;
    let state = store.load_store_maintenance_state()?;
    if !should_run_auto_prune(
        state
            .as_ref()
            .and_then(|value| value.last_prune_finished_at_ms),
        sqlite.retention.auto_prune_cadence_minutes,
        now_ms,
    ) {
        return Ok(None);
    }

    Ok(Some(store.prune_retention(&build_prune_request(
        &sqlite.retention,
        now_ms,
    ))?))
}

pub(crate) fn should_run_auto_prune(
    last_prune_finished_at_ms: Option<i64>,
    cadence_minutes: u32,
    now_ms: i64,
) -> bool {
    match last_prune_finished_at_ms {
        Some(last_prune) => now_ms.saturating_sub(last_prune) >= minutes_to_ms(cadence_minutes),
        None => true,
    }
}

pub(crate) fn current_time_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time should be monotonic after unix epoch")
        .as_millis() as i64
}

fn days_to_ms(days: u16) -> i64 {
    i64::from(days) * 24 * 60 * 60 * 1_000
}

fn minutes_to_ms(minutes: u32) -> i64 {
    i64::from(minutes) * 60 * 1_000
}

fn invalid_input(message: &str) -> Error {
    Error::new(ErrorKind::InvalidInput, message.to_owned())
}
