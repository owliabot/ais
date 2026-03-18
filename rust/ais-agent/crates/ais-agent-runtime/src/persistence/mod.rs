//! Runtime persistence and checkpoint orchestration.

mod audit_archive;
mod checkpoint_repo;
mod claim_repo;
mod durable_executor;
mod durable_mutation;
mod event_archive;
mod in_memory;
mod mission_repo;
mod persist;
mod restore;
mod run_catalog;
mod wait_state_store;

pub use audit_archive::{
    RuntimeAuditArchive, RuntimeAuditArchiveError, RuntimeAuditQuery, RuntimeAuditSlice,
};
pub use checkpoint_repo::{
    CheckpointArchive, CheckpointArchiveEntry, CheckpointArchiveError, CheckpointArchiveKind,
    CheckpointRepository, CheckpointRepositoryError,
};
pub use claim_repo::{
    ClaimExpireRequest, ClaimReleaseRequest, ClaimRenewRequest, ClaimSupersedeRequest,
    ClaimSupersedeResult, RunClaimRepository, RunClaimRepositoryError,
};
pub use durable_executor::{
    DurableCommitError, DurableCommitReceipt, DurableMutationExecutor, DurableMutationMember,
    LinearDurableMutationExecutor,
};
pub use durable_mutation::{
    validate_durable_mutation_unit, AuditWriteBatch, CatalogWrite, CheckpointWrite,
    DurableMutationContractError, DurableMutationKind, DurableMutationUnit, EventWriteBatch,
    MissionWrite, MissionWriteMode, RunWaitStateWrite, SignerStateWrite,
};
pub use event_archive::{EventArchive, EventArchiveError, EventArchiveQuery, EventArchiveSlice};
pub use in_memory::{
    InMemoryCheckpointRepository, InMemoryEventArchive, InMemoryMissionRepository,
    InMemoryRunCatalogRepository, InMemoryRunClaimRepository, InMemoryRunWaitStateStore,
    InMemoryRuntimeAuditArchive, InMemorySignerStateStore,
};
pub use mission_repo::{MissionRepository, MissionRepositoryError};
pub use persist::{
    persist_boundary_checkpoint, persist_progress_checkpoint, persist_side_effect_checkpoint,
};
pub use restore::{restore_active_run, restore_active_run_from_parts, RestoreRuntimeError};
pub use run_catalog::{RunCatalogEntry, RunCatalogRepository, RunCatalogRepositoryError};
pub use wait_state_store::{
    signer_state_into_wait_state_record, wait_state_record_into_signer_state, RunWaitStateRecord,
    RunWaitStateStore, RunWaitStateStoreError, SignerStateStore, SignerStateStoreError,
};
