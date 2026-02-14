use crate::commands::{
    apply_command_with_dedupe, CommandDeduper, DuplicateCommandMode, EngineCommandEnvelope,
    EngineCommandType,
};
use crate::engine::apply_patches_from_command;
use crate::events::{EngineEvent, EngineEventRecord, EngineEventStream, EngineEventType};
use crate::executor::{RouterExecuteError, RouterExecutor};
use crate::policy::{
    enforce_policy_gate, enrich_need_user_confirm_output, extract_policy_gate_input,
    PolicyEnforcementOptions, PolicyGateOutput,
};
use crate::solver::{build_solver_event, Solver, SolverDecision};
use ais_sdk::{
    evaluate_value_ref_with_options, get_node_readiness, PlanDocument, ResolverContext, ValueRef,
    ValueRefEvalOptions,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineRunStatus {
    Completed,
    Paused,
    Stopped,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EngineRunnerState {
    pub runtime: Value,
    #[serde(default)]
    pub completed_node_ids: Vec<String>,
    #[serde(default)]
    pub approved_node_ids: Vec<String>,
    #[serde(default)]
    pub seen_command_ids: Vec<String>,
    #[serde(default)]
    pub paused_reason: Option<String>,
    #[serde(default)]
    pub pending_retries: Map<String, Value>,
    #[serde(default)]
    pub next_seq: u64,
}

impl Default for EngineRunnerState {
    fn default() -> Self {
        Self {
            runtime: Value::Object(Map::new()),
            completed_node_ids: Vec::new(),
            approved_node_ids: Vec::new(),
            seen_command_ids: Vec::new(),
            paused_reason: None,
            pending_retries: Map::new(),
            next_seq: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EngineRunnerOptions {
    pub duplicate_command_mode: DuplicateCommandMode,
    pub policy: PolicyEnforcementOptions,
    pub solver_context: crate::solver::SolverContext,
}

impl Default for EngineRunnerOptions {
    fn default() -> Self {
        Self {
            duplicate_command_mode: DuplicateCommandMode::Reject,
            policy: PolicyEnforcementOptions::default(),
            solver_context: crate::solver::SolverContext::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EngineRunResult {
    pub status: EngineRunStatus,
    pub events: Vec<EngineEventRecord>,
}

pub fn run_plan_once(
    run_id: &str,
    plan: &PlanDocument,
    state: &mut EngineRunnerState,
    router: &RouterExecutor,
    solver: &dyn Solver,
    commands: &[EngineCommandEnvelope],
    options: &EngineRunnerOptions,
) -> EngineRunResult {
    ensure_runtime_object(&mut state.runtime);

    let mut events = Vec::<EngineEventRecord>::new();
    let mut stream = EngineEventStream::with_start_seq(run_id.to_string(), state.next_seq);
    let mut deduper = CommandDeduper::with_seen_ids(
        options.duplicate_command_mode,
        state.seen_command_ids.clone(),
    );

    for command in commands {
        let command_event = apply_command_with_dedupe(
            &mut deduper,
            &mut stream,
            "1970-01-01T00:00:00Z",
            command,
        );
        events.push(command_event.event_record.clone());
        if !command_event.accepted || command_event.duplicate {
            continue;
        }
        match command.command.command_type {
            EngineCommandType::ApplyPatches => {
                if let Ok(execution) = apply_patches_from_command(
                    &mut state.runtime,
                    command,
                    &ais_core::build_runtime_patch_guard_policy(),
                    &mut stream,
                    "1970-01-01T00:00:00Z",
                ) {
                    events.extend(execution.events);
                }
            }
            EngineCommandType::UserConfirm => {
                let node_id = command
                    .command
                    .data
                    .get("node_id")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let decision = command
                    .command
                    .data
                    .get("decision")
                    .and_then(Value::as_str)
                    .unwrap_or("approve");
                if let Some(node_id) = node_id {
                    if decision == "approve" {
                        insert_unique_sorted(&mut state.approved_node_ids, node_id);
                    } else {
                        state.paused_reason = Some("user_confirm_denied".to_string());
                    }
                }
            }
            EngineCommandType::Cancel => {
                state.paused_reason = Some("cancelled_by_command".to_string());
                events.push(stream.next_record(
                    "1970-01-01T00:00:00Z",
                    paused_event("cancelled_by_command"),
                ));
                persist_state_from_runtime(state, &mut deduper, &stream);
                return EngineRunResult {
                    status: EngineRunStatus::Paused,
                    events,
                };
            }
            EngineCommandType::SelectProvider => {}
        }
    }

    let mut progress = false;
    let mut completed_set = state
        .completed_node_ids
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut approved_set = state.approved_node_ids.iter().cloned().collect::<BTreeSet<_>>();

    for node in &plan.nodes {
        let Some(node_obj) = node.as_object() else {
            continue;
        };
        let Some(node_id) = node_obj.get("id").and_then(Value::as_str).map(str::to_string) else {
            continue;
        };
        if completed_set.contains(&node_id) {
            continue;
        }
        if !deps_satisfied(node_obj, &completed_set) {
            continue;
        }

        match evaluate_node_condition(node_obj, &state.runtime) {
            NodeConditionOutcome::Pass => {}
            NodeConditionOutcome::Skip => {
                events.push(stream.next_record(
                    "1970-01-01T00:00:00Z",
                    condition_skipped_event(&node_id),
                ));
                clear_retry_state(state, &node_id);
                completed_set.insert(node_id.clone());
                approved_set.remove(&node_id);
                progress = true;
                continue;
            }
            NodeConditionOutcome::Fail { message } => {
                events.push(stream.next_record(
                    "1970-01-01T00:00:00Z",
                    condition_failed_event(&node_id, &message),
                ));
                state.paused_reason = Some(format!("condition_failed:{node_id}"));
                events.push(stream.next_record(
                    "1970-01-01T00:00:00Z",
                    paused_event("condition_failed"),
                ));
                persist_state_from_runtime(state, &mut deduper, &stream);
                return EngineRunResult {
                    status: EngineRunStatus::Paused,
                    events,
                };
            }
        }

        let context = ResolverContext::with_runtime(state.runtime.clone());
        let readiness = get_node_readiness(node, &context, &ValueRefEvalOptions::default());
        if readiness.state != ais_sdk::NodeRunState::Ready {
            events.push(stream.next_record(
                "1970-01-01T00:00:00Z",
                node_blocked_event(&node_id, &readiness),
            ));
            let decision = solver.solve(node, &readiness, &options.solver_context);
            if let Some(event) = build_solver_event(Some(&node_id), &decision) {
                events.push(stream.next_record("1970-01-01T00:00:00Z", event));
            }
            match decision {
                SolverDecision::ApplyPatches { patches, .. } => {
                    let apply_result = ais_core::apply_runtime_patches(
                        &mut state.runtime,
                        &patches,
                        &ais_core::build_runtime_patch_guard_policy(),
                    );
                    if apply_result.audit.applied_count > 0 {
                        progress = true;
                        continue;
                    }
                }
                SolverDecision::NeedUserConfirm { .. } => {
                    state.paused_reason = Some(format!("need_user_confirm:{node_id}"));
                    events.push(stream.next_record(
                        "1970-01-01T00:00:00Z",
                        paused_event("need_user_confirm"),
                    ));
                    persist_state_from_runtime(state, &mut deduper, &stream);
                    return EngineRunResult {
                        status: EngineRunStatus::Paused,
                        events,
                    };
                }
                SolverDecision::SelectProvider { .. } | SolverDecision::Noop => {}
            }
            continue;
        }

        events.push(stream.next_record(
            "1970-01-01T00:00:00Z",
            node_ready_event(&node_id),
        ));

        let simulate_mode = should_simulate_node(plan, node_obj, &node_id);
        if simulate_mode {
            let simulated_result = json!({
                "simulated": true,
                "node_id": node_id,
            });
            apply_node_writes(node_obj, &simulated_result, &mut state.runtime);
            events.push(stream.next_record(
                "1970-01-01T00:00:00Z",
                preflight_simulated_event(&node_id),
            ));
            match evaluate_node_assert(plan, node_obj, &node_id, &state.runtime) {
                NodeAssertOutcome::Pass => {
                    match handle_node_until(
                        node_obj,
                        &node_id,
                        state,
                        &mut stream,
                        &mut events,
                        &mut deduper,
                    ) {
                        NodeUntilHandle::Complete => {
                            completed_set.insert(node_id.clone());
                            approved_set.remove(&node_id);
                        }
                        NodeUntilHandle::RetryScheduled => {}
                        NodeUntilHandle::Paused => {
                            return EngineRunResult {
                                status: EngineRunStatus::Paused,
                                events,
                            };
                        }
                    }
                    progress = true;
                    continue;
                }
                NodeAssertOutcome::Fail { message, strategy } => {
                        events.push(stream.next_record(
                            "1970-01-01T00:00:00Z",
                            assert_failed_event(
                                &node_id,
                                &message,
                                "preflight_simulate",
                                node_obj.get("assert"),
                            ),
                        ));
                    state.paused_reason = Some(format!("assert_failed:{node_id}"));
                    if strategy == AssertFailStrategy::Stop {
                        events.push(stream.next_record(
                            "1970-01-01T00:00:00Z",
                            node_paused_event(&node_id, "assert_failed_stop"),
                        ));
                        persist_state_from_runtime(state, &mut deduper, &stream);
                        return EngineRunResult {
                            status: EngineRunStatus::Stopped,
                            events,
                        };
                    }
                    events.push(stream.next_record(
                        "1970-01-01T00:00:00Z",
                        paused_event("assert_failed"),
                    ));
                    persist_state_from_runtime(state, &mut deduper, &stream);
                    return EngineRunResult {
                        status: EngineRunStatus::Paused,
                        events,
                    };
                }
            }
        }

        let gate_input = extract_policy_gate_input(
            node,
            readiness.resolved_params.as_ref(),
            None,
            None,
            Vec::new(),
        );
        let gate_output = enforce_policy_gate(&gate_input, &options.policy);
        let gate_output = enrich_need_user_confirm_output(&gate_input, &gate_output).unwrap_or(gate_output);

        match gate_output {
            PolicyGateOutput::Ok { .. } => {}
            PolicyGateOutput::NeedUserConfirm { reason, details } => {
                if !approved_set.contains(&node_id) {
                    events.push(stream.next_record(
                        "1970-01-01T00:00:00Z",
                        need_user_confirm_event(&node_id, &reason, &details),
                    ));
                    state.paused_reason = Some(format!("need_user_confirm:{node_id}"));
                    events.push(stream.next_record(
                        "1970-01-01T00:00:00Z",
                        paused_event("need_user_confirm"),
                    ));
                    persist_state_from_runtime(state, &mut deduper, &stream);
                    return EngineRunResult {
                        status: EngineRunStatus::Paused,
                        events,
                    };
                }
            }
            PolicyGateOutput::HardBlock { reason, details } => {
                events.push(stream.next_record(
                    "1970-01-01T00:00:00Z",
                    hard_block_event(&node_id, &reason, &details),
                ));
                state.paused_reason = Some(format!("hard_block:{node_id}"));
                events.push(stream.next_record(
                    "1970-01-01T00:00:00Z",
                    paused_event("hard_block"),
                ));
                persist_state_from_runtime(state, &mut deduper, &stream);
                return EngineRunResult {
                    status: EngineRunStatus::Paused,
                    events,
                };
            }
        }

        let executable_node = match materialize_node_execution(
            node,
            &state.runtime,
            readiness.resolved_params.as_ref(),
        ) {
            Ok(node) => node,
            Err(reason) => {
                let error = RouterExecuteError::ExecutorFailed {
                    executor: "engine:materialize".to_string(),
                    node_id: node_id.clone(),
                    reason,
                };
                events.push(stream.next_record(
                    "1970-01-01T00:00:00Z",
                    executor_error_event(&node_id, &error),
                ));
                state.paused_reason = Some(format!("executor_error:{node_id}"));
                events.push(stream.next_record(
                    "1970-01-01T00:00:00Z",
                    paused_event("executor_error"),
                ));
                persist_state_from_runtime(state, &mut deduper, &stream);
                return EngineRunResult {
                    status: EngineRunStatus::Paused,
                    events,
                };
            }
        };

        match router.execute(&executable_node, &mut state.runtime) {
            Ok(result) => {
                apply_node_writes(node_obj, &result.output.result, &mut state.runtime);
                match evaluate_node_assert(plan, node_obj, &node_id, &state.runtime) {
                    NodeAssertOutcome::Pass => {}
                    NodeAssertOutcome::Fail { message, strategy } => {
                        events.push(stream.next_record(
                            "1970-01-01T00:00:00Z",
                            assert_failed_event(&node_id, &message, "execute", node_obj.get("assert")),
                        ));
                        state.paused_reason = Some(format!("assert_failed:{node_id}"));
                        if strategy == AssertFailStrategy::Stop {
                            events.push(stream.next_record(
                                "1970-01-01T00:00:00Z",
                                node_paused_event(&node_id, "assert_failed_stop"),
                            ));
                            persist_state_from_runtime(state, &mut deduper, &stream);
                            return EngineRunResult {
                                status: EngineRunStatus::Stopped,
                                events,
                            };
                        }
                        events.push(stream.next_record(
                            "1970-01-01T00:00:00Z",
                            paused_event("assert_failed"),
                        ));
                        persist_state_from_runtime(state, &mut deduper, &stream);
                        return EngineRunResult {
                            status: EngineRunStatus::Paused,
                            events,
                        };
                    }
                }
                match handle_node_until(
                    node_obj,
                    &node_id,
                    state,
                    &mut stream,
                    &mut events,
                    &mut deduper,
                ) {
                    NodeUntilHandle::Complete => {
                        completed_set.insert(node_id.clone());
                        approved_set.remove(&node_id);
                    }
                    NodeUntilHandle::RetryScheduled => {}
                    NodeUntilHandle::Paused => {
                        return EngineRunResult {
                            status: EngineRunStatus::Paused,
                            events,
                        };
                    }
                }
                progress = true;
            }
            Err(error) => {
                events.push(stream.next_record(
                    "1970-01-01T00:00:00Z",
                    executor_error_event(&node_id, &error),
                ));
                state.paused_reason = Some(format!("executor_error:{node_id}"));
                events.push(stream.next_record(
                    "1970-01-01T00:00:00Z",
                    paused_event("executor_error"),
                ));
                persist_state_from_runtime(state, &mut deduper, &stream);
                return EngineRunResult {
                    status: EngineRunStatus::Paused,
                    events,
                };
            }
        }
    }

    state.completed_node_ids = completed_set.into_iter().collect();
    state.approved_node_ids = approved_set.into_iter().collect();

    if state.completed_node_ids.len() == plan.nodes.len() {
        state.paused_reason = None;
        persist_state_from_runtime(state, &mut deduper, &stream);
        return EngineRunResult {
            status: EngineRunStatus::Completed,
            events,
        };
    }

    if !progress {
        state.paused_reason = Some("no_progress".to_string());
        events.push(stream.next_record(
            "1970-01-01T00:00:00Z",
            paused_event("no_progress"),
        ));
        persist_state_from_runtime(state, &mut deduper, &stream);
        return EngineRunResult {
            status: EngineRunStatus::Paused,
            events,
        };
    }

    state.paused_reason = None;
    persist_state_from_runtime(state, &mut deduper, &stream);
    EngineRunResult {
        status: EngineRunStatus::Paused,
        events,
    }
}

fn materialize_node_execution(
    node: &Value,
    runtime: &Value,
    resolved_params: Option<&Map<String, Value>>,
) -> Result<Value, String> {
    let mut node_obj = node
        .as_object()
        .cloned()
        .ok_or_else(|| "node must be object".to_string())?;
    let Some(execution) = node_obj.get("execution") else {
        return Ok(Value::Object(node_obj));
    };

    let context = ResolverContext::with_runtime(runtime.clone());
    let mut options = ValueRefEvalOptions::default();
    if let Some(params) = resolved_params {
        options
            .root_overrides
            .insert("params".to_string(), Value::Object(params.clone()));
    }

    let resolved_execution = materialize_value_refs(execution, &context, &options)?;
    node_obj.insert("execution".to_string(), resolved_execution);
    Ok(Value::Object(node_obj))
}

fn materialize_value_refs(
    value: &Value,
    context: &ResolverContext,
    options: &ValueRefEvalOptions,
) -> Result<Value, String> {
    if let Ok(value_ref) = serde_json::from_value::<ValueRef>(value.clone()) {
        let resolved = evaluate_value_ref_with_options(&value_ref, context, options)
            .map_err(|error| format!("materialize execution ValueRef failed: {error}"))?;
        return materialize_value_refs(&resolved, context, options);
    }

    match value {
        Value::Array(items) => {
            let mut out = Vec::<Value>::with_capacity(items.len());
            for item in items {
                out.push(materialize_value_refs(item, context, options)?);
            }
            Ok(Value::Array(out))
        }
        Value::Object(object) => {
            let mut out = Map::<String, Value>::new();
            for (key, child) in object {
                out.insert(key.clone(), materialize_value_refs(child, context, options)?);
            }
            Ok(Value::Object(out))
        }
        _ => Ok(value.clone()),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AssertFailStrategy {
    Pause,
    Stop,
}

enum NodeAssertOutcome {
    Pass,
    Fail {
        message: String,
        strategy: AssertFailStrategy,
    },
}

enum NodeConditionOutcome {
    Pass,
    Skip,
    Fail { message: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum NodeUntilOutcome {
    Pass,
    Retry,
    Fail { message: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RetryConfig {
    interval_ms: u64,
    max_attempts: Option<u64>,
    backoff: String,
}

enum NodeUntilHandle {
    Complete,
    RetryScheduled,
    Paused,
}

fn should_simulate_node(plan: &PlanDocument, node_obj: &Map<String, Value>, node_id: &str) -> bool {
    if node_obj.get("simulate").and_then(Value::as_bool) == Some(true) {
        return true;
    }
    if node_obj
        .get("preflight")
        .and_then(|value| extract_simulate_bool(value, node_id))
        == Some(true)
    {
        return true;
    }
    if node_obj
        .get("extensions")
        .and_then(|value| value.as_object())
        .and_then(|extensions| extensions.get("preflight"))
        .and_then(|value| extract_simulate_bool(value, node_id))
        == Some(true)
    {
        return true;
    }
    if plan
        .meta
        .as_ref()
        .and_then(Value::as_object)
        .and_then(|meta| meta.get("preflight"))
        .and_then(|value| extract_simulate_bool(value, node_id))
        == Some(true)
    {
        return true;
    }
    plan.extensions
        .get("preflight")
        .and_then(|value| extract_simulate_bool(value, node_id))
        == Some(true)
}

fn extract_simulate_bool(value: &Value, node_id: &str) -> Option<bool> {
    if let Some(boolean) = value.as_bool() {
        return Some(boolean);
    }
    let object = value.as_object()?;
    if let Some(simulate) = object.get("simulate") {
        if let Some(boolean) = simulate.as_bool() {
            return Some(boolean);
        }
        let simulate_map = simulate.as_object()?;
        if let Some(boolean) = simulate_map.get(node_id).and_then(Value::as_bool) {
            return Some(boolean);
        }
        if let Some(boolean) = simulate_map.get("*").and_then(Value::as_bool) {
            return Some(boolean);
        }
    }
    None
}

fn evaluate_node_condition(node_obj: &Map<String, Value>, runtime: &Value) -> NodeConditionOutcome {
    let Some(condition_raw) = node_obj.get("condition") else {
        return NodeConditionOutcome::Pass;
    };
    let condition_value_ref = match serde_json::from_value::<ValueRef>(condition_raw.clone()) {
        Ok(value_ref) => value_ref,
        Err(error) => {
            return NodeConditionOutcome::Fail {
                message: format!("condition is invalid: {error}"),
            };
        }
    };
    let context = ResolverContext::with_runtime(runtime.clone());
    let evaluated = match evaluate_value_ref_with_options(
        &condition_value_ref,
        &context,
        &ValueRefEvalOptions::default(),
    ) {
        Ok(value) => value,
        Err(error) => {
            return NodeConditionOutcome::Fail {
                message: format!("condition evaluation failed: {error}"),
            };
        }
    };
    match evaluated {
        Value::Bool(true) => NodeConditionOutcome::Pass,
        Value::Bool(false) => NodeConditionOutcome::Skip,
        other => NodeConditionOutcome::Fail {
            message: format!(
                "condition must evaluate to boolean, got {}",
                json_type_name(&other)
            ),
        },
    }
}

fn evaluate_node_until(node_obj: &Map<String, Value>, runtime: &Value) -> NodeUntilOutcome {
    let Some(until_raw) = node_obj.get("until") else {
        return NodeUntilOutcome::Pass;
    };
    let until_value_ref = match serde_json::from_value::<ValueRef>(until_raw.clone()) {
        Ok(value_ref) => value_ref,
        Err(error) => {
            return NodeUntilOutcome::Fail {
                message: format!("until is invalid: {error}"),
            };
        }
    };
    let context = ResolverContext::with_runtime(runtime.clone());
    let evaluated = match evaluate_value_ref_with_options(
        &until_value_ref,
        &context,
        &ValueRefEvalOptions::default(),
    ) {
        Ok(value) => value,
        Err(error) => {
            return NodeUntilOutcome::Fail {
                message: format!("until evaluation failed: {error}"),
            };
        }
    };
    match evaluated {
        Value::Bool(true) => NodeUntilOutcome::Pass,
        Value::Bool(false) => NodeUntilOutcome::Retry,
        other => NodeUntilOutcome::Fail {
            message: format!("until must evaluate to boolean, got {}", json_type_name(&other)),
        },
    }
}

fn parse_retry_config(node_obj: &Map<String, Value>) -> Result<Option<RetryConfig>, String> {
    let Some(retry_raw) = node_obj.get("retry") else {
        return Ok(None);
    };
    let retry_object = retry_raw
        .as_object()
        .ok_or_else(|| "retry must be an object".to_string())?;
    let interval_ms = retry_object
        .get("interval_ms")
        .and_then(Value::as_u64)
        .filter(|value| *value > 0)
        .ok_or_else(|| "retry.interval_ms must be a positive integer".to_string())?;
    let max_attempts = match retry_object.get("max_attempts") {
        Some(value) => Some(
            value
                .as_u64()
                .filter(|item| *item > 0)
                .ok_or_else(|| "retry.max_attempts must be a positive integer".to_string())?,
        ),
        None => None,
    };
    let backoff = retry_object
        .get("backoff")
        .and_then(Value::as_str)
        .unwrap_or("fixed")
        .to_string();
    if backoff != "fixed" {
        return Err(format!("retry.backoff `{backoff}` is not supported (expected `fixed`)"));
    }
    Ok(Some(RetryConfig {
        interval_ms,
        max_attempts,
        backoff,
    }))
}

fn parse_timeout_ms(node_obj: &Map<String, Value>) -> Result<Option<u64>, String> {
    match node_obj.get("timeout_ms") {
        Some(value) => value
            .as_u64()
            .filter(|item| *item > 0)
            .map(Some)
            .ok_or_else(|| "timeout_ms must be a positive integer".to_string()),
        None => Ok(None),
    }
}

fn handle_node_until(
    node_obj: &Map<String, Value>,
    node_id: &str,
    state: &mut EngineRunnerState,
    stream: &mut EngineEventStream,
    events: &mut Vec<EngineEventRecord>,
    deduper: &mut CommandDeduper,
) -> NodeUntilHandle {
    match evaluate_node_until(node_obj, &state.runtime) {
        NodeUntilOutcome::Pass => {
            clear_retry_state(state, node_id);
            NodeUntilHandle::Complete
        }
        NodeUntilOutcome::Retry => {
            let retry_config = match parse_retry_config(node_obj) {
                Ok(value) => value,
                Err(message) => {
                    events.push(stream.next_record(
                        "1970-01-01T00:00:00Z",
                        until_failed_event(node_id, &message),
                    ));
                    state.paused_reason = Some(format!("until_failed:{node_id}"));
                    events.push(stream.next_record(
                        "1970-01-01T00:00:00Z",
                        paused_event("until_failed"),
                    ));
                    persist_state_from_runtime(state, deduper, stream);
                    return NodeUntilHandle::Paused;
                }
            };
            let Some(retry_config) = retry_config else {
                events.push(stream.next_record(
                    "1970-01-01T00:00:00Z",
                    until_failed_event(node_id, "until evaluated false and retry is not configured"),
                ));
                state.paused_reason = Some(format!("until_not_met:{node_id}"));
                events.push(stream.next_record(
                    "1970-01-01T00:00:00Z",
                    paused_event("until_not_met"),
                ));
                persist_state_from_runtime(state, deduper, stream);
                return NodeUntilHandle::Paused;
            };
            let timeout_ms = match parse_timeout_ms(node_obj) {
                Ok(value) => value,
                Err(message) => {
                    events.push(stream.next_record(
                        "1970-01-01T00:00:00Z",
                        until_failed_event(node_id, &message),
                    ));
                    state.paused_reason = Some(format!("until_failed:{node_id}"));
                    events.push(stream.next_record(
                        "1970-01-01T00:00:00Z",
                        paused_event("until_failed"),
                    ));
                    persist_state_from_runtime(state, deduper, stream);
                    return NodeUntilHandle::Paused;
                }
            };
            let (attempt, waited_ms) = next_retry_attempt(state, node_id, &retry_config);
            if retry_config.max_attempts.is_some_and(|max_attempts| attempt > max_attempts) {
                events.push(stream.next_record(
                    "1970-01-01T00:00:00Z",
                    retry_exhausted_event(node_id, attempt, &retry_config),
                ));
                state.paused_reason = Some(format!("retry_exhausted:{node_id}"));
                events.push(stream.next_record(
                    "1970-01-01T00:00:00Z",
                    paused_event("retry_exhausted"),
                ));
                persist_state_from_runtime(state, deduper, stream);
                return NodeUntilHandle::Paused;
            }
            if timeout_ms.is_some_and(|timeout| waited_ms > timeout) {
                events.push(stream.next_record(
                    "1970-01-01T00:00:00Z",
                    retry_timeout_event(node_id, waited_ms, timeout_ms.unwrap_or_default()),
                ));
                state.paused_reason = Some(format!("retry_timeout:{node_id}"));
                events.push(stream.next_record(
                    "1970-01-01T00:00:00Z",
                    paused_event("retry_timeout"),
                ));
                persist_state_from_runtime(state, deduper, stream);
                return NodeUntilHandle::Paused;
            }
            state.paused_reason = None;
            events.push(stream.next_record(
                "1970-01-01T00:00:00Z",
                node_waiting_retry_event(node_id, attempt, waited_ms, timeout_ms, &retry_config),
            ));
            NodeUntilHandle::RetryScheduled
        }
        NodeUntilOutcome::Fail { message } => {
            events.push(stream.next_record(
                "1970-01-01T00:00:00Z",
                until_failed_event(node_id, &message),
            ));
            state.paused_reason = Some(format!("until_failed:{node_id}"));
            events.push(stream.next_record(
                "1970-01-01T00:00:00Z",
                paused_event("until_failed"),
            ));
            persist_state_from_runtime(state, deduper, stream);
            NodeUntilHandle::Paused
        }
    }
}

fn next_retry_attempt(state: &mut EngineRunnerState, node_id: &str, retry: &RetryConfig) -> (u64, u64) {
    let previous_attempt = state
        .pending_retries
        .get(node_id)
        .and_then(Value::as_object)
        .and_then(|object| object.get("attempt"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let previous_waited_ms = state
        .pending_retries
        .get(node_id)
        .and_then(Value::as_object)
        .and_then(|object| object.get("waited_ms"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let attempt = previous_attempt.saturating_add(1);
    let waited_ms = previous_waited_ms.saturating_add(retry.interval_ms);
    let mut retry_state = Map::new();
    retry_state.insert("attempt".to_string(), Value::Number(attempt.into()));
    retry_state.insert(
        "interval_ms".to_string(),
        Value::Number(retry.interval_ms.into()),
    );
    retry_state.insert("waited_ms".to_string(), Value::Number(waited_ms.into()));
    if let Some(max_attempts) = retry.max_attempts {
        retry_state.insert(
            "max_attempts".to_string(),
            Value::Number(max_attempts.into()),
        );
    }
    retry_state.insert("backoff".to_string(), Value::String(retry.backoff.clone()));
    state
        .pending_retries
        .insert(node_id.to_string(), Value::Object(retry_state));
    (attempt, waited_ms)
}

fn clear_retry_state(state: &mut EngineRunnerState, node_id: &str) {
    state.pending_retries.remove(node_id);
}

fn evaluate_node_assert(
    plan: &PlanDocument,
    node_obj: &Map<String, Value>,
    node_id: &str,
    runtime: &Value,
) -> NodeAssertOutcome {
    let strategy = resolve_assert_fail_strategy(plan, node_obj);
    let Some(assert_raw) = node_obj.get("assert") else {
        return NodeAssertOutcome::Pass;
    };
    let assert_value_ref = match serde_json::from_value::<ValueRef>(assert_raw.clone()) {
        Ok(value_ref) => value_ref,
        Err(error) => {
            return NodeAssertOutcome::Fail {
                message: format!("assert is invalid: {error}"),
                strategy,
            };
        }
    };
    let context = ResolverContext::with_runtime(runtime.clone());
    let evaluated = match evaluate_value_ref_with_options(
        &assert_value_ref,
        &context,
        &ValueRefEvalOptions::default(),
    ) {
        Ok(value) => value,
        Err(error) => {
            return NodeAssertOutcome::Fail {
                message: format!("assert evaluation failed: {error}"),
                strategy,
            };
        }
    };
    match evaluated {
        Value::Bool(true) => NodeAssertOutcome::Pass,
        Value::Bool(false) => NodeAssertOutcome::Fail {
            message: node_obj
                .get("assert_message")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| format!("assert failed for node `{node_id}`")),
            strategy,
        },
        other => NodeAssertOutcome::Fail {
            message: format!(
                "assert must evaluate to boolean, got {}",
                json_type_name(&other)
            ),
            strategy,
        },
    }
}

fn resolve_assert_fail_strategy(plan: &PlanDocument, node_obj: &Map<String, Value>) -> AssertFailStrategy {
    if let Some(strategy) = node_obj
        .get("extensions")
        .and_then(Value::as_object)
        .and_then(|extensions| extensions.get("assert"))
        .and_then(Value::as_object)
        .and_then(|assert_obj| assert_obj.get("on_fail"))
        .and_then(Value::as_str)
    {
        return parse_assert_fail_strategy(strategy);
    }
    if let Some(strategy) = node_obj
        .get("extensions")
        .and_then(Value::as_object)
        .and_then(|extensions| extensions.get("on_assert_fail"))
        .and_then(Value::as_str)
    {
        return parse_assert_fail_strategy(strategy);
    }
    if let Some(strategy) = plan
        .extensions
        .get("assert")
        .and_then(Value::as_object)
        .and_then(|assert_obj| assert_obj.get("on_fail"))
        .and_then(Value::as_str)
    {
        return parse_assert_fail_strategy(strategy);
    }
    AssertFailStrategy::Pause
}

fn parse_assert_fail_strategy(value: &str) -> AssertFailStrategy {
    if value.eq_ignore_ascii_case("stop") {
        return AssertFailStrategy::Stop;
    }
    AssertFailStrategy::Pause
}

fn deps_satisfied(node_obj: &Map<String, Value>, completed_set: &BTreeSet<String>) -> bool {
    let Some(deps) = node_obj.get("deps").and_then(Value::as_array) else {
        return true;
    };
    deps.iter()
        .filter_map(Value::as_str)
        .all(|dep| completed_set.contains(dep))
}

fn ensure_runtime_object(runtime: &mut Value) {
    if !runtime.is_object() {
        *runtime = Value::Object(Map::new());
    }
}

fn insert_unique_sorted(list: &mut Vec<String>, value: String) {
    if !list.iter().any(|item| item == &value) {
        list.push(value);
        list.sort();
    }
}

fn persist_state_from_runtime(
    state: &mut EngineRunnerState,
    deduper: &mut CommandDeduper,
    stream: &EngineEventStream,
) {
    state.seen_command_ids = deduper.seen_command_ids();
    state.completed_node_ids.sort();
    state.completed_node_ids.dedup();
    state.approved_node_ids.sort();
    state.approved_node_ids.dedup();
    state.next_seq = stream.next_seq();
}

fn apply_node_writes(node_obj: &Map<String, Value>, result: &Value, runtime: &mut Value) {
    let writes = node_obj
        .get("writes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    if writes.is_empty() {
        if let Some(node_id) = node_obj.get("id").and_then(Value::as_str) {
            set_runtime_path(runtime, &format!("nodes.{node_id}.outputs"), result.clone());
        }
        return;
    }

    for write in writes {
        let Some(write_object) = write.as_object() else {
            continue;
        };
        let Some(path) = write_object.get("path").and_then(Value::as_str) else {
            continue;
        };
        let mode = write_object
            .get("mode")
            .and_then(Value::as_str)
            .unwrap_or("set");
        let write_value = project_write_value(node_obj, result, path);
        if mode == "merge" {
            merge_runtime_path(runtime, path, write_value);
        } else {
            set_runtime_path(runtime, path, write_value);
        }
    }
}

fn project_write_value(node_obj: &Map<String, Value>, result: &Value, path: &str) -> Value {
    let Some(node_id) = node_obj.get("id").and_then(Value::as_str) else {
        return result.clone();
    };
    let is_query_type = node_obj
        .get("type")
        .and_then(Value::as_str)
        .is_some_and(|node_type| node_type == "query_ref");
    let is_query_source = node_obj
        .get("source")
        .and_then(Value::as_object)
        .is_some_and(|source| source.contains_key("query"));
    let is_query = is_query_type || is_query_source;
    if !is_query || path != format!("nodes.{node_id}.outputs") {
        return result.clone();
    }
    result
        .as_object()
        .and_then(|object| object.get("outputs"))
        .cloned()
        .unwrap_or_else(|| result.clone())
}

fn set_runtime_path(runtime: &mut Value, path: &str, value: Value) {
    let parts = path.split('.').filter(|part| !part.is_empty()).collect::<Vec<_>>();
    if parts.is_empty() {
        return;
    }
    let mut current = runtime;
    for part in &parts[..parts.len() - 1] {
        let Some(object) = current.as_object_mut() else {
            return;
        };
        current = object
            .entry((*part).to_string())
            .or_insert_with(|| Value::Object(Map::new()));
    }
    if let Some(object) = current.as_object_mut() {
        object.insert(parts[parts.len() - 1].to_string(), value);
    }
}

fn merge_runtime_path(runtime: &mut Value, path: &str, value: Value) {
    let parts = path.split('.').filter(|part| !part.is_empty()).collect::<Vec<_>>();
    if parts.is_empty() {
        return;
    }
    let mut current = runtime;
    for part in &parts[..parts.len() - 1] {
        let Some(object) = current.as_object_mut() else {
            return;
        };
        current = object
            .entry((*part).to_string())
            .or_insert_with(|| Value::Object(Map::new()));
    }
    let Some(object) = current.as_object_mut() else {
        return;
    };
    let key = parts[parts.len() - 1].to_string();
    let target = object.entry(key).or_insert_with(|| Value::Object(Map::new()));
    if let (Some(target_object), Some(value_object)) = (target.as_object_mut(), value.as_object()) {
        for (key, value) in value_object {
            target_object.insert(key.clone(), value.clone());
        }
    } else {
        *target = value;
    }
}

fn node_blocked_event(node_id: &str, readiness: &ais_sdk::NodeReadinessResult) -> EngineEvent {
    let mut event = EngineEvent::new(EngineEventType::NodeBlocked);
    event.node_id = Some(node_id.to_string());
    event.data.insert(
        "missing_refs".to_string(),
        Value::Array(
            readiness
                .missing_refs
                .iter()
                .cloned()
                .map(Value::String)
                .collect(),
        ),
    );
    event
        .data
        .insert("needs_detect".to_string(), Value::Bool(readiness.needs_detect));
    event
}

fn node_ready_event(node_id: &str) -> EngineEvent {
    let mut event = EngineEvent::new(EngineEventType::NodeReady);
    event.node_id = Some(node_id.to_string());
    event
}

fn need_user_confirm_event(node_id: &str, reason: &str, details: &Map<String, Value>) -> EngineEvent {
    let mut event = EngineEvent::new(EngineEventType::NeedUserConfirm);
    event.node_id = Some(node_id.to_string());
    event
        .data
        .insert("reason".to_string(), Value::String(reason.to_string()));
    event
        .data
        .insert("details".to_string(), Value::Object(details.clone()));
    event
}

fn hard_block_event(node_id: &str, reason: &str, details: &Map<String, Value>) -> EngineEvent {
    let mut event = EngineEvent::new(EngineEventType::Error);
    event.node_id = Some(node_id.to_string());
    event
        .data
        .insert("reason".to_string(), Value::String(reason.to_string()));
    event
        .data
        .insert("details".to_string(), Value::Object(details.clone()));
    event
}

fn executor_error_event(node_id: &str, error: &RouterExecuteError) -> EngineEvent {
    let mut event = EngineEvent::new(EngineEventType::Error);
    event.node_id = Some(node_id.to_string());
    event
        .data
        .insert("reason".to_string(), Value::String(error.to_string()));
    event
}

fn paused_event(reason: &str) -> EngineEvent {
    let mut event = EngineEvent::new(EngineEventType::EnginePaused);
    event
        .data
        .insert("reason".to_string(), Value::String(reason.to_string()));
    event
}

fn preflight_simulated_event(node_id: &str) -> EngineEvent {
    let mut event = EngineEvent::new(EngineEventType::Skipped);
    event.node_id = Some(node_id.to_string());
    event
        .data
        .insert("reason".to_string(), Value::String("preflight_simulate".to_string()));
    event
}

fn condition_skipped_event(node_id: &str) -> EngineEvent {
    let mut event = EngineEvent::new(EngineEventType::Skipped);
    event.node_id = Some(node_id.to_string());
    event
        .data
        .insert("reason".to_string(), Value::String("condition_false".to_string()));
    event
}

fn condition_failed_event(node_id: &str, message: &str) -> EngineEvent {
    let mut event = EngineEvent::new(EngineEventType::Error);
    event.node_id = Some(node_id.to_string());
    event
        .data
        .insert("reason".to_string(), Value::String("condition_failed".to_string()));
    event
        .data
        .insert("message".to_string(), Value::String(message.to_string()));
    event
}

fn node_waiting_retry_event(
    node_id: &str,
    attempt: u64,
    waited_ms: u64,
    timeout_ms: Option<u64>,
    retry: &RetryConfig,
) -> EngineEvent {
    let mut event = EngineEvent::new(EngineEventType::NodeWaiting);
    event.node_id = Some(node_id.to_string());
    event
        .data
        .insert("reason".to_string(), Value::String("until_retry".to_string()));
    event
        .data
        .insert("attempt".to_string(), Value::Number(attempt.into()));
    event.data.insert(
        "interval_ms".to_string(),
        Value::Number(retry.interval_ms.into()),
    );
    event
        .data
        .insert("waited_ms".to_string(), Value::Number(waited_ms.into()));
    if let Some(max_attempts) = retry.max_attempts {
        event.data.insert(
            "max_attempts".to_string(),
            Value::Number(max_attempts.into()),
        );
    }
    if let Some(timeout_ms) = timeout_ms {
        event
            .data
            .insert("timeout_ms".to_string(), Value::Number(timeout_ms.into()));
    }
    event
        .data
        .insert("backoff".to_string(), Value::String(retry.backoff.clone()));
    event
}

fn until_failed_event(node_id: &str, message: &str) -> EngineEvent {
    let mut event = EngineEvent::new(EngineEventType::Error);
    event.node_id = Some(node_id.to_string());
    event
        .data
        .insert("reason".to_string(), Value::String("until_failed".to_string()));
    event
        .data
        .insert("message".to_string(), Value::String(message.to_string()));
    event
}

fn retry_exhausted_event(node_id: &str, attempt: u64, retry: &RetryConfig) -> EngineEvent {
    let mut event = EngineEvent::new(EngineEventType::Error);
    event.node_id = Some(node_id.to_string());
    event
        .data
        .insert("reason".to_string(), Value::String("retry_exhausted".to_string()));
    event
        .data
        .insert("attempt".to_string(), Value::Number(attempt.into()));
    if let Some(max_attempts) = retry.max_attempts {
        event.data.insert(
            "max_attempts".to_string(),
            Value::Number(max_attempts.into()),
        );
    }
    event
}

fn retry_timeout_event(node_id: &str, waited_ms: u64, timeout_ms: u64) -> EngineEvent {
    let mut event = EngineEvent::new(EngineEventType::Error);
    event.node_id = Some(node_id.to_string());
    event
        .data
        .insert("reason".to_string(), Value::String("retry_timeout".to_string()));
    event
        .data
        .insert("waited_ms".to_string(), Value::Number(waited_ms.into()));
    event
        .data
        .insert("timeout_ms".to_string(), Value::Number(timeout_ms.into()));
    event
}

fn assert_failed_event(
    node_id: &str,
    message: &str,
    phase: &str,
    assert_expr: Option<&Value>,
) -> EngineEvent {
    let mut event = EngineEvent::new(EngineEventType::Error);
    event.node_id = Some(node_id.to_string());
    event
        .data
        .insert("reason".to_string(), Value::String("assert_failed".to_string()));
    event
        .data
        .insert("message".to_string(), Value::String(message.to_string()));
    event
        .data
        .insert("phase".to_string(), Value::String(phase.to_string()));
    if let Some(assert_expr) = assert_expr {
        event
            .data
            .insert("assert".to_string(), assert_expr.clone());
    }
    event
}

fn node_paused_event(node_id: &str, reason: &str) -> EngineEvent {
    let mut event = EngineEvent::new(EngineEventType::NodePaused);
    event.node_id = Some(node_id.to_string());
    event
        .data
        .insert("reason".to_string(), Value::String(reason.to_string()));
    event
}

fn json_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[cfg(test)]
#[path = "runner_test.rs"]
mod tests;
