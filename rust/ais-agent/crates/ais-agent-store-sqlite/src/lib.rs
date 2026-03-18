//! SQLite durable store for `ais-agent` runtime persistence adapters.

mod audit_store;
mod checkpoint_store;
mod claim_store;
mod durable_executor;
mod event_store;
mod inspect;
mod maintenance;
mod maintenance_journal;
mod maintenance_state;
mod migrate;
mod mission_store;
mod run_catalog;
mod run_projection;
mod run_store;
mod schema;
mod store;
mod wait_state_store;

pub use inspect::{inspect_store, StoreInspectCommand, StoreInspectError};
pub use maintenance::{
    StoreMaintenanceError, StorePruneRequest, StorePruneResult, StorePurgeRequest,
    StorePurgeResult, StorePurgeTable, StorePurgeTarget, StoreVacuumRequest, StoreVacuumResult,
    STORE_RETENTION_SCHEMA_VERSION,
};
pub use maintenance_journal::{
    MaintenanceJournalAppend, MaintenanceJournalEntry, MaintenanceJournalError,
    MaintenanceOperationKind, MaintenanceOperationStatus,
};
pub use maintenance_state::{
    StoreMaintenanceState, StoreMaintenanceStateError, STORE_METADATA_SCHEMA_VERSION,
};
pub use migrate::{migrate_connection, SCHEMA_VERSION};
pub use run_store::{
    RunStoreError, StoredRunAudit, StoredRunAuditQuery, StoredRunAuditSlice, StoredRunCheckpoint,
    StoredRunClaim, StoredRunEvent, StoredRunEventQuery, StoredRunEventSlice, StoredRunHead,
    StoredRunInput, StoredRunWaitState,
};
pub use store::{SqliteStore, SqliteStoreError};

#[cfg(test)]
mod tests;
