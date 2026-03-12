use super::{
    resolve_node_bindings, resolve_query_bindings, ResolvedNodeBindings, ResolvedQueryBindings,
};
use crate::resolver::ValueRefEvalOptions;
use serde_json::{json, Map};

#[test]
fn resolve_node_bindings_collects_node_and_runtime_roots() {
    let node = json!({
        "extensions": {
            "protocol": {
                "contracts": {
                    "router": "0x1111111111111111111111111111111111111111"
                }
            },
            "policy": {
                "required_fields": ["spender"],
                "param_roles": {
                    "spender": "spender_address"
                }
            }
        }
    });
    let runtime = json!({
        "query": {
            "quote": {
                "amount_out": "123"
            }
        },
        "calculated": {
            "amount_atomic": "1000000"
        }
    });
    let params = Map::from_iter([("amount".to_string(), json!("42"))]);

    let bindings = resolve_node_bindings(&node, Some(&runtime), Some(&params), None);
    assert_eq!(
        bindings,
        ResolvedNodeBindings {
            params: Some(params),
            contracts: Some(Map::from_iter([(
                "router".to_string(),
                json!("0x1111111111111111111111111111111111111111")
            )])),
            calculated: Some(json!({"amount_atomic":"1000000"})),
            query: Some(json!({"quote":{"amount_out":"123"}})),
            policy: Some(json!({
                "required_fields": ["spender"],
                "param_roles": {
                    "spender": "spender_address"
                }
            })),
        }
    );
}

#[test]
fn resolved_node_bindings_to_eval_options_overrides_all_shared_roots() {
    let bindings = ResolvedNodeBindings {
        params: Some(Map::from_iter([("amount".to_string(), json!("42"))])),
        contracts: Some(Map::from_iter([("router".to_string(), json!("0x1"))])),
        calculated: Some(json!({"amount_atomic":"100"})),
        query: Some(json!({"quote":{"amount_out":"99"}})),
        policy: Some(json!({"required_fields":["spender"]})),
    };

    let options = bindings.to_eval_options(&ValueRefEvalOptions::default());
    assert_eq!(
        options.root_overrides.get("params"),
        Some(&json!({"amount":"42"}))
    );
    assert_eq!(
        options.root_overrides.get("contracts"),
        Some(&json!({"router":"0x1"}))
    );
    assert_eq!(
        options.root_overrides.get("calculated"),
        Some(&json!({"amount_atomic":"100"}))
    );
    assert_eq!(
        options.root_overrides.get("query"),
        Some(&json!({"quote":{"amount_out":"99"}}))
    );
    assert_eq!(
        options.root_overrides.get("policy"),
        Some(&json!({"required_fields":["spender"]}))
    );
}

#[test]
fn resolve_query_bindings_projects_required_query_outputs_from_runtime_nodes() {
    let node = json!({
        "extensions": {
            "operation": {
                "requires_queries": ["allowance-token"],
                "query_bindings": {
                    "allowance-token": {
                        "node_id": "q_allowance",
                        "query_ref": "aave-v3@0.0.2/allowance-token"
                    }
                }
            }
        }
    });
    let runtime = json!({
        "nodes": {
            "q_allowance": {
                "outputs": {
                    "allowance_atomic": "1000000"
                }
            }
        }
    });

    let query = resolve_query_bindings(&node, Some(&runtime));
    assert_eq!(
        query,
        ResolvedQueryBindings {
            query: Some(json!({
                "allowance-token": {
                    "allowance_atomic": "1000000"
                }
            })),
            missing_refs: Vec::new(),
        }
    );
}

#[test]
fn resolve_query_bindings_reports_missing_required_query_results() {
    let node = json!({
        "extensions": {
            "operation": {
                "requires_queries": ["allowance-token"]
            }
        }
    });

    let query = resolve_query_bindings(&node, Some(&json!({})));
    assert_eq!(
        query,
        ResolvedQueryBindings {
            query: None,
            missing_refs: vec!["query.allowance-token".to_string()],
        }
    );
}
