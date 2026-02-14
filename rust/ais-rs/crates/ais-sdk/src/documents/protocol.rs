use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProtocolDocument {
    pub schema: String,
    pub meta: Value,
    pub deployments: Vec<Value>,
    pub actions: Map<String, Value>,
    #[serde(default)]
    pub queries: Map<String, Value>,
    #[serde(default)]
    pub risks: Vec<Value>,
    #[serde(default)]
    pub supported_assets: Vec<Value>,
    #[serde(default)]
    pub capabilities_required: Vec<String>,
    #[serde(default)]
    pub tests: Vec<Value>,
    #[serde(default)]
    pub extensions: Map<String, Value>,
}
