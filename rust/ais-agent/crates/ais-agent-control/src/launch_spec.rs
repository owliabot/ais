use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::execution_artifact::ExecutionArtifactLaunchSpec;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LaunchSpecSubmission {
    PrebuiltFragment(PrebuiltFragmentLaunchSpec),
    ReflectionRequest(ReflectionRequestLaunchSpec),
    ExecutionArtifact(ExecutionArtifactLaunchSpec),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PrebuiltFragmentLaunchSpec {
    #[serde(default)]
    pub action_graph: Option<Value>,
    #[serde(default)]
    pub evidence_graph: Option<Value>,
    #[serde(default)]
    pub effect_contracts: Option<Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReflectionRequestLaunchSpec {
    #[serde(default)]
    pub request: Value,
}
