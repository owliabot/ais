use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

use ais_agent_control::{
    execution_artifact::{ExecutionOutputKey, ExecutionPackageEntry, ExecutionStageId},
    ids::SignerRequestId,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingSignerTimeoutPolicyView {
    pub requested_at_ms: u64,
    pub expires_at_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingSignerRequestView {
    pub request_id: SignerRequestId,
    pub node_id: Option<String>,
    pub chain: Option<String>,
    pub summary: String,
    #[serde(default)]
    pub payload: Option<Value>,
    #[serde(default)]
    pub timeout_policy: Option<PendingSignerTimeoutPolicyView>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingConfirmationView {
    pub confirmation_id: String,
    pub kind: String,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingContinuationView {
    pub stage_id: ExecutionStageId,
    pub package_entry: ExecutionPackageEntry,
    #[serde(default)]
    pub required_outputs: Vec<ExecutionOutputKey>,
    #[serde(default)]
    pub resolved_outputs: BTreeMap<ExecutionOutputKey, Value>,
    pub summary: String,
}
