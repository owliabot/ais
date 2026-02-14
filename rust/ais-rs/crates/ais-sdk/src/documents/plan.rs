use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanDocument {
    pub schema: String,
    #[serde(default)]
    pub meta: Option<Value>,
    pub nodes: Vec<Value>,
    #[serde(default)]
    pub extensions: Map<String, Value>,
}
