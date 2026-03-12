use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{ChainId, ConfirmationDepth, FinalityLevel};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadRequest {
    pub chain_id: ChainId,
    pub method: String,
    pub params: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadResponse {
    pub payload: Value,
    pub source_hint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulationRequest {
    pub chain_id: ChainId,
    pub mode: String,
    pub payload: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulationResponse {
    pub accepted: bool,
    pub payload: Value,
    pub state_delta_hint: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BroadcastRequest {
    pub chain_id: ChainId,
    pub signed_payload: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BroadcastResponse {
    pub tx_hash: String,
    pub accepted_by: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReceiptQuery {
    pub chain_id: ChainId,
    pub tx_hash: String,
    pub min_confirmation_depth: Option<ConfirmationDepth>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReceiptView {
    pub tx_hash: String,
    pub finality: FinalityLevel,
    pub confirmation_depth: Option<ConfirmationDepth>,
    pub payload: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateQuery {
    pub chain_id: ChainId,
    pub subject: String,
    pub params: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateView {
    pub subject: String,
    pub observed_at_ms: Option<u64>,
    pub payload: Value,
}
