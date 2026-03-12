use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::checkpoint::CheckpointSnapshot;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointPointer {
    pub run_id: String,
    pub checkpoint_seq: u64,
}

#[derive(Debug, Error)]
pub enum CheckpointStoreError {
    #[error("checkpoint not found for run `{run_id}`")]
    NotFound { run_id: String },
}

/// Storage abstraction for checkpoint snapshots.
pub trait CheckpointStore {
    fn save(
        &mut self,
        snapshot: CheckpointSnapshot,
    ) -> Result<CheckpointPointer, CheckpointStoreError>;
    fn latest(&self, run_id: &str) -> Result<CheckpointSnapshot, CheckpointStoreError>;
    fn load(&self, pointer: &CheckpointPointer)
        -> Result<CheckpointSnapshot, CheckpointStoreError>;
}

/// Minimal in-memory store used while the runtime is still greenfield.
#[derive(Debug, Default)]
pub struct InMemoryCheckpointStore {
    snapshots: BTreeMap<(String, u64), CheckpointSnapshot>,
}

impl CheckpointStore for InMemoryCheckpointStore {
    fn save(
        &mut self,
        snapshot: CheckpointSnapshot,
    ) -> Result<CheckpointPointer, CheckpointStoreError> {
        let pointer = CheckpointPointer {
            run_id: snapshot.run_id.clone(),
            checkpoint_seq: snapshot.checkpoint_seq,
        };
        self.snapshots
            .insert((pointer.run_id.clone(), pointer.checkpoint_seq), snapshot);
        Ok(pointer)
    }

    fn latest(&self, run_id: &str) -> Result<CheckpointSnapshot, CheckpointStoreError> {
        self.snapshots
            .iter()
            .filter(|((candidate_run_id, _), _)| candidate_run_id == run_id)
            .max_by_key(|((_, checkpoint_seq), _)| *checkpoint_seq)
            .map(|(_, snapshot)| snapshot.clone())
            .ok_or_else(|| CheckpointStoreError::NotFound {
                run_id: run_id.to_owned(),
            })
    }

    fn load(
        &self,
        pointer: &CheckpointPointer,
    ) -> Result<CheckpointSnapshot, CheckpointStoreError> {
        self.snapshots
            .get(&(pointer.run_id.clone(), pointer.checkpoint_seq))
            .cloned()
            .ok_or_else(|| CheckpointStoreError::NotFound {
                run_id: pointer.run_id.clone(),
            })
    }
}
