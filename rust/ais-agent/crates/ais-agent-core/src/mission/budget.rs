use serde::{Deserialize, Serialize};

/// Runtime-owned execution budget.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct MissionBudget {
    pub max_steps: Option<u32>,
    pub max_signer_requests: Option<u32>,
    pub max_wall_clock_ms: Option<u64>,
}
