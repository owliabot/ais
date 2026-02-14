use crate::documents::CatalogDocument;
use serde_json::Value;
use std::collections::HashMap;

pub const CATALOG_INDEX_SCHEMA_0_0_1: &str = "ais-catalog-index/0.0.1";

#[derive(Debug, Clone)]
pub struct CatalogIndex {
    pub schema: String,
    pub catalog_schema: String,
    pub catalog_hash: String,
    pub actions: Vec<Value>,
    pub queries: Vec<Value>,
    pub packs: Vec<Value>,
    pub actions_by_ref: HashMap<String, Value>,
    pub queries_by_ref: HashMap<String, Value>,
    pub packs_by_key: HashMap<String, Value>,
    pub actions_by_protocol_version: HashMap<String, Vec<Value>>,
    pub queries_by_protocol_version: HashMap<String, Vec<Value>>,
    pub detect_providers: Option<Vec<Value>>,
    pub execution_plugins: Option<Vec<Value>>,
}

pub fn build_catalog_index(catalog: &CatalogDocument) -> CatalogIndex {
    build_index_from_parts(
        &catalog.schema,
        catalog.hash.as_deref().unwrap_or(""),
        catalog.actions.clone(),
        catalog.queries.clone(),
        catalog.packs.clone(),
    )
}

pub(crate) fn build_index_from_parts(
    catalog_schema: &str,
    catalog_hash: &str,
    actions: Vec<Value>,
    queries: Vec<Value>,
    packs: Vec<Value>,
) -> CatalogIndex {
    let mut actions_by_ref = HashMap::new();
    let mut queries_by_ref = HashMap::new();
    let mut packs_by_key = HashMap::new();
    let mut actions_by_protocol_version: HashMap<String, Vec<Value>> = HashMap::new();
    let mut queries_by_protocol_version: HashMap<String, Vec<Value>> = HashMap::new();

    for action in &actions {
        if let Some(reference) = value_str(action, "ref") {
            actions_by_ref.insert(reference.to_string(), action.clone());
        }
        if let (Some(protocol), Some(version)) = (value_str(action, "protocol"), value_str(action, "version")) {
            let key = format!("{protocol}@{version}");
            actions_by_protocol_version
                .entry(key)
                .or_default()
                .push(action.clone());
        }
    }

    for query in &queries {
        if let Some(reference) = value_str(query, "ref") {
            queries_by_ref.insert(reference.to_string(), query.clone());
        }
        if let (Some(protocol), Some(version)) = (value_str(query, "protocol"), value_str(query, "version")) {
            let key = format!("{protocol}@{version}");
            queries_by_protocol_version
                .entry(key)
                .or_default()
                .push(query.clone());
        }
    }

    for pack in &packs {
        if let (Some(name), Some(version)) = (value_str(pack, "name"), value_str(pack, "version")) {
            packs_by_key.insert(format!("{name}@{version}"), pack.clone());
        }
    }

    CatalogIndex {
        schema: CATALOG_INDEX_SCHEMA_0_0_1.to_string(),
        catalog_schema: catalog_schema.to_string(),
        catalog_hash: catalog_hash.to_string(),
        actions,
        queries,
        packs,
        actions_by_ref,
        queries_by_ref,
        packs_by_key,
        actions_by_protocol_version,
        queries_by_protocol_version,
        detect_providers: None,
        execution_plugins: None,
    }
}

fn value_str<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.as_object()?.get(key)?.as_str()
}

#[cfg(test)]
#[path = "index_test.rs"]
mod tests;
