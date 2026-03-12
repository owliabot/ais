use serde::{Deserialize, Serialize};

use crate::runtime::RunPhase;

/// Replay cursor used to resume from a checkpoint boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplayCursor {
    pub run_id: String,
    pub from_checkpoint_seq: u64,
    pub resume_phase: RunPhase,
    pub last_completed_node_id: Option<String>,
}
