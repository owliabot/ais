use serde::{Deserialize, Serialize};

use ais_agent_control::ids::RunId;

use crate::{
    envelope::HostEnvelopeSubmission, evidence::HostEvidenceSubmission,
    signer::HostSignerResolution,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostIngestKind {
    Evidence,
    Envelope,
    SignerResolution,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HostIngestSubmission {
    Evidence(HostEvidenceSubmission),
    Envelope(HostEnvelopeSubmission),
    SignerResolution(HostSignerResolution),
}

impl HostIngestSubmission {
    pub fn kind(&self) -> HostIngestKind {
        match self {
            Self::Evidence(_) => HostIngestKind::Evidence,
            Self::Envelope(_) => HostIngestKind::Envelope,
            Self::SignerResolution(_) => HostIngestKind::SignerResolution,
        }
    }

    pub fn run_id(&self) -> &RunId {
        match self {
            Self::Evidence(value) => &value.run_id,
            Self::Envelope(value) => &value.run_id,
            Self::SignerResolution(value) => &value.run_id,
        }
    }
}
