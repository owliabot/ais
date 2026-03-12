use std::path::Path;
use std::time::Duration;

use rusqlite::Connection;
use thiserror::Error;

use crate::migrate_connection;

#[derive(Debug, Error)]
pub enum SqliteStoreError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

#[derive(Debug)]
pub struct SqliteStore {
    conn: Connection,
}

impl SqliteStore {
    pub fn open_in_memory() -> Result<Self, SqliteStoreError> {
        let conn = Connection::open_in_memory()?;
        configure_connection(&conn, false)?;
        migrate_connection(&conn)?;
        Ok(Self { conn })
    }

    pub fn open_path(path: impl AsRef<Path>) -> Result<Self, SqliteStoreError> {
        let conn = Connection::open(path)?;
        configure_connection(&conn, true)?;
        migrate_connection(&conn)?;
        Ok(Self { conn })
    }

    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    pub(crate) fn connection_mut(&mut self) -> &mut Connection {
        &mut self.conn
    }
}

fn configure_connection(conn: &Connection, enable_wal: bool) -> Result<(), SqliteStoreError> {
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.busy_timeout(Duration::from_millis(5_000))?;
    if enable_wal {
        conn.pragma_update(None, "journal_mode", "WAL")?;
    }
    Ok(())
}
