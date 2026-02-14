use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogDocument {
    pub schema: String,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub hash: Option<String>,
    #[serde(default)]
    pub documents: Vec<Value>,
    #[serde(default)]
    pub actions: Vec<Value>,
    #[serde(default)]
    pub queries: Vec<Value>,
    #[serde(default)]
    pub packs: Vec<Value>,
    #[serde(default)]
    pub extensions: Map<String, Value>,
}
