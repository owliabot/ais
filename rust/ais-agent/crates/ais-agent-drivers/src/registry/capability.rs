use serde::{Deserialize, Serialize};

use ais_agent_core::evidence::EvidenceKind;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DriverPathKind {
    StandardDriver,
    ReflectionPath,
    ApiNativePath,
    RawEnvelopeFallback,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriverCapability {
    pub driver_id: String,
    pub label: String,
    pub path_kind: DriverPathKind,
    #[serde(default)]
    pub supported_chains: Vec<String>,
    #[serde(default)]
    pub required_evidence_kinds: Vec<EvidenceKind>,
    #[serde(default)]
    pub goal_keywords: Vec<String>,
}
