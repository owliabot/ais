use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanSkeletonDocument {
    pub schema: String,
    #[serde(default)]
    pub default_chain: Option<String>,
    pub nodes: Vec<Value>,
    #[serde(default)]
    pub policy_hints: Option<Value>,
    #[serde(default)]
    pub extensions: Map<String, Value>,
}
