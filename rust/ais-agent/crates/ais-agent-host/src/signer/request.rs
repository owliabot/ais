use serde::{Deserialize, Serialize};
use serde_json::Value;

use ais_agent_control::ids::{RunId, SignerRequestId};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostSignerTimeoutPolicy {
    pub requested_at_ms: u64,
    pub expires_at_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostSignerRequest {
    pub run_id: RunId,
    pub request_id: SignerRequestId,
    pub node_id: Option<String>,
    pub chain: String,
    pub summary: String,
    pub payload: Value,
    pub timeout_policy: Option<HostSignerTimeoutPolicy>,
}
