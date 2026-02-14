use super::{build_catalog, CatalogBuildInput, CatalogBuildOptions};
use crate::documents::{PackDocument, ProtocolDocument, WorkflowDocument};
use serde_json::{json, Map, Value};

#[test]
fn build_catalog_produces_stable_sorting() {
    let protocol_b = protocol_doc(
        "b-protocol",
        "0.0.2",
        vec![("bbb_action", "evm_call"), ("aaa_action", "evm_read")],
        vec![("z_query", "evm_read"), ("a_query", "evm_read")],
    );
    let protocol_a = protocol_doc(
        "a-protocol",
        "0.0.2",
        vec![("swap", "evm_call")],
        vec![("quote", "evm_read")],
    );
    let pack = pack_doc(
        "safe-pack",
        "0.0.1",
        vec![
            json!({"protocol":"b-protocol","version":"0.0.2"}),
            json!({"protocol":"a-protocol","version":"0.0.2","chain_scope":["eip155:1"]}),
        ],
    );

    let catalog = build_catalog(
        CatalogBuildInput {
            protocols: &[protocol_b, protocol_a],
            packs: &[pack],
            workflows: &[],
        },
        &CatalogBuildOptions::default(),
    )
    .expect("catalog must build");

    let action_refs = collect_field_values(&catalog.actions, "ref");
    assert_eq!(
        action_refs,
        vec![
            "a-protocol@0.0.2/swap".to_string(),
            "b-protocol@0.0.2/aaa_action".to_string(),
            "b-protocol@0.0.2/bbb_action".to_string()
        ]
    );

    let query_refs = collect_field_values(&catalog.queries, "ref");
    assert_eq!(
        query_refs,
        vec![
            "a-protocol@0.0.2/quote".to_string(),
            "b-protocol@0.0.2/a_query".to_string(),
            "b-protocol@0.0.2/z_query".to_string()
        ]
    );
}

#[test]
fn catalog_hash_ignores_created_at() {
    let protocol = protocol_doc(
        "uniswap-v3",
        "0.0.2",
        vec![("swap_exact_in", "evm_call")],
        vec![("quote_exact_in", "evm_read")],
    );
    let workflow = workflow_doc("wf", "0.0.1");

    let first = build_catalog(
        CatalogBuildInput {
            protocols: std::slice::from_ref(&protocol),
            packs: &[],
            workflows: std::slice::from_ref(&workflow),
        },
        &CatalogBuildOptions {
            created_at: Some("2026-02-13T01:00:00Z".to_string()),
        },
    )
    .expect("catalog must build");

    let second = build_catalog(
        CatalogBuildInput {
            protocols: std::slice::from_ref(&protocol),
            packs: &[],
            workflows: std::slice::from_ref(&workflow),
        },
        &CatalogBuildOptions {
            created_at: Some("2026-02-13T02:00:00Z".to_string()),
        },
    )
    .expect("catalog must build");

    assert_eq!(first.hash, second.hash);
}

#[test]
fn action_card_contains_minimal_required_fields() {
    let protocol = protocol_doc(
        "p",
        "0.0.2",
        vec![("swap", "evm_call")],
        vec![("quote", "evm_read")],
    );
    let catalog = build_catalog(
        CatalogBuildInput {
            protocols: &[protocol],
            packs: &[],
            workflows: &[],
        },
        &CatalogBuildOptions::default(),
    )
    .expect("catalog must build");

    let action = catalog.actions.first().expect("one action");
    assert_eq!(
        action.get("ref").and_then(Value::as_str),
        Some("p@0.0.2/swap")
    );
    assert_eq!(action.get("protocol").and_then(Value::as_str), Some("p"));
    assert_eq!(action.get("version").and_then(Value::as_str), Some("0.0.2"));
    assert_eq!(action.get("id").and_then(Value::as_str), Some("swap"));
}

fn protocol_doc(
    protocol: &str,
    version: &str,
    actions: Vec<(&str, &str)>,
    queries: Vec<(&str, &str)>,
) -> ProtocolDocument {
    let mut action_map = Map::new();
    for (id, typ) in actions {
        action_map.insert(
            id.to_string(),
            json!({
                "execution": {
                    "eip155:1": { "type": typ }
                }
            }),
        );
    }

    let mut query_map = Map::new();
    for (id, typ) in queries {
        query_map.insert(
            id.to_string(),
            json!({
                "execution": {
                    "eip155:1": { "type": typ }
                }
            }),
        );
    }

    ProtocolDocument {
        schema: "ais/0.0.2".to_string(),
        meta: json!({
            "protocol": protocol,
            "version": version
        }),
        deployments: vec![json!({"chain":"eip155:1","contracts":{}})],
        actions: action_map,
        queries: query_map,
        risks: Vec::new(),
        supported_assets: Vec::new(),
        capabilities_required: Vec::new(),
        tests: Vec::new(),
        extensions: Map::new(),
    }
}

fn workflow_doc(name: &str, version: &str) -> WorkflowDocument {
    WorkflowDocument {
        schema: "ais-flow/0.0.3".to_string(),
        meta: json!({ "name": name, "version": version }),
        default_chain: None,
        imports: None,
        requires_pack: None,
        inputs: Map::new(),
        nodes: vec![],
        policy: None,
        preflight: None,
        outputs: Map::new(),
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
        plugins: None,
        overrides: None,
        extensions: Map::new(),
    }
}

fn collect_field_values(values: &[Value], field: &str) -> Vec<String> {
    values
        .iter()
        .filter_map(Value::as_object)
        .filter_map(|obj| obj.get(field))
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}
