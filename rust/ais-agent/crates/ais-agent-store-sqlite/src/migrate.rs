use rusqlite::Connection;

use crate::schema::{
    CREATE_MAINTENANCE_JOURNAL_LATEST_INDEX, CREATE_MAINTENANCE_JOURNAL_TABLE, CREATE_RUNS_TABLE,
    CREATE_RUNS_UPDATED_AT_INDEX, CREATE_RUN_AUDITS_KIND_TIME_INDEX,
    CREATE_RUN_AUDITS_LATEST_INDEX, CREATE_RUN_AUDITS_TABLE, CREATE_RUN_CHECKPOINTS_LATEST_INDEX,
    CREATE_RUN_CHECKPOINTS_RETENTION_INDEX, CREATE_RUN_CHECKPOINTS_TABLE,
    CREATE_RUN_CHECKPOINTS_UNIQUE_INDEX, CREATE_RUN_CLAIM_HISTORY_ACTIVE_INDEX,
    CREATE_RUN_CLAIM_HISTORY_RUN_LOOKUP_INDEX, CREATE_RUN_CLAIM_HISTORY_TABLE,
    CREATE_RUN_EVENTS_KIND_TIME_INDEX, CREATE_RUN_EVENTS_LATEST_INDEX, CREATE_RUN_EVENTS_TABLE,
    CREATE_RUN_INPUTS_TABLE, CREATE_RUN_WAIT_STATES_KIND_INDEX, CREATE_RUN_WAIT_STATES_TABLE,
    CREATE_STORE_MAINTENANCE_STATE_TABLE,
};

pub const SCHEMA_VERSION: i32 = 9;

pub fn migrate_connection(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(CREATE_MAINTENANCE_JOURNAL_TABLE)?;
    conn.execute_batch(CREATE_MAINTENANCE_JOURNAL_LATEST_INDEX)?;
    conn.execute_batch(CREATE_STORE_MAINTENANCE_STATE_TABLE)?;
    ensure_store_maintenance_state_columns(conn)?;
    conn.execute_batch(CREATE_RUNS_TABLE)?;
    conn.execute_batch(CREATE_RUNS_UPDATED_AT_INDEX)?;
    conn.execute_batch(CREATE_RUN_INPUTS_TABLE)?;
    conn.execute_batch(CREATE_RUN_EVENTS_TABLE)?;
    conn.execute_batch(CREATE_RUN_EVENTS_LATEST_INDEX)?;
    conn.execute_batch(CREATE_RUN_EVENTS_KIND_TIME_INDEX)?;
    conn.execute_batch(CREATE_RUN_AUDITS_TABLE)?;
    conn.execute_batch(CREATE_RUN_AUDITS_LATEST_INDEX)?;
    conn.execute_batch(CREATE_RUN_AUDITS_KIND_TIME_INDEX)?;
    conn.execute_batch(CREATE_RUN_CHECKPOINTS_TABLE)?;
    conn.execute_batch(CREATE_RUN_CHECKPOINTS_UNIQUE_INDEX)?;
    conn.execute_batch(CREATE_RUN_CHECKPOINTS_LATEST_INDEX)?;
    conn.execute_batch(CREATE_RUN_CHECKPOINTS_RETENTION_INDEX)?;
    conn.execute_batch(CREATE_RUN_WAIT_STATES_TABLE)?;
    conn.execute_batch(CREATE_RUN_WAIT_STATES_KIND_INDEX)?;
    conn.execute_batch(CREATE_RUN_CLAIM_HISTORY_TABLE)?;
    conn.execute_batch(CREATE_RUN_CLAIM_HISTORY_ACTIVE_INDEX)?;
    conn.execute_batch(CREATE_RUN_CLAIM_HISTORY_RUN_LOOKUP_INDEX)?;
    conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    Ok(())
}

const STORE_MAINTENANCE_STATE_COLUMNS: &[(&str, &str)] = &[
    ("last_store_opened_at_ms", "INTEGER"),
    ("last_prune_deleted_rows", "INTEGER"),
    ("last_purge_deleted_rows", "INTEGER"),
    ("last_vacuum_started_at_ms", "INTEGER"),
    ("last_vacuum_finished_at_ms", "INTEGER"),
    ("last_wal_checkpoint_at_ms", "INTEGER"),
    ("last_known_page_count", "INTEGER"),
    ("last_known_freelist_count", "INTEGER"),
    ("last_known_db_bytes", "INTEGER"),
    ("last_growth_sampled_at_ms", "INTEGER"),
    ("metadata_schema_version", "INTEGER NOT NULL DEFAULT 1"),
];

fn ensure_store_maintenance_state_columns(conn: &Connection) -> rusqlite::Result<()> {
    for (column_name, definition) in STORE_MAINTENANCE_STATE_COLUMNS {
        add_column_if_missing(conn, "store_maintenance_state", column_name, definition)?;
    }
    Ok(())
}

fn add_column_if_missing(
    conn: &Connection,
    table_name: &str,
    column_name: &str,
    definition: &str,
) -> rusqlite::Result<()> {
    let pragma = format!("PRAGMA table_info({table_name})");
    let mut stmt = conn.prepare(&pragma)?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        if row.get::<_, String>(1)? == column_name {
            return Ok(());
        }
    }

    let alter = format!("ALTER TABLE {table_name} ADD COLUMN {column_name} {definition}");
    conn.execute_batch(&alter)?;
    Ok(())
}
