use serde::{Deserialize, Serialize};
use serde_json::Value;

use ais_agent_core::{driver::DriverBuildOutput, evidence::EvidenceGraph, mission::Mission};

use crate::ChainFamily;

pub type ReflectionDriverOutput = DriverBuildOutput;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReflectionArtifactKind {
    EvmAbi,
    SolanaIdl,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReflectionRequest {
    pub mission: Mission,
    pub evidence: EvidenceGraph,
    pub chain_family: ChainFamily,
    pub artifact_kind: ReflectionArtifactKind,
    pub artifact: Value,
    pub action_selector: String,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ReflectionDriverError {
    #[error("unsupported chain family for reflection: {0:?}")]
    UnsupportedFamily(ChainFamily),
    #[error("unsupported reflection artifact kind")]
    UnsupportedArtifact,
    #[error("unsupported reflected action selector: {0}")]
    UnsupportedAction(String),
    #[error("invalid reflection output: {0}")]
    InvalidOutput(String),
}

pub trait ReflectionDriver {
    fn driver_id(&self) -> &'static str;
    fn family(&self) -> ChainFamily;
    fn build(
        &self,
        request: &ReflectionRequest,
    ) -> Result<ReflectionDriverOutput, ReflectionDriverError>;
}
