use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectContractKind {
    AssetDelta,
    StateTransition,
    ExternalJobOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectAssertion {
    pub expression: String,
    pub description: String,
}

/// Runtime-owned expected-effect object.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EffectContract {
    pub effect_id: String,
    pub kind: EffectContractKind,
    #[serde(default)]
    pub assertions: Vec<EffectAssertion>,
    pub tolerance_hint: Option<String>,
}
