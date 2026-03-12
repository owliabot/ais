use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeriveKind {
    Parameter,
    Risk,
    SlippageBound,
    ExpectedEffect,
    Budget,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeriveAction {
    pub derive_kind: DeriveKind,
    pub derivation_hint: String,
    pub output_key: Option<String>,
}
