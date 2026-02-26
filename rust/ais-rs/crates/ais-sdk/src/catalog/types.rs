use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CatalogCardLevel {
    #[default]
    Index,
    Detail,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogParam {
    pub name: String,
    #[serde(rename = "type")]
    pub param_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogReturn {
    pub name: String,
    #[serde(rename = "type")]
    pub return_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionCard {
    pub level: CatalogCardLevel,
    pub ref_id: String,
    pub protocol: String,
    pub version: String,
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub risk_level: u8,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub risk_tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Vec<CatalogParam>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub returns: Option<Vec<CatalogReturn>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires_queries: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities_required: Vec<String>,
    pub execution_types: Vec<String>,
    pub execution_chains: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryCard {
    pub level: CatalogCardLevel,
    pub ref_id: String,
    pub protocol: String,
    pub version: String,
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Vec<CatalogParam>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub returns: Option<Vec<CatalogReturn>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities_required: Vec<String>,
    pub execution_types: Vec<String>,
    pub execution_chains: Vec<String>,
}

impl ActionCard {
    pub fn into_json_value(self) -> serde_json::Result<Value> {
        let mut value = serde_json::to_value(self)?;
        if let Some(object) = value.as_object_mut() {
            if let Some(ref_id) = object.remove("ref_id") {
                object.insert("ref".to_string(), ref_id);
            }
        }
        Ok(value)
    }
}

impl QueryCard {
    pub fn into_json_value(self) -> serde_json::Result<Value> {
        let mut value = serde_json::to_value(self)?;
        if let Some(object) = value.as_object_mut() {
            if let Some(ref_id) = object.remove("ref_id") {
                object.insert("ref".to_string(), ref_id);
            }
        }
        Ok(value)
    }
}
