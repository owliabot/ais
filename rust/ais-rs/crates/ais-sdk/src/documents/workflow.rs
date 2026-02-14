use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowDocument {
    pub schema: String,
    pub meta: Value,
    #[serde(default)]
    pub default_chain: Option<String>,
    #[serde(default)]
    pub imports: Option<Value>,
    #[serde(default)]
    pub requires_pack: Option<Value>,
    #[serde(default)]
    pub inputs: Map<String, Value>,
    pub nodes: Vec<Value>,
    #[serde(default)]
    pub policy: Option<Value>,
    #[serde(default)]
    pub preflight: Option<Value>,
    #[serde(default)]
    pub outputs: Map<String, Value>,
    #[serde(default)]
    pub extensions: Map<String, Value>,
}
