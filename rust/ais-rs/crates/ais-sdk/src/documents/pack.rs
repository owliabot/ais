use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackDocument {
    pub schema: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub meta: Option<Value>,
    pub includes: Vec<Value>,
    #[serde(default)]
    pub policy: Option<Value>,
    #[serde(default)]
    pub token_policy: Option<Value>,
    #[serde(default)]
    pub providers: Option<Value>,
    #[serde(default)]
    pub plugins: Option<Value>,
    #[serde(default)]
    pub overrides: Option<Value>,
    #[serde(default)]
    pub extensions: Map<String, Value>,
}
