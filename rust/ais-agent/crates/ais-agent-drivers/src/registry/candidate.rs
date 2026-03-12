use serde::{Deserialize, Serialize};

use crate::registry::DriverPathKind;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriverCandidate {
    pub driver_id: String,
    pub label: String,
    pub path_kind: DriverPathKind,
    pub score: i32,
    #[serde(default)]
    pub matched_reasons: Vec<String>,
    #[serde(default)]
    pub missing_evidence_kinds: Vec<String>,
}
