use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeEnvelopeKind {
    EvmEnvelope,
    SolanaEnvelope,
    ExternalJob,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeEnvelope {
    pub envelope_id: String,
    pub kind: RuntimeEnvelopeKind,
    pub chain: String,
    pub payload: Value,
    pub provenance: Option<String>,
}
