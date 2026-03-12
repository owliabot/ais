use super::{get_node_readiness, get_node_readiness_async, NodeRunState};
use crate::resolver::{ResolverContext, ValueRefEvalOptions};
use futures::executor::block_on;
use serde_json::{json, Value};

fn make_evm_call_node(execution: Value) -> Value {
    json!({
        "id": "node-1",
        "kind": "execution",
        "chain": "eip155:1",
        "execution": execution
    })
}

#[test]
fn readiness_missing_ref_is_blocked() {
    let context = ResolverContext::with_runtime(json!({
        "inputs": {
            "amount": "100"
        }
    }));
    let node = make_evm_call_node(json!({
        "type": "evm_call",
        "to": {"ref": "contracts.router"},
        "abi": {"type": "function", "name": "swap", "inputs": [], "outputs": []},
        "method": "swap",
        "args": {"amount": {"ref": "inputs.amount"}}
    }));

    let readiness = get_node_readiness(&node, &context, &ValueRefEvalOptions::default());
    assert_eq!(readiness.state, NodeRunState::Blocked);
    assert_eq!(readiness.missing_refs, vec!["contracts.router".to_string()]);
}

#[test]
fn readiness_uses_protocol_contracts_from_node_extensions() {
    let context = ResolverContext::with_runtime(json!({
        "inputs": {
            "amount": "100"
        }
    }));
    let node = json!({
        "id": "node-1",
        "kind": "execution",
        "chain": "eip155:1",
        "extensions": {
            "protocol": {
                "contracts": {
                    "router": "0x1111111111111111111111111111111111111111"
                }
            }
        },
        "execution": {
            "type": "evm_call",
            "to": {"ref": "contracts.router"},
            "abi": {"type": "function", "name": "swap", "inputs": [], "outputs": []},
            "method": "swap",
            "args": {"amount": {"ref": "inputs.amount"}}
        }
    });

    let readiness = get_node_readiness(&node, &context, &ValueRefEvalOptions::default());
    assert_eq!(readiness.state, NodeRunState::Ready);
    assert!(readiness.missing_refs.is_empty());
    assert!(readiness.errors.is_empty());
}

#[test]
fn readiness_condition_false_is_skipped() {
    let context = ResolverContext::new();
    let node = json!({
        "id": "node-1",
        "kind": "execution",
        "chain": "eip155:1",
        "condition": {"lit": false},
        "execution": {
            "type": "evm_read",
            "to": {"lit": "0x0000000000000000000000000000000000000001"},
            "abi": {"type": "function", "name": "balanceOf", "inputs": [], "outputs": []},
            "method": "balanceOf",
            "args": {}
        }
    });

    let readiness = get_node_readiness(&node, &context, &ValueRefEvalOptions::default());
    assert_eq!(readiness.state, NodeRunState::Skipped);
    assert!(readiness.missing_refs.is_empty());
    assert!(readiness.errors.is_empty());
    assert_eq!(readiness.resolved_params, None);
}

#[test]
fn readiness_unknown_object_arg_is_treated_as_literal() {
    let context = ResolverContext::new();
    let node = make_evm_call_node(json!({
        "type": "evm_call",
        "to": {"lit": "0x0000000000000000000000000000000000000001"},
        "abi": {"type": "function", "name": "swap", "inputs": [], "outputs": []},
        "method": "swap",
        "args": {
            "route": {"unknown": "x"}
        }
    }));

    let readiness = get_node_readiness(&node, &context, &ValueRefEvalOptions::default());
    assert_eq!(readiness.state, NodeRunState::Ready);
    assert!(readiness.missing_refs.is_empty());
    assert!(readiness.errors.is_empty());
}

#[test]
fn readiness_async_matches_sync() {
    let context = ResolverContext::new();
    let node = make_evm_call_node(json!({
        "type": "evm_call",
        "to": {"lit": "0x0000000000000000000000000000000000000001"},
        "abi": {"type": "function", "name": "swap", "inputs": [], "outputs": []},
        "method": "swap",
        "args": {
            "route": {"lit": "static"}
        }
    }));

    let readiness = block_on(get_node_readiness_async(
        &node,
        &context,
        &ValueRefEvalOptions::default(),
    ));

    assert_eq!(readiness.state, NodeRunState::Ready);
    assert!(readiness.missing_refs.is_empty());
    assert!(readiness.errors.is_empty());
}

#[test]
fn readiness_resolves_bindings_params_for_execution_refs() {
    let context = ResolverContext::with_runtime(json!({
        "inputs": {
            "amount": "100"
        }
    }));
    let node = json!({
        "id": "node-1",
        "kind": "execution",
        "chain": "eip155:1",
        "bindings": {
            "params": {
                "amount": {"ref": "inputs.amount"}
            }
        },
        "execution": {
            "type": "evm_call",
            "to": {"lit": "0x0000000000000000000000000000000000000001"},
            "abi": {"type": "function", "name": "swap", "inputs": [], "outputs": []},
            "method": "swap",
            "args": {
                "amount": {"ref": "params.amount"}
            }
        }
    });

    let readiness = get_node_readiness(&node, &context, &ValueRefEvalOptions::default());
    assert_eq!(readiness.state, NodeRunState::Ready);
    assert_eq!(
        readiness.resolved_params,
        Some(serde_json::Map::from_iter([(
            "amount".to_string(),
            json!("100")
        )]))
    );
}

#[test]
fn readiness_uses_policy_query_and_calculated_roots_from_shared_bindings() {
    let context = ResolverContext::with_runtime(json!({
        "query": {
            "quote": {
                "amount_out": "123"
            }
        },
        "calculated": {
            "amount_atomic": "1000000"
        }
    }));
    let node = json!({
        "id": "node-1",
        "kind": "execution",
        "chain": "eip155:1",
        "extensions": {
            "policy": {
                "required_fields": ["spender"]
            }
        },
        "execution": {
            "type": "evm_call",
            "to": {"lit": "0x0000000000000000000000000000000000000001"},
            "abi": {"type": "function", "name": "swap", "inputs": [], "outputs": []},
            "method": "swap",
            "args": {
                "required": {"ref": "policy.required_fields[0]"},
                "quoted": {"ref": "query.quote.amount_out"},
                "amount": {"ref": "calculated.amount_atomic"}
            }
        }
    });

    let readiness = get_node_readiness(&node, &context, &ValueRefEvalOptions::default());
    assert_eq!(readiness.state, NodeRunState::Ready);
    assert!(readiness.missing_refs.is_empty());
    assert!(readiness.errors.is_empty());
}

#[test]
fn readiness_condition_uses_node_scoped_params_and_calculated_roots() {
    let context = ResolverContext::with_runtime(json!({
        "inputs": {
            "amount": "42"
        }
    }));
    let node = json!({
        "id": "node-1",
        "kind": "execution",
        "chain": "eip155:1",
        "bindings": {
            "params": {
                "amount": {"ref": "inputs.amount"}
            }
        },
        "calculated_overrides": {
            "amount_atomic": {
                "expr": {"ref": "params.amount"}
            }
        },
        "condition": {"cel": "params.amount == '42' && calculated.amount_atomic == '42'"},
        "execution": {
            "type": "evm_call",
            "to": {"lit": "0x0000000000000000000000000000000000000001"},
            "abi": {"type": "function", "name": "swap", "inputs": [], "outputs": []},
            "method": "swap",
            "args": {
                "amount": {"ref": "calculated.amount_atomic"}
            }
        }
    });

    let readiness = get_node_readiness(&node, &context, &ValueRefEvalOptions::default());
    assert_eq!(readiness.state, NodeRunState::Ready);
    assert!(readiness.missing_refs.is_empty());
    assert!(readiness.errors.is_empty());
    assert_eq!(
        readiness.resolved_params,
        Some(serde_json::Map::from_iter([(
            "amount".to_string(),
            json!("42")
        )]))
    );
}

#[test]
fn readiness_projects_query_root_from_required_query_node_outputs() {
    let context = ResolverContext::with_runtime(json!({
        "nodes": {
            "q_allowance": {
                "outputs": {
                    "allowance_atomic": "1000000"
                }
            }
        }
    }));
    let node = json!({
        "id": "node-1",
        "kind": "execution",
        "chain": "eip155:1",
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
        },
        "execution": {
            "type": "evm_call",
            "to": {"lit": "0x0000000000000000000000000000000000000001"},
            "abi": {"type": "function", "name": "supply", "inputs": [], "outputs": []},
            "method": "supply",
            "args": {
                "allowance": {"ref": "query.allowance-token.allowance_atomic"}
            }
        }
    });

    let readiness = get_node_readiness(&node, &context, &ValueRefEvalOptions::default());
    assert_eq!(readiness.state, NodeRunState::Ready);
    assert!(readiness.missing_refs.is_empty());
    assert!(readiness.errors.is_empty());
}
