use super::{Executor, ExecutorOutput, RouterExecuteError, RouterExecutor};
use serde_json::{json, Map, Value};
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Clone)]
struct MockExecutor {
    name: String,
    calls: Rc<RefCell<Vec<String>>>,
}

impl MockExecutor {
    fn new(name: &str, calls: Rc<RefCell<Vec<String>>>) -> Self {
        Self {
            name: name.to_string(),
            calls,
        }
    }
}

impl Executor for MockExecutor {
    fn execute(&self, node: &Value, _runtime: &mut Value) -> Result<ExecutorOutput, String> {
        let node_id = node
            .as_object()
            .and_then(|object| object.get("id"))
            .and_then(Value::as_str)
            .unwrap_or("-");
        self.calls
            .borrow_mut()
            .push(format!("{}:{node_id}", self.name));
        Ok(ExecutorOutput {
            result: json!({"executor": self.name, "node_id": node_id}),
            writes: Map::new(),
        })
    }
}

#[test]
fn router_routes_by_exact_chain_with_multiple_executors() {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let mut router = RouterExecutor::new();
    router.register(
        "evm-mainnet",
        "eip155:1",
        Box::new(MockExecutor::new("evm-mainnet", Rc::clone(&calls))),
    );
    router.register(
        "solana-mainnet",
        "solana:mainnet-beta",
        Box::new(MockExecutor::new("solana-mainnet", Rc::clone(&calls))),
    );

    let mut runtime = json!({});
    let node = json!({
        "id": "swap-1",
        "chain": "solana:mainnet-beta",
        "execution": {"type": "solana_instruction"}
    });
    let result = router.execute(&node, &mut runtime).expect("must route");

    assert_eq!(result.executor_name, "solana-mainnet");
    assert_eq!(result.chain, "solana:mainnet-beta");
    assert_eq!(calls.borrow().as_slice(), ["solana-mainnet:swap-1"]);
}

#[test]
fn router_chain_mismatch_is_rejected() {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let mut router = RouterExecutor::new();
    router.register(
        "evm-mainnet",
        "eip155:1",
        Box::new(MockExecutor::new("evm-mainnet", Rc::clone(&calls))),
    );

    let mut runtime = json!({});
    let node = json!({
        "id": "swap-2",
        "chain": "eip155:137",
        "execution": {"type": "evm_call"}
    });

    let error = router.execute(&node, &mut runtime).expect_err("must reject");
    assert_eq!(
        error,
        RouterExecuteError::ChainMismatch {
            node_id: "swap-2".to_string(),
            chain: "eip155:137".to_string(),
        }
    );
    assert!(calls.borrow().is_empty());
}

#[test]
fn router_ambiguous_chain_is_rejected() {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let mut router = RouterExecutor::new();
    router.register(
        "evm-a",
        "eip155:1",
        Box::new(MockExecutor::new("evm-a", Rc::clone(&calls))),
    );
    router.register(
        "evm-b",
        "eip155:1",
        Box::new(MockExecutor::new("evm-b", Rc::clone(&calls))),
    );

    let mut runtime = json!({});
    let node = json!({
        "id": "swap-3",
        "chain": "eip155:1",
        "execution": {"type": "evm_call"}
    });

    let error = router.execute(&node, &mut runtime).expect_err("must reject");
    match error {
        RouterExecuteError::AmbiguousRoute { node_id, chain, executors } => {
            assert_eq!(node_id, "swap-3");
            assert_eq!(chain, "eip155:1");
            assert!(executors.contains("evm-a"));
            assert!(executors.contains("evm-b"));
        }
        _ => panic!("expected ambiguous route"),
    }
}
