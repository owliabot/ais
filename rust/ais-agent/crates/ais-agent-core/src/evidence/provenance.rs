use serde::{Deserialize, Serialize};

/// Provenance for a runtime-owned evidence record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceProvenance {
    pub source: String,
    pub chain_scope: Option<String>,
    pub trace_hint: Option<String>,
}
