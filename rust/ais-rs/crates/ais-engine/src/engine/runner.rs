use crate::checkpoint::{
    canonical_side_effect_status, CheckpointSideEffectRecord, SIDE_EFFECT_RECORD_SCHEMA_0_1_0,
    SIDE_EFFECT_STATUS_CONFIRMED, SIDE_EFFECT_STATUS_UNKNOWN,
};
use crate::commands::{
    apply_command_with_dedupe, CommandDeduper, DuplicateCommandMode, EngineCommandEnvelope,
    EngineCommandType,
};
use crate::engine::apply_patches_from_command;
use crate::events::{
    wall_clock_timestamp_rfc3339, EngineEvent, EngineEventRecord, EngineEventStream,
    EngineEventType, ENGINE_EVENT_CHECKS_SCHEMA_0_0_1,
};
use crate::executor::{RouterExecuteError, RouterExecutor};
use crate::policy::{
    enforce_policy_gate, enrich_need_user_confirm_output, extract_policy_gate_input,
    PolicyEnforcementOptions, PolicyGateInput, PolicyGateOutput, PolicyGateReasonCode,
};
use crate::solver::{build_solver_event, Solver, SolverDecision};
use ais_sdk::{
    evaluate_value_ref_with_options, get_node_readiness, resolve_calculated_bindings,
    resolve_node_bindings, resolve_query_bindings, PlanDocument, ResolverContext, ValueRef,
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
    pub plan_epoch: u64,
    #[serde(default)]
    pub plan_hash_history: Vec<String>,
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
            plan_epoch: 0,
            plan_hash_history: Vec::new(),
            next_seq: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EngineRunnerOptions {
    pub duplicate_command_mode: DuplicateCommandMode,
    pub policy: PolicyEnforcementOptions,
    pub solver_context: crate::solver::SolverContext,
    #[serde(default)]
    pub safety: EngineSafetyOptions,
}

impl Default for EngineRunnerOptions {
    fn default() -> Self {
        Self {
            duplicate_command_mode: DuplicateCommandMode::Reject,
            policy: PolicyEnforcementOptions::default(),
            solver_context: crate::solver::SolverContext::default(),
            safety: EngineSafetyOptions::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EngineSafetyOptions {
    #[serde(default)]
    pub blocked_execution_types: Vec<String>,
    #[serde(default = "default_true")]
    pub sanitize_executor_output: bool,
    #[serde(default = "default_max_output_string_chars")]
    pub max_output_string_chars: usize,
    #[serde(default = "default_true")]
    pub hard_block_prompt_injection: bool,
}

impl Default for EngineSafetyOptions {
    fn default() -> Self {
        Self {
            blocked_execution_types: vec![],
            sanitize_executor_output: true,
            max_output_string_chars: default_max_output_string_chars(),
            hard_block_prompt_injection: true,
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_max_output_string_chars() -> usize {
    512
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
            wall_clock_timestamp_rfc3339(),
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
                    wall_clock_timestamp_rfc3339(),
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
            EngineCommandType::UserInput => {
                if let Err(reason) =
                    apply_user_input_command(&mut state.runtime, &command.command.data)
                {
                    events.push(stream.next_record(
                        wall_clock_timestamp_rfc3339(),
                        need_user_input_event(
                            "invalid_user_input",
                            reason.as_str(),
                            &command.command.id,
                        ),
                    ));
                    state.paused_reason = Some("need_user_input:command".to_string());
                    events.push(stream.next_record(
                        wall_clock_timestamp_rfc3339(),
                        paused_event("need_user_input"),
                    ));
                    persist_state_from_runtime(state, &mut deduper, &stream);
                    return EngineRunResult {
                        status: EngineRunStatus::Paused,
                        events,
                    };
                }
                if is_user_input_pause_reason(state.paused_reason.as_deref()) {
                    state.paused_reason = None;
                }
            }
            EngineCommandType::UserSelect => {
                if let Err(reason) =
                    apply_user_select_command(&mut state.runtime, &command.command.data)
                {
                    events.push(stream.next_record(
                        wall_clock_timestamp_rfc3339(),
                        need_user_input_event(
                            "invalid_user_select",
                            reason.as_str(),
                            &command.command.id,
                        ),
                    ));
                    state.paused_reason = Some("need_user_input:command".to_string());
                    events.push(stream.next_record(
                        wall_clock_timestamp_rfc3339(),
                        paused_event("need_user_input"),
                    ));
                    persist_state_from_runtime(state, &mut deduper, &stream);
                    return EngineRunResult {
                        status: EngineRunStatus::Paused,
                        events,
                    };
                }
                if is_user_input_pause_reason(state.paused_reason.as_deref()) {
                    state.paused_reason = None;
                }
            }
            EngineCommandType::Cancel => {
                state.paused_reason = Some("cancelled_by_command".to_string());
                events.push(stream.next_record(
                    wall_clock_timestamp_rfc3339(),
                    paused_event("cancelled_by_command"),
                ));
                persist_state_from_runtime(state, &mut deduper, &stream);
                return EngineRunResult {
                    status: EngineRunStatus::Paused,
                    events,
                };
            }
            EngineCommandType::ReplacePlan => {}
        }
    }

    let mut progress = false;
    let mut completed_set = state
        .completed_node_ids
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut approved_set = state
        .approved_node_ids
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();

    for node in &plan.nodes {
        sync_progress_sets_to_state(state, &completed_set, &approved_set);
        let Some(node_obj) = node.as_object() else {
            continue;
        };
        let Some(node_id) = node_obj
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_string)
        else {
            continue;
        };
        if completed_set.contains(&node_id) {
            continue;
        }
        if !deps_satisfied(node_obj, &completed_set) {
            continue;
        }

        let control_kind = control_step_kind(node_obj);
        match evaluate_node_condition(node_obj, &state.runtime) {
            NodeConditionOutcome::Pass => {}
            NodeConditionOutcome::Skip => {
                events.push(stream.next_record(
                    wall_clock_timestamp_rfc3339(),
                    condition_skipped_event(&node_id, control_kind),
                ));
                clear_retry_state(state, &node_id);
                completed_set.insert(node_id.clone());
                approved_set.remove(&node_id);
                progress = true;
                continue;
            }
            NodeConditionOutcome::Fail { message } => {
                events.push(stream.next_record(
                    wall_clock_timestamp_rfc3339(),
                    condition_failed_event(&node_id, &message, control_kind),
                ));
                state.paused_reason = Some(format!("condition_failed:{node_id}"));
                events.push(stream.next_record(
                    wall_clock_timestamp_rfc3339(),
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
                wall_clock_timestamp_rfc3339(),
                node_blocked_event(&node_id, &readiness),
            ));
            let decision = solver.solve(node, &readiness, &options.solver_context);
            if !matches!(
                decision,
                SolverDecision::NeedUserConfirm { .. } | SolverDecision::NeedUserInput { .. }
            ) {
                if let Some(event) = build_solver_event(Some(&node_id), &decision) {
                    events.push(stream.next_record(wall_clock_timestamp_rfc3339(), event));
                }
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
                SolverDecision::NeedUserInput { reason, details } => {
                    let action_ref = extract_action_ref_from_node(node_obj);
                    let (reason_code, event_details) =
                        need_user_input_fields(&node_id, action_ref.as_deref(), &reason, &details);
                    events.push(stream.next_record(
                        wall_clock_timestamp_rfc3339(),
                        need_user_input_event_with_details(
                            Some(&node_id),
                            &reason_code,
                            &reason,
                            &event_details,
                        ),
                    ));
                    state.paused_reason = Some(format!("need_user_input:{node_id}"));
                    events.push(stream.next_record(
                        wall_clock_timestamp_rfc3339(),
                        paused_event("need_user_input"),
                    ));
                    persist_state_from_runtime(state, &mut deduper, &stream);
                    return EngineRunResult {
                        status: EngineRunStatus::Paused,
                        events,
                    };
                }
                SolverDecision::NeedUserConfirm { reason, details } => {
                    let action_ref = extract_action_ref_from_node(node_obj);
                    let risk_observation = observe_risk_level_from_node(node_obj);
                    let risk_tags = extract_risk_tags_from_node(node_obj);
                    let gate_input = extract_policy_gate_input(
                        node,
                        Some(&state.runtime),
                        readiness.resolved_params.as_ref(),
                        action_ref.clone(),
                        risk_observation.risk_level,
                        risk_tags,
                    );
                    let gate_output = PolicyGateOutput::NeedUserConfirm {
                        reason_code: PolicyGateReasonCode::UnknownFields,
                        reason: reason.clone(),
                        details: details.clone(),
                    };
                    let gate_output = enrich_need_user_confirm_output(&gate_input, &gate_output)
                        .unwrap_or(gate_output);
                    let (event_reason_code, event_reason, event_details) = need_user_confirm_fields(
                        &gate_input,
                        &gate_output,
                        action_ref.as_deref(),
                        Some(&risk_observation),
                        "fallback",
                    );
                    if should_route_confirm_to_missing_required_input(
                        event_reason_code.as_str(),
                        &event_details,
                    ) {
                        let normalized_details =
                            normalize_missing_required_input_details(&event_details);
                        let mut event = need_user_input_event_with_details(
                            Some(&node_id),
                            "missing_required_input",
                            "missing_inputs_or_runtime_refs",
                            &normalized_details,
                        );
                        annotate_gate_check(
                            &mut event,
                            false,
                            Some(event_reason_code.as_str()),
                            control_kind,
                        );
                        events.push(stream.next_record(wall_clock_timestamp_rfc3339(), event));
                        state.paused_reason = Some(format!("need_user_input:{node_id}"));
                        events.push(stream.next_record(
                            wall_clock_timestamp_rfc3339(),
                            paused_event("need_user_input"),
                        ));
                        persist_state_from_runtime(state, &mut deduper, &stream);
                        return EngineRunResult {
                            status: EngineRunStatus::Paused,
                            events,
                        };
                    }

                    events.push(stream.next_record(
                        wall_clock_timestamp_rfc3339(),
                        need_user_confirm_event(
                            &node_id,
                            &event_reason_code,
                            &event_reason,
                            &event_details,
                            control_kind,
                        ),
                    ));
                    state.paused_reason = Some(format!("need_user_confirm:{node_id}"));
                    events.push(stream.next_record(
                        wall_clock_timestamp_rfc3339(),
                        paused_event("need_user_confirm"),
                    ));
                    persist_state_from_runtime(state, &mut deduper, &stream);
                    return EngineRunResult {
                        status: EngineRunStatus::Paused,
                        events,
                    };
                }
                SolverDecision::Noop => {}
            }
            continue;
        }

        events.push(stream.next_record(
            wall_clock_timestamp_rfc3339(),
            node_ready_event(&node_id, node_obj.contains_key("condition"), control_kind),
        ));

        let simulate_mode = should_simulate_node(plan, node_obj, &node_id);
        if simulate_mode {
            let query = resolve_query_bindings(node, Some(&state.runtime));
            if !query.missing_refs.is_empty() {
                let mut details = Map::new();
                details.insert(
                    "reason".to_string(),
                    Value::String(format!(
                        "simulate prerequisite queries failed: {}",
                        query.missing_refs.join(", ")
                    )),
                );
                events.push(
                    stream.next_record(
                        wall_clock_timestamp_rfc3339(),
                        executor_error_event(
                            &node_id,
                            &RouterExecuteError::ExecutorFailed {
                                executor: "engine:materialize".to_string(),
                                node_id: node_id.clone(),
                                reason: details
                                    .get("reason")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default()
                                    .to_string(),
                            },
                        ),
                    ),
                );
                state.paused_reason = Some(format!("executor_error:{node_id}"));
                events.push(stream.next_record(
                    wall_clock_timestamp_rfc3339(),
                    paused_event("executor_error"),
                ));
                persist_state_from_runtime(state, &mut deduper, &stream);
                return EngineRunResult {
                    status: EngineRunStatus::Paused,
                    events,
                };
            }
            let calculated = resolve_calculated_bindings(
                node,
                &ResolverContext::with_runtime(state.runtime.clone()),
                &ValueRefEvalOptions::default(),
                readiness.resolved_params.as_ref(),
            );
            if !calculated.missing_refs.is_empty() || !calculated.errors.is_empty() {
                let mut details = Map::new();
                details.insert(
                    "reason".to_string(),
                    Value::String(format!(
                        "simulate calculated bindings failed: {}",
                        calculated
                            .missing_refs
                            .into_iter()
                            .chain(calculated.errors.into_iter())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )),
                );
                events.push(
                    stream.next_record(
                        wall_clock_timestamp_rfc3339(),
                        executor_error_event(
                            &node_id,
                            &RouterExecuteError::ExecutorFailed {
                                executor: "engine:materialize".to_string(),
                                node_id: node_id.clone(),
                                reason: details
                                    .get("reason")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default()
                                    .to_string(),
                            },
                        ),
                    ),
                );
                state.paused_reason = Some(format!("executor_error:{node_id}"));
                events.push(stream.next_record(
                    wall_clock_timestamp_rfc3339(),
                    paused_event("executor_error"),
                ));
                persist_state_from_runtime(state, &mut deduper, &stream);
                return EngineRunResult {
                    status: EngineRunStatus::Paused,
                    events,
                };
            }
            if !calculated.calculated.is_empty() {
                set_runtime_path(
                    &mut state.runtime,
                    &format!("nodes.{node_id}.calculated"),
                    Value::Object(calculated.calculated),
                );
            }
            let simulated_result = json!({
                "simulated": true,
                "node_id": node_id,
            });
            apply_node_writes(node_obj, &simulated_result, &mut state.runtime);
            events.push(stream.next_record(
                wall_clock_timestamp_rfc3339(),
                preflight_simulated_event(&node_id),
            ));
            match evaluate_node_assert(plan, node_obj, &node_id, &state.runtime) {
                NodeAssertOutcome::NotConfigured => {
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
                NodeAssertOutcome::Pass => {
                    set_latest_node_ready_assert_result(
                        &mut events,
                        &node_id,
                        "preflight_simulate",
                        control_kind,
                    );
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
                        wall_clock_timestamp_rfc3339(),
                        assert_failed_event(
                            &node_id,
                            &message,
                            "preflight_simulate",
                            node_obj.get("assert"),
                            control_kind,
                        ),
                    ));
                    state.paused_reason = Some(format!("assert_failed:{node_id}"));
                    if strategy == AssertFailStrategy::Stop {
                        events.push(stream.next_record(
                            wall_clock_timestamp_rfc3339(),
                            node_paused_event(&node_id, "assert_failed_stop"),
                        ));
                        persist_state_from_runtime(state, &mut deduper, &stream);
                        return EngineRunResult {
                            status: EngineRunStatus::Stopped,
                            events,
                        };
                    }
                    events.push(stream.next_record(
                        wall_clock_timestamp_rfc3339(),
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

        let action_ref = extract_action_ref_from_node(node_obj);
        let risk_observation = observe_risk_level_from_node(node_obj);
        let risk_tags = extract_risk_tags_from_node(node_obj);
        let gate_input = extract_policy_gate_input(
            node,
            Some(&state.runtime),
            readiness.resolved_params.as_ref(),
            action_ref.clone(),
            risk_observation.risk_level,
            risk_tags,
        );
        let gate_output = enforce_policy_gate(&gate_input, &options.policy);
        let gate_output =
            enrich_need_user_confirm_output(&gate_input, &gate_output).unwrap_or(gate_output);

        let gate_passed = match gate_output {
            PolicyGateOutput::Ok { .. } => true,
            PolicyGateOutput::NeedUserConfirm {
                reason_code,
                reason,
                details,
            } => {
                let gate_output = PolicyGateOutput::NeedUserConfirm {
                    reason_code,
                    reason,
                    details,
                };
                if !approved_set.contains(&node_id) {
                    let (event_reason_code, event_reason, event_details) = need_user_confirm_fields(
                        &gate_input,
                        &gate_output,
                        action_ref.as_deref(),
                        Some(&risk_observation),
                        "unknown",
                    );
                    if should_route_confirm_to_missing_required_input(
                        event_reason_code.as_str(),
                        &event_details,
                    ) {
                        let normalized_details =
                            normalize_missing_required_input_details(&event_details);
                        let mut event = need_user_input_event_with_details(
                            Some(&node_id),
                            "missing_required_input",
                            "missing_inputs_or_runtime_refs",
                            &normalized_details,
                        );
                        annotate_gate_check(
                            &mut event,
                            false,
                            Some(event_reason_code.as_str()),
                            control_kind,
                        );
                        events.push(stream.next_record(wall_clock_timestamp_rfc3339(), event));
                        state.paused_reason = Some(format!("need_user_input:{node_id}"));
                        events.push(stream.next_record(
                            wall_clock_timestamp_rfc3339(),
                            paused_event("need_user_input"),
                        ));
                        persist_state_from_runtime(state, &mut deduper, &stream);
                        return EngineRunResult {
                            status: EngineRunStatus::Paused,
                            events,
                        };
                    }
                    events.push(stream.next_record(
                        wall_clock_timestamp_rfc3339(),
                        need_user_confirm_event(
                            &node_id,
                            &event_reason_code,
                            &event_reason,
                            &event_details,
                            control_kind,
                        ),
                    ));
                    state.paused_reason = Some(format!("need_user_confirm:{node_id}"));
                    events.push(stream.next_record(
                        wall_clock_timestamp_rfc3339(),
                        paused_event("need_user_confirm"),
                    ));
                    persist_state_from_runtime(state, &mut deduper, &stream);
                    return EngineRunResult {
                        status: EngineRunStatus::Paused,
                        events,
                    };
                }
                true
            }
            PolicyGateOutput::HardBlock {
                reason_code,
                reason,
                details,
            } => {
                events.push(stream.next_record(
                    wall_clock_timestamp_rfc3339(),
                    hard_block_event(
                        &node_id,
                        reason_code.as_str(),
                        &reason,
                        &details,
                        true,
                        control_kind,
                    ),
                ));
                state.paused_reason = Some(format!("hard_block:{node_id}"));
                events.push(
                    stream.next_record(wall_clock_timestamp_rfc3339(), paused_event("hard_block")),
                );
                persist_state_from_runtime(state, &mut deduper, &stream);
                return EngineRunResult {
                    status: EngineRunStatus::Paused,
                    events,
                };
            }
        };
        if gate_passed {
            set_latest_node_ready_gate_result(&mut events, &node_id, control_kind);
        }

        let (executable_node, resolved_calculated) = match materialize_node_execution(
            node,
            &state.runtime,
            readiness.resolved_params.as_ref(),
        ) {
            Ok(payload) => payload,
            Err(reason) => {
                let error = RouterExecuteError::ExecutorFailed {
                    executor: "engine:materialize".to_string(),
                    node_id: node_id.clone(),
                    reason,
                };
                events.push(stream.next_record(
                    wall_clock_timestamp_rfc3339(),
                    executor_error_event(&node_id, &error),
                ));
                state.paused_reason = Some(format!("executor_error:{node_id}"));
                events.push(stream.next_record(
                    wall_clock_timestamp_rfc3339(),
                    paused_event("executor_error"),
                ));
                persist_state_from_runtime(state, &mut deduper, &stream);
                return EngineRunResult {
                    status: EngineRunStatus::Paused,
                    events,
                };
            }
        };
        if let Some((reason_code, reason, details)) =
            safety_hook_before_execute(node_obj, &options.safety)
        {
            events.push(stream.next_record(
                wall_clock_timestamp_rfc3339(),
                hard_block_event(
                    &node_id,
                    reason_code.as_str(),
                    reason.as_str(),
                    &details,
                    false,
                    control_kind,
                ),
            ));
            state.paused_reason = Some(format!("hard_block:{node_id}"));
            events.push(
                stream.next_record(wall_clock_timestamp_rfc3339(), paused_event("hard_block")),
            );
            persist_state_from_runtime(state, &mut deduper, &stream);
            return EngineRunResult {
                status: EngineRunStatus::Paused,
                events,
            };
        }
        if !resolved_calculated.is_empty() {
            set_runtime_path(
                &mut state.runtime,
                &format!("nodes.{node_id}.calculated"),
                Value::Object(resolved_calculated.clone()),
            );
        }
        match router.execute(&executable_node, &mut state.runtime) {
            Ok(mut result) => {
                if options.safety.sanitize_executor_output {
                    sanitize_executor_output_value(&mut result.output.result, &options.safety);
                }
                if options.safety.hard_block_prompt_injection
                    && contains_prompt_injection_pattern(&result.output.result)
                {
                    let mut details = Map::new();
                    details.insert(
                        "reason".to_string(),
                        Value::String(
                            "potential prompt-injection content detected in executor output"
                                .to_string(),
                        ),
                    );
                    events.push(stream.next_record(
                        wall_clock_timestamp_rfc3339(),
                        hard_block_event(
                            &node_id,
                            "safety_output_prompt_injection",
                            "safety layer blocked suspicious executor output",
                            &details,
                            false,
                            control_kind,
                        ),
                    ));
                    state.paused_reason = Some(format!("hard_block:{node_id}"));
                    events.push(
                        stream.next_record(
                            wall_clock_timestamp_rfc3339(),
                            paused_event("hard_block"),
                        ),
                    );
                    persist_state_from_runtime(state, &mut deduper, &stream);
                    return EngineRunResult {
                        status: EngineRunStatus::Paused,
                        events,
                    };
                }

                let chain = node_obj.get("chain").and_then(Value::as_str);
                let execution_type = node_obj
                    .get("execution")
                    .and_then(Value::as_object)
                    .and_then(|execution| execution.get("type"))
                    .and_then(Value::as_str);
                let side_effects = std::mem::take(&mut result.output.side_effects);
                for mut side_effect in side_effects {
                    if !normalize_side_effect_record(
                        &mut side_effect,
                        node_id.as_str(),
                        chain,
                        execution_type,
                    ) {
                        continue;
                    }
                    if side_effect.status == SIDE_EFFECT_STATUS_CONFIRMED
                        && events.iter().any(|record| {
                            if record.event.event_type != EngineEventType::SideEffectObserved {
                                return false;
                            }
                            record.event.node_id.as_deref() == Some(node_id.as_str())
                                && record
                                    .event
                                    .data
                                    .get("record")
                                    .and_then(Value::as_object)
                                    .and_then(|record| record.get("status"))
                                    .and_then(Value::as_str)
                                    == Some(SIDE_EFFECT_STATUS_CONFIRMED)
                        })
                    {
                        continue;
                    }
                    events.push(stream.next_record(
                        wall_clock_timestamp_rfc3339(),
                        side_effect_observed_event(&node_id, &side_effect),
                    ));
                }

                apply_node_writes(node_obj, &result.output.result, &mut state.runtime);
                match evaluate_node_assert(plan, node_obj, &node_id, &state.runtime) {
                    NodeAssertOutcome::NotConfigured => {}
                    NodeAssertOutcome::Pass => {
                        set_latest_node_ready_assert_result(
                            &mut events,
                            &node_id,
                            "execute",
                            control_kind,
                        );
                    }
                    NodeAssertOutcome::Fail { message, strategy } => {
                        events.push(stream.next_record(
                            wall_clock_timestamp_rfc3339(),
                            assert_failed_event(
                                &node_id,
                                &message,
                                "execute",
                                node_obj.get("assert"),
                                control_kind,
                            ),
                        ));
                        state.paused_reason = Some(format!("assert_failed:{node_id}"));
                        if strategy == AssertFailStrategy::Stop {
                            events.push(stream.next_record(
                                wall_clock_timestamp_rfc3339(),
                                node_paused_event(&node_id, "assert_failed_stop"),
                            ));
                            persist_state_from_runtime(state, &mut deduper, &stream);
                            return EngineRunResult {
                                status: EngineRunStatus::Stopped,
                                events,
                            };
                        }
                        events.push(stream.next_record(
                            wall_clock_timestamp_rfc3339(),
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
                    wall_clock_timestamp_rfc3339(),
                    executor_error_event(&node_id, &error),
                ));
                state.paused_reason = Some(format!("executor_error:{node_id}"));
                events.push(stream.next_record(
                    wall_clock_timestamp_rfc3339(),
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
        events
            .push(stream.next_record(wall_clock_timestamp_rfc3339(), paused_event("no_progress")));
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
) -> Result<(Value, Map<String, Value>), String> {
    let mut node_obj = node
        .as_object()
        .cloned()
        .ok_or_else(|| "node must be object".to_string())?;
    let Some(execution) = node_obj.get("execution") else {
        return Ok((Value::Object(node_obj), Map::new()));
    };

    let context = ResolverContext::with_runtime(runtime.clone());
    let query = resolve_query_bindings(node, Some(runtime));
    if !query.missing_refs.is_empty() {
        return Err(format!(
            "materialize prerequisite queries failed: {}",
            query.missing_refs.join(", ")
        ));
    }
    let calculated = resolve_calculated_bindings(
        node,
        &context,
        &ValueRefEvalOptions::default(),
        resolved_params,
    );
    if !calculated.missing_refs.is_empty() || !calculated.errors.is_empty() {
        let mut issues = calculated.missing_refs;
        issues.extend(calculated.errors);
        return Err(format!(
            "materialize calculated bindings failed: {}",
            issues.join(", ")
        ));
    }
    let calculated_root = node
        .get("calculated_overrides")
        .and_then(Value::as_object)
        .map(|_| &calculated.calculated);
    let options = resolve_node_bindings(node, Some(runtime), resolved_params, calculated_root)
        .to_eval_options(&ValueRefEvalOptions::default());

    let resolved_execution = materialize_value_refs(execution, &context, &options)?;
    node_obj.insert("execution".to_string(), resolved_execution);
    Ok((Value::Object(node_obj), calculated.calculated))
}

fn prepare_node_expression_eval_options(
    node_obj: &Map<String, Value>,
    runtime: &Value,
) -> Result<ValueRefEvalOptions, String> {
    let node_value = Value::Object(node_obj.clone());
    let context = ResolverContext::with_runtime(runtime.clone());
    let base_options = resolve_node_bindings(&node_value, Some(runtime), None, None)
        .to_eval_options(&ValueRefEvalOptions::default());
    let resolved_params = resolve_node_params(&node_value, &context, &base_options)?;
    let resolved_params = has_param_bindings(&node_value).then_some(resolved_params);
    let query = resolve_query_bindings(&node_value, Some(runtime));
    if !query.missing_refs.is_empty() {
        return Err(format!(
            "prerequisite query bindings failed: {}",
            query.missing_refs.join(", ")
        ));
    }
    let calculated = resolve_calculated_bindings(
        &node_value,
        &context,
        &ValueRefEvalOptions::default(),
        resolved_params.as_ref(),
    );
    if !calculated.missing_refs.is_empty() || !calculated.errors.is_empty() {
        let mut issues = calculated.missing_refs;
        issues.extend(calculated.errors);
        return Err(format!(
            "node derived bindings failed: {}",
            issues.join(", ")
        ));
    }
    let calculated_root = node_value
        .get("calculated_overrides")
        .and_then(Value::as_object)
        .map(|_| &calculated.calculated);
    Ok(resolve_node_bindings(
        &node_value,
        Some(runtime),
        resolved_params.as_ref(),
        calculated_root,
    )
    .to_eval_options(&ValueRefEvalOptions::default()))
}

fn resolve_node_params(
    node: &Value,
    context: &ResolverContext,
    options: &ValueRefEvalOptions,
) -> Result<Map<String, Value>, String> {
    let mut resolved_params = Map::new();
    let Some(params) = node.pointer("/bindings/params").and_then(Value::as_object) else {
        return Ok(resolved_params);
    };

    for (key, value) in params {
        let value_ref = serde_json::from_value::<ValueRef>(value.clone())
            .map_err(|error| format!("invalid ValueRef at `bindings.params.{key}`: {error}"))?;
        let resolved = evaluate_value_ref_with_options(&value_ref, context, options)
            .map_err(|error| format!("bindings.params.{key} evaluation failed: {error}"))?;
        resolved_params.insert(key.clone(), resolved);
    }

    Ok(resolved_params)
}

fn has_param_bindings(node: &Value) -> bool {
    node.pointer("/bindings/params")
        .and_then(Value::as_object)
        .is_some()
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
                out.insert(
                    key.clone(),
                    materialize_value_refs(child, context, options)?,
                );
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
    NotConfigured,
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
    let options = match prepare_node_expression_eval_options(node_obj, runtime) {
        Ok(options) => options,
        Err(error) => {
            return NodeConditionOutcome::Fail {
                message: format!("condition evaluation failed: {error}"),
            };
        }
    };
    let evaluated = match evaluate_value_ref_with_options(&condition_value_ref, &context, &options)
    {
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
    let options = match prepare_node_expression_eval_options(node_obj, runtime) {
        Ok(options) => options,
        Err(error) => {
            return NodeUntilOutcome::Fail {
                message: format!("until evaluation failed: {error}"),
            };
        }
    };
    let evaluated = match evaluate_value_ref_with_options(&until_value_ref, &context, &options) {
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
            message: format!(
                "until must evaluate to boolean, got {}",
                json_type_name(&other)
            ),
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
        return Err(format!(
            "retry.backoff `{backoff}` is not supported (expected `fixed`)"
        ));
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
            let retry_config =
                match parse_retry_config(node_obj) {
                    Ok(value) => value,
                    Err(message) => {
                        events.push(stream.next_record(
                            wall_clock_timestamp_rfc3339(),
                            until_failed_event(node_id, &message),
                        ));
                        state.paused_reason = Some(format!("until_failed:{node_id}"));
                        events.push(stream.next_record(
                            wall_clock_timestamp_rfc3339(),
                            paused_event("until_failed"),
                        ));
                        persist_state_from_runtime(state, deduper, stream);
                        return NodeUntilHandle::Paused;
                    }
                };
            let Some(retry_config) = retry_config else {
                events.push(stream.next_record(
                    wall_clock_timestamp_rfc3339(),
                    until_failed_event(
                        node_id,
                        "until evaluated false and retry is not configured",
                    ),
                ));
                state.paused_reason = Some(format!("until_not_met:{node_id}"));
                events.push(stream.next_record(
                    wall_clock_timestamp_rfc3339(),
                    paused_event("until_not_met"),
                ));
                persist_state_from_runtime(state, deduper, stream);
                return NodeUntilHandle::Paused;
            };
            let timeout_ms =
                match parse_timeout_ms(node_obj) {
                    Ok(value) => value,
                    Err(message) => {
                        events.push(stream.next_record(
                            wall_clock_timestamp_rfc3339(),
                            until_failed_event(node_id, &message),
                        ));
                        state.paused_reason = Some(format!("until_failed:{node_id}"));
                        events.push(stream.next_record(
                            wall_clock_timestamp_rfc3339(),
                            paused_event("until_failed"),
                        ));
                        persist_state_from_runtime(state, deduper, stream);
                        return NodeUntilHandle::Paused;
                    }
                };
            let (attempt, waited_ms) = next_retry_attempt(state, node_id, &retry_config);
            if retry_config
                .max_attempts
                .is_some_and(|max_attempts| attempt > max_attempts)
            {
                events.push(stream.next_record(
                    wall_clock_timestamp_rfc3339(),
                    retry_exhausted_event(node_id, attempt, &retry_config),
                ));
                state.paused_reason = Some(format!("retry_exhausted:{node_id}"));
                events.push(stream.next_record(
                    wall_clock_timestamp_rfc3339(),
                    paused_event("retry_exhausted"),
                ));
                persist_state_from_runtime(state, deduper, stream);
                return NodeUntilHandle::Paused;
            }
            if timeout_ms.is_some_and(|timeout| waited_ms > timeout) {
                events.push(stream.next_record(
                    wall_clock_timestamp_rfc3339(),
                    retry_timeout_event(node_id, waited_ms, timeout_ms.unwrap_or_default()),
                ));
                state.paused_reason = Some(format!("retry_timeout:{node_id}"));
                events.push(stream.next_record(
                    wall_clock_timestamp_rfc3339(),
                    paused_event("retry_timeout"),
                ));
                persist_state_from_runtime(state, deduper, stream);
                return NodeUntilHandle::Paused;
            }
            state.paused_reason = None;
            events.push(stream.next_record(
                wall_clock_timestamp_rfc3339(),
                node_waiting_retry_event(node_id, attempt, waited_ms, timeout_ms, &retry_config),
            ));
            NodeUntilHandle::RetryScheduled
        }
        NodeUntilOutcome::Fail { message } => {
            events.push(stream.next_record(
                wall_clock_timestamp_rfc3339(),
                until_failed_event(node_id, &message),
            ));
            state.paused_reason = Some(format!("until_failed:{node_id}"));
            events.push(
                stream.next_record(wall_clock_timestamp_rfc3339(), paused_event("until_failed")),
            );
            persist_state_from_runtime(state, deduper, stream);
            NodeUntilHandle::Paused
        }
    }
}

fn next_retry_attempt(
    state: &mut EngineRunnerState,
    node_id: &str,
    retry: &RetryConfig,
) -> (u64, u64) {
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
        return NodeAssertOutcome::NotConfigured;
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
    let options = match prepare_node_expression_eval_options(node_obj, runtime) {
        Ok(options) => options,
        Err(error) => {
            return NodeAssertOutcome::Fail {
                message: format!("assert evaluation failed: {error}"),
                strategy,
            };
        }
    };
    let evaluated = match evaluate_value_ref_with_options(&assert_value_ref, &context, &options) {
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

fn resolve_assert_fail_strategy(
    plan: &PlanDocument,
    node_obj: &Map<String, Value>,
) -> AssertFailStrategy {
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

fn safety_hook_before_execute(
    node_obj: &Map<String, Value>,
    safety: &EngineSafetyOptions,
) -> Option<(String, String, Map<String, Value>)> {
    if safety.blocked_execution_types.is_empty() {
        return None;
    }
    let execution_type = node_obj
        .get("execution")
        .and_then(Value::as_object)
        .and_then(|execution| execution.get("type"))
        .and_then(Value::as_str)?;
    if !safety
        .blocked_execution_types
        .iter()
        .any(|value| value == execution_type)
    {
        return None;
    }
    let mut details = Map::new();
    details.insert(
        "execution_type".to_string(),
        Value::String(execution_type.to_string()),
    );
    details.insert(
        "blocked_execution_types".to_string(),
        Value::Array(
            safety
                .blocked_execution_types
                .iter()
                .cloned()
                .map(Value::String)
                .collect(),
        ),
    );
    Some((
        "safety_hook_execution_type_blocked".to_string(),
        "execution type is blocked by safety hook".to_string(),
        details,
    ))
}

fn sanitize_executor_output_value(value: &mut Value, safety: &EngineSafetyOptions) {
    match value {
        Value::Object(object) => {
            for (key, child) in object.iter_mut() {
                if is_sensitive_key(key) {
                    *child = Value::String("[REDACTED]".to_string());
                    continue;
                }
                sanitize_executor_output_value(child, safety);
            }
        }
        Value::Array(items) => {
            for item in items {
                sanitize_executor_output_value(item, safety);
            }
        }
        Value::String(text) => {
            let mut normalized = text.trim().to_string();
            if normalized.chars().count() > safety.max_output_string_chars {
                normalized = normalized
                    .chars()
                    .take(safety.max_output_string_chars)
                    .collect::<String>();
                normalized.push_str("...");
            }
            *text = normalized;
        }
        _ => {}
    }
}

fn is_sensitive_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "private_key"
            | "mnemonic"
            | "seed"
            | "api_key"
            | "authorization"
            | "password"
            | "secret"
            | "access_token"
            | "refresh_token"
            | "signature"
    )
}

fn contains_prompt_injection_pattern(value: &Value) -> bool {
    match value {
        Value::String(text) => contains_prompt_injection_text(text),
        Value::Array(items) => items.iter().any(contains_prompt_injection_pattern),
        Value::Object(object) => object.values().any(contains_prompt_injection_pattern),
        _ => false,
    }
}

fn contains_prompt_injection_text(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("ignore previous instructions")
        || lower.contains("ignore all previous")
        || lower.contains("system prompt")
        || lower.contains("developer message")
        || lower.contains("tool_call")
        || lower.contains("<script")
}

fn insert_unique_sorted(list: &mut Vec<String>, value: String) {
    if !list.iter().any(|item| item == &value) {
        list.push(value);
        list.sort();
    }
}

fn apply_user_input_command(runtime: &mut Value, data: &Map<String, Value>) -> Result<(), String> {
    let input_id = data
        .get("input_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "data.input_id is required".to_string())?;
    let value = data
        .get("value")
        .cloned()
        .ok_or_else(|| "data.value is required".to_string())?;
    let target_path = resolve_user_input_target_path(input_id, data.get("target_path"))?;
    set_runtime_path(runtime, target_path.as_str(), value);
    Ok(())
}

fn apply_user_select_command(runtime: &mut Value, data: &Map<String, Value>) -> Result<(), String> {
    let input_id = data
        .get("input_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "data.input_id is required".to_string())?;
    let selected = if let Some(value) = data.get("selected_value") {
        value.clone()
    } else {
        let selected_index = data
            .get("selected_index")
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .filter(|value| *value > 0)
            .ok_or_else(|| {
                "data.selected_value is required, or data.selected_index (>=1) with data.options"
                    .to_string()
            })?;
        let options = data
            .get("options")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                "data.options must be an array when selected_index is used".to_string()
            })?;
        if selected_index > options.len() {
            return Err(format!(
                "data.selected_index {} out of range (options={})",
                selected_index,
                options.len()
            ));
        }
        let option = &options[selected_index - 1];
        if let Some(value) = option.get("value") {
            value.clone()
        } else if let Some(label) = option.get("label").and_then(Value::as_str) {
            Value::String(label.to_string())
        } else if let Some(text) = option.as_str() {
            Value::String(text.to_string())
        } else {
            return Err("selected option must be string or object with label/value".to_string());
        }
    };
    let target_path = resolve_user_input_target_path(input_id, data.get("target_path"))?;
    set_runtime_path(runtime, target_path.as_str(), selected);
    Ok(())
}

fn resolve_user_input_target_path(
    input_id: &str,
    raw_target_path: Option<&Value>,
) -> Result<String, String> {
    let target_path = raw_target_path
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("inputs.{input_id}"));
    if !target_path.starts_with("inputs.") {
        return Err("target_path must start with `inputs.`".to_string());
    }
    Ok(target_path)
}

fn is_user_input_pause_reason(paused_reason: Option<&str>) -> bool {
    match paused_reason {
        Some("missing_required_input") => true,
        Some(reason) => reason.starts_with("need_user_input:"),
        None => false,
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

fn sync_progress_sets_to_state(
    state: &mut EngineRunnerState,
    completed_set: &BTreeSet<String>,
    approved_set: &BTreeSet<String>,
) {
    state.completed_node_ids = completed_set.iter().cloned().collect();
    state.approved_node_ids = approved_set.iter().cloned().collect();
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
    let parts = path
        .split('.')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
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
    let parts = path
        .split('.')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
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
    let target = object
        .entry(key)
        .or_insert_with(|| Value::Object(Map::new()));
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
}

fn node_ready_event(node_id: &str, has_condition: bool, control_kind: Option<&str>) -> EngineEvent {
    let mut event = EngineEvent::new(EngineEventType::NodeReady);
    event.node_id = Some(node_id.to_string());
    if has_condition {
        annotate_condition_check(&mut event, true, control_kind);
    }
    event
}

fn set_latest_node_ready_gate_result(
    events: &mut [EngineEventRecord],
    node_id: &str,
    control_kind: Option<&str>,
) {
    if let Some(record) = events.iter_mut().rev().find(|record| {
        record.event.event_type == EngineEventType::NodeReady
            && record.event.node_id.as_deref() == Some(node_id)
    }) {
        annotate_gate_check(&mut record.event, true, None, control_kind);
    }
}

fn set_latest_node_ready_assert_result(
    events: &mut [EngineEventRecord],
    node_id: &str,
    phase: &str,
    control_kind: Option<&str>,
) {
    if let Some(record) = events.iter_mut().rev().find(|record| {
        record.event.event_type == EngineEventType::NodeReady
            && record.event.node_id.as_deref() == Some(node_id)
    }) {
        annotate_assert_check(&mut record.event, true, phase, control_kind);
    }
}

fn control_step_kind(node_obj: &Map<String, Value>) -> Option<&str> {
    node_obj
        .get("extensions")
        .and_then(Value::as_object)
        .and_then(|extensions| extensions.get("control"))
        .and_then(Value::as_object)
        .and_then(|control| control.get("step_kind"))
        .and_then(Value::as_str)
}

fn annotate_condition_check(event: &mut EngineEvent, result: bool, control_kind: Option<&str>) {
    let mut fields = Map::new();
    fields.insert("result".to_string(), Value::Bool(result));
    if let Some(control_kind) = control_kind {
        fields.insert(
            "control_kind".to_string(),
            Value::String(control_kind.to_string()),
        );
    }
    insert_event_check_fields(event, "condition", fields);
}

fn annotate_gate_check(
    event: &mut EngineEvent,
    result: bool,
    reason_code: Option<&str>,
    control_kind: Option<&str>,
) {
    let mut fields = Map::new();
    fields.insert("result".to_string(), Value::Bool(result));
    if let Some(reason_code) = reason_code {
        fields.insert(
            "reason_code".to_string(),
            Value::String(reason_code.to_string()),
        );
    }
    if let Some(control_kind) = control_kind {
        fields.insert(
            "control_kind".to_string(),
            Value::String(control_kind.to_string()),
        );
    }
    insert_event_check_fields(event, "gate", fields);
}

fn annotate_assert_check(
    event: &mut EngineEvent,
    result: bool,
    phase: &str,
    control_kind: Option<&str>,
) {
    let mut fields = Map::new();
    fields.insert("result".to_string(), Value::Bool(result));
    fields.insert("phase".to_string(), Value::String(phase.to_string()));
    if let Some(control_kind) = control_kind {
        fields.insert(
            "control_kind".to_string(),
            Value::String(control_kind.to_string()),
        );
    }
    insert_event_check_fields(event, "assert", fields);
}

fn insert_event_check_fields(
    event: &mut EngineEvent,
    check_name: &str,
    fields: Map<String, Value>,
) {
    let checks = event
        .data
        .entry("checks".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if !checks.is_object() {
        *checks = Value::Object(Map::new());
    }
    let Some(checks_obj) = checks.as_object_mut() else {
        return;
    };
    checks_obj.insert(
        "schema".to_string(),
        Value::String(ENGINE_EVENT_CHECKS_SCHEMA_0_0_1.to_string()),
    );
    checks_obj.insert(check_name.to_string(), Value::Object(fields));
}

fn normalize_side_effect_record(
    record: &mut CheckpointSideEffectRecord,
    node_id: &str,
    chain: Option<&str>,
    execution_type: Option<&str>,
) -> bool {
    if record.schema.is_none() {
        record.schema = Some(SIDE_EFFECT_RECORD_SCHEMA_0_1_0.to_string());
    }
    if record.node_id.trim().is_empty() {
        record.node_id = node_id.to_string();
    }
    if record.chain.is_none() {
        record.chain = chain.map(str::to_string);
    }
    if record.execution_type.is_none() {
        record.execution_type = execution_type.map(str::to_string);
    }
    if record.effect_type.trim().is_empty() {
        record.effect_type = "tx".to_string();
    }
    record.status = canonical_side_effect_status(record.status.as_str()).to_string();
    if record.status.trim().is_empty() {
        record.status = SIDE_EFFECT_STATUS_UNKNOWN.to_string();
    }
    if record.observed_at.trim().is_empty() {
        record.observed_at = wall_clock_timestamp_rfc3339();
    }

    record
        .chain
        .as_ref()
        .is_some_and(|value| !value.trim().is_empty())
        && record
            .execution_type
            .as_ref()
            .is_some_and(|value| !value.trim().is_empty())
        && !record.effect_type.trim().is_empty()
        && !record.idempotency_key.trim().is_empty()
        && !record.status.trim().is_empty()
        && !record.observed_at.trim().is_empty()
}

fn side_effect_observed_event(node_id: &str, record: &CheckpointSideEffectRecord) -> EngineEvent {
    let mut event = EngineEvent::new(EngineEventType::SideEffectObserved);
    event.node_id = Some(node_id.to_string());
    if let Ok(value) = serde_json::to_value(record) {
        event.data.insert("record".to_string(), value);
    }
    event
}

fn need_user_confirm_event(
    node_id: &str,
    reason_code: &str,
    reason: &str,
    details: &Map<String, Value>,
    control_kind: Option<&str>,
) -> EngineEvent {
    let mut event = EngineEvent::new(EngineEventType::NeedUserConfirm);
    event.node_id = Some(node_id.to_string());
    event.data.insert(
        "reason_code".to_string(),
        Value::String(reason_code.to_string()),
    );
    event
        .data
        .insert("reason".to_string(), Value::String(reason.to_string()));
    event
        .data
        .insert("details".to_string(), Value::Object(details.clone()));
    annotate_gate_check(&mut event, false, Some(reason_code), control_kind);
    event
}

fn extract_action_ref_from_node(node_obj: &Map<String, Value>) -> Option<String> {
    let source = node_obj.get("source").and_then(Value::as_object)?;
    let protocol = source
        .get("protocol")
        .and_then(Value::as_str)
        .map(str::to_string);

    if let Some(action) = source.get("action").and_then(Value::as_str) {
        return Some(match protocol {
            Some(protocol) => format!("action:{protocol}/{action}"),
            None => format!("action:{action}"),
        });
    }
    if let Some(query) = source.get("query").and_then(Value::as_str) {
        return Some(match protocol {
            Some(protocol) => format!("query:{protocol}/{query}"),
            None => format!("query:{query}"),
        });
    }

    None
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RiskLevelObservation {
    risk_level: Option<u8>,
    source: &'static str,
    unknown_cause_code: Option<&'static str>,
    unknown_cause: Option<&'static str>,
}

fn observe_risk_level_from_node(node_obj: &Map<String, Value>) -> RiskLevelObservation {
    let Some(extensions) = node_obj.get("extensions") else {
        return RiskLevelObservation {
            risk_level: None,
            source: "unknown",
            unknown_cause_code: Some("risk_level_input_missing"),
            unknown_cause: Some("node.extensions is missing"),
        };
    };

    let Some(extensions_obj) = extensions.as_object() else {
        return RiskLevelObservation {
            risk_level: None,
            source: "unknown",
            unknown_cause_code: Some("risk_level_parse_failed"),
            unknown_cause: Some("node.extensions is not an object"),
        };
    };

    let Some(risk_level_value) = extensions_obj.get("risk_level") else {
        return RiskLevelObservation {
            risk_level: None,
            source: "unknown",
            unknown_cause_code: Some("risk_level_input_missing"),
            unknown_cause: Some("node.extensions.risk_level is missing"),
        };
    };

    let Some(raw) = risk_level_value.as_u64() else {
        return RiskLevelObservation {
            risk_level: None,
            source: "unknown",
            unknown_cause_code: Some("risk_level_parse_failed"),
            unknown_cause: Some("node.extensions.risk_level must be an integer in [1,5]"),
        };
    };

    if !(1..=5).contains(&raw) {
        return RiskLevelObservation {
            risk_level: None,
            source: "unknown",
            unknown_cause_code: Some("risk_level_parse_failed"),
            unknown_cause: Some("node.extensions.risk_level is out of allowed range [1,5]"),
        };
    }

    RiskLevelObservation {
        risk_level: u8::try_from(raw).ok(),
        source: "extensions",
        unknown_cause_code: None,
        unknown_cause: None,
    }
}

fn extract_risk_tags_from_node(node_obj: &Map<String, Value>) -> Vec<String> {
    let Some(tags) = node_obj
        .get("extensions")
        .and_then(Value::as_object)
        .and_then(|extensions| extensions.get("risk_tags"))
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };
    tags.iter()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .filter(|tag| !tag.trim().is_empty())
        .collect()
}

fn need_user_confirm_fields(
    gate_input: &PolicyGateInput,
    gate_output: &PolicyGateOutput,
    fallback_action_ref: Option<&str>,
    risk_observation: Option<&RiskLevelObservation>,
    default_risk_source: &str,
) -> (String, String, Map<String, Value>) {
    let (reason_code, reason, mut details) = match gate_output {
        PolicyGateOutput::NeedUserConfirm {
            reason_code,
            reason,
            details,
        } => (
            reason_code.as_str().to_string(),
            reason.clone(),
            details.clone(),
        ),
        _ => (
            "need_user_confirm".to_string(),
            "manual review required".to_string(),
            Map::new(),
        ),
    };

    details.insert(
        "node_id".to_string(),
        Value::String(
            gate_input
                .node_id
                .clone()
                .unwrap_or_else(|| "unknown".to_string()),
        ),
    );

    let action_ref = gate_input
        .action_ref
        .as_deref()
        .or(fallback_action_ref)
        .unwrap_or("unknown")
        .to_string();
    details.insert("action_ref".to_string(), Value::String(action_ref));

    if !details.contains_key("hit_reasons") {
        details.insert(
            "hit_reasons".to_string(),
            Value::Array(vec![Value::String(reason_code.clone())]),
        );
    }
    if !details.contains_key("risk_source") {
        let risk_source = if gate_input.risk_level.is_some() {
            "extensions"
        } else if let Some(observation) = risk_observation {
            match observation.source {
                "extensions" => "extensions",
                _ => default_risk_source,
            }
        } else {
            default_risk_source
        };
        details.insert(
            "risk_source".to_string(),
            Value::String(risk_source.to_string()),
        );
    }
    if reason_code == PolicyGateReasonCode::ThresholdRiskLevelUnknown.as_str() {
        if !details.contains_key("risk_level_unknown_cause_code") {
            let cause_code = risk_observation
                .and_then(|observation| observation.unknown_cause_code)
                .unwrap_or("risk_level_input_missing");
            details.insert(
                "risk_level_unknown_cause_code".to_string(),
                Value::String(cause_code.to_string()),
            );
        }
        if !details.contains_key("risk_level_unknown_cause") {
            let cause = risk_observation
                .and_then(|observation| observation.unknown_cause)
                .unwrap_or("node.extensions.risk_level is missing or invalid");
            details.insert(
                "risk_level_unknown_cause".to_string(),
                Value::String(cause.to_string()),
            );
        }
        details
            .entry("risk_level_expected_path".to_string())
            .or_insert_with(|| Value::String("extensions.risk_level".to_string()));
    }

    if !details.contains_key("confirmation_summary") || !details.contains_key("confirmation_hash") {
        if let Ok(enriched) = enrich_need_user_confirm_output(gate_input, gate_output) {
            if let PolicyGateOutput::NeedUserConfirm {
                details: enriched, ..
            } = enriched
            {
                if let Some(value) = enriched.get("confirmation_summary").cloned() {
                    details.insert("confirmation_summary".to_string(), value);
                }
                if let Some(value) = enriched.get("confirmation_hash").cloned() {
                    details.insert("confirmation_hash".to_string(), value);
                }
            }
        }
    }

    (reason_code, reason, details)
}

fn hard_block_event(
    node_id: &str,
    reason_code: &str,
    reason: &str,
    details: &Map<String, Value>,
    is_gate_result: bool,
    control_kind: Option<&str>,
) -> EngineEvent {
    let mut event = EngineEvent::new(EngineEventType::Error);
    event.node_id = Some(node_id.to_string());
    event.data.insert(
        "reason_code".to_string(),
        Value::String(reason_code.to_string()),
    );
    event
        .data
        .insert("reason".to_string(), Value::String(reason.to_string()));
    event
        .data
        .insert("details".to_string(), Value::Object(details.clone()));
    if is_gate_result {
        annotate_gate_check(&mut event, false, Some(reason_code), control_kind);
    }
    event
}

fn executor_error_event(node_id: &str, error: &RouterExecuteError) -> EngineEvent {
    let mut event = EngineEvent::new(EngineEventType::Error);
    event.node_id = Some(node_id.to_string());
    event.data.insert(
        "reason_code".to_string(),
        Value::String("executor_error".to_string()),
    );
    event
        .data
        .insert("reason".to_string(), Value::String(error.to_string()));
    event
}

fn paused_event(reason: &str) -> EngineEvent {
    let mut event = EngineEvent::new(EngineEventType::EnginePaused);
    event
        .data
        .insert("reason_code".to_string(), Value::String(reason.to_string()));
    event
        .data
        .insert("reason".to_string(), Value::String(reason.to_string()));
    event
}

fn need_user_input_event(reason_code: &str, reason: &str, command_id: &str) -> EngineEvent {
    let mut details = Map::new();
    details.insert(
        "command_id".to_string(),
        Value::String(command_id.to_string()),
    );
    need_user_input_event_with_details(None, reason_code, reason, &details)
}

fn need_user_input_event_with_details(
    node_id: Option<&str>,
    reason_code: &str,
    reason: &str,
    details: &Map<String, Value>,
) -> EngineEvent {
    let mut event = EngineEvent::new(EngineEventType::NeedUserInput);
    event.node_id = node_id.map(str::to_string);
    event.data.insert(
        "reason_code".to_string(),
        Value::String(reason_code.to_string()),
    );
    event
        .data
        .insert("reason".to_string(), Value::String(reason.to_string()));
    event
        .data
        .insert("details".to_string(), Value::Object(details.clone()));
    event
}

fn need_user_input_fields(
    node_id: &str,
    action_ref: Option<&str>,
    reason: &str,
    details: &Map<String, Value>,
) -> (String, Map<String, Value>) {
    let mut out = details.clone();
    out.entry("node_id".to_string())
        .or_insert_with(|| Value::String(node_id.to_string()));
    if let Some(action_ref) = action_ref {
        out.entry("action_ref".to_string())
            .or_insert_with(|| Value::String(action_ref.to_string()));
    }
    let reason_code = match reason {
        "missing_inputs_or_runtime_refs" => "missing_required_input",
        _ => "need_user_input",
    };
    if reason_code == "missing_required_input" {
        out = normalize_missing_required_input_details(&out);
    }
    (reason_code.to_string(), out)
}

fn should_route_confirm_to_missing_required_input(
    reason_code: &str,
    details: &Map<String, Value>,
) -> bool {
    if reason_code == PolicyGateReasonCode::MissingFields.as_str() {
        return true;
    }
    if reason_code == PolicyGateReasonCode::UnknownFields.as_str() {
        return has_non_empty_string_array(details, "missing_refs")
            || has_non_empty_string_array(details, "missing_fields")
            || has_non_empty_string_array(details, "unknown_fields")
            || has_non_empty_array(details, "questions");
    }
    has_non_empty_string_array(details, "missing_refs")
        || has_non_empty_string_array(details, "missing_fields")
}

fn has_non_empty_string_array(details: &Map<String, Value>, key: &str) -> bool {
    details
        .get(key)
        .and_then(Value::as_array)
        .is_some_and(|items| {
            items.iter().any(|item| {
                item.as_str()
                    .map(|text| !text.trim().is_empty())
                    .unwrap_or(false)
            })
        })
}

fn has_non_empty_array(details: &Map<String, Value>, key: &str) -> bool {
    details
        .get(key)
        .and_then(Value::as_array)
        .is_some_and(|items| !items.is_empty())
}

fn normalize_missing_required_input_details(details: &Map<String, Value>) -> Map<String, Value> {
    let mut out = details.clone();
    let mut missing_refs = BTreeSet::<String>::new();
    append_string_array_field(&mut missing_refs, details.get("missing_refs"));
    append_missing_field_refs(&mut missing_refs, details.get("missing_fields"));
    append_missing_field_refs(&mut missing_refs, details.get("unknown_fields"));

    let missing_refs_vec = missing_refs.into_iter().collect::<Vec<_>>();
    out.insert(
        "missing_refs".to_string(),
        Value::Array(
            missing_refs_vec
                .iter()
                .cloned()
                .map(Value::String)
                .collect::<Vec<_>>(),
        ),
    );

    if !out.contains_key("suggested_paths") {
        out.insert(
            "suggested_paths".to_string(),
            Value::Array(
                build_missing_input_suggested_paths(missing_refs_vec.as_slice())
                    .into_iter()
                    .map(Value::String)
                    .collect::<Vec<_>>(),
            ),
        );
    }

    let has_questions = out
        .get("questions")
        .and_then(Value::as_array)
        .is_some_and(|items| !items.is_empty());
    if !has_questions {
        out.insert(
            "questions".to_string(),
            Value::Array(build_missing_input_questions(missing_refs_vec.as_slice())),
        );
    }

    out
}

fn append_string_array_field(output: &mut BTreeSet<String>, value: Option<&Value>) {
    let Some(items) = value.and_then(Value::as_array) else {
        return;
    };
    for item in items {
        let Some(text) = item.as_str().map(str::trim) else {
            continue;
        };
        if text.is_empty() {
            continue;
        }
        output.insert(text.to_string());
    }
}

fn append_missing_field_refs(output: &mut BTreeSet<String>, value: Option<&Value>) {
    let Some(items) = value.and_then(Value::as_array) else {
        return;
    };
    for item in items {
        let Some(field) = item.as_str().map(str::trim) else {
            continue;
        };
        if field.is_empty() {
            continue;
        }
        if field.contains('.') {
            output.insert(field.to_string());
            continue;
        }
        output.insert(format!("inputs.{field}"));
        output.insert(format!("params.{field}"));
    }
}

fn build_missing_input_suggested_paths(missing_refs: &[String]) -> Vec<String> {
    let mut out = BTreeSet::<String>::new();
    for reference in missing_refs {
        let reference = reference.trim();
        if reference.is_empty() {
            continue;
        }
        out.insert(reference.to_string());
        if let Some(slot) = missing_slot_id_from_ref(reference) {
            out.insert(format!("inputs.{slot}"));
            out.insert(format!("params.{slot}"));
        }
    }
    out.into_iter().collect::<Vec<_>>()
}

fn build_missing_input_questions(missing_refs: &[String]) -> Vec<Value> {
    let mut slots = BTreeSet::<String>::new();
    for reference in missing_refs {
        if let Some(slot) = missing_slot_id_from_ref(reference.as_str()) {
            slots.insert(slot);
        }
    }
    slots
        .into_iter()
        .map(|slot| {
            json!({
                "id": slot,
                "question": format!("Provide value for `{slot}`."),
                "required": true,
                "options": []
            })
        })
        .collect::<Vec<_>>()
}

fn missing_slot_id_from_ref(reference: &str) -> Option<String> {
    let trimmed = reference.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(slot) = trimmed.strip_prefix("inputs.") {
        return (!slot.trim().is_empty()).then_some(slot.to_string());
    }
    if let Some(slot) = trimmed.strip_prefix("params.") {
        return (!slot.trim().is_empty()).then_some(slot.to_string());
    }
    if let Some(slot) = trimmed.rsplit('.').next() {
        let slot = slot.trim();
        if !slot.is_empty() {
            return Some(slot.to_string());
        }
    }
    Some(trimmed.to_string())
}

fn preflight_simulated_event(node_id: &str) -> EngineEvent {
    let mut event = EngineEvent::new(EngineEventType::Skipped);
    event.node_id = Some(node_id.to_string());
    event.data.insert(
        "reason_code".to_string(),
        Value::String("preflight_simulate".to_string()),
    );
    event.data.insert(
        "reason".to_string(),
        Value::String("preflight_simulate".to_string()),
    );
    event
}

fn condition_skipped_event(node_id: &str, control_kind: Option<&str>) -> EngineEvent {
    let mut event = EngineEvent::new(EngineEventType::Skipped);
    event.node_id = Some(node_id.to_string());
    event.data.insert(
        "reason_code".to_string(),
        Value::String("condition_false".to_string()),
    );
    event.data.insert(
        "reason".to_string(),
        Value::String("condition_false".to_string()),
    );
    annotate_condition_check(&mut event, false, control_kind);
    event
}

fn condition_failed_event(node_id: &str, message: &str, control_kind: Option<&str>) -> EngineEvent {
    let mut event = EngineEvent::new(EngineEventType::Error);
    event.node_id = Some(node_id.to_string());
    event.data.insert(
        "reason_code".to_string(),
        Value::String("condition_failed".to_string()),
    );
    event.data.insert(
        "reason".to_string(),
        Value::String("condition_failed".to_string()),
    );
    event
        .data
        .insert("message".to_string(), Value::String(message.to_string()));
    annotate_condition_check(&mut event, false, control_kind);
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
    event.data.insert(
        "reason_code".to_string(),
        Value::String("until_retry".to_string()),
    );
    event.data.insert(
        "reason".to_string(),
        Value::String("until_retry".to_string()),
    );
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
    event.data.insert(
        "reason_code".to_string(),
        Value::String("until_failed".to_string()),
    );
    event.data.insert(
        "reason".to_string(),
        Value::String("until_failed".to_string()),
    );
    event
        .data
        .insert("message".to_string(), Value::String(message.to_string()));
    event
}

fn retry_exhausted_event(node_id: &str, attempt: u64, retry: &RetryConfig) -> EngineEvent {
    let mut event = EngineEvent::new(EngineEventType::Error);
    event.node_id = Some(node_id.to_string());
    event.data.insert(
        "reason_code".to_string(),
        Value::String("retry_exhausted".to_string()),
    );
    event.data.insert(
        "reason".to_string(),
        Value::String("retry_exhausted".to_string()),
    );
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
    event.data.insert(
        "reason_code".to_string(),
        Value::String("retry_timeout".to_string()),
    );
    event.data.insert(
        "reason".to_string(),
        Value::String("retry_timeout".to_string()),
    );
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
    control_kind: Option<&str>,
) -> EngineEvent {
    let mut event = EngineEvent::new(EngineEventType::Error);
    event.node_id = Some(node_id.to_string());
    event.data.insert(
        "reason_code".to_string(),
        Value::String("assert_failed".to_string()),
    );
    event.data.insert(
        "reason".to_string(),
        Value::String("assert_failed".to_string()),
    );
    event
        .data
        .insert("message".to_string(), Value::String(message.to_string()));
    event
        .data
        .insert("phase".to_string(), Value::String(phase.to_string()));
    if let Some(assert_expr) = assert_expr {
        event.data.insert("assert".to_string(), assert_expr.clone());
    }
    annotate_assert_check(&mut event, false, phase, control_kind);
    event
}

fn node_paused_event(node_id: &str, reason: &str) -> EngineEvent {
    let mut event = EngineEvent::new(EngineEventType::NodePaused);
    event.node_id = Some(node_id.to_string());
    event
        .data
        .insert("reason_code".to_string(), Value::String(reason.to_string()));
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
