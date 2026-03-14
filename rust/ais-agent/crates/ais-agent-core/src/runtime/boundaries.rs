use serde::{Deserialize, Serialize};

use ais_agent_control::ids::SignerRequestId;

/// Stable runtime boundaries where a host may safely observe or take over.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoundaryKind {
    Pause,
    Evidence,
    Signer,
    Confirmation,
    ArtifactContinuation,
    Completion,
    Failure,
    Cancellation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StableBoundary {
    pub kind: BoundaryKind,
    pub summary: String,
    #[serde(default)]
    pub blocking_refs: Vec<String>,
    pub signer_request_id: Option<SignerRequestId>,
}
