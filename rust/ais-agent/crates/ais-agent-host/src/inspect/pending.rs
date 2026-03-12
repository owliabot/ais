use serde::{Deserialize, Serialize};

use ais_agent_control::ids::SignerRequestId;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingSignerRequestView {
    pub request_id: SignerRequestId,
    pub chain: Option<String>,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingConfirmationView {
    pub confirmation_id: String,
    pub kind: String,
    pub summary: String,
}
