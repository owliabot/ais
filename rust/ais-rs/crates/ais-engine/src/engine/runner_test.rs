use super::{run_plan_once, EngineRunnerOptions, EngineRunnerState, EngineRunStatus};
use crate::commands::{EngineCommand, EngineCommandEnvelope, EngineCommandType};
use crate::executor::{Executor, ExecutorOutput, RouterExecutor};
use crate::solver::DefaultSolver;
use ais_sdk::PlanDocument;
use serde_json::{json, Map, Value};
use std::cell::RefCell;
use std::rc::Rc;

struct MockExecutor;

impl Executor for MockExecutor {
    fn execute(&self, node: &Value, _runtime: &mut Value) -> Result<ExecutorOutput, String> {
        let node_id = node
            .as_object()
            .and_then(|object| object.get("id"))
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        Ok(ExecutorOutput {
            result: json!({"ok": true, "node_id": node_id}),
            writes: Map::new(),
        })
    }
}

struct CountingExecutor {
    calls: Rc<RefCell<usize>>,
}

impl Executor for CountingExecutor {
    fn execute(&self, node: &Value, runtime: &mut Value) -> Result<ExecutorOutput, String> {
        *self.calls.borrow_mut() += 1;
        Executor::execute(&MockExecutor, node, runtime)
    }
}

struct UntilExecutor {
    calls: Rc<RefCell<usize>>,
    succeed_after: usize,
}

impl Executor for UntilExecutor {
    fn execute(&self, _node: &Value, _runtime: &mut Value) -> Result<ExecutorOutput, String> {
        let mut calls = self.calls.borrow_mut();
        *calls += 1;
        let ready = *calls >= self.succeed_after;
        Ok(ExecutorOutput {
            result: json!({"ready": ready, "attempt": *calls}),
            writes: Map::new(),
        })
    }
}

struct CaptureNodeExecutor {
    last_node: Rc<RefCell<Option<Value>>>,
}

impl Executor for CaptureNodeExecutor {
    fn execute(&self, node: &Value, _runtime: &mut Value) -> Result<ExecutorOutput, String> {
        *self.last_node.borrow_mut() = Some(node.clone());
        Ok(ExecutorOutput {
            result: json!({"ok": true}),
            writes: Map::new(),
        })
    }
}

struct QueryOutputExecutor;

impl Executor for QueryOutputExecutor {
    fn execute(&self, _node: &Value, _runtime: &mut Value) -> Result<ExecutorOutput, String> {
        Ok(ExecutorOutput {
            result: json!({
                "execution_type": "evm_read",
                "outputs": {
                    "balance": "123"
                }
            }),
            writes: Map::new(),
        })
    }
}

fn sample_plan() -> PlanDocument {
    PlanDocument {
        schema: "ais-plan/0.0.3".to_string(),
        meta: Some(json!({"name": "runner"})),
        nodes: vec![json!({
            "id": "swap-1",
            "kind": "execution",
            "chain": "eip155:1",
            "bindings": {
                "params": {
                    "spend_amount": {"ref": "inputs.amount"}
                }
            },
            "execution": {
                "type": "evm_call",
                "to": {"lit": "0x0000000000000000000000000000000000000001"},
                "abi": {"type": "function", "name": "swapExactTokensForTokens", "inputs": [], "outputs": []},
                "method": "swapExactTokensForTokens",
                "args": {
                    "amountIn": {"ref": "params.spend_amount"}
                }
            },
            "writes": [{"path": "nodes.swap-1.outputs", "mode": "set"}]
        })],
        extensions: Map::new(),
    }
}

fn assert_plan(assert: Value, assert_message: Option<&str>, strategy: Option<&str>) -> PlanDocument {
    let mut node = json!({
        "id": "assert-1",
        "kind": "execution",
        "chain": "eip155:1",
        "execution": {
            "type": "evm_read",
            "to": {"lit": "0x0000000000000000000000000000000000000001"},
            "abi": {"type": "function", "name": "balanceOf", "inputs": [], "outputs": []},
            "method": "balanceOf",
            "args": {}
        },
        "assert": assert,
        "writes": [{"path":"nodes.assert-1.outputs","mode":"set"}]
    });
    if let Some(message) = assert_message {
        node.as_object_mut()
            .expect("object")
            .insert("assert_message".to_string(), Value::String(message.to_string()));
    }
    if let Some(strategy) = strategy {
        node.as_object_mut().expect("object").insert(
            "extensions".to_string(),
            json!({
                "assert": {
                    "on_fail": strategy
                }
            }),
        );
    }
    PlanDocument {
        schema: "ais-plan/0.0.3".to_string(),
        meta: Some(json!({"name": "assert-plan"})),
        nodes: vec![node],
        extensions: Map::new(),
    }
}

fn preflight_simulate_plan() -> PlanDocument {
    PlanDocument {
        schema: "ais-plan/0.0.3".to_string(),
        meta: Some(json!({
            "name": "simulate-plan",
            "preflight": { "simulate": { "sim1": true } }
        })),
        nodes: vec![json!({
            "id": "sim1",
            "kind": "execution",
            "chain": "eip155:1",
            "execution": {
                "type": "evm_call",
                "to": {"lit": "0x0000000000000000000000000000000000000001"},
                "abi": {"type": "function", "name": "swapExactTokensForTokens", "inputs": [], "outputs": []},
                "method": "swapExactTokensForTokens",
                "args": {}
            },
            "assert": {"lit": true},
            "writes": [{"path":"nodes.sim1.outputs","mode":"set"}]
        })],
        extensions: Map::new(),
    }
}

fn condition_plan(condition: Value) -> PlanDocument {
    PlanDocument {
        schema: "ais-plan/0.0.3".to_string(),
        meta: Some(json!({"name": "condition-plan"})),
        nodes: vec![json!({
            "id": "cond-1",
            "kind": "execution",
            "chain": "eip155:1",
            "condition": condition,
            "execution": {
                "type": "evm_read",
                "to": {"lit": "0x0000000000000000000000000000000000000001"},
                "abi": {"type": "function", "name": "balanceOf", "inputs": [], "outputs": []},
                "method": "balanceOf",
                "args": {}
            },
            "writes": [{"path":"nodes.cond-1.outputs","mode":"set"}]
        })],
        extensions: Map::new(),
    }
}

fn until_plan(until: Value, retry: Option<Value>) -> PlanDocument {
    let mut node = json!({
        "id": "until1",
        "kind": "execution",
        "chain": "eip155:1",
        "until": until,
        "execution": {
            "type": "evm_read",
            "to": {"lit": "0x0000000000000000000000000000000000000001"},
            "abi": {"type": "function", "name": "balanceOf", "inputs": [], "outputs": []},
            "method": "balanceOf",
            "args": {}
        },
        "writes": [{"path":"nodes.until1.outputs","mode":"set"}]
    });
    if let Some(retry) = retry {
        node.as_object_mut()
            .expect("node object")
            .insert("retry".to_string(), retry);
    }
    PlanDocument {
        schema: "ais-plan/0.0.3".to_string(),
        meta: Some(json!({"name": "until-plan"})),
        nodes: vec![node],
        extensions: Map::new(),
    }
}

fn query_plan() -> PlanDocument {
    PlanDocument {
        schema: "ais-plan/0.0.3".to_string(),
        meta: Some(json!({"name": "query-plan"})),
        nodes: vec![json!({
            "id": "q1",
            "type": "query_ref",
            "kind": "execution",
            "chain": "eip155:1",
            "execution": {
                "type": "evm_read",
                "to": {"lit": "0x0000000000000000000000000000000000000001"},
                "abi": {"type": "function", "name": "balanceOf", "inputs": [], "outputs": [{"name":"balance","type":"uint256"}]},
                "args": {}
            },
            "writes": [{"path":"nodes.q1.outputs","mode":"set"}]
        })],
        extensions: Map::new(),
    }
}

fn query_plan_with_source_only() -> PlanDocument {
    PlanDocument {
        schema: "ais-plan/0.0.3".to_string(),
        meta: Some(json!({"name": "query-plan-source-only"})),
        nodes: vec![json!({
            "id": "q2",
            "kind": "execution",
            "chain": "eip155:1",
            "source": {
                "workflow": {"name":"wf","version":"0.0.3"},
                "node_id": "q2",
                "protocol": "erc20@0.0.2",
                "query": "balance-of"
            },
            "execution": {
                "type": "evm_read",
                "to": {"lit": "0x0000000000000000000000000000000000000001"},
                "abi": {"type": "function", "name": "balanceOf", "inputs": [], "outputs": [{"name":"balance","type":"uint256"}]},
                "args": {}
            },
            "writes": [{"path":"nodes.q2.outputs","mode":"set"}]
        })],
        extensions: Map::new(),
    }
}

fn apply_patch_command() -> EngineCommandEnvelope {
    EngineCommandEnvelope::new(EngineCommand {
        id: "cmd-patch".to_string(),
        command_type: EngineCommandType::ApplyPatches,
        data: Map::from_iter([(
            "patches".to_string(),
            json!([
                {"op":"set","path":"inputs.amount","value":"100"}
            ]),
        )]),
    })
}

fn approve_command() -> EngineCommandEnvelope {
    EngineCommandEnvelope::new(EngineCommand {
        id: "cmd-approve".to_string(),
        command_type: EngineCommandType::UserConfirm,
        data: Map::from_iter([
            ("node_id".to_string(), json!("swap-1")),
            ("decision".to_string(), json!("approve")),
        ]),
    })
}

#[test]
fn run_plan_no_progress_emits_engine_paused() {
    let plan = sample_plan();
    let mut state = EngineRunnerState::default();
    let mut router = RouterExecutor::new();
    router.register("evm", "eip155:1", Box::new(MockExecutor));

    let result = run_plan_once(
        "run-1",
        &plan,
        &mut state,
        &router,
        &DefaultSolver,
        &[],
        &EngineRunnerOptions::default(),
    );

    assert_eq!(result.status, EngineRunStatus::Paused);
    assert!(result
        .events
        .iter()
        .any(|record| record.event.event_type == crate::events::EngineEventType::EnginePaused));
}

#[test]
fn query_ref_default_write_projects_result_outputs() {
    let plan = query_plan();
    let mut state = EngineRunnerState::default();
    let mut router = RouterExecutor::new();
    router.register("evm", "eip155:1", Box::new(QueryOutputExecutor));

    let result = run_plan_once(
        "run-query",
        &plan,
        &mut state,
        &router,
        &DefaultSolver,
        &[],
        &EngineRunnerOptions::default(),
    );

    assert_eq!(result.status, EngineRunStatus::Completed);
    assert_eq!(
        state.runtime.pointer("/nodes/q1/outputs/balance"),
        Some(&json!("123"))
    );
    assert!(state
        .runtime
        .pointer("/nodes/q1/outputs/outputs")
        .is_none());
}

#[test]
fn query_source_default_write_projects_result_outputs() {
    let plan = query_plan_with_source_only();
    let mut state = EngineRunnerState::default();
    let mut router = RouterExecutor::new();
    router.register("evm", "eip155:1", Box::new(QueryOutputExecutor));

    let result = run_plan_once(
        "run-query-source",
        &plan,
        &mut state,
        &router,
        &DefaultSolver,
        &[],
        &EngineRunnerOptions::default(),
    );

    assert_eq!(result.status, EngineRunStatus::Completed);
    assert_eq!(
        state.runtime.pointer("/nodes/q2/outputs/balance"),
        Some(&json!("123"))
    );
    assert!(state
        .runtime
        .pointer("/nodes/q2/outputs/outputs")
        .is_none());
}

#[test]
fn run_plan_minimal_loop_with_apply_patches_and_user_confirm() {
    let plan = sample_plan();
    let mut state = EngineRunnerState::default();
    let mut router = RouterExecutor::new();
    router.register("evm", "eip155:1", Box::new(MockExecutor));

    let first = run_plan_once(
        "run-2",
        &plan,
        &mut state,
        &router,
        &DefaultSolver,
        &[apply_patch_command()],
        &EngineRunnerOptions::default(),
    );
    assert_eq!(first.status, EngineRunStatus::Paused);
    assert!(first
        .events
        .iter()
        .any(|record| record.event.event_type == crate::events::EngineEventType::NeedUserConfirm));

    let second = run_plan_once(
        "run-2",
        &plan,
        &mut state,
        &router,
        &DefaultSolver,
        &[approve_command()],
        &EngineRunnerOptions::default(),
    );
    assert_eq!(second.status, EngineRunStatus::Completed);
    assert_eq!(state.completed_node_ids, vec!["swap-1".to_string()]);
    assert_eq!(
        state
            .runtime
            .get("nodes")
            .and_then(|value| value.get("swap-1"))
            .and_then(|value| value.get("outputs"))
            .and_then(|value| value.get("ok")),
        Some(&json!(true))
    );
}

#[test]
fn execution_is_materialized_before_executor_dispatch() {
    let plan = sample_plan();
    let mut state = EngineRunnerState::default();
    state.runtime = json!({"inputs": {"amount": "42"}});
    let captured = Rc::new(RefCell::new(None::<Value>));

    let mut router = RouterExecutor::new();
    router.register(
        "evm",
        "eip155:1",
        Box::new(CaptureNodeExecutor {
            last_node: captured.clone(),
        }),
    );

    let result = run_plan_once(
        "run-materialize",
        &plan,
        &mut state,
        &router,
        &DefaultSolver,
        &[approve_command()],
        &EngineRunnerOptions::default(),
    );

    assert_eq!(result.status, EngineRunStatus::Completed);
    let node = captured.borrow().clone().expect("executor should receive node");
    assert_eq!(
        node.pointer("/execution/args/amountIn"),
        Some(&Value::String("42".to_string()))
    );
}

#[test]
fn assert_failure_pauses_with_error_event() {
    let plan = assert_plan(json!({"lit": false}), Some("assert failed for test"), None);
    let mut state = EngineRunnerState::default();
    let mut router = RouterExecutor::new();
    router.register("evm", "eip155:1", Box::new(MockExecutor));

    let result = run_plan_once(
        "run-assert-pause",
        &plan,
        &mut state,
        &router,
        &DefaultSolver,
        &[],
        &EngineRunnerOptions::default(),
    );
    assert_eq!(result.status, EngineRunStatus::Paused);
    assert_eq!(state.paused_reason.as_deref(), Some("assert_failed:assert-1"));
    assert!(result.events.iter().any(|record| {
        record.event.event_type == crate::events::EngineEventType::Error
            && record.event.data.get("reason") == Some(&json!("assert_failed"))
    }));
    assert!(result
        .events
        .iter()
        .any(|record| record.event.event_type == crate::events::EngineEventType::EnginePaused));
}

#[test]
fn assert_failure_can_stop_run() {
    let plan = assert_plan(json!({"lit": false}), Some("stop assert"), Some("stop"));
    let mut state = EngineRunnerState::default();
    let mut router = RouterExecutor::new();
    router.register("evm", "eip155:1", Box::new(MockExecutor));

    let result = run_plan_once(
        "run-assert-stop",
        &plan,
        &mut state,
        &router,
        &DefaultSolver,
        &[],
        &EngineRunnerOptions::default(),
    );
    assert_eq!(result.status, EngineRunStatus::Stopped);
    assert_eq!(state.paused_reason.as_deref(), Some("assert_failed:assert-1"));
    assert!(result
        .events
        .iter()
        .any(|record| record.event.event_type == crate::events::EngineEventType::NodePaused));
    assert!(!result
        .events
        .iter()
        .any(|record| record.event.event_type == crate::events::EngineEventType::EnginePaused));
}

#[test]
fn preflight_simulate_skips_executor_and_completes_node() {
    let plan = preflight_simulate_plan();
    let mut state = EngineRunnerState::default();
    let mut router = RouterExecutor::new();
    let calls = Rc::new(RefCell::new(0usize));
    router.register(
        "evm",
        "eip155:1",
        Box::new(CountingExecutor {
            calls: calls.clone(),
        }),
    );

    let result = run_plan_once(
        "run-preflight-simulate",
        &plan,
        &mut state,
        &router,
        &DefaultSolver,
        &[],
        &EngineRunnerOptions::default(),
    );
    assert_eq!(result.status, EngineRunStatus::Completed);
    assert_eq!(*calls.borrow(), 0);
    assert!(state.completed_node_ids.iter().any(|id| id == "sim1"));
    assert_eq!(
        state
            .runtime
            .get("nodes")
            .and_then(|value| value.get("sim1"))
            .and_then(|value| value.get("outputs"))
            .and_then(|value| value.get("simulated")),
        Some(&json!(true))
    );
    assert!(result.events.iter().any(|record| {
        record.event.event_type == crate::events::EngineEventType::Skipped
            && record.event.data.get("reason") == Some(&json!("preflight_simulate"))
    }));
}

#[test]
fn condition_false_skips_executor_and_completes_node() {
    let plan = condition_plan(json!({"lit": false}));
    let mut state = EngineRunnerState::default();
    let mut router = RouterExecutor::new();
    let calls = Rc::new(RefCell::new(0usize));
    router.register(
        "evm",
        "eip155:1",
        Box::new(CountingExecutor {
            calls: calls.clone(),
        }),
    );

    let result = run_plan_once(
        "run-condition-false",
        &plan,
        &mut state,
        &router,
        &DefaultSolver,
        &[],
        &EngineRunnerOptions::default(),
    );

    assert_eq!(result.status, EngineRunStatus::Completed);
    assert_eq!(*calls.borrow(), 0);
    assert!(state.completed_node_ids.iter().any(|id| id == "cond-1"));
    assert!(result.events.iter().any(|record| {
        record.event.event_type == crate::events::EngineEventType::Skipped
            && record.event.data.get("reason") == Some(&json!("condition_false"))
    }));
}

#[test]
fn condition_true_executes_node() {
    let plan = condition_plan(json!({"lit": true}));
    let mut state = EngineRunnerState::default();
    let mut router = RouterExecutor::new();
    let calls = Rc::new(RefCell::new(0usize));
    router.register(
        "evm",
        "eip155:1",
        Box::new(CountingExecutor {
            calls: calls.clone(),
        }),
    );

    let result = run_plan_once(
        "run-condition-true",
        &plan,
        &mut state,
        &router,
        &DefaultSolver,
        &[],
        &EngineRunnerOptions::default(),
    );

    assert_eq!(result.status, EngineRunStatus::Completed);
    assert_eq!(*calls.borrow(), 1);
    assert_eq!(
        state
            .runtime
            .get("nodes")
            .and_then(|value| value.get("cond-1"))
            .and_then(|value| value.get("outputs"))
            .and_then(|value| value.get("ok")),
        Some(&json!(true))
    );
}

#[test]
fn invalid_condition_pauses_with_error_event() {
    let plan = condition_plan(json!({"cel": "size("}));
    let mut state = EngineRunnerState::default();
    let mut router = RouterExecutor::new();
    router.register("evm", "eip155:1", Box::new(MockExecutor));

    let result = run_plan_once(
        "run-condition-invalid",
        &plan,
        &mut state,
        &router,
        &DefaultSolver,
        &[],
        &EngineRunnerOptions::default(),
    );

    assert_eq!(result.status, EngineRunStatus::Paused);
    assert_eq!(state.paused_reason.as_deref(), Some("condition_failed:cond-1"));
    assert!(result.events.iter().any(|record| {
        record.event.event_type == crate::events::EngineEventType::Error
            && record.event.data.get("reason") == Some(&json!("condition_failed"))
    }));
    assert!(result.events.iter().any(|record| {
        record.event.event_type == crate::events::EngineEventType::EnginePaused
            && record.event.data.get("reason") == Some(&json!("condition_failed"))
    }));
}

#[test]
fn until_false_without_retry_pauses() {
    let plan = until_plan(json!({"lit": false}), None);
    let mut state = EngineRunnerState::default();
    let mut router = RouterExecutor::new();
    router.register("evm", "eip155:1", Box::new(MockExecutor));

    let result = run_plan_once(
        "run-until-no-retry",
        &plan,
        &mut state,
        &router,
        &DefaultSolver,
        &[],
        &EngineRunnerOptions::default(),
    );

    assert_eq!(result.status, EngineRunStatus::Paused);
    assert_eq!(state.paused_reason.as_deref(), Some("until_not_met:until1"));
    assert!(result.events.iter().any(|record| {
        record.event.event_type == crate::events::EngineEventType::Error
            && record.event.data.get("reason") == Some(&json!("until_failed"))
    }));
}

#[test]
fn until_retry_then_complete() {
    let plan = until_plan(
        json!({"cel": "nodes.until1.outputs.ready == true"}),
        Some(json!({"interval_ms": 1000, "max_attempts": 3, "backoff": "fixed"})),
    );
    let mut state = EngineRunnerState::default();
    let mut router = RouterExecutor::new();
    let calls = Rc::new(RefCell::new(0usize));
    router.register(
        "evm",
        "eip155:1",
        Box::new(UntilExecutor {
            calls: calls.clone(),
            succeed_after: 2,
        }),
    );

    let first = run_plan_once(
        "run-until-retry",
        &plan,
        &mut state,
        &router,
        &DefaultSolver,
        &[],
        &EngineRunnerOptions::default(),
    );
    assert_eq!(first.status, EngineRunStatus::Paused);
    assert!(state.paused_reason.is_none());
    assert_eq!(*calls.borrow(), 1);
    assert!(first.events.iter().any(|record| {
        record.event.event_type == crate::events::EngineEventType::NodeWaiting
            && record.event.data.get("reason") == Some(&json!("until_retry"))
    }));

    let second = run_plan_once(
        "run-until-retry",
        &plan,
        &mut state,
        &router,
        &DefaultSolver,
        &[],
        &EngineRunnerOptions::default(),
    );
    assert_eq!(second.status, EngineRunStatus::Completed);
    assert_eq!(*calls.borrow(), 2);
    assert!(state.pending_retries.is_empty());
}

#[test]
fn until_retry_exhausted_pauses() {
    let plan = until_plan(
        json!({"cel": "nodes.until1.outputs.ready == true"}),
        Some(json!({"interval_ms": 1000, "max_attempts": 1, "backoff": "fixed"})),
    );
    let mut state = EngineRunnerState::default();
    let mut router = RouterExecutor::new();
    let calls = Rc::new(RefCell::new(0usize));
    router.register(
        "evm",
        "eip155:1",
        Box::new(UntilExecutor {
            calls: calls.clone(),
            succeed_after: 10,
        }),
    );

    let first = run_plan_once(
        "run-until-exhaust",
        &plan,
        &mut state,
        &router,
        &DefaultSolver,
        &[],
        &EngineRunnerOptions::default(),
    );
    assert_eq!(first.status, EngineRunStatus::Paused);
    assert!(state.paused_reason.is_none());
    assert_eq!(*calls.borrow(), 1);

    let second = run_plan_once(
        "run-until-exhaust",
        &plan,
        &mut state,
        &router,
        &DefaultSolver,
        &[],
        &EngineRunnerOptions::default(),
    );
    assert_eq!(second.status, EngineRunStatus::Paused);
    assert_eq!(state.paused_reason.as_deref(), Some("retry_exhausted:until1"));
    assert_eq!(*calls.borrow(), 2);
    assert!(second.events.iter().any(|record| {
        record.event.event_type == crate::events::EngineEventType::Error
            && record.event.data.get("reason") == Some(&json!("retry_exhausted"))
    }));
}

#[test]
fn until_retry_timeout_pauses_when_budget_exceeded_immediately() {
    let plan = until_plan(
        json!({"cel": "nodes.until1.outputs.ready == true"}),
        Some(json!({"interval_ms": 1000, "max_attempts": 5, "backoff": "fixed"})),
    );
    let mut state = EngineRunnerState::default();
    let mut router = RouterExecutor::new();
    let calls = Rc::new(RefCell::new(0usize));
    router.register(
        "evm",
        "eip155:1",
        Box::new(UntilExecutor {
            calls: calls.clone(),
            succeed_after: 10,
        }),
    );
    if let Some(node) = plan.nodes.first().and_then(Value::as_object) {
        assert!(node.get("timeout_ms").is_none());
    }
    let mut plan = plan;
    if let Some(node) = plan.nodes.first_mut().and_then(Value::as_object_mut) {
        node.insert("timeout_ms".to_string(), json!(500));
    }

    let first = run_plan_once(
        "run-until-timeout-1",
        &plan,
        &mut state,
        &router,
        &DefaultSolver,
        &[],
        &EngineRunnerOptions::default(),
    );
    assert_eq!(first.status, EngineRunStatus::Paused);
    assert_eq!(state.paused_reason.as_deref(), Some("retry_timeout:until1"));
    assert_eq!(*calls.borrow(), 1);
    assert!(first.events.iter().any(|record| {
        record.event.event_type == crate::events::EngineEventType::Error
            && record.event.data.get("reason") == Some(&json!("retry_timeout"))
    }));
}

#[test]
fn until_retry_timeout_pauses_after_multiple_waits() {
    let mut plan = until_plan(
        json!({"cel": "nodes.until1.outputs.ready == true"}),
        Some(json!({"interval_ms": 1000, "max_attempts": 5, "backoff": "fixed"})),
    );
    if let Some(node) = plan.nodes.first_mut().and_then(Value::as_object_mut) {
        node.insert("timeout_ms".to_string(), json!(1500));
    }
    let mut state = EngineRunnerState::default();
    let mut router = RouterExecutor::new();
    let calls = Rc::new(RefCell::new(0usize));
    router.register(
        "evm",
        "eip155:1",
        Box::new(UntilExecutor {
            calls: calls.clone(),
            succeed_after: 10,
        }),
    );

    let first = run_plan_once(
        "run-until-timeout-2",
        &plan,
        &mut state,
        &router,
        &DefaultSolver,
        &[],
        &EngineRunnerOptions::default(),
    );
    assert_eq!(first.status, EngineRunStatus::Paused);
    assert!(state.paused_reason.is_none());
    assert_eq!(*calls.borrow(), 1);

    let second = run_plan_once(
        "run-until-timeout-2",
        &plan,
        &mut state,
        &router,
        &DefaultSolver,
        &[],
        &EngineRunnerOptions::default(),
    );
    assert_eq!(second.status, EngineRunStatus::Paused);
    assert_eq!(state.paused_reason.as_deref(), Some("retry_timeout:until1"));
    assert_eq!(*calls.borrow(), 2);
    assert!(second.events.iter().any(|record| {
        record.event.event_type == crate::events::EngineEventType::Error
            && record.event.data.get("reason") == Some(&json!("retry_timeout"))
    }));
}
