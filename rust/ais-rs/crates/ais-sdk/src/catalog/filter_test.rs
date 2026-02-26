use super::{
    filter_by_engine_capabilities, filter_by_pack, get_executable_candidates, EngineCapabilities,
};
use crate::catalog::build_catalog_index;
use crate::documents::{CatalogDocument, PackDocument};
use serde_json::{json, Map, Value};

#[test]
fn filter_by_pack_enforces_includes_and_chain_scope() {
    let catalog = sample_catalog();
    let index = build_catalog_index(&catalog);
    let pack = pack_doc(
        "pack",
        "1.0.0",
        vec![json!({
            "protocol":"p1",
            "version":"0.0.1",
            "chain_scope":["eip155:1"]
        })],
    );

    let filtered = filter_by_pack(&index, &pack);
    assert!(filtered
        .actions
        .iter()
        .all(|card| value_str(card, "protocol") == "p1"));

    let action = filtered
        .actions
        .iter()
        .find(|card| value_str(card, "id") == "a_evm")
        .expect("a_evm exists");
    assert_eq!(
        value_array_str(action, "execution_chains"),
        vec!["eip155:1"]
    );
}

#[test]
fn filter_by_pack_matches_wildcard_execution_chain_with_concrete_scope() {
    let catalog = CatalogDocument {
        schema: "ais-catalog/0.0.1".to_string(),
        created_at: None,
        hash: Some("hash-wildcard".to_string()),
        documents: vec![],
        actions: vec![json!({
            "level":"index",
            "ref":"p1@0.0.1/a_wild",
            "protocol":"p1",
            "version":"0.0.1",
            "id":"a_wild",
            "risk_level":2,
            "execution_types":["evm_call"],
            "execution_chains":["eip155:*"],
        })],
        queries: vec![json!({
            "level":"index",
            "ref":"p1@0.0.1/q_wild",
            "protocol":"p1",
            "version":"0.0.1",
            "id":"q_wild",
            "execution_types":["evm_read"],
            "execution_chains":["eip155:*"],
        })],
        packs: vec![],
        extensions: Map::new(),
    };
    let index = build_catalog_index(&catalog);
    let pack = pack_doc(
        "pack",
        "1.0.0",
        vec![json!({
            "protocol":"p1",
            "version":"0.0.1",
            "chain_scope":["eip155:31338"]
        })],
    );

    let filtered = filter_by_pack(&index, &pack);
    assert_eq!(filtered.actions.len(), 1);
    assert_eq!(filtered.queries.len(), 1);
    assert_eq!(
        value_array_str(&filtered.actions[0], "execution_chains"),
        vec!["eip155:*"]
    );
}

#[test]
fn filter_by_engine_capabilities_filters_cards_and_derived_candidates() {
    let catalog = sample_catalog();
    let index = build_catalog_index(&catalog);
    let pack = pack_doc(
        "pack",
        "1.0.0",
        vec![json!({
            "protocol":"p1",
            "version":"0.0.1"
        })],
    );
    let pack_filtered = filter_by_pack(&index, &pack);

    let out = filter_by_engine_capabilities(
        &pack_filtered,
        &EngineCapabilities {
            capabilities: vec![],
            execution_types: vec!["evm_call".to_string()],
        },
    );

    assert!(out.actions.iter().all(|card| {
        value_array_str(card, "execution_types")
            .iter()
            .all(|typ| typ == "evm_call")
    }));
    assert!(out
        .execution_plugins
        .as_ref()
        .expect("plugins")
        .iter()
        .all(|plugin| value_str(plugin, "type") == "evm_call"));
}

#[test]
fn get_executable_candidates_is_stable_and_scoped() {
    let catalog = sample_catalog();
    let index = build_catalog_index(&catalog);
    let pack = pack_doc(
        "pack",
        "1.0.0",
        vec![json!({
            "protocol":"p1",
            "version":"0.0.1"
        })],
    );
    let scope = vec!["eip155:1".to_string()];

    let first = get_executable_candidates(
        &index,
        Some(&pack),
        Some(&EngineCapabilities {
            capabilities: vec![],
            execution_types: vec!["evm_call".to_string()],
        }),
        Some(&scope),
        Some("2026-02-13T01:00:00Z".to_string()),
    )
    .expect("must build candidates");

    let second = get_executable_candidates(
        &index,
        Some(&pack),
        Some(&EngineCapabilities {
            capabilities: vec![],
            execution_types: vec!["evm_call".to_string()],
        }),
        Some(&scope),
        Some("2026-02-13T02:00:00Z".to_string()),
    )
    .expect("must build candidates");

    assert_eq!(first.hash, second.hash);
    assert!(first.actions.iter().all(|card| {
        value_array_str(card, "execution_chains")
            .iter()
            .all(|chain| chain == "eip155:1")
    }));
}

fn sample_catalog() -> CatalogDocument {
    CatalogDocument {
        schema: "ais-catalog/0.0.1".to_string(),
        created_at: None,
        hash: Some("hash-1".to_string()),
        documents: vec![],
        actions: vec![
            json!({
                "level":"index",
                "ref":"p1@0.0.1/a_evm",
                "protocol":"p1",
                "version":"0.0.1",
                "id":"a_evm",
                "risk_level":3,
                "execution_types":["evm_call"],
                "execution_chains":["eip155:1","eip155:137"],
            }),
            json!({
                "level":"index",
                "ref":"p1@0.0.1/a_solana",
                "protocol":"p1",
                "version":"0.0.1",
                "id":"a_solana",
                "risk_level":2,
                "execution_types":["solana_instruction"],
                "execution_chains":["solana:mainnet"],
            }),
            json!({
                "level":"index",
                "ref":"p2@0.0.1/a2",
                "protocol":"p2",
                "version":"0.0.1",
                "id":"a2",
                "risk_level":1,
                "execution_types":["evm_call"],
                "execution_chains":["eip155:1"],
            }),
        ],
        queries: vec![json!({
            "level":"index",
            "ref":"p1@0.0.1/q1",
            "protocol":"p1",
            "version":"0.0.1",
            "id":"q1",
            "execution_types":["evm_read"],
            "execution_chains":["eip155:1"],
        })],
        packs: vec![json!({
            "name":"pack",
            "version":"1.0.0"
        })],
        extensions: Map::new(),
    }
}

fn pack_doc(name: &str, version: &str, includes: Vec<Value>) -> PackDocument {
    PackDocument {
        schema: "ais-pack/0.0.2".to_string(),
        name: Some(name.to_string()),
        version: Some(version.to_string()),
        description: None,
        meta: None,
        includes,
        policy: None,
        token_policy: None,
        providers: None,
        plugins: Some(json!({
            "execution": {
                "enabled": [
                    {"type":"evm_call","chains":["eip155:1"]},
                    {"type":"solana_instruction","chains":["solana:mainnet"]}
                ]
            }
        })),
        overrides: None,
        extensions: Map::new(),
    }
}

fn value_str<'a>(value: &'a Value, key: &str) -> &'a str {
    value
        .as_object()
        .and_then(|object| object.get(key))
        .and_then(Value::as_str)
        .unwrap_or("")
}

fn value_array_str(value: &Value, key: &str) -> Vec<String> {
    value
        .as_object()
        .and_then(|object| object.get(key))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}
