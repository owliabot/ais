//! SQLite durable store for `ais-agent` runtime archives.

mod audit_archive;
mod checkpoint_archive;
mod durable_executor;
mod event_archive;
mod migrate;
mod mission_store;
mod run_catalog;
mod schema;
mod signer_archive;
mod store;

pub use migrate::{migrate_connection, SCHEMA_VERSION};
pub use store::{SqliteStore, SqliteStoreError};

#[cfg(test)]
mod tests;
