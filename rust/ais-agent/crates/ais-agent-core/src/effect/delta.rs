use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectDeltaStatus {
    Pending,
    Satisfied,
    Violated,
    UnknownDueToMissingObservation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EffectDelta {
    pub effect_id: String,
    pub assertion_description: Option<String>,
    pub status: EffectDeltaStatus,
    pub summary: String,
    #[serde(default)]
    pub missing_bindings: Vec<String>,
}
