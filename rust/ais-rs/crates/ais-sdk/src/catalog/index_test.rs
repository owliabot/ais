use super::build_catalog_index;
use crate::documents::CatalogDocument;
use serde_json::{json, Map};

#[test]
fn build_catalog_index_provides_fast_lookups() {
    let catalog = CatalogDocument {
        schema: "ais-catalog/0.0.1".to_string(),
        created_at: None,
        hash: Some("hash-1".to_string()),
        documents: vec![],
        actions: vec![json!({
            "ref":"p1@0.0.1/a1",
            "protocol":"p1",
            "version":"0.0.1",
            "id":"a1",
            "execution_types":["evm_call"],
            "execution_chains":["eip155:1"],
        })],
        queries: vec![json!({
            "ref":"p1@0.0.1/q1",
            "protocol":"p1",
            "version":"0.0.1",
            "id":"q1",
            "execution_types":["evm_read"],
            "execution_chains":["eip155:1"],
        })],
        packs: vec![json!({
            "name":"pack",
            "version":"1.0.0",
            "includes":[]
        })],
        extensions: Map::new(),
    };

    let index = build_catalog_index(&catalog);
    assert!(index.actions_by_ref.contains_key("p1@0.0.1/a1"));
    assert!(index.queries_by_ref.contains_key("p1@0.0.1/q1"));
    assert!(index.packs_by_key.contains_key("pack@1.0.0"));
    assert_eq!(
        index
            .actions_by_protocol_version
            .get("p1@0.0.1")
            .map(|items| items.len()),
        Some(1)
    );
}
