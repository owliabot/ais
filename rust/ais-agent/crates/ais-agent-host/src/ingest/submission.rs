use serde::{Deserialize, Serialize};

use ais_agent_control::ids::RunId;

use crate::{
    envelope::HostEnvelopeSubmission, evidence::HostEvidenceSubmission, signer::HostSignerDecision,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostIngestKind {
    Evidence,
    Envelope,
    SignerDecision,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HostIngestSubmission {
    Evidence(HostEvidenceSubmission),
    Envelope(HostEnvelopeSubmission),
    SignerDecision(HostSignerDecision),
}

impl HostIngestSubmission {
    pub fn kind(&self) -> HostIngestKind {
        match self {
            Self::Evidence(_) => HostIngestKind::Evidence,
            Self::Envelope(_) => HostIngestKind::Envelope,
            Self::SignerDecision(_) => HostIngestKind::SignerDecision,
        }
    }

    pub fn run_id(&self) -> &RunId {
        match self {
            Self::Evidence(value) => &value.run_id,
            Self::Envelope(value) => &value.run_id,
            Self::SignerDecision(value) => &value.run_id,
        }
    }
}
