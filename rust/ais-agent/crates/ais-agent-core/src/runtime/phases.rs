use serde::{Deserialize, Serialize};

/// High-level runtime phase. This is intentionally coarse and host-facing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunPhase {
    MissionAccepted,
    Planning,
    Simulating,
    Governing,
    AwaitingHost,
    Broadcasting,
    Verifying,
    Recovering,
    Finalized,
}
