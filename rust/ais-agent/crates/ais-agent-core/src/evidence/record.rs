use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::evidence::{EvidenceFreshness, EvidenceProvenance};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    Fact,
    QueryResult,
    RouteOrQuote,
    Metadata,
    ExternalObservation,
}

/// Runtime-owned evidence object.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceRecord {
    pub evidence_id: String,
    pub kind: EvidenceKind,
    pub provenance: EvidenceProvenance,
    pub freshness: EvidenceFreshness,
    pub confidence_ppm: Option<u32>,
    pub payload: Value,
}
