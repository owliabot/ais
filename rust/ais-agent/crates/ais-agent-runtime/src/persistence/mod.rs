//! Runtime persistence and checkpoint orchestration.

mod audit_archive;
mod checkpoint_repo;
mod durable_executor;
mod durable_mutation;
mod event_archive;
mod in_memory;
mod mission_repo;
mod persist;
mod restore;
mod run_catalog;
mod signer_archive;

pub use audit_archive::{
    RuntimeAuditArchive, RuntimeAuditArchiveError, RuntimeAuditQuery, RuntimeAuditSlice,
};
pub use checkpoint_repo::{
    CheckpointArchive, CheckpointArchiveEntry, CheckpointArchiveError, CheckpointArchiveKind,
    CheckpointRepository, CheckpointRepositoryError,
};
pub use durable_executor::{
    DurableCommitError, DurableCommitReceipt, DurableMutationExecutor, DurableMutationMember,
    LinearDurableMutationExecutor,
};
pub use durable_mutation::{
    validate_durable_mutation_unit, AuditWriteBatch, CatalogWrite, CheckpointWrite,
    DurableMutationContractError, DurableMutationKind, DurableMutationUnit, EventWriteBatch,
    MissionWrite, MissionWriteMode, SignerStateWrite,
};
pub use event_archive::{EventArchive, EventArchiveError, EventArchiveQuery, EventArchiveSlice};
pub use in_memory::{
    InMemoryCheckpointRepository, InMemoryEventArchive, InMemoryMissionRepository,
    InMemoryRunCatalogRepository, InMemoryRuntimeAuditArchive, InMemorySignerStateArchive,
};
pub use mission_repo::{MissionRepository, MissionRepositoryError};
pub use persist::{
    persist_boundary_checkpoint, persist_progress_checkpoint, persist_side_effect_checkpoint,
};
pub use restore::{restore_active_run, restore_active_run_from_parts, RestoreRuntimeError};
pub use run_catalog::{RunCatalogEntry, RunCatalogRepository, RunCatalogRepositoryError};
pub use signer_archive::{SignerStateArchive, SignerStateArchiveError};
