use super::{get_node_readiness, get_node_readiness_async, NodeRunState};
use crate::resolver::{DetectResolver, DetectSpec, ResolverContext, ValueRefEvalError, ValueRefEvalOptions};
use futures::executor::block_on;
use futures::FutureExt;
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
    assert!(!readiness.needs_detect);
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
    assert!(!readiness.needs_detect);
    assert!(readiness.errors.is_empty());
    assert_eq!(readiness.resolved_params, None);
}

#[test]
fn readiness_detect_in_sync_path_is_blocked() {
    let context = ResolverContext::new();
    let node = make_evm_call_node(json!({
        "type": "evm_call",
        "to": {"lit": "0x0000000000000000000000000000000000000001"},
        "abi": {"type": "function", "name": "swap", "inputs": [], "outputs": []},
        "method": "swap",
        "args": {
            "route": {"detect": {"kind": "choose_one"}}
        }
    }));

    let readiness = get_node_readiness(&node, &context, &ValueRefEvalOptions::default());
    assert_eq!(readiness.state, NodeRunState::Blocked);
    assert!(readiness.missing_refs.is_empty());
    assert!(readiness.needs_detect);
}

struct StaticDetectResolver;

impl DetectResolver for StaticDetectResolver {
    fn resolve<'a>(
        &'a self,
        detect: &'a DetectSpec,
        _context: &'a ResolverContext,
        _options: &'a ValueRefEvalOptions,
    ) -> futures::future::LocalBoxFuture<'a, Result<Value, ValueRefEvalError>> {
        async move { Ok(json!({"selected": detect.kind})) }.boxed_local()
    }
}

#[test]
fn readiness_async_detect_resolver_unblocks_node() {
    let context = ResolverContext::new();
    let node = make_evm_call_node(json!({
        "type": "evm_call",
        "to": {"lit": "0x0000000000000000000000000000000000000001"},
        "abi": {"type": "function", "name": "swap", "inputs": [], "outputs": []},
        "method": "swap",
        "args": {
            "route": {"detect": {"kind": "choose_one"}}
        }
    }));
    let resolver = StaticDetectResolver;

    let readiness = block_on(get_node_readiness_async(
        &node,
        &context,
        &ValueRefEvalOptions::default(),
        Some(&resolver),
    ));

    assert_eq!(readiness.state, NodeRunState::Ready);
    assert!(!readiness.needs_detect);
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
        Some(serde_json::Map::from_iter([("amount".to_string(), json!("100"))]))
    );
}
