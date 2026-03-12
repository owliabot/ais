use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActuationKind {
    EnvelopeBuilt,
    SignerRequested,
    BroadcastSubmitted,
    ReceiptObserved,
    ExternalJobSubmitted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActuationStatus {
    Pending,
    Succeeded,
    Failed,
}

/// Runtime-owned side-effect ledger entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActuationRecord {
    pub record_id: String,
    pub node_id: String,
    pub kind: ActuationKind,
    pub status: ActuationStatus,
    pub chain: Option<String>,
    pub tx_hash: Option<String>,
    pub summary: String,
}
