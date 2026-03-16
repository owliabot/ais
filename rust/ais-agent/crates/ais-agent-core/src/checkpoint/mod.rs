//! Checkpoint domain objects.

mod replay;
mod snapshot;
mod store;

#[cfg(test)]
mod tests;

pub use replay::ReplayCursor;
pub use snapshot::{
    ArtifactBranchTraceSnapshot, ArtifactContinuationSnapshot, CheckpointSnapshot,
    ExecutionArtifactRuntimeSnapshot, PendingRequestsSnapshot,
};
pub use store::{
    CheckpointPointer, CheckpointStore, CheckpointStoreError, InMemoryCheckpointStore,
};
