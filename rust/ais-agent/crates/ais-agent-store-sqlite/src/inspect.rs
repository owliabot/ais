use std::{fs, path::Path};

use rusqlite::{types::ValueRef, Connection, OpenFlags, OptionalExtension, Row};
use serde::Serialize;
use serde_json::{json, Map, Value};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreInspectCommand {
    Overview {
        limit: usize,
        status: Option<String>,
        phase: Option<String>,
        active_boundary_kind: Option<String>,
        run_id_prefix: Option<String>,
    },
    Run {
        run_id: String,
    },
    Events {
        run_id: String,
        after_event_seq: Option<u64>,
        checkpoint_seq: Option<u64>,
        event_kind: Option<String>,
        limit: Option<usize>,
    },
    Audits {
        run_id: String,
        after_audit_seq: Option<u64>,
        checkpoint_seq: Option<u64>,
        audit_type: Option<String>,
        recovery_disposition: Option<String>,
        limit: Option<usize>,
    },
    Checkpoints {
        run_id: String,
        latest: bool,
        archive_kind: Option<String>,
        limit: Option<usize>,
    },
    Waits {
        run_id: Option<String>,
        wait_kind: Option<String>,
        limit: usize,
    },
    Claims {
        run_id: Option<String>,
        status: Option<String>,
        owner_kind: Option<String>,
        host_session_id: Option<String>,
        limit: usize,
    },
    Retention,
    Storage,
    Sql {
        query: String,
        limit: Option<usize>,
    },
}

#[derive(Debug, Error)]
pub enum StoreInspectError {
    #[error("SQLite store not found: {0}")]
    NotFound(String),
    #[error("sqlite query failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("only read-only SELECT/WITH/PRAGMA queries are allowed")]
    InvalidSqlQuery,
}

#[derive(Debug, Serialize)]
struct StoreOverviewOutput {
    sqlite_path: String,
    schema_version: i64,
    table_counts: Value,
    runs: Vec<Value>,
}

pub fn inspect_store(
    sqlite_path: &Path,
    command: StoreInspectCommand,
) -> Result<Value, StoreInspectError> {
    let conn = open_readonly(sqlite_path)?;
    match command {
        StoreInspectCommand::Overview {
            limit,
            status,
            phase,
            active_boundary_kind,
            run_id_prefix,
        } => query_overview(
            &conn,
            sqlite_path,
            limit,
            status.as_deref(),
            phase.as_deref(),
            active_boundary_kind.as_deref(),
            run_id_prefix.as_deref(),
        ),
        StoreInspectCommand::Run { run_id } => query_run(&conn, sqlite_path, &run_id),
        StoreInspectCommand::Events {
            run_id,
            after_event_seq,
            checkpoint_seq,
            event_kind,
            limit,
        } => query_events(
            &conn,
            sqlite_path,
            &run_id,
            after_event_seq,
            checkpoint_seq,
            event_kind.as_deref(),
            limit,
        ),
        StoreInspectCommand::Audits {
            run_id,
            after_audit_seq,
            checkpoint_seq,
            audit_type,
            recovery_disposition,
            limit,
        } => query_audits(
            &conn,
            sqlite_path,
            &run_id,
            after_audit_seq,
            checkpoint_seq,
            audit_type.as_deref(),
            recovery_disposition.as_deref(),
            limit,
        ),
        StoreInspectCommand::Checkpoints {
            run_id,
            latest,
            archive_kind,
            limit,
        } => query_checkpoints(
            &conn,
            sqlite_path,
            &run_id,
            latest,
            archive_kind.as_deref(),
            limit,
        ),
        StoreInspectCommand::Waits {
            run_id,
            wait_kind,
            limit,
        } => query_waits(
            &conn,
            sqlite_path,
            run_id.as_deref(),
            wait_kind.as_deref(),
            limit,
        ),
        StoreInspectCommand::Claims {
            run_id,
            status,
            owner_kind,
            host_session_id,
            limit,
        } => query_claims(
            &conn,
            sqlite_path,
            run_id.as_deref(),
            status.as_deref(),
            owner_kind.as_deref(),
            host_session_id.as_deref(),
            limit,
        ),
        StoreInspectCommand::Retention => query_retention(&conn, sqlite_path),
        StoreInspectCommand::Storage => query_storage(&conn, sqlite_path),
        StoreInspectCommand::Sql { query, limit } => query_sql(&conn, sqlite_path, &query, limit),
    }
}

fn open_readonly(sqlite_path: &Path) -> Result<Connection, StoreInspectError> {
    if !sqlite_path.exists() {
        return Err(StoreInspectError::NotFound(
            sqlite_path.display().to_string(),
        ));
    }

    Connection::open_with_flags(
        sqlite_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(StoreInspectError::from)
}

fn query_overview(
    conn: &Connection,
    sqlite_path: &Path,
    limit: usize,
    status: Option<&str>,
    phase: Option<&str>,
    active_boundary_kind: Option<&str>,
    run_id_prefix: Option<&str>,
) -> Result<Value, StoreInspectError> {
    let schema_version: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    let table_counts = json!({
        "maintenance_journal": count_rows(conn, "maintenance_journal")?,
        "store_maintenance_state": count_rows(conn, "store_maintenance_state")?,
        "runs": count_rows(conn, "runs")?,
        "run_inputs": count_rows(conn, "run_inputs")?,
        "run_events": count_rows(conn, "run_events")?,
        "run_audits": count_rows(conn, "run_audits")?,
        "run_checkpoints": count_rows(conn, "run_checkpoints")?,
        "run_wait_states": count_rows(conn, "run_wait_states")?,
        "run_claim_history": count_rows(conn, "run_claim_history")?,
    });

    let rows = query_overview_runs(
        conn,
        limit,
        status,
        phase,
        active_boundary_kind,
        run_id_prefix,
    )?;

    Ok(json!(StoreOverviewOutput {
        sqlite_path: sqlite_path.display().to_string(),
        schema_version,
        table_counts,
        runs: rows,
    }))
}

fn query_run(
    conn: &Connection,
    sqlite_path: &Path,
    run_id: &str,
) -> Result<Value, StoreInspectError> {
    let catalog = query_run_catalog(conn, run_id)?;
    let mission = query_run_mission(conn, run_id)?;
    let latest_checkpoint = query_latest_checkpoint(conn, run_id, None)?;
    let wait_state = query_wait_state(conn, run_id)?;
    let active_claim = query_active_claim(conn, run_id)?;
    let latest_claim = query_latest_claim(conn, run_id)?;

    Ok(json!({
        "sqlite_path": sqlite_path.display().to_string(),
        "run_id": run_id,
        "catalog": catalog.unwrap_or(Value::Null),
        "mission": mission,
        "latest_checkpoint": latest_checkpoint,
        "counts": {
            "checkpoints": count_for_run(conn, "run_checkpoints", run_id)?,
            "events": count_for_run(conn, "run_events", run_id)?,
            "audits": count_for_run(conn, "run_audits", run_id)?,
            "claims": count_for_run(conn, "run_claim_history", run_id)?,
        },
        "wait_state": wait_state,
        "active_claim": active_claim,
        "latest_claim": latest_claim,
    }))
}

fn query_events(
    conn: &Connection,
    sqlite_path: &Path,
    run_id: &str,
    after_event_seq: Option<u64>,
    checkpoint_seq: Option<u64>,
    event_kind: Option<&str>,
    limit: Option<usize>,
) -> Result<Value, StoreInspectError> {
    let (latest_event_seq, rows) = query_event_rows(
        conn,
        run_id,
        after_event_seq,
        checkpoint_seq,
        event_kind,
        limit,
    )?;

    Ok(json!({
        "sqlite_path": sqlite_path.display().to_string(),
        "run_id": run_id,
        "after_event_seq": after_event_seq,
        "checkpoint_seq": checkpoint_seq,
        "event_kind": event_kind,
        "latest_event_seq": latest_event_seq,
        "events": rows,
    }))
}

fn query_audits(
    conn: &Connection,
    sqlite_path: &Path,
    run_id: &str,
    after_audit_seq: Option<u64>,
    checkpoint_seq: Option<u64>,
    audit_type: Option<&str>,
    recovery_disposition: Option<&str>,
    limit: Option<usize>,
) -> Result<Value, StoreInspectError> {
    let (latest_audit_seq, rows) = query_audit_rows(
        conn,
        run_id,
        after_audit_seq,
        checkpoint_seq,
        audit_type,
        recovery_disposition,
        limit,
    )?;

    Ok(json!({
        "sqlite_path": sqlite_path.display().to_string(),
        "run_id": run_id,
        "after_audit_seq": after_audit_seq,
        "checkpoint_seq": checkpoint_seq,
        "audit_type": audit_type,
        "recovery_disposition": recovery_disposition,
        "latest_audit_seq": latest_audit_seq,
        "audits": rows,
    }))
}

fn query_checkpoints(
    conn: &Connection,
    sqlite_path: &Path,
    run_id: &str,
    latest: bool,
    archive_kind: Option<&str>,
    limit: Option<usize>,
) -> Result<Value, StoreInspectError> {
    let latest_checkpoint = query_latest_checkpoint(conn, run_id, archive_kind)?;
    if latest {
        return Ok(json!({
            "sqlite_path": sqlite_path.display().to_string(),
            "run_id": run_id,
            "archive_kind": archive_kind,
            "latest_checkpoint": latest_checkpoint,
        }));
    }

    let rows = query_checkpoint_rows(conn, run_id, archive_kind, limit)?;

    Ok(json!({
        "sqlite_path": sqlite_path.display().to_string(),
        "run_id": run_id,
        "archive_kind": archive_kind,
        "latest_checkpoint": latest_checkpoint,
        "checkpoints": rows,
    }))
}

fn query_waits(
    conn: &Connection,
    sqlite_path: &Path,
    run_id: Option<&str>,
    wait_kind: Option<&str>,
    limit: usize,
) -> Result<Value, StoreInspectError> {
    if let Some(run_id) = run_id {
        let wait_state = query_wait_state(conn, run_id)?;
        return Ok(json!({
            "sqlite_path": sqlite_path.display().to_string(),
            "run_id": run_id,
            "wait_state": wait_state,
        }));
    }

    let waits = query_wait_rows(conn, wait_kind, limit)?;
    Ok(json!({
        "sqlite_path": sqlite_path.display().to_string(),
        "run_id": Value::Null,
        "wait_kind": wait_kind,
        "limit": limit,
        "waits": waits,
    }))
}

fn query_claims(
    conn: &Connection,
    sqlite_path: &Path,
    run_id: Option<&str>,
    status: Option<&str>,
    owner_kind: Option<&str>,
    host_session_id: Option<&str>,
    limit: usize,
) -> Result<Value, StoreInspectError> {
    if let Some(run_id) = run_id {
        let claims = query_claim_rows(conn, run_id)?;
        return Ok(json!({
            "sqlite_path": sqlite_path.display().to_string(),
            "run_id": run_id,
            "claims": claims,
        }));
    }

    let claims = query_claim_rows_filtered(conn, status, owner_kind, host_session_id, limit)?;
    Ok(json!({
        "sqlite_path": sqlite_path.display().to_string(),
        "run_id": Value::Null,
        "status": status,
        "owner_kind": owner_kind,
        "host_session_id": host_session_id,
        "limit": limit,
        "claims": claims,
    }))
}

fn query_retention(conn: &Connection, sqlite_path: &Path) -> Result<Value, StoreInspectError> {
    let storage_sample = current_storage_sample(conn, sqlite_path);
    let recent_maintenance = query_recent_maintenance(conn, 5)?;
    let latest_maintenance = conn
        .query_row(
            r#"
            SELECT operation_kind, started_at_ms, finished_at_ms, status, summary_json
            FROM maintenance_journal
            ORDER BY started_at_ms DESC, journal_id DESC
            LIMIT 1
            "#,
            [],
            |row| {
                Ok(json!({
                    "operation_kind": row.get::<_, String>(0)?,
                    "started_at_ms": row.get::<_, i64>(1)?,
                    "finished_at_ms": row.get::<_, Option<i64>>(2)?,
                    "status": row.get::<_, String>(3)?,
                    "summary": parse_json_text(row.get::<_, String>(4)?),
                }))
            },
        )
        .optional()?
        .unwrap_or(Value::Null);
    let maintenance_state = conn
        .query_row(
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
            WHERE singleton_key = 'default'
            "#,
            [],
            |row| {
                Ok(json!({
                    "last_operation_kind": row.get::<_, Option<String>>(0)?,
                    "last_operation_status": row.get::<_, Option<String>>(1)?,
                    "last_store_opened_at_ms": row.get::<_, Option<i64>>(2)?,
                    "last_prune_started_at_ms": row.get::<_, Option<i64>>(3)?,
                    "last_prune_finished_at_ms": row.get::<_, Option<i64>>(4)?,
                    "last_pruned_terminal_before_ms": row.get::<_, Option<i64>>(5)?,
                    "last_prune_deleted_rows": row.get::<_, Option<i64>>(6)?,
                    "last_purge_deleted_rows": row.get::<_, Option<i64>>(7)?,
                    "last_vacuum_started_at_ms": row.get::<_, Option<i64>>(8)?,
                    "last_vacuum_finished_at_ms": row.get::<_, Option<i64>>(9)?,
                    "last_vacuum_at_ms": row.get::<_, Option<i64>>(10)?,
                    "last_wal_checkpoint_at_ms": row.get::<_, Option<i64>>(11)?,
                    "last_known_page_count": row.get::<_, Option<i64>>(12)?,
                    "last_known_freelist_count": row.get::<_, Option<i64>>(13)?,
                    "last_known_db_bytes": row.get::<_, Option<i64>>(14)?,
                    "last_growth_sampled_at_ms": row.get::<_, Option<i64>>(15)?,
                    "schema_retention_version": row.get::<_, i64>(16)?,
                    "metadata_schema_version": row.get::<_, i64>(17)?,
                }))
            },
        )
        .optional()?
        .unwrap_or(Value::Null);
    let last_sample = maintenance_state_storage_sample(&maintenance_state);

    Ok(json!({
        "sqlite_path": sqlite_path.display().to_string(),
        "run_retention_modes": {
            "active_full": count_runs_for_retention_mode(conn, "active_full")?,
            "terminal_tiered": count_runs_for_retention_mode(conn, "terminal_tiered")?,
            "unknown_or_unset": count_runs_with_unknown_retention_mode(conn)?,
        },
        "checkpoint_tiers": checkpoint_tier_counts(conn)?,
        "terminal_runs": {
            "total": count_terminal_runs(conn)?,
            "with_terminal_at_ms": count_terminal_runs_with_terminal_at(conn)?,
            "missing_terminal_at_ms": count_terminal_runs_missing_terminal_at(conn)?,
        },
        "active_wait_states": count_rows(conn, "run_wait_states")?,
        "maintenance_state": maintenance_state,
        "latest_maintenance": latest_maintenance,
        "growth_trend": {
            "current_sample": storage_sample,
            "last_recorded_sample": last_sample,
            "delta_since_last_recorded_sample": storage_delta_between(last_sample.as_ref(), &storage_sample),
            "recent_maintenance": recent_maintenance,
        },
    }))
}

fn query_storage(conn: &Connection, sqlite_path: &Path) -> Result<Value, StoreInspectError> {
    let page_size: i64 = conn.pragma_query_value(None, "page_size", |row| row.get(0))?;
    let page_count: i64 = conn.pragma_query_value(None, "page_count", |row| row.get(0))?;
    let freelist_count: i64 = conn.pragma_query_value(None, "freelist_count", |row| row.get(0))?;
    let file_bytes = fs::metadata(sqlite_path)
        .map(|meta| meta.len())
        .unwrap_or(0);
    let wal_path = sqlite_sidecar_path(sqlite_path, "-wal");
    let shm_path = sqlite_sidecar_path(sqlite_path, "-shm");
    let recent_maintenance = query_recent_maintenance(conn, 5)?;
    let maintenance_state = query_maintenance_state(conn)?;
    let current_sample = current_storage_sample(conn, sqlite_path);
    let last_sample = maintenance_state_storage_sample(&maintenance_state);

    Ok(json!({
        "sqlite_path": sqlite_path.display().to_string(),
        "page_size": page_size,
        "page_count": page_count,
        "freelist_count": freelist_count,
        "approx_used_bytes": (page_count - freelist_count).max(0) * page_size,
        "approx_free_bytes": freelist_count * page_size,
        "main_db_bytes": file_bytes,
        "wal_bytes": fs::metadata(&wal_path).map(|meta| meta.len()).unwrap_or(0),
        "shm_bytes": fs::metadata(&shm_path).map(|meta| meta.len()).unwrap_or(0),
        "growth_trend": {
            "current_sample": current_sample,
            "last_recorded_sample": last_sample,
            "delta_since_last_recorded_sample": storage_delta_between(last_sample.as_ref(), &current_sample),
            "recent_maintenance": recent_maintenance,
        },
        "table_rows": {
            "runs": count_rows(conn, "runs")?,
            "run_inputs": count_rows(conn, "run_inputs")?,
            "run_events": count_rows(conn, "run_events")?,
            "run_audits": count_rows(conn, "run_audits")?,
            "run_checkpoints": count_rows(conn, "run_checkpoints")?,
            "run_wait_states": count_rows(conn, "run_wait_states")?,
            "run_claim_history": count_rows(conn, "run_claim_history")?,
            "maintenance_journal": count_rows(conn, "maintenance_journal")?,
            "store_maintenance_state": count_rows(conn, "store_maintenance_state")?,
        }
    }))
}

fn query_recent_maintenance(
    conn: &Connection,
    limit: usize,
) -> Result<Vec<Value>, StoreInspectError> {
    let mut stmt = conn.prepare(
        r#"
        SELECT operation_kind, started_at_ms, finished_at_ms, status, summary_json
        FROM maintenance_journal
        ORDER BY started_at_ms DESC, journal_id DESC
        LIMIT ?1
        "#,
    )?;
    let rows = stmt.query_map([limit as i64], |row| {
        let summary = parse_json_text(row.get::<_, String>(4)?);
        Ok(json!({
            "operation_kind": row.get::<_, String>(0)?,
            "started_at_ms": row.get::<_, i64>(1)?,
            "finished_at_ms": row.get::<_, Option<i64>>(2)?,
            "status": row.get::<_, String>(3)?,
            "summary": summary,
            "storage_before": summary.get("storage_before").cloned().unwrap_or(Value::Null),
            "storage_after": summary.get("storage_after").cloned().unwrap_or(Value::Null),
            "storage_delta": summary.get("storage_delta").cloned().unwrap_or(Value::Null),
        }))
    })?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(StoreInspectError::from)
}

fn query_maintenance_state(conn: &Connection) -> Result<Value, StoreInspectError> {
    conn.query_row(
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
        WHERE singleton_key = 'default'
        "#,
        [],
        |row| {
            Ok(json!({
                "last_operation_kind": row.get::<_, Option<String>>(0)?,
                "last_operation_status": row.get::<_, Option<String>>(1)?,
                "last_store_opened_at_ms": row.get::<_, Option<i64>>(2)?,
                "last_prune_started_at_ms": row.get::<_, Option<i64>>(3)?,
                "last_prune_finished_at_ms": row.get::<_, Option<i64>>(4)?,
                "last_pruned_terminal_before_ms": row.get::<_, Option<i64>>(5)?,
                "last_prune_deleted_rows": row.get::<_, Option<i64>>(6)?,
                "last_purge_deleted_rows": row.get::<_, Option<i64>>(7)?,
                "last_vacuum_started_at_ms": row.get::<_, Option<i64>>(8)?,
                "last_vacuum_finished_at_ms": row.get::<_, Option<i64>>(9)?,
                "last_vacuum_at_ms": row.get::<_, Option<i64>>(10)?,
                "last_wal_checkpoint_at_ms": row.get::<_, Option<i64>>(11)?,
                "last_known_page_count": row.get::<_, Option<i64>>(12)?,
                "last_known_freelist_count": row.get::<_, Option<i64>>(13)?,
                "last_known_db_bytes": row.get::<_, Option<i64>>(14)?,
                "last_growth_sampled_at_ms": row.get::<_, Option<i64>>(15)?,
                "schema_retention_version": row.get::<_, i64>(16)?,
                "metadata_schema_version": row.get::<_, i64>(17)?,
            }))
        },
    )
    .optional()?
    .map_or(Ok(Value::Null), Ok)
}

fn current_storage_sample(conn: &Connection, sqlite_path: &Path) -> Value {
    let page_size = conn
        .pragma_query_value(None, "page_size", |row| row.get::<_, i64>(0))
        .unwrap_or(0);
    let page_count = conn
        .pragma_query_value(None, "page_count", |row| row.get::<_, i64>(0))
        .unwrap_or(0);
    let freelist_count = conn
        .pragma_query_value(None, "freelist_count", |row| row.get::<_, i64>(0))
        .unwrap_or(0);
    let db_bytes = page_count.saturating_mul(page_size);
    let main_db_bytes = fs::metadata(sqlite_path)
        .map(|meta| meta.len() as i64)
        .unwrap_or(db_bytes);

    json!({
        "page_count": page_count,
        "freelist_count": freelist_count,
        "db_bytes": db_bytes,
        "main_db_bytes": main_db_bytes,
        "used_bytes": page_count.saturating_sub(freelist_count).max(0) * page_size,
        "free_bytes": freelist_count.max(0) * page_size,
    })
}

fn maintenance_state_storage_sample(state: &Value) -> Option<Value> {
    let page_count = state.get("last_known_page_count")?.as_i64()?;
    let freelist_count = state.get("last_known_freelist_count")?.as_i64()?;
    let db_bytes = state.get("last_known_db_bytes")?.as_i64()?;
    Some(json!({
        "page_count": page_count,
        "freelist_count": freelist_count,
        "db_bytes": db_bytes,
        "sampled_at_ms": state.get("last_growth_sampled_at_ms").cloned().unwrap_or(Value::Null),
    }))
}

fn storage_delta_between(last_sample: Option<&Value>, current_sample: &Value) -> Value {
    let Some(last_sample) = last_sample else {
        return Value::Null;
    };
    json!({
        "page_count": current_sample["page_count"].as_i64().unwrap_or(0)
            - last_sample["page_count"].as_i64().unwrap_or(0),
        "freelist_count": current_sample["freelist_count"].as_i64().unwrap_or(0)
            - last_sample["freelist_count"].as_i64().unwrap_or(0),
        "db_bytes": current_sample["db_bytes"].as_i64().unwrap_or(0)
            - last_sample["db_bytes"].as_i64().unwrap_or(0),
    })
}

fn sqlite_sidecar_path(sqlite_path: &Path, suffix: &str) -> std::path::PathBuf {
    let file_name = sqlite_path
        .file_name()
        .and_then(|value| value.to_str())
        .map(|value| format!("{value}{suffix}"))
        .unwrap_or_else(|| format!("sqlite{suffix}"));
    sqlite_path.with_file_name(file_name)
}

fn query_sql(
    conn: &Connection,
    sqlite_path: &Path,
    query: &str,
    limit: Option<usize>,
) -> Result<Value, StoreInspectError> {
    if !is_read_only_query(query) {
        return Err(StoreInspectError::InvalidSqlQuery);
    }

    let mut stmt = conn.prepare(query)?;
    let column_names = stmt
        .column_names()
        .iter()
        .map(|name| (*name).to_owned())
        .collect::<Vec<_>>();
    let mut rows = stmt.query([])?;
    let max_rows = limit.unwrap_or(200);
    let mut out = Vec::new();
    let mut truncated = false;

    while let Some(row) = rows.next()? {
        if out.len() == max_rows {
            truncated = true;
            break;
        }
        out.push(sql_row_to_value(row, &column_names)?);
    }

    Ok(json!({
        "sqlite_path": sqlite_path.display().to_string(),
        "query": query,
        "row_count": out.len(),
        "truncated": truncated,
        "rows": out,
    }))
}

fn count_rows(conn: &Connection, table: &str) -> Result<i64, StoreInspectError> {
    let sql = format!("SELECT COUNT(*) FROM {table}");
    Ok(conn.query_row(&sql, [], |row| row.get(0))?)
}

fn count_for_run(conn: &Connection, table: &str, run_id: &str) -> Result<i64, StoreInspectError> {
    let sql = format!("SELECT COUNT(*) FROM {table} WHERE run_id = ?1");
    Ok(conn.query_row(&sql, [run_id], |row| row.get(0))?)
}

fn count_runs_for_retention_mode(
    conn: &Connection,
    retention_mode: &str,
) -> Result<i64, StoreInspectError> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM runs WHERE retention_mode = ?1",
        [retention_mode],
        |row| row.get(0),
    )?)
}

fn count_runs_with_unknown_retention_mode(conn: &Connection) -> Result<i64, StoreInspectError> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM runs WHERE retention_mode IS NULL OR retention_mode NOT IN ('active_full', 'terminal_tiered')",
        [],
        |row| row.get(0),
    )?)
}

fn count_terminal_runs(conn: &Connection) -> Result<i64, StoreInspectError> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM runs WHERE status IN ('completed', 'failed', 'cancelled')",
        [],
        |row| row.get(0),
    )?)
}

fn count_terminal_runs_with_terminal_at(conn: &Connection) -> Result<i64, StoreInspectError> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM runs WHERE status IN ('completed', 'failed', 'cancelled') AND terminal_at_ms IS NOT NULL",
        [],
        |row| row.get(0),
    )?)
}

fn count_terminal_runs_missing_terminal_at(conn: &Connection) -> Result<i64, StoreInspectError> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM runs WHERE status IN ('completed', 'failed', 'cancelled') AND terminal_at_ms IS NULL",
        [],
        |row| row.get(0),
    )?)
}

fn checkpoint_tier_counts(conn: &Connection) -> Result<Value, StoreInspectError> {
    let mut stmt = conn.prepare(
        r#"
        SELECT retention_tier, COUNT(*)
        FROM run_checkpoints
        GROUP BY retention_tier
        ORDER BY retention_tier ASC
        "#,
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })?;
    let mut object = Map::new();
    for row in rows {
        let (tier, count) = row?;
        object.insert(tier, json!(count));
    }
    Ok(Value::Object(object))
}

fn query_overview_runs(
    conn: &Connection,
    limit: usize,
    status: Option<&str>,
    phase: Option<&str>,
    active_boundary_kind: Option<&str>,
    run_id_prefix: Option<&str>,
) -> Result<Vec<Value>, StoreInspectError> {
    let mut stmt = conn.prepare(
        r#"
        SELECT
            run_id,
            mission_id,
            status,
            phase,
            active_boundary_kind,
            latest_checkpoint_seq,
            latest_event_seq,
            COALESCE(latest_checkpoint_seq, 0) AS latest_revision,
            created_at_ms,
            updated_at_ms,
            terminal_at_ms
        FROM runs
        WHERE (?1 IS NULL OR status = ?1)
          AND (?2 IS NULL OR phase = ?2)
          AND (?3 IS NULL OR active_boundary_kind = ?3)
          AND (?4 IS NULL OR run_id LIKE (?4 || '%'))
        ORDER BY COALESCE(updated_at_ms, created_at_ms, 0) DESC, run_id DESC
        LIMIT ?5
        "#,
    )?;
    let rows = stmt
        .query_map(
            (
                status,
                phase,
                active_boundary_kind,
                run_id_prefix,
                limit as i64,
            ),
            run_head_row_to_value,
        )?
        .collect::<Result<Vec<_>, _>>()
        .map_err(StoreInspectError::from)?;
    Ok(rows)
}

fn query_run_catalog(conn: &Connection, run_id: &str) -> Result<Option<Value>, StoreInspectError> {
    conn.query_row(
        r#"
        SELECT
            run_id,
            mission_id,
            status,
            phase,
            active_boundary_kind,
            latest_checkpoint_seq,
            latest_event_seq,
            COALESCE(latest_checkpoint_seq, 0) AS latest_revision,
            created_at_ms,
            updated_at_ms,
            terminal_at_ms
        FROM runs
        WHERE run_id = ?1
        "#,
        [run_id],
        run_head_row_to_value,
    )
    .optional()
    .map_err(StoreInspectError::from)
}

fn query_run_mission(conn: &Connection, run_id: &str) -> Result<Value, StoreInspectError> {
    Ok(conn
        .query_row(
            "SELECT mission_json FROM run_inputs WHERE run_id = ?1",
            [run_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .map(parse_json_text)
        .unwrap_or(Value::Null))
}

fn query_latest_checkpoint(
    conn: &Connection,
    run_id: &str,
    archive_kind: Option<&str>,
) -> Result<Value, StoreInspectError> {
    Ok(conn
        .query_row(
            r#"
            SELECT checkpoint_seq, plan_epoch, checkpoint_kind, snapshot_json
            FROM run_checkpoints
            WHERE run_id = ?1
              AND (?2 IS NULL OR checkpoint_kind = ?2)
            ORDER BY checkpoint_seq DESC, plan_epoch DESC, checkpoint_id DESC
            LIMIT 1
            "#,
            (run_id, archive_kind),
            checkpoint_row_to_value,
        )
        .optional()?
        .unwrap_or(Value::Null))
}

fn query_wait_state(conn: &Connection, run_id: &str) -> Result<Value, StoreInspectError> {
    Ok(conn
        .query_row(
            "SELECT wait_kind, request_id, entered_at_ms, expires_at_ms, state_json FROM run_wait_states WHERE run_id = ?1",
            [run_id],
            |row| {
                Ok(json!({
                    "wait_kind": row.get::<_, String>(0)?,
                    "request_id": row.get::<_, String>(1)?,
                    "entered_at_ms": row.get::<_, i64>(2)?,
                    "expires_at_ms": row.get::<_, Option<i64>>(3)?,
                    "state": parse_json_text(row.get::<_, String>(4)?),
                }))
            },
        )
        .optional()?
        .unwrap_or(Value::Null))
}

fn query_active_claim(conn: &Connection, run_id: &str) -> Result<Value, StoreInspectError> {
    Ok(conn
        .query_row(
            r#"
            SELECT
                claim_id,
                host_session_id,
                owner_kind,
                owner_instance_id,
                lease_started_at_ms,
                lease_expires_at_ms,
                last_renewed_at_ms,
                claim_epoch,
                mode,
                status
            FROM run_claim_history
            WHERE run_id = ?1 AND status = 'active'
            ORDER BY claim_epoch DESC
            LIMIT 1
            "#,
            [run_id],
            stored_claim_row_to_value,
        )
        .optional()?
        .unwrap_or(Value::Null))
}

fn query_latest_claim(conn: &Connection, run_id: &str) -> Result<Value, StoreInspectError> {
    Ok(conn
        .query_row(
            r#"
            SELECT
                claim_id,
                host_session_id,
                owner_kind,
                owner_instance_id,
                lease_started_at_ms,
                lease_expires_at_ms,
                last_renewed_at_ms,
                claim_epoch,
                mode,
                status
            FROM run_claim_history
            WHERE run_id = ?1
            ORDER BY lease_started_at_ms DESC, claim_epoch DESC, claim_id DESC
            LIMIT 1
            "#,
            [run_id],
            stored_claim_row_to_value,
        )
        .optional()?
        .unwrap_or(Value::Null))
}

fn query_event_rows(
    conn: &Connection,
    run_id: &str,
    after_event_seq: Option<u64>,
    checkpoint_seq: Option<u64>,
    event_kind: Option<&str>,
    limit: Option<usize>,
) -> Result<(Option<i64>, Vec<Value>), StoreInspectError> {
    let latest_event_seq: Option<i64> = conn.query_row(
        "SELECT MAX(event_seq) FROM run_events WHERE run_id = ?1",
        [run_id],
        |row| row.get(0),
    )?;
    let sql = if limit.is_some() {
        r#"
        SELECT event_seq, checkpoint_seq, event_kind, payload_json
        FROM run_events
        WHERE run_id = ?1
          AND (?2 IS NULL OR event_seq > ?2)
          AND (?3 IS NULL OR checkpoint_seq = ?3)
          AND (?4 IS NULL OR event_kind = ?4)
        ORDER BY event_seq ASC
        LIMIT ?5
        "#
    } else {
        r#"
        SELECT event_seq, checkpoint_seq, event_kind, payload_json
        FROM run_events
        WHERE run_id = ?1
          AND (?2 IS NULL OR event_seq > ?2)
          AND (?3 IS NULL OR checkpoint_seq = ?3)
          AND (?4 IS NULL OR event_kind = ?4)
        ORDER BY event_seq ASC
        "#
    };
    let mut stmt = conn.prepare(sql)?;
    let rows = if let Some(limit) = limit {
        stmt.query_map(
            (
                run_id,
                after_event_seq.map(|v| v as i64),
                checkpoint_seq.map(|v| v as i64),
                event_kind,
                limit as i64,
            ),
            event_row_to_value,
        )?
        .collect::<Result<Vec<_>, _>>()?
    } else {
        stmt.query_map(
            (
                run_id,
                after_event_seq.map(|v| v as i64),
                checkpoint_seq.map(|v| v as i64),
                event_kind,
            ),
            event_row_to_value,
        )?
        .collect::<Result<Vec<_>, _>>()?
    };
    Ok((latest_event_seq, rows))
}

fn query_audit_rows(
    conn: &Connection,
    run_id: &str,
    after_audit_seq: Option<u64>,
    checkpoint_seq: Option<u64>,
    audit_type: Option<&str>,
    recovery_disposition: Option<&str>,
    limit: Option<usize>,
) -> Result<(Option<i64>, Vec<Value>), StoreInspectError> {
    let latest_audit_seq: Option<i64> = conn.query_row(
        "SELECT MAX(audit_seq) FROM run_audits WHERE run_id = ?1",
        [run_id],
        |row| row.get(0),
    )?;
    let sql = if limit.is_some() {
        r#"
        SELECT audit_seq, checkpoint_seq, audit_kind, decision_class, payload_json
        FROM run_audits
        WHERE run_id = ?1
          AND (?2 IS NULL OR audit_seq > ?2)
          AND (?3 IS NULL OR checkpoint_seq = ?3)
          AND (?4 IS NULL OR audit_kind = ?4)
          AND (?5 IS NULL OR json_extract(payload_json, '$.audit.recovery_disposition') = ?5)
        ORDER BY audit_seq ASC
        LIMIT ?6
        "#
    } else {
        r#"
        SELECT audit_seq, checkpoint_seq, audit_kind, decision_class, payload_json
        FROM run_audits
        WHERE run_id = ?1
          AND (?2 IS NULL OR audit_seq > ?2)
          AND (?3 IS NULL OR checkpoint_seq = ?3)
          AND (?4 IS NULL OR audit_kind = ?4)
          AND (?5 IS NULL OR json_extract(payload_json, '$.audit.recovery_disposition') = ?5)
        ORDER BY audit_seq ASC
        "#
    };
    let mut stmt = conn.prepare(sql)?;
    let rows = if let Some(limit) = limit {
        stmt.query_map(
            (
                run_id,
                after_audit_seq.map(|v| v as i64),
                checkpoint_seq.map(|v| v as i64),
                audit_type,
                recovery_disposition,
                limit as i64,
            ),
            audit_row_to_value,
        )?
        .collect::<Result<Vec<_>, _>>()?
    } else {
        stmt.query_map(
            (
                run_id,
                after_audit_seq.map(|v| v as i64),
                checkpoint_seq.map(|v| v as i64),
                audit_type,
                recovery_disposition,
            ),
            audit_row_to_value,
        )?
        .collect::<Result<Vec<_>, _>>()?
    };
    Ok((latest_audit_seq, rows))
}

fn query_checkpoint_rows(
    conn: &Connection,
    run_id: &str,
    archive_kind: Option<&str>,
    limit: Option<usize>,
) -> Result<Vec<Value>, StoreInspectError> {
    let sql = if limit.is_some() {
        r#"
        SELECT checkpoint_seq, plan_epoch, checkpoint_kind, snapshot_json
        FROM run_checkpoints
        WHERE run_id = ?1
          AND (?2 IS NULL OR checkpoint_kind = ?2)
        ORDER BY checkpoint_seq DESC, plan_epoch DESC, checkpoint_id DESC
        LIMIT ?3
        "#
    } else {
        r#"
        SELECT checkpoint_seq, plan_epoch, checkpoint_kind, snapshot_json
        FROM run_checkpoints
        WHERE run_id = ?1
          AND (?2 IS NULL OR checkpoint_kind = ?2)
        ORDER BY checkpoint_seq DESC, plan_epoch DESC, checkpoint_id DESC
        "#
    };
    let mut stmt = conn.prepare(sql)?;
    if let Some(limit) = limit {
        stmt.query_map((run_id, archive_kind, limit as i64), checkpoint_row_to_value)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(StoreInspectError::from)
    } else {
        stmt.query_map((run_id, archive_kind), checkpoint_row_to_value)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(StoreInspectError::from)
    }
}

fn query_claim_rows(conn: &Connection, run_id: &str) -> Result<Vec<Value>, StoreInspectError> {
    let mut stmt = conn.prepare(
        r#"
        SELECT
            claim_id,
            host_session_id,
            owner_kind,
            owner_instance_id,
            lease_started_at_ms,
            lease_expires_at_ms,
            last_renewed_at_ms,
            claim_epoch,
            mode,
            status
        FROM run_claim_history
        WHERE run_id = ?1
        ORDER BY lease_started_at_ms DESC, claim_epoch DESC, claim_id DESC
        "#,
    )?;
    let claims = stmt
        .query_map([run_id], stored_claim_row_to_value)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(StoreInspectError::from)?;
    Ok(claims)
}

fn query_wait_rows(
    conn: &Connection,
    wait_kind: Option<&str>,
    limit: usize,
) -> Result<Vec<Value>, StoreInspectError> {
    let mut stmt = conn.prepare(
        r#"
        SELECT run_id, wait_kind, request_id, entered_at_ms, expires_at_ms, state_json
        FROM run_wait_states
        WHERE (?1 IS NULL OR wait_kind = ?1)
        ORDER BY entered_at_ms DESC, run_id DESC
        LIMIT ?2
        "#,
    )?;
    let rows = stmt
        .query_map((wait_kind, limit as i64), wait_row_to_value)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(StoreInspectError::from)?;
    Ok(rows)
}

fn query_claim_rows_filtered(
    conn: &Connection,
    status: Option<&str>,
    owner_kind: Option<&str>,
    host_session_id: Option<&str>,
    limit: usize,
) -> Result<Vec<Value>, StoreInspectError> {
    let mut stmt = conn.prepare(
        r#"
        SELECT
            claim_id,
            run_id,
            host_session_id,
            owner_kind,
            owner_instance_id,
            lease_started_at_ms,
            lease_expires_at_ms,
            last_renewed_at_ms,
            claim_epoch,
            mode,
            status
        FROM run_claim_history
        WHERE (?1 IS NULL OR status = ?1)
          AND (?2 IS NULL OR owner_kind = ?2)
          AND (?3 IS NULL OR host_session_id = ?3)
        ORDER BY lease_started_at_ms DESC, claim_epoch DESC, claim_id DESC
        LIMIT ?4
        "#,
    )?;
    let claims = stmt
        .query_map(
            (status, owner_kind, host_session_id, limit as i64),
            filtered_claim_row_to_value,
        )?
        .collect::<Result<Vec<_>, _>>()
        .map_err(StoreInspectError::from)?;
    Ok(claims)
}

fn run_head_row_to_value(row: &Row<'_>) -> rusqlite::Result<Value> {
    Ok(json!({
        "run_id": row.get::<_, String>(0)?,
        "mission_id": row.get::<_, String>(1)?,
        "status": row.get::<_, String>(2)?,
        "phase": row.get::<_, Option<String>>(3)?,
        "active_boundary_kind": row.get::<_, Option<String>>(4)?,
        "latest_checkpoint_seq": row.get::<_, Option<i64>>(5)?,
        "latest_event_seq": row.get::<_, Option<i64>>(6)?,
        "latest_revision": row.get::<_, i64>(7)?,
        "created_at_ms": row.get::<_, Option<i64>>(8)?,
        "updated_at_ms": row.get::<_, Option<i64>>(9)?,
        "terminal_at_ms": row.get::<_, Option<i64>>(10)?,
    }))
}

fn stored_claim_row_to_value(row: &Row<'_>) -> rusqlite::Result<Value> {
    Ok(json!({
        "claim_id": row.get::<_, String>(0)?,
        "host_session_id": row.get::<_, String>(1)?,
        "owner_kind": row.get::<_, String>(2)?,
        "owner_instance_id": row.get::<_, String>(3)?,
        "lease_started_at_ms": row.get::<_, i64>(4)?,
        "lease_expires_at_ms": row.get::<_, Option<i64>>(5)?,
        "last_renewed_at_ms": row.get::<_, Option<i64>>(6)?,
        "claim_epoch": row.get::<_, i64>(7)?,
        "mode": row.get::<_, String>(8)?,
        "status": row.get::<_, String>(9)?,
    }))
}

fn filtered_claim_row_to_value(row: &Row<'_>) -> rusqlite::Result<Value> {
    Ok(json!({
        "claim_id": row.get::<_, String>(0)?,
        "run_id": row.get::<_, String>(1)?,
        "host_session_id": row.get::<_, String>(2)?,
        "owner_kind": row.get::<_, String>(3)?,
        "owner_instance_id": row.get::<_, String>(4)?,
        "lease_started_at_ms": row.get::<_, i64>(5)?,
        "lease_expires_at_ms": row.get::<_, Option<i64>>(6)?,
        "last_renewed_at_ms": row.get::<_, Option<i64>>(7)?,
        "claim_epoch": row.get::<_, i64>(8)?,
        "mode": row.get::<_, String>(9)?,
        "status": row.get::<_, String>(10)?,
    }))
}

fn wait_row_to_value(row: &Row<'_>) -> rusqlite::Result<Value> {
    Ok(json!({
        "run_id": row.get::<_, String>(0)?,
        "wait_kind": row.get::<_, String>(1)?,
        "request_id": row.get::<_, String>(2)?,
        "entered_at_ms": row.get::<_, i64>(3)?,
        "expires_at_ms": row.get::<_, Option<i64>>(4)?,
        "state": parse_json_text(row.get::<_, String>(5)?),
    }))
}

fn checkpoint_row_to_value(row: &Row<'_>) -> rusqlite::Result<Value> {
    Ok(json!({
        "checkpoint_seq": row.get::<_, i64>(0)?,
        "plan_epoch": row.get::<_, i64>(1)?,
        "archive_kind": row.get::<_, String>(2)?,
        "snapshot": parse_json_text(row.get::<_, String>(3)?),
    }))
}

fn event_row_to_value(row: &Row<'_>) -> rusqlite::Result<Value> {
    let event = parse_json_text(row.get::<_, String>(3)?);
    let plan_epoch = event.get("plan_epoch").cloned().unwrap_or(Value::Null);
    Ok(json!({
        "event_seq": row.get::<_, i64>(0)?,
        "checkpoint_seq": row.get::<_, Option<i64>>(1)?,
        "event_kind": row.get::<_, String>(2)?,
        "plan_epoch": plan_epoch,
        "event": event,
    }))
}

fn audit_row_to_value(row: &Row<'_>) -> rusqlite::Result<Value> {
    let audit = parse_json_text(row.get::<_, String>(4)?);
    let plan_epoch = audit.get("plan_epoch").cloned().unwrap_or(Value::Null);
    let audit_id = audit.get("audit_id").cloned().unwrap_or(Value::Null);
    let recovery_disposition = audit
        .get("audit")
        .and_then(|entry| entry.get("recovery_disposition"))
        .cloned()
        .unwrap_or(Value::Null);
    Ok(json!({
        "audit_seq": row.get::<_, i64>(0)?,
        "checkpoint_seq": row.get::<_, Option<i64>>(1)?,
        "audit_type": row.get::<_, String>(2)?,
        "decision_class": row.get::<_, Option<String>>(3)?,
        "plan_epoch": plan_epoch,
        "audit_id": audit_id,
        "recovery_disposition": recovery_disposition,
        "audit": audit,
    }))
}

fn sql_row_to_value(row: &Row<'_>, column_names: &[String]) -> rusqlite::Result<Value> {
    let mut object = Map::new();
    for (index, name) in column_names.iter().enumerate() {
        let value = match row.get_ref(index)? {
            ValueRef::Null => Value::Null,
            ValueRef::Integer(value) => json!(value),
            ValueRef::Real(value) => json!(value),
            ValueRef::Text(value) => Value::String(String::from_utf8_lossy(value).into_owned()),
            ValueRef::Blob(value) => Value::String(format!("<{} bytes blob>", value.len())),
        };
        object.insert(name.clone(), value);
    }
    Ok(Value::Object(object))
}

fn parse_json_text(text: String) -> Value {
    serde_json::from_str(&text).unwrap_or(Value::String(text))
}

fn is_read_only_query(query: &str) -> bool {
    let lower = query.trim_start().to_ascii_lowercase();
    lower.starts_with("select ") || lower.starts_with("with ") || lower.starts_with("pragma ")
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use crate::{inspect_store, SqliteStore, StoreInspectCommand};

    fn temp_sqlite_path(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        std::env::temp_dir().join(format!("ais-agent-store-sqlite-{name}-{unique}.sqlite"))
    }

    fn seed_store(path: &PathBuf) {
        let store = SqliteStore::open_path(path).expect("open sqlite");
        let conn = store.connection();

        conn.execute(
            r#"
            INSERT INTO runs (
                run_id, mission_id, status, phase, active_boundary_kind, active_wait_kind,
                latest_checkpoint_seq, latest_event_seq, latest_audit_seq, latest_claim_epoch,
                retention_mode, created_at_ms, updated_at_ms, terminal_at_ms
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
            "#,
            rusqlite::params![
                "run-1",
                "mission-1",
                "completed",
                "terminal",
                "confirmation",
                "confirmation",
                3i64,
                2i64,
                1i64,
                1i64,
                "terminal_tiered",
                1000i64,
                2000i64,
                2100i64
            ],
        )
        .expect("insert run");
        conn.execute(
            "INSERT INTO run_inputs (run_id, mission_json, launch_input_json, created_at_ms) VALUES (?1, ?2, NULL, ?3)",
            rusqlite::params!["run-1", r#"{"mission_id":"mission-1","goal":"transfer"}"#, 1000i64],
        )
        .expect("insert run input");
        conn.execute(
            r#"
            INSERT INTO run_checkpoints (
                run_id, checkpoint_seq, plan_epoch, checkpoint_kind, retention_tier,
                created_at_ms, is_terminal, is_side_effect_boundary, is_recovery_boundary,
                is_first_wait_checkpoint, snapshot_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
            "#,
            rusqlite::params![
                "run-1",
                3i64,
                0i64,
                "boundary",
                "terminal_final",
                2000i64,
                1i64,
                0i64,
                0i64,
                0i64,
                r#"{"run_id":"run-1","checkpoint_seq":3,"plan_epoch":0,"status":"completed"}"#
            ],
        )
        .expect("insert checkpoint");
        conn.execute(
            r#"
            INSERT INTO run_events (
                run_id, event_seq, event_kind, phase, boundary_kind, emitted_at_ms,
                checkpoint_seq, revision, payload_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
            rusqlite::params![
                "run-1",
                1i64,
                "run.started",
                "executing",
                "pause",
                1000i64,
                1i64,
                1i64,
                r#"{"run_id":"run-1","event_seq":1,"checkpoint_seq":1,"plan_epoch":0,"event":{"type":"started"}}"#
            ],
        )
        .expect("insert event");
        conn.execute(
            r#"
            INSERT INTO run_audits (
                run_id, audit_seq, audit_kind, decision_class, emitted_at_ms,
                checkpoint_seq, revision, payload_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
            rusqlite::params![
                "run-1",
                1i64,
                "governor_decision",
                "allow",
                1500i64,
                2i64,
                2i64,
                r#"{"audit_id":"audit-1","run_id":"run-1","audit_seq":1,"checkpoint_seq":2,"plan_epoch":0,"audit":{"type":"governor_decision"}}"#
            ],
        )
        .expect("insert audit");
        conn.execute(
            r#"
            INSERT INTO run_wait_states (run_id, wait_kind, request_id, entered_at_ms, expires_at_ms, state_json)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            "#,
            rusqlite::params![
                "run-1",
                "signer",
                "signer-1",
                1100i64,
                1600i64,
                r#"{"run_id":"run-1","request_id":"signer-1","status":"awaiting"}"#
            ],
        )
        .expect("insert wait state");
        conn.execute(
            r#"
            INSERT INTO run_claim_history (
                claim_id, run_id, host_session_id, owner_kind, owner_instance_id,
                lease_started_at_ms, lease_expires_at_ms, last_renewed_at_ms, claim_epoch, mode, status
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
            "#,
            rusqlite::params![
                "claim-1",
                "run-1",
                "session-1",
                "host_session",
                "ais-agent-dev",
                1000i64,
                1600i64,
                1200i64,
                1i64,
                "exclusive",
                "active"
            ],
        )
        .expect("insert claim");
        conn.execute(
            r#"
            INSERT INTO maintenance_journal (
                operation_kind, started_at_ms, finished_at_ms, status, summary_json
            ) VALUES (?1, ?2, ?3, ?4, ?5)
            "#,
            rusqlite::params![
                "prune",
                3_000i64,
                3_100i64,
                "succeeded",
                r#"{"deleted_checkpoints":2,"storage_before":{"page_count":12,"freelist_count":1,"db_bytes":49152,"sampled_at_ms":3000},"storage_after":{"page_count":12,"freelist_count":3,"db_bytes":49152,"sampled_at_ms":3100},"storage_delta":{"page_count":0,"freelist_count":2,"db_bytes":0}}"#
            ],
        )
        .expect("insert maintenance journal");
        conn.execute(
            r#"
            INSERT INTO store_maintenance_state (
                singleton_key, last_operation_kind, last_operation_status,
                last_store_opened_at_ms,
                last_prune_started_at_ms, last_prune_finished_at_ms,
                last_pruned_terminal_before_ms, last_prune_deleted_rows, last_purge_deleted_rows,
                last_vacuum_started_at_ms, last_vacuum_finished_at_ms, last_vacuum_at_ms,
                last_wal_checkpoint_at_ms, last_known_page_count, last_known_freelist_count,
                last_known_db_bytes, last_growth_sampled_at_ms, schema_retention_version,
                metadata_schema_version
            ) VALUES ('default', ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)
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
                "prune",
                "succeeded",
                2_500i64,
                3_000i64,
                3_100i64,
                2_500i64,
                2i64,
                0i64,
                3_900i64,
                4_000i64,
                4_000i64,
                3_900i64,
                9i64,
                0i64,
                36_864i64,
                4_000i64,
                1i64,
                1i64
            ],
        )
        .expect("insert maintenance state");
    }

    #[test]
    fn overview_reports_counts_and_run_rows() {
        let path = temp_sqlite_path("overview");
        seed_store(&path);

        let output = inspect_store(
            &path,
            StoreInspectCommand::Overview {
                limit: 5,
                status: None,
                phase: None,
                active_boundary_kind: None,
                run_id_prefix: None,
            },
        )
        .expect("overview");

        assert_eq!(output["table_counts"]["runs"], 1);
        assert_eq!(output["table_counts"]["run_inputs"], 1);
        assert_eq!(output["table_counts"]["run_events"], 1);
        assert_eq!(output["table_counts"]["run_audits"], 1);
        assert_eq!(output["table_counts"]["run_checkpoints"], 1);
        assert_eq!(output["table_counts"]["run_wait_states"], 1);
        assert_eq!(output["table_counts"]["run_claim_history"], 1);
        assert_eq!(output["runs"][0]["run_id"], "run-1");
        assert_eq!(output["runs"][0]["status"], "completed");

        let _ = fs::remove_file(path);
    }

    #[test]
    fn run_query_aggregates_run_input_checkpoint_and_claim_state() {
        let path = temp_sqlite_path("run");
        seed_store(&path);

        let output = inspect_store(
            &path,
            StoreInspectCommand::Run {
                run_id: "run-1".to_owned(),
            },
        )
        .expect("run");

        assert_eq!(output["catalog"]["mission_id"], "mission-1");
        assert_eq!(output["mission"]["goal"], "transfer");
        assert_eq!(output["latest_checkpoint"]["checkpoint_seq"], 3);
        assert_eq!(output["wait_state"]["request_id"], "signer-1");
        assert_eq!(output["wait_state"]["wait_kind"], "signer");
        assert_eq!(output["active_claim"]["claim_id"], "claim-1");

        let _ = fs::remove_file(path);
    }

    #[test]
    fn overview_and_run_queries_read_final_store_rows() {
        let path = temp_sqlite_path("run-final");
        seed_store(&path);

        let overview = inspect_store(
            &path,
            StoreInspectCommand::Overview {
                limit: 5,
                status: None,
                phase: None,
                active_boundary_kind: None,
                run_id_prefix: None,
            },
        )
        .expect("overview");
        assert_eq!(overview["table_counts"]["runs"], 1);
        assert_eq!(overview["runs"][0]["run_id"], "run-1");
        assert_eq!(overview["runs"][0]["status"], "completed");
        assert_eq!(overview["runs"][0]["active_boundary_kind"], "confirmation");

        let run_output = inspect_store(
            &path,
            StoreInspectCommand::Run {
                run_id: "run-1".to_owned(),
            },
        )
        .expect("run");
        assert_eq!(run_output["catalog"]["mission_id"], "mission-1");
        assert_eq!(run_output["mission"]["goal"], "transfer");
        assert_eq!(run_output["latest_checkpoint"]["checkpoint_seq"], 3);
        assert_eq!(run_output["latest_checkpoint"]["archive_kind"], "boundary");
        assert_eq!(run_output["wait_state"]["request_id"], "signer-1");
        assert_eq!(run_output["active_claim"]["claim_id"], "claim-1");
        assert_eq!(run_output["counts"]["events"], 1);
        assert_eq!(run_output["counts"]["audits"], 1);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn events_audits_checkpoints_waits_and_claims_queries_read_final_rows() {
        let path = temp_sqlite_path("timeline-final");
        seed_store(&path);

        let events = inspect_store(
            &path,
            StoreInspectCommand::Events {
                run_id: "run-1".to_owned(),
                after_event_seq: None,
                checkpoint_seq: None,
                event_kind: None,
                limit: Some(10),
            },
        )
        .expect("events");
        assert_eq!(events["latest_event_seq"], 1);
        assert_eq!(events["events"][0]["event_seq"], 1);
        assert_eq!(events["events"][0]["plan_epoch"], 0);

        let audits = inspect_store(
            &path,
            StoreInspectCommand::Audits {
                run_id: "run-1".to_owned(),
                after_audit_seq: None,
                checkpoint_seq: None,
                audit_type: None,
                recovery_disposition: None,
                limit: Some(10),
            },
        )
        .expect("audits");
        assert_eq!(audits["latest_audit_seq"], 1);
        assert_eq!(audits["audits"][0]["audit_id"], "audit-1");
        assert_eq!(audits["audits"][0]["plan_epoch"], 0);

        let checkpoints = inspect_store(
            &path,
            StoreInspectCommand::Checkpoints {
                run_id: "run-1".to_owned(),
                latest: false,
                archive_kind: None,
                limit: Some(10),
            },
        )
        .expect("checkpoints");
        assert_eq!(checkpoints["latest_checkpoint"]["checkpoint_seq"], 3);
        assert_eq!(checkpoints["checkpoints"][0]["archive_kind"], "boundary");

        let waits = inspect_store(
            &path,
            StoreInspectCommand::Waits {
                run_id: Some("run-1".to_owned()),
                wait_kind: None,
                limit: 10,
            },
        )
        .expect("waits");
        assert_eq!(waits["wait_state"]["request_id"], "signer-1");
        assert_eq!(waits["wait_state"]["wait_kind"], "signer");

        let claims = inspect_store(
            &path,
            StoreInspectCommand::Claims {
                run_id: Some("run-1".to_owned()),
                status: None,
                owner_kind: None,
                host_session_id: None,
                limit: 10,
            },
        )
        .expect("claims");
        assert_eq!(claims["claims"][0]["claim_id"], "claim-1");
        assert_eq!(claims["claims"][0]["status"], "active");

        let _ = fs::remove_file(path);
    }

    #[test]
    fn raw_sql_query_is_read_only_and_returns_rows() {
        let path = temp_sqlite_path("sql");
        seed_store(&path);

        let output = inspect_store(
            &path,
            StoreInspectCommand::Sql {
                query: "select run_id, mission_id from runs".to_owned(),
                limit: None,
            },
        )
        .expect("sql");

        assert_eq!(output["row_count"], 1);
        assert_eq!(output["rows"][0]["run_id"], "run-1");

        let error = inspect_store(
            &path,
            StoreInspectCommand::Sql {
                query: "delete from runs".to_owned(),
                limit: None,
            },
        )
        .expect_err("delete should fail");
        assert!(error.to_string().contains("read-only"));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn retention_query_reports_modes_tiers_and_maintenance_state() {
        let path = temp_sqlite_path("retention");
        seed_store(&path);

        let output = inspect_store(&path, StoreInspectCommand::Retention).expect("retention");

        assert_eq!(output["run_retention_modes"]["terminal_tiered"], 1);
        assert_eq!(output["checkpoint_tiers"]["terminal_final"], 1);
        assert_eq!(output["terminal_runs"]["with_terminal_at_ms"], 1);
        assert_eq!(output["maintenance_state"]["last_operation_kind"], "prune");
        assert_eq!(output["maintenance_state"]["last_prune_deleted_rows"], 2);
        assert_eq!(output["maintenance_state"]["last_known_page_count"], 9);
        assert_eq!(output["maintenance_state"]["metadata_schema_version"], 1);
        assert_eq!(output["latest_maintenance"]["operation_kind"], "prune");
        assert_eq!(
            output["growth_trend"]["recent_maintenance"][0]["storage_delta"]["freelist_count"],
            2
        );
        assert_eq!(
            output["growth_trend"]["last_recorded_sample"]["db_bytes"],
            36_864
        );
        assert!(
            output["growth_trend"]["current_sample"]["db_bytes"]
                .as_i64()
                .expect("current db bytes")
                > 0
        );

        let _ = fs::remove_file(path);
    }

    #[test]
    fn storage_query_reports_page_stats_and_table_rows() {
        let path = temp_sqlite_path("storage");
        seed_store(&path);

        let output = inspect_store(&path, StoreInspectCommand::Storage).expect("storage");

        assert!(output["page_size"].as_i64().expect("page size") > 0);
        assert!(output["page_count"].as_i64().expect("page count") > 0);
        assert_eq!(output["table_rows"]["runs"], 1);
        assert_eq!(output["table_rows"]["run_checkpoints"], 1);
        assert_eq!(output["table_rows"]["maintenance_journal"], 1);
        assert_eq!(output["table_rows"]["store_maintenance_state"], 1);
        assert_eq!(
            output["growth_trend"]["recent_maintenance"][0]["storage_before"]["page_count"],
            12
        );
        assert!(
            output["growth_trend"]["current_sample"]["main_db_bytes"]
                .as_i64()
                .expect("main db bytes")
                > 0
        );
        assert!(output["growth_trend"]["delta_since_last_recorded_sample"]["db_bytes"].is_i64());

        let _ = fs::remove_file(path);
    }

    #[test]
    fn overview_filters_by_status_phase_boundary_and_prefix() {
        let path = temp_sqlite_path("overview-filters");
        seed_store(&path);

        let store = SqliteStore::open_path(&path).expect("open sqlite");
        let conn = store.connection();
        conn.execute(
            r#"
            INSERT INTO runs (
                run_id, mission_id, status, phase, active_boundary_kind, active_wait_kind,
                latest_checkpoint_seq, latest_event_seq, latest_audit_seq, latest_claim_epoch,
                retention_mode, created_at_ms, updated_at_ms, terminal_at_ms
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
            "#,
            rusqlite::params![
                "debug-2",
                "mission-2",
                "awaiting_signer",
                "awaiting_host",
                "signer",
                "signer",
                2i64,
                3i64,
                1i64,
                1i64,
                "active_full",
                3000i64,
                4000i64,
                Option::<i64>::None
            ],
        )
        .expect("insert second run");

        let output = inspect_store(
            &path,
            StoreInspectCommand::Overview {
                limit: 10,
                status: Some("awaiting_signer".to_owned()),
                phase: Some("awaiting_host".to_owned()),
                active_boundary_kind: Some("signer".to_owned()),
                run_id_prefix: Some("debug-".to_owned()),
            },
        )
        .expect("overview");

        assert_eq!(output["runs"].as_array().map(Vec::len), Some(1));
        assert_eq!(output["runs"][0]["run_id"], "debug-2");
        assert_eq!(output["runs"][0]["status"], "awaiting_signer");

        let _ = fs::remove_file(path);
    }

    #[test]
    fn waits_support_filtered_list_mode() {
        let path = temp_sqlite_path("waits-filtered-list");
        seed_store(&path);

        let store = SqliteStore::open_path(&path).expect("open sqlite");
        let conn = store.connection();
        conn.execute(
            r#"
            INSERT INTO run_wait_states (run_id, wait_kind, request_id, entered_at_ms, expires_at_ms, state_json)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            "#,
            rusqlite::params![
                "run-2",
                "confirmation",
                "submission-2",
                2100i64,
                Option::<i64>::None,
                r#"{"run_id":"run-2","request_id":"submission-2","status":"waiting"}"#
            ],
        )
        .expect("insert confirmation wait");

        let output = inspect_store(
            &path,
            StoreInspectCommand::Waits {
                run_id: None,
                wait_kind: Some("signer".to_owned()),
                limit: 10,
            },
        )
        .expect("waits");

        assert_eq!(output["waits"].as_array().map(Vec::len), Some(1));
        assert_eq!(output["waits"][0]["run_id"], "run-1");
        assert_eq!(output["waits"][0]["wait_kind"], "signer");

        let _ = fs::remove_file(path);
    }

    #[test]
    fn claims_support_filtered_list_mode() {
        let path = temp_sqlite_path("claims-filtered-list");
        seed_store(&path);

        let store = SqliteStore::open_path(&path).expect("open sqlite");
        let conn = store.connection();
        conn.execute(
            r#"
            INSERT INTO run_claim_history (
                claim_id, run_id, host_session_id, owner_kind, owner_instance_id,
                lease_started_at_ms, lease_expires_at_ms, last_renewed_at_ms, claim_epoch, mode, status
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
            "#,
            rusqlite::params![
                "claim-2",
                "run-2",
                "session-2",
                "interactive_host",
                "ais-agent-dev-2",
                3000i64,
                3600i64,
                3200i64,
                1i64,
                "exclusive_mutation",
                "released"
            ],
        )
        .expect("insert second claim");

        let output = inspect_store(
            &path,
            StoreInspectCommand::Claims {
                run_id: None,
                status: Some("active".to_owned()),
                owner_kind: Some("host_session".to_owned()),
                host_session_id: Some("session-1".to_owned()),
                limit: 10,
            },
        )
        .expect("claims");

        assert_eq!(output["claims"].as_array().map(Vec::len), Some(1));
        assert_eq!(output["claims"][0]["claim_id"], "claim-1");
        assert_eq!(output["claims"][0]["host_session_id"], "session-1");

        let _ = fs::remove_file(path);
    }

    #[test]
    fn events_and_audits_support_checkpoint_seq_filter() {
        let path = temp_sqlite_path("timeline-checkpoint-filter");
        seed_store(&path);

        let store = SqliteStore::open_path(&path).expect("open sqlite");
        let conn = store.connection();
        conn.execute(
            r#"
            INSERT INTO run_events (
                run_id, event_seq, event_kind, phase, boundary_kind, emitted_at_ms,
                checkpoint_seq, revision, payload_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
            rusqlite::params![
                "run-1",
                2i64,
                "run.progressed",
                "executing",
                "pause",
                1200i64,
                2i64,
                2i64,
                r#"{"run_id":"run-1","event_seq":2,"checkpoint_seq":2,"plan_epoch":0,"event":{"type":"progressed"}}"#
            ],
        )
        .expect("insert second event");
        conn.execute(
            r#"
            INSERT INTO run_audits (
                run_id, audit_seq, audit_kind, decision_class, emitted_at_ms,
                checkpoint_seq, revision, payload_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
            rusqlite::params![
                "run-1",
                2i64,
                "recovery",
                "allow",
                1600i64,
                3i64,
                3i64,
                r#"{"audit_id":"audit-2","run_id":"run-1","audit_seq":2,"checkpoint_seq":3,"plan_epoch":0,"audit":{"type":"recovery"}}"#
            ],
        )
        .expect("insert second audit");

        let events = inspect_store(
            &path,
            StoreInspectCommand::Events {
                run_id: "run-1".to_owned(),
                after_event_seq: None,
                checkpoint_seq: Some(2),
                event_kind: None,
                limit: Some(10),
            },
        )
        .expect("events");
        assert_eq!(events["events"].as_array().map(Vec::len), Some(1));
        assert_eq!(events["events"][0]["event_seq"], 2);
        assert_eq!(events["events"][0]["event_kind"], "run.progressed");

        let audits = inspect_store(
            &path,
            StoreInspectCommand::Audits {
                run_id: "run-1".to_owned(),
                after_audit_seq: None,
                checkpoint_seq: Some(3),
                audit_type: None,
                recovery_disposition: None,
                limit: Some(10),
            },
        )
        .expect("audits");
        assert_eq!(audits["audits"].as_array().map(Vec::len), Some(1));
        assert_eq!(audits["audits"][0]["audit_id"], "audit-2");
        assert_eq!(audits["audits"][0]["audit_type"], "recovery");

        let _ = fs::remove_file(path);
    }

    #[test]
    fn events_audits_and_checkpoints_support_semantic_filters() {
        let path = temp_sqlite_path("timeline-semantic-filters");
        seed_store(&path);

        let store = SqliteStore::open_path(&path).expect("open sqlite");
        let conn = store.connection();
        conn.execute(
            r#"
            INSERT INTO run_checkpoints (
                run_id, checkpoint_seq, plan_epoch, checkpoint_kind, retention_tier,
                created_at_ms, is_terminal, is_side_effect_boundary, is_recovery_boundary,
                is_first_wait_checkpoint, snapshot_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
            "#,
            rusqlite::params![
                "run-1",
                2i64,
                0i64,
                "side_effect",
                "active_full",
                1800i64,
                0i64,
                1i64,
                0i64,
                0i64,
                r#"{"run_id":"run-1","checkpoint_seq":2,"plan_epoch":0,"status":"running"}"#
            ],
        )
        .expect("insert second checkpoint");
        conn.execute(
            r#"
            INSERT INTO run_events (
                run_id, event_seq, event_kind, phase, boundary_kind, emitted_at_ms,
                checkpoint_seq, revision, payload_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
            rusqlite::params![
                "run-1",
                2i64,
                "awaiting_signer",
                "awaiting_host",
                "signer",
                1200i64,
                2i64,
                2i64,
                r#"{"run_id":"run-1","event_seq":2,"checkpoint_seq":2,"plan_epoch":0,"event":{"type":"awaiting_signer"}}"#
            ],
        )
        .expect("insert second event");
        conn.execute(
            r#"
            INSERT INTO run_audits (
                run_id, audit_seq, audit_kind, decision_class, emitted_at_ms,
                checkpoint_seq, revision, payload_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
            rusqlite::params![
                "run-1",
                2i64,
                "recovery",
                "allow",
                1600i64,
                3i64,
                3i64,
                r#"{"audit_id":"audit-2","run_id":"run-1","audit_seq":2,"checkpoint_seq":3,"plan_epoch":0,"audit":{"type":"recovery","recovery_disposition":"await_signer"}}"#
            ],
        )
        .expect("insert second audit");

        let events = inspect_store(
            &path,
            StoreInspectCommand::Events {
                run_id: "run-1".to_owned(),
                after_event_seq: None,
                checkpoint_seq: None,
                event_kind: Some("awaiting_signer".to_owned()),
                limit: Some(10),
            },
        )
        .expect("events");
        assert_eq!(events["events"].as_array().map(Vec::len), Some(1));
        assert_eq!(events["events"][0]["event_kind"], "awaiting_signer");

        let audits = inspect_store(
            &path,
            StoreInspectCommand::Audits {
                run_id: "run-1".to_owned(),
                after_audit_seq: None,
                checkpoint_seq: None,
                audit_type: Some("recovery".to_owned()),
                recovery_disposition: Some("await_signer".to_owned()),
                limit: Some(10),
            },
        )
        .expect("audits");
        assert_eq!(audits["audits"].as_array().map(Vec::len), Some(1));
        assert_eq!(audits["audits"][0]["audit_type"], "recovery");
        assert_eq!(audits["audits"][0]["recovery_disposition"], "await_signer");

        let checkpoints = inspect_store(
            &path,
            StoreInspectCommand::Checkpoints {
                run_id: "run-1".to_owned(),
                latest: false,
                archive_kind: Some("side_effect".to_owned()),
                limit: Some(10),
            },
        )
        .expect("checkpoints");
        assert_eq!(checkpoints["checkpoints"].as_array().map(Vec::len), Some(1));
        assert_eq!(checkpoints["checkpoints"][0]["archive_kind"], "side_effect");
        assert_eq!(checkpoints["checkpoints"][0]["checkpoint_seq"], 2);

        let latest_side_effect_checkpoint = inspect_store(
            &path,
            StoreInspectCommand::Checkpoints {
                run_id: "run-1".to_owned(),
                latest: true,
                archive_kind: Some("side_effect".to_owned()),
                limit: Some(10),
            },
        )
        .expect("latest filtered checkpoint");
        assert_eq!(
            latest_side_effect_checkpoint["latest_checkpoint"]["archive_kind"],
            "side_effect"
        );
        assert_eq!(
            latest_side_effect_checkpoint["latest_checkpoint"]["checkpoint_seq"],
            2
        );

        let _ = fs::remove_file(path);
    }
}
