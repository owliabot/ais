use alloy_primitives::{Address, Bytes, U256};
use serde_json::Value;
use solana_sdk::instruction::Instruction;

use ais_agent_core::{evidence::EvidenceGraph, mission::Mission};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiNativeProviderKind {
    QuoteProvider,
    RouteProvider,
    DirectEnvelopeProvider,
}

#[derive(Debug, Clone)]
pub struct EvmNativeEnvelope {
    pub to: Address,
    pub data: Bytes,
    pub value: U256,
}

#[derive(Debug, Clone)]
pub struct SolanaNativeEnvelope {
    pub instructions: Vec<Instruction>,
}

#[derive(Debug, Clone)]
pub enum DirectEnvelopePayload {
    Evm(EvmNativeEnvelope),
    Solana(SolanaNativeEnvelope),
    ExternalJob(Value),
}

#[derive(Debug, Clone)]
pub struct ApiNativeRequest {
    pub mission: Mission,
    pub evidence: EvidenceGraph,
    pub provider_id: String,
    pub provider_kind: ApiNativeProviderKind,
    pub chain: Option<String>,
    pub payload: Value,
    pub direct_envelope: Option<DirectEnvelopePayload>,
}
