use rusqlite::Connection;

use crate::schema::{
    CREATE_CHECKPOINT_ARCHIVE_LATEST_INDEX, CREATE_CHECKPOINT_ARCHIVE_TABLE,
    CREATE_CHECKPOINT_ARCHIVE_UNIQUE_INDEX, CREATE_EVENT_ARCHIVE_TABLE, CREATE_MISSIONS_TABLE,
    CREATE_RUNTIME_AUDIT_ARCHIVE_LATEST_INDEX, CREATE_RUNTIME_AUDIT_ARCHIVE_TABLE,
    CREATE_RUN_CATALOG_TABLE, CREATE_SIGNER_STATE_ARCHIVE_TABLE,
};

pub const SCHEMA_VERSION: i32 = 4;

pub fn migrate_connection(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(CREATE_MISSIONS_TABLE)?;
    conn.execute_batch(CREATE_RUN_CATALOG_TABLE)?;
    conn.execute_batch(CREATE_CHECKPOINT_ARCHIVE_TABLE)?;
    conn.execute_batch(CREATE_CHECKPOINT_ARCHIVE_UNIQUE_INDEX)?;
    conn.execute_batch(CREATE_CHECKPOINT_ARCHIVE_LATEST_INDEX)?;
    conn.execute_batch(CREATE_EVENT_ARCHIVE_TABLE)?;
    conn.execute_batch(CREATE_RUNTIME_AUDIT_ARCHIVE_TABLE)?;
    conn.execute_batch(CREATE_RUNTIME_AUDIT_ARCHIVE_LATEST_INDEX)?;
    conn.execute_batch(CREATE_SIGNER_STATE_ARCHIVE_TABLE)?;
    conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    Ok(())
}
