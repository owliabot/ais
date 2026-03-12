use serde::{Deserialize, Serialize};

use crate::runtime::ActiveRun;

/// Version tuple exposed to hosts for optimistic concurrency.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeVersion {
    pub checkpoint_seq: u64,
    pub plan_epoch: u64,
    pub revision: u64,
}

impl RuntimeVersion {
    pub fn from_runtime(runtime: &ActiveRun) -> Self {
        Self {
            checkpoint_seq: runtime.checkpoint_seq(),
            plan_epoch: runtime.plan_epoch(),
            revision: runtime.revision,
        }
    }
}
