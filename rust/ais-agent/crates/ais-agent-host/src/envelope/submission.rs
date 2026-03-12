use serde::{Deserialize, Serialize};
use serde_json::Value;

use ais_agent_control::ids::RunId;
use ais_agent_core::{
    effect::EffectContract,
    envelope::{RuntimeEnvelope, RuntimeEnvelopeKind},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostEnvelopeKind {
    EvmEnvelope,
    SolanaEnvelope,
    ExternalJob,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostEnvelopeSubmission {
    pub run_id: RunId,
    pub envelope_id: String,
    pub kind: HostEnvelopeKind,
    pub chain: String,
    pub payload: Value,
    pub expected_effect_ref: Option<String>,
    pub expected_effect_contract: Option<EffectContract>,
    pub provenance: Option<String>,
}

impl HostEnvelopeSubmission {
    pub fn into_runtime_envelope(self) -> RuntimeEnvelope {
        RuntimeEnvelope {
            envelope_id: self.envelope_id,
            kind: match self.kind {
                HostEnvelopeKind::EvmEnvelope => RuntimeEnvelopeKind::EvmEnvelope,
                HostEnvelopeKind::SolanaEnvelope => RuntimeEnvelopeKind::SolanaEnvelope,
                HostEnvelopeKind::ExternalJob => RuntimeEnvelopeKind::ExternalJob,
            },
            chain: self.chain,
            payload: self.payload,
            provenance: self.provenance,
        }
    }
}
