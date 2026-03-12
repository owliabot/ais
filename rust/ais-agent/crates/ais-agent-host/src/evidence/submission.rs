use serde::{Deserialize, Serialize};
use serde_json::Value;

use ais_agent_control::ids::RunId;
use ais_agent_core::evidence::{
    EvidenceFreshness, EvidenceKind, EvidenceProvenance, EvidenceRecord,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostEvidenceSubmission {
    pub run_id: RunId,
    pub evidence_id: String,
    pub kind: EvidenceKind,
    pub source: String,
    pub observed_at_ms: Option<u64>,
    pub expires_at_ms: Option<u64>,
    pub max_age_ms: Option<u64>,
    pub chain_scope: Option<String>,
    pub trace_hint: Option<String>,
    pub confidence_ppm: Option<u32>,
    pub payload: Value,
}

impl HostEvidenceSubmission {
    pub fn into_evidence_record(self) -> EvidenceRecord {
        EvidenceRecord {
            evidence_id: self.evidence_id,
            kind: self.kind,
            provenance: EvidenceProvenance {
                source: self.source,
                chain_scope: self.chain_scope,
                trace_hint: self.trace_hint,
            },
            freshness: EvidenceFreshness {
                observed_at_ms: self.observed_at_ms,
                expires_at_ms: self.expires_at_ms,
                max_age_ms: self.max_age_ms,
            },
            confidence_ppm: self.confidence_ppm,
            payload: self.payload,
        }
    }
}
