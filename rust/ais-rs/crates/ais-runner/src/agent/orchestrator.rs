use super::context_view::PlanningContextManager;
use super::*;
use ais_engine::{
    DefaultSolver, EngineEventRecord, EngineEventType, EngineRunStatus, EngineRunnerOptions,
    EngineRunnerState,
};
use ais_sdk::documents::PlanSketchSegment;
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;

const MAX_PLANNER_OUTPUT_REPAIR_RETRIES: usize = 2;
const GROUND_INPUT_CONFIDENCE_THRESHOLD: u8 = 80;
const GROUND_FACT_CONFIDENCE_THRESHOLD: u8 = 65;
const TOOL_MEMORY_PROJECTION_MIN_TOKENS: usize = 1200;
const TOOL_MEMORY_PROJECTION_DEFAULT_TOKENS: usize = 2400;
const TOOL_MEMORY_PROJECTION_MAX_TOKENS: usize = 6000;
const TOOL_MEMORY_PROJECTION_TIGHT_THRESHOLD_BPS: u64 = 2000;
const TOOL_MEMORY_PROJECTION_RELAXED_THRESHOLD_BPS: u64 = 6000;
const TOOL_MEMORY_REMAINING_ABS_MIN: u64 = 4_000;
const TOOL_MEMORY_REMAINING_ABS_MAX: u64 = 24_000;

#[derive(Debug, Clone)]
struct PlannedSegment {
    todo_id: String,
    summary: Option<String>,
    segment: PlanSketchSegment,
    cursor_next: String,
    done: bool,
    issues: Vec<Value>,
}

#[derive(Debug, Clone)]
struct ExecuteRoundOutcome {
    status: EngineRunStatus,
    iterations: usize,
    round_events: Vec<EngineEventRecord>,
    last_iteration_events: Vec<EngineEventRecord>,
}

#[derive(Debug)]
pub(super) struct SegmentedAgentContext {
    intent: String,
    session: intent_segmented::SegmentPlanningSession,
    fact_store: FactStore,
    todo_board: TodoBoard,
    state_summary: Option<Value>,
    previous_error: Option<Value>,
    last_segment: Option<PlanSketchSegment>,
    completed_segments: usize,
    final_status: EngineRunStatus,
    planning_rounds: usize,
    planner_output_retries: usize,
    planner_round_limit: usize,
    segment_limit: usize,
    context_manager: PlanningContextManager,
    tool_memory_projection: Option<Value>,
    checkpoint_extensions: checkpoint_ext::AgentCheckpointExtensions,
}

impl SegmentedAgentContext {
    fn new(
        intent: String,
        session: intent_segmented::SegmentPlanningSession,
        fact_store: FactStore,
        todo_board: TodoBoard,
        planner_round_limit: usize,
        segment_limit: usize,
        planner_context_token_budget: usize,
        checkpoint_extensions: checkpoint_ext::AgentCheckpointExtensions,
    ) -> Self {
        Self {
            intent,
            session,
            fact_store,
            todo_board,
            state_summary: None,
            previous_error: None,
            last_segment: None,
            completed_segments: 0,
            final_status: EngineRunStatus::Completed,
            planning_rounds: 0,
            planner_output_retries: 0,
            planner_round_limit,
            segment_limit,
            context_manager: PlanningContextManager::with_token_budget(
                planner_context_token_budget,
            ),
            tool_memory_projection: None,
            checkpoint_extensions,
        }
    }

    fn can_continue(&self) -> bool {
        self.completed_segments < self.segment_limit
    }

    fn refresh_state_summary(&mut self, state: &EngineRunnerState, done: bool) {
        self.state_summary = Some(self.context_manager.next_summary(
            state,
            self.completed_segments,
            done,
            self.previous_error.as_ref(),
            Some(&self.fact_store),
            self.tool_memory_projection.as_ref(),
        ));
    }

    fn update_tool_memory_projection(&mut self, projection: Option<Value>) {
        self.tool_memory_projection = projection;
    }

    fn set_previous_error_and_refresh(
        &mut self,
        state: &EngineRunnerState,
        done: bool,
        error: Value,
    ) {
        self.previous_error = Some(error);
        self.refresh_state_summary(state, done);
    }
}

pub(super) fn execute_segmented_intent_agent(
    command: &AgentCommand,
    config: &RunnerConfig,
    pack: Option<&ais_sdk::PackDocument>,
    candidate_context: Option<CandidateContext>,
    prompt_catalog: &PromptCatalog,
) -> Result<String, RunnerError> {
    let intent = super::resolve_intent_text(command)?;
    let candidate_context = candidate_context.ok_or_else(|| {
        RunnerError::Llm(
            "intent segmented mode requires `--workspace` with protocol documents".to_string(),
        )
    })?;

    let provider = super::load_llm_provider(command, config)?.ok_or_else(|| {
        RunnerError::Llm(
            "intent mode requires configured llm provider (runner config `llm` or demo script)"
                .to_string(),
        )
    })?;
    let segmented_prompt_overrides = super::load_segmented_prompt_overrides(prompt_catalog);
    let llm_context_limit_tokens = super::resolve_llm_context_limit_tokens(config);
    let segmented_max_tool_rounds = super::resolve_segmented_max_tool_rounds(command, config);
    let mut planner = LlmSegmentedIntentPlanner::new(provider)
        .with_candidate_context(Some(candidate_context.clone()))
        .with_prompt_overrides(segmented_prompt_overrides)
        .with_max_tool_rounds(segmented_max_tool_rounds)
        .with_context_limit_tokens(llm_context_limit_tokens)
        .with_verbose_llm(command.verbose_llm);
    if command.verbose_llm {
        eprintln!("[agent] segmented_max_tool_rounds={segmented_max_tool_rounds}");
    }

    let runtime = match &command.runtime {
        Some(path) => {
            let runtime_text =
                std::fs::read_to_string(path).map_err(|source| RunnerError::ReadFile {
                    path: path.display().to_string(),
                    source,
                })?;
            super::parse_runtime_value(runtime_text.as_str())?
        }
        None => Value::Object(serde_json::Map::new()),
    };

    let pack_snapshot_hash = super::derive_pack_snapshot_hash(pack)?;
    let catalog_hash = candidate_context.executable_candidates.catalog_hash.clone();
    let chain_scope = super::derive_chain_scope(&candidate_context);
    let snapshot_hash = super::derive_planning_snapshot_hash(
        pack_snapshot_hash.as_str(),
        catalog_hash.as_str(),
        chain_scope.as_slice(),
        command.approvals_mode,
    )?;
    let mut session = planner.begin_session(SegmentBeginRequest {
        intent: intent.clone(),
        pack_snapshot_hash,
        catalog_hash,
        chain_scope: chain_scope.clone(),
    })?;
    if session.snapshot_hash != snapshot_hash {
        if command.verbose_llm {
            eprintln!(
                "[llm] segmented planner snapshot_hash={} (host expects {})",
                session.snapshot_hash, snapshot_hash
            );
        }
        session.snapshot_hash = snapshot_hash;
    }

    let mut active_plan = super::empty_plan_document();
    let mut active_plan_hash = super::hash_plan(&active_plan)?;
    let run_id = format!(
        "run-{}",
        active_plan_hash
            .get(0..12)
            .unwrap_or(active_plan_hash.as_str())
    );
    let (
        mut state,
        resumed_from_checkpoint,
        checkpoint_plan,
        checkpoint_plan_hash,
        mut checkpoint_ledger,
        checkpoint_extensions,
    ) = super::load_or_init_state(command, &active_plan_hash, runtime)?;
    let checkpoint_extensions = super::decode_agent_checkpoint_extensions(
        &mut state.runtime,
        checkpoint_extensions.as_ref(),
        command.verbose_llm,
    );
    if let Some(memory) = checkpoint_extensions.planning_memory() {
        let restored = planner.restore_planning_memory_from_checkpoint(Some(memory));
        if command.verbose_llm {
            eprintln!("[checkpoint] planning_memory restored={restored}");
        }
    }
    if let Some(plan) = checkpoint_plan {
        active_plan = plan;
        active_plan_hash = checkpoint_plan_hash.unwrap_or(super::hash_plan(&active_plan)?);
    }
    let mut fact_store =
        super::build_initial_fact_store(&state.runtime, config, chain_scope.as_slice())?;
    if let Some(restored) = checkpoint_extensions.fact_store() {
        fact_store.merge(restored);
    }
    if let Some(intent_facts) = checkpoint_extensions.intent_facts() {
        for (key, value) in intent_facts {
            fact_store.upsert(
                key.clone(),
                value.clone(),
                FactLayer::Seed,
                FactSource::IntentInferred,
                format!("checkpoint.intent_facts.{key}"),
            );
        }
    }
    super::record_runtime_agent_field(
        &mut state.runtime,
        "capability_view",
        candidate_context.capability_view(),
    );
    let capability_ready = capability_view_ready(&state);
    super::record_runtime_agent_field(
        &mut state.runtime,
        "capability_ready",
        Value::Bool(capability_ready),
    );
    super::record_side_effect_lifecycle(&mut state.runtime, &checkpoint_ledger);
    let runtime_has_intent_grounding = state.runtime.pointer("/agent/intent_grounding").is_some();
    let runtime_has_todo_progress = state.runtime.pointer("/agent/todo_progress").is_some();
    let mut todo_board = TodoBoard::restore_or_bootstrap(&state.runtime, intent.as_str());
    todo_board.ensure_current();
    super::record_todo_progress(&mut state.runtime, &todo_board);

    let initial_router = build_router_executor_for_plan(&active_plan, config)
        .map_err(RunnerError::ConfigInvalidForPlan)?;
    if resumed_from_checkpoint {
        if let Some(paused_reason) = super::reconcile_pending_side_effects(
            &mut checkpoint_ledger,
            &initial_router,
            &mut state,
        ) {
            super::record_side_effect_lifecycle(&mut state.runtime, &checkpoint_ledger);
            state.paused_reason = Some(paused_reason);
            checkpoint_round(
                command,
                run_id.as_str(),
                &active_plan_hash,
                &active_plan,
                &state,
                &checkpoint_ledger,
                planner.planning_memory_checkpoint_value(),
                &fact_store,
                &checkpoint_extensions,
            )?;
            record_planner_llm_usage(&mut state, &planner);
            return super::render_agent_output(
                command,
                &state,
                EngineRunStatus::Paused,
                0,
                0,
                resumed_from_checkpoint,
            );
        }
    }

    let derived_mode = command
        .approvals_mode
        .or_else(|| pack.and_then(approvals_mode_from_pack))
        .unwrap_or(crate::cli::ApprovalsMode::Safe);
    let assist_threshold = if derived_mode == crate::cli::ApprovalsMode::Assist {
        pack.and_then(llm_may_approve_max_risk_level_from_pack)
    } else {
        None
    };
    let mut decision_policy = super::build_decision_policy(
        command,
        config,
        derived_mode,
        assist_threshold,
        Some(candidate_context.clone()),
        prompt_catalog,
    )?;

    let mut engine_options = EngineRunnerOptions::default();
    if let Some(pack) = pack {
        engine_options.policy = policy_from_pack(pack)
            .map_err(|error| RunnerError::WorkspaceValidate(error.to_string()))?;
    }

    let mut total_events = 0usize;
    let mut total_iterations = 0usize;
    let mut command_builder = CommandBuilder::new(run_id.as_str());
    let planner_round_limit = command
        .max_planner_rounds
        .unwrap_or(session.max_rounds)
        .max(1);
    let planner_context_token_budget = super::resolve_planner_context_token_budget(command, config);
    if command.verbose_llm {
        eprintln!("[agent] planner_context_token_budget={planner_context_token_budget}");
    }
    let segment_limit = usize::from(session.max_segments.max(1));
    let mut context = SegmentedAgentContext::new(
        intent.clone(),
        session,
        fact_store,
        todo_board,
        usize::from(planner_round_limit),
        segment_limit,
        planner_context_token_budget,
        checkpoint_extensions,
    );
    refresh_tool_memory_projection(&mut context, &planner, &state);
    context.refresh_state_summary(&state, false);
    let grounding_ready = bootstrap_intent_grounding_if_needed(
        command,
        &mut planner,
        &mut state,
        &mut context,
        runtime_has_intent_grounding,
    )?;
    if !grounding_ready {
        checkpoint_round(
            command,
            run_id.as_str(),
            &active_plan_hash,
            &active_plan,
            &state,
            &checkpoint_ledger,
            planner.planning_memory_checkpoint_value(),
            &context.fact_store,
            &context.checkpoint_extensions,
        )?;
        record_planner_llm_usage(&mut state, &planner);
        return super::render_agent_output(
            command,
            &state,
            EngineRunStatus::Paused,
            0,
            0,
            resumed_from_checkpoint,
        );
    }
    bootstrap_todos_if_needed(
        command,
        &mut planner,
        &mut state,
        &mut context,
        runtime_has_todo_progress,
    )?;
    if state.paused_reason.as_deref() == Some("missing_required_input") {
        checkpoint_round(
            command,
            run_id.as_str(),
            &active_plan_hash,
            &active_plan,
            &state,
            &checkpoint_ledger,
            planner.planning_memory_checkpoint_value(),
            &context.fact_store,
            &context.checkpoint_extensions,
        )?;
        record_planner_llm_usage(&mut state, &planner);
        return super::render_agent_output(
            command,
            &state,
            EngineRunStatus::Paused,
            0,
            0,
            resumed_from_checkpoint,
        );
    }
    context.previous_error = None;
    refresh_tool_memory_projection(&mut context, &planner, &state);
    context.refresh_state_summary(&state, false);

    while context.can_continue() {
        context.todo_board.ensure_current();
        super::record_todo_progress(&mut state.runtime, &context.todo_board);

        let draft = plan_round(&mut planner, &state, &mut context)?;
        let current_todo_id = context
            .todo_board
            .current_todo_id()
            .ok_or_else(|| RunnerError::Llm("todo board has no current todo".to_string()))?
            .to_string();
        let planned_segment = match draft {
            SegmentDraft::Proposed {
                summary,
                segment,
                cursor_next,
                done,
                issues,
            } => PlannedSegment {
                todo_id: current_todo_id.clone(),
                summary,
                segment,
                cursor_next,
                done,
                issues,
            },
            SegmentDraft::Unavailable {
                reason_code,
                message,
                done,
                issues,
                questions,
            } => {
                if reason_code == "missing_required_input" {
                    let payload = super::missing_required_input_payload(
                        message.as_deref(),
                        questions.as_slice(),
                        issues.as_slice(),
                        context.completed_segments as u8,
                    );
                    if let Some(answers) =
                        super::maybe_collect_missing_input_answers(questions.as_slice())?
                    {
                        super::apply_missing_input_answers(
                            &mut state,
                            &mut context.fact_store,
                            &answers,
                        );
                        context.todo_board.mark_current_todo();
                        super::record_todo_progress(&mut state.runtime, &context.todo_board);
                        state.paused_reason = None;
                        context.set_previous_error_and_refresh(
                            &state,
                            done,
                            super::missing_required_input_resolved_payload(
                                &answers,
                                context.completed_segments as u8,
                            ),
                        );
                        checkpoint_round(
                            command,
                            run_id.as_str(),
                            &active_plan_hash,
                            &active_plan,
                            &state,
                            &checkpoint_ledger,
                            planner.planning_memory_checkpoint_value(),
                            &context.fact_store,
                            &context.checkpoint_extensions,
                        )?;
                        if command.verbose {
                            eprintln!(
                                "[agent] missing_required_input resolved via user answers keys={}",
                                answers.keys().cloned().collect::<Vec<_>>().join(",")
                            );
                        }
                        continue;
                    }
                    state.paused_reason = Some("missing_required_input".to_string());
                    context
                        .todo_board
                        .mark_current_blocked("missing_required_input");
                    super::record_todo_progress(&mut state.runtime, &context.todo_board);
                    super::record_missing_required_input(&mut state.runtime, &payload);
                    checkpoint_round(
                        command,
                        run_id.as_str(),
                        &active_plan_hash,
                        &active_plan,
                        &state,
                        &checkpoint_ledger,
                        planner.planning_memory_checkpoint_value(),
                        &context.fact_store,
                        &context.checkpoint_extensions,
                    )?;
                    context.final_status = EngineRunStatus::Paused;
                    break;
                }
                return Err(RunnerError::Llm(format!(
                    "segment unavailable reason_code={reason_code} done={done} issues={} questions={} message={}",
                    issues.len(),
                    questions.len(),
                    message.unwrap_or_default()
                )));
            }
            SegmentDraft::Invalid {
                reason_code,
                message,
                done,
                issues,
            } => {
                return Err(RunnerError::Llm(format!(
                    "segment invalid reason_code={reason_code} done={done} issues={} message={}",
                    issues.len(),
                    message.unwrap_or_default()
                )));
            }
        };
        let mut planned_segment = planned_segment;
        bind_segment_todo_id(
            &mut planned_segment.segment,
            planned_segment.todo_id.as_str(),
        );

        context.todo_board.mark_current_in_progress(
            planned_segment.summary.as_deref(),
            planned_segment.segment.segment_id.as_str(),
        );
        super::record_todo_progress(&mut state.runtime, &context.todo_board);
        if command.verbose_llm {
            eprintln!(
                "[agent] segment proposed id={} steps={} done={} cursor_next={} summary={}",
                planned_segment.segment.segment_id,
                planned_segment.segment.steps.len(),
                planned_segment.done,
                planned_segment.cursor_next,
                planned_segment.summary.clone().unwrap_or_default()
            );
            if !planned_segment.issues.is_empty() {
                eprintln!(
                    "[agent] segment issues={}",
                    Value::Array(planned_segment.issues.clone())
                );
            }
        }

        let segment_plan = match compile_guard(
            &planned_segment,
            &context,
            &candidate_context,
            pack,
            chain_scope.as_slice(),
        ) {
            Ok(plan) => plan,
            Err(error_payload) => {
                eprintln!(
                    "[agent] compile_guard_failed segment_id={} reason={}",
                    planned_segment.segment.segment_id,
                    compile_error_compact(&error_payload)
                );
                context.set_previous_error_and_refresh(
                    &state,
                    planned_segment.done,
                    super::compile_error_state_payload(
                        &error_payload,
                        context.completed_segments as u8,
                    ),
                );
                continue;
            }
        };

        let execute_outcome = execute_round(
            command,
            run_id.as_str(),
            config,
            &engine_options,
            &mut decision_policy,
            &mut command_builder,
            &mut checkpoint_ledger,
            &mut state,
            &mut active_plan,
            &mut active_plan_hash,
            &planned_segment.segment,
            &segment_plan,
            planner.planning_memory_checkpoint_value(),
            &mut context.fact_store,
            &context.checkpoint_extensions,
            &mut total_events,
            planned_segment.todo_id.as_str(),
        )?;
        total_iterations = total_iterations.saturating_add(execute_outcome.iterations);
        let execute_status = execute_outcome.status.clone();
        let todo_receipt = build_todo_receipt(
            &planned_segment,
            execute_status.clone(),
            &state,
            execute_outcome.round_events.as_slice(),
        );
        context
            .todo_board
            .record_receipt_for_todo(planned_segment.todo_id.as_str(), todo_receipt);

        match execute_status {
            EngineRunStatus::Completed | EngineRunStatus::Stopped => {
                context.completed_segments = context.completed_segments.saturating_add(1);
                context.session.cursor = planned_segment.cursor_next;
                context.todo_board.mark_current_done();
                if !planned_segment.done && context.todo_board.current().is_none() {
                    context.todo_board.open_follow_up_todo();
                }
                super::record_todo_progress(&mut state.runtime, &context.todo_board);
                context.previous_error = None;
                context.last_segment = Some(planned_segment.segment);
                context.final_status = execute_outcome.status;
                context.refresh_state_summary(&state, planned_segment.done);
                if planned_segment.done {
                    break;
                }
            }
            EngineRunStatus::Paused => {
                if let Some(payload) = missing_required_input_payload_from_pause(
                    &state,
                    execute_outcome.last_iteration_events.as_slice(),
                    context.completed_segments as u8,
                ) {
                    let questions = payload
                        .get("questions")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default();
                    super::record_missing_required_input(&mut state.runtime, &payload);
                    if let Some(answers) =
                        super::maybe_collect_missing_input_answers(questions.as_slice())?
                    {
                        super::apply_missing_input_answers(
                            &mut state,
                            &mut context.fact_store,
                            &answers,
                        );
                        context.todo_board.mark_current_todo();
                        super::record_todo_progress(&mut state.runtime, &context.todo_board);
                        state.paused_reason = None;
                        context.set_previous_error_and_refresh(
                            &state,
                            planned_segment.done,
                            super::missing_required_input_resolved_payload(
                                &answers,
                                context.completed_segments as u8,
                            ),
                        );
                        checkpoint_round(
                            command,
                            run_id.as_str(),
                            &active_plan_hash,
                            &active_plan,
                            &state,
                            &checkpoint_ledger,
                            planner.planning_memory_checkpoint_value(),
                            &context.fact_store,
                            &context.checkpoint_extensions,
                        )?;
                        if command.verbose {
                            eprintln!(
                                "[agent] execution missing_required_input resolved via user answers keys={}",
                                answers.keys().cloned().collect::<Vec<_>>().join(",")
                            );
                        }
                        continue;
                    }
                    state.paused_reason = Some("missing_required_input".to_string());
                    context
                        .todo_board
                        .mark_current_blocked("missing_required_input");
                    super::record_todo_progress(&mut state.runtime, &context.todo_board);
                    context.final_status = EngineRunStatus::Paused;
                    break;
                }
                context.final_status = EngineRunStatus::Paused;
                if !super::should_attempt_intent_repair(state.paused_reason.as_deref()) {
                    let blocked_reason = state
                        .paused_reason
                        .clone()
                        .unwrap_or_else(|| "paused".to_string());
                    context.todo_board.mark_current_blocked(blocked_reason);
                    super::record_todo_progress(&mut state.runtime, &context.todo_board);
                    break;
                }
                context.set_previous_error_and_refresh(
                    &state,
                    planned_segment.done,
                    super::intent_execution_error_payload(
                        state.paused_reason.as_deref(),
                        &execute_outcome.last_iteration_events,
                        context.completed_segments as u8,
                    ),
                );
                super::record_todo_progress(&mut state.runtime, &context.todo_board);
                context.last_segment = Some(planned_segment.segment);
            }
        }
    }

    record_planner_llm_usage(&mut state, &planner);
    super::render_agent_output(
        command,
        &state,
        context.final_status,
        total_iterations,
        total_events,
        resumed_from_checkpoint,
    )
}

fn record_planner_llm_usage<P>(
    state: &mut EngineRunnerState,
    planner: &LlmSegmentedIntentPlanner<P>,
) {
    super::record_runtime_agent_field(&mut state.runtime, "llm_usage", planner.llm_usage_value());
}

fn refresh_tool_memory_projection<P>(
    context: &mut SegmentedAgentContext,
    planner: &LlmSegmentedIntentPlanner<P>,
    state: &EngineRunnerState,
) {
    let planner_usage = planner.llm_usage_value();
    let runtime_usage = state.runtime.pointer("/agent/llm_usage");
    let token_budget =
        resolve_tool_memory_projection_token_budget(Some(&planner_usage), runtime_usage);
    context.update_tool_memory_projection(planner.tool_memory_projection_value(token_budget));
}

fn resolve_tool_memory_projection_token_budget(
    planner_usage: Option<&Value>,
    runtime_usage: Option<&Value>,
) -> usize {
    let remaining_tokens = usage_field_u64(planner_usage, "context_remaining_tokens")
        .or_else(|| usage_field_u64(runtime_usage, "context_remaining_tokens"));
    let soft_limit_tokens = usage_field_u64(planner_usage, "context_soft_limit_tokens")
        .or_else(|| usage_field_u64(runtime_usage, "context_soft_limit_tokens"));

    if let (Some(remaining), Some(soft_limit)) = (remaining_tokens, soft_limit_tokens) {
        if soft_limit > 0 {
            let ratio_bps = remaining.saturating_mul(10_000) / soft_limit;
            return budget_from_ratio_bps(ratio_bps);
        }
    }
    if let Some(remaining) = remaining_tokens {
        return budget_from_remaining_tokens(remaining);
    }
    TOOL_MEMORY_PROJECTION_DEFAULT_TOKENS
}

fn usage_field_u64(usage: Option<&Value>, key: &str) -> Option<u64> {
    usage
        .and_then(|value| value.get(key))
        .and_then(Value::as_u64)
}

fn budget_from_ratio_bps(ratio_bps: u64) -> usize {
    if ratio_bps <= TOOL_MEMORY_PROJECTION_TIGHT_THRESHOLD_BPS {
        return TOOL_MEMORY_PROJECTION_MIN_TOKENS;
    }
    if ratio_bps >= TOOL_MEMORY_PROJECTION_RELAXED_THRESHOLD_BPS {
        return TOOL_MEMORY_PROJECTION_MAX_TOKENS;
    }
    let span_bps =
        TOOL_MEMORY_PROJECTION_RELAXED_THRESHOLD_BPS - TOOL_MEMORY_PROJECTION_TIGHT_THRESHOLD_BPS;
    let progress_bps = ratio_bps.saturating_sub(TOOL_MEMORY_PROJECTION_TIGHT_THRESHOLD_BPS);
    let span_tokens = TOOL_MEMORY_PROJECTION_MAX_TOKENS - TOOL_MEMORY_PROJECTION_MIN_TOKENS;
    TOOL_MEMORY_PROJECTION_MIN_TOKENS
        + usize::try_from(
            progress_bps.saturating_mul(u64::try_from(span_tokens).unwrap_or(0)) / span_bps,
        )
        .unwrap_or(0)
}

fn budget_from_remaining_tokens(remaining_tokens: u64) -> usize {
    if remaining_tokens <= TOOL_MEMORY_REMAINING_ABS_MIN {
        return TOOL_MEMORY_PROJECTION_MIN_TOKENS;
    }
    if remaining_tokens >= TOOL_MEMORY_REMAINING_ABS_MAX {
        return TOOL_MEMORY_PROJECTION_MAX_TOKENS;
    }
    let span_remaining = TOOL_MEMORY_REMAINING_ABS_MAX - TOOL_MEMORY_REMAINING_ABS_MIN;
    let progress = remaining_tokens.saturating_sub(TOOL_MEMORY_REMAINING_ABS_MIN);
    let span_tokens = TOOL_MEMORY_PROJECTION_MAX_TOKENS - TOOL_MEMORY_PROJECTION_MIN_TOKENS;
    TOOL_MEMORY_PROJECTION_MIN_TOKENS
        + usize::try_from(
            progress.saturating_mul(u64::try_from(span_tokens).unwrap_or(0)) / span_remaining,
        )
        .unwrap_or(0)
}

fn bootstrap_todos_if_needed<P: LlmProvider>(
    command: &AgentCommand,
    planner: &mut LlmSegmentedIntentPlanner<P>,
    state: &mut EngineRunnerState,
    context: &mut SegmentedAgentContext,
    runtime_has_todo_progress: bool,
) -> Result<(), RunnerError> {
    if runtime_has_todo_progress {
        return Ok(());
    }
    if !intent_grounding_ready_for_todos(state) || !capability_view_ready(state) {
        let questions = state
            .runtime
            .pointer("/agent/intent_grounding/questions")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let payload = super::missing_required_input_payload(
            Some("planning_readiness_not_met"),
            questions.as_slice(),
            &[],
            context.completed_segments as u8,
        );
        state.paused_reason = Some("missing_required_input".to_string());
        super::record_missing_required_input(&mut state.runtime, &payload);
        context.set_previous_error_and_refresh(
            state,
            false,
            super::todo_phase_error_payload(
                "planning_readiness_not_met",
                Some("planning_readiness_not_met"),
                &[],
                questions.as_slice(),
                context.completed_segments as u8,
            ),
        );
        return Ok(());
    }
    let draft = planner.propose_todos(TodoPlanningRequest {
        intent: context.intent.clone(),
        session: context.session.clone(),
        state_summary: context.state_summary.clone(),
    });
    refresh_tool_memory_projection(context, planner, state);
    match draft {
        Ok(TodoDraft::Proposed {
            summary,
            todos,
            issues,
        }) => {
            context
                .todo_board
                .replace_from_specs(context.intent.as_str(), &todos);
            context.todo_board.ensure_current();
            super::record_todo_progress(&mut state.runtime, &context.todo_board);
            context.previous_error = None;
            context.refresh_state_summary(state, false);
            if command.verbose_llm {
                eprintln!(
                    "[agent] todo plan proposed count={} summary={}",
                    todos.len(),
                    summary.unwrap_or_default()
                );
                if !issues.is_empty() {
                    eprintln!("[agent] todo plan issues={}", Value::Array(issues));
                }
            }
        }
        Ok(TodoDraft::Unavailable {
            reason_code,
            message,
            issues,
            questions,
        }) => {
            if reason_code == "missing_required_input" {
                if let Some(answers) =
                    super::maybe_collect_missing_input_answers(questions.as_slice())?
                {
                    super::apply_missing_input_answers(state, &mut context.fact_store, &answers);
                    context.previous_error = None;
                    context.refresh_state_summary(state, false);
                    if command.verbose_llm {
                        eprintln!(
                            "[agent] todo plan missing_required_input resolved via user answers keys={}",
                            answers.keys().cloned().collect::<Vec<_>>().join(",")
                        );
                    }
                    return Ok(());
                }
                let payload = super::missing_required_input_payload(
                    message.as_deref(),
                    questions.as_slice(),
                    issues.as_slice(),
                    context.completed_segments as u8,
                );
                state.paused_reason = Some("missing_required_input".to_string());
                super::record_missing_required_input(&mut state.runtime, &payload);
                context.set_previous_error_and_refresh(
                    state,
                    false,
                    super::todo_phase_error_payload(
                        "missing_required_input",
                        message.as_deref(),
                        issues.as_slice(),
                        questions.as_slice(),
                        context.completed_segments as u8,
                    ),
                );
                return Ok(());
            }
            context.set_previous_error_and_refresh(
                state,
                false,
                super::todo_phase_error_payload(
                    "unavailable",
                    message.as_deref(),
                    issues.as_slice(),
                    questions.as_slice(),
                    context.completed_segments as u8,
                ),
            );
            if command.verbose_llm {
                eprintln!(
                    "[agent] todo plan unavailable reason_code={} message={} issues={} questions={} (fallback bootstrap todo)",
                    reason_code,
                    message.unwrap_or_default(),
                    issues.len(),
                    questions.len(),
                );
            }
        }
        Ok(TodoDraft::Invalid {
            reason_code,
            message,
            issues,
        }) => {
            context.set_previous_error_and_refresh(
                state,
                false,
                super::todo_phase_error_payload(
                    "invalid",
                    message.as_deref(),
                    issues.as_slice(),
                    &[],
                    context.completed_segments as u8,
                ),
            );
            if command.verbose_llm {
                eprintln!(
                    "[agent] todo plan invalid reason_code={} message={} issues={} (fallback bootstrap todo)",
                    reason_code,
                    message.unwrap_or_default(),
                    issues.len(),
                );
            }
        }
        Err(error) => {
            let error_message = error.to_string();
            context.set_previous_error_and_refresh(
                state,
                false,
                super::todo_phase_error_payload(
                    "planner_call_failed",
                    Some(error_message.as_str()),
                    &[],
                    &[],
                    context.completed_segments as u8,
                ),
            );
            if command.verbose_llm {
                eprintln!(
                    "[agent] todo plan failed error={} (fallback bootstrap todo)",
                    error
                );
            }
        }
    }
    Ok(())
}

fn bootstrap_intent_grounding_if_needed<P: LlmProvider>(
    command: &AgentCommand,
    planner: &mut LlmSegmentedIntentPlanner<P>,
    state: &mut EngineRunnerState,
    context: &mut SegmentedAgentContext,
    runtime_has_intent_grounding: bool,
) -> Result<bool, RunnerError> {
    if runtime_has_intent_grounding {
        return Ok(intent_grounding_ready_for_todos(state));
    }
    let draft_result = planner.ground_intent(IntentGroundingRequest {
        intent: context.intent.clone(),
        session: context.session.clone(),
        state_summary: context.state_summary.clone(),
    });
    refresh_tool_memory_projection(context, planner, state);
    let draft = match draft_result {
        Ok(draft) => draft,
        Err(error) => {
            let error_message = error.to_string();
            if command.verbose_llm {
                eprintln!("[agent] intent grounding unavailable; fallback to ready=true ({error})");
            }
            super::record_runtime_agent_field(
                &mut state.runtime,
                "intent_grounding",
                json!({
                    "status":"fallback",
                    "ready_for_todos": true,
                    "reason_code": "grounding_fallback",
                    "message": error_message.as_str(),
                }),
            );
            context.set_previous_error_and_refresh(
                state,
                false,
                super::grounding_phase_error_payload(
                    "planner_call_failed",
                    Some(error_message.as_str()),
                    &[],
                    &[],
                    context.completed_segments as u8,
                ),
            );
            return Ok(true);
        }
    };

    match draft {
        IntentGroundingDraft::Proposed {
            summary,
            ready_for_todos,
            resolved_inputs,
            intent_facts,
            confidence,
            issues,
            questions,
        } => {
            let apply_summary = apply_intent_grounding(
                state,
                &mut context.fact_store,
                &resolved_inputs,
                &intent_facts,
                &confidence,
            );
            let answered_questions = if let Some(answers) =
                super::maybe_collect_missing_input_answers(questions.as_slice())?
            {
                super::apply_missing_input_answers(state, &mut context.fact_store, &answers);
                answers
            } else {
                Map::new()
            };
            let remaining_questions = filter_unanswered_questions(
                questions.as_slice(),
                answered_questions.keys().collect::<Vec<_>>().as_slice(),
            );
            let ready = ready_for_todos && remaining_questions.is_empty();
            super::record_runtime_agent_field(
                &mut state.runtime,
                "intent_grounding",
                json!({
                    "status":"proposed",
                    "summary": summary,
                    "ready_for_todos": ready,
                    "resolved_inputs": resolved_inputs,
                    "intent_facts": intent_facts,
                    "confidence": confidence,
                    "issues": issues,
                    "questions": remaining_questions,
                    "answers": answered_questions,
                    "applied": apply_summary.applied,
                    "skipped_low_confidence": apply_summary.skipped_low_confidence,
                }),
            );
            context.refresh_state_summary(state, false);
            if !ready {
                context.set_previous_error_and_refresh(
                    state,
                    false,
                    super::grounding_phase_error_payload(
                        "missing_required_input",
                        Some("intent_grounding_missing_inputs"),
                        &[],
                        remaining_questions.as_slice(),
                        context.completed_segments as u8,
                    ),
                );
                let payload = super::missing_required_input_payload(
                    Some("intent_grounding_missing_inputs"),
                    remaining_questions.as_slice(),
                    &[],
                    context.completed_segments as u8,
                );
                state.paused_reason = Some("missing_required_input".to_string());
                super::record_missing_required_input(&mut state.runtime, &payload);
                return Ok(false);
            }
            state.paused_reason = None;
            context.previous_error = None;
            context.refresh_state_summary(state, false);
            Ok(true)
        }
        IntentGroundingDraft::Unavailable {
            reason_code,
            message,
            issues,
            questions,
        } => {
            if reason_code == "missing_required_input" {
                if let Some(answers) =
                    super::maybe_collect_missing_input_answers(questions.as_slice())?
                {
                    super::apply_missing_input_answers(state, &mut context.fact_store, &answers);
                    super::record_runtime_agent_field(
                        &mut state.runtime,
                        "intent_grounding",
                        json!({
                            "status":"resolved_by_user_input",
                            "ready_for_todos": true,
                            "reason_code": reason_code,
                            "answers": answers,
                        }),
                    );
                    context.refresh_state_summary(state, false);
                    return Ok(true);
                }
                let payload = super::missing_required_input_payload(
                    message.as_deref(),
                    questions.as_slice(),
                    issues.as_slice(),
                    context.completed_segments as u8,
                );
                state.paused_reason = Some("missing_required_input".to_string());
                super::record_missing_required_input(&mut state.runtime, &payload);
                super::record_runtime_agent_field(
                    &mut state.runtime,
                    "intent_grounding",
                    json!({
                        "status":"unavailable",
                        "ready_for_todos": false,
                        "reason_code": reason_code,
                        "message": message,
                        "issues": issues,
                        "questions": questions,
                    }),
                );
                context.set_previous_error_and_refresh(
                    state,
                    false,
                    super::grounding_phase_error_payload(
                        "missing_required_input",
                        message.as_deref(),
                        issues.as_slice(),
                        questions.as_slice(),
                        context.completed_segments as u8,
                    ),
                );
                return Ok(false);
            }
            context.set_previous_error_and_refresh(
                state,
                false,
                super::grounding_phase_error_payload(
                    "unavailable",
                    message.as_deref(),
                    issues.as_slice(),
                    questions.as_slice(),
                    context.completed_segments as u8,
                ),
            );
            Err(RunnerError::Llm(format!(
                "intent grounding unavailable reason_code={} message={} issues={} questions={}",
                reason_code,
                message.unwrap_or_default(),
                issues.len(),
                questions.len()
            )))
        }
        IntentGroundingDraft::Invalid {
            reason_code,
            message,
            issues,
        } => {
            context.set_previous_error_and_refresh(
                state,
                false,
                super::grounding_phase_error_payload(
                    "invalid",
                    message.as_deref(),
                    issues.as_slice(),
                    &[],
                    context.completed_segments as u8,
                ),
            );
            Err(RunnerError::Llm(format!(
                "intent grounding invalid reason_code={} message={} issues={}",
                reason_code,
                message.unwrap_or_default(),
                issues.len()
            )))
        }
    }
}

#[derive(Debug, Default)]
struct GroundingApplySummary {
    applied: Vec<String>,
    skipped_low_confidence: Vec<String>,
}

fn apply_intent_grounding(
    state: &mut EngineRunnerState,
    fact_store: &mut FactStore,
    resolved_inputs: &std::collections::BTreeMap<String, Value>,
    intent_facts: &std::collections::BTreeMap<String, Value>,
    confidence: &std::collections::BTreeMap<String, u8>,
) -> GroundingApplySummary {
    let mut summary = GroundingApplySummary::default();
    for (key, value) in resolved_inputs {
        let score = confidence.get(key.as_str()).copied().unwrap_or(85);
        if score < GROUND_INPUT_CONFIDENCE_THRESHOLD {
            summary
                .skipped_low_confidence
                .push(format!("inputs.{key}:{score}"));
            continue;
        }
        super::set_runtime_input_value(&mut state.runtime, key.as_str(), value.clone());
        let provenance = format!("intent_grounding.input.{key}");
        fact_store.upsert(
            key.clone(),
            value.clone(),
            FactLayer::Seed,
            FactSource::IntentInferred,
            provenance.clone(),
        );
        fact_store.upsert(
            format!("inputs.{key}"),
            value.clone(),
            FactLayer::Seed,
            FactSource::IntentInferred,
            provenance,
        );
        summary.applied.push(format!("inputs.{key}:{score}"));
    }
    for (key, value) in intent_facts {
        let score = confidence
            .get(format!("fact:{key}").as_str())
            .copied()
            .or_else(|| confidence.get(key.as_str()).copied())
            .unwrap_or(70);
        if score < GROUND_FACT_CONFIDENCE_THRESHOLD {
            summary
                .skipped_low_confidence
                .push(format!("fact:{key}:{score}"));
            continue;
        }
        fact_store.upsert(
            key.clone(),
            value.clone(),
            FactLayer::Seed,
            FactSource::IntentInferred,
            format!("intent_grounding.fact.{key}"),
        );
        summary.applied.push(format!("fact:{key}:{score}"));
    }
    summary
}

fn filter_unanswered_questions(questions: &[Value], answered_ids: &[&String]) -> Vec<Value> {
    let answered = answered_ids
        .iter()
        .map(|value| value.as_str())
        .collect::<BTreeSet<_>>();
    questions
        .iter()
        .filter(|question| {
            let Some(id) = question.get("id").and_then(Value::as_str) else {
                return true;
            };
            !answered.contains(id)
        })
        .cloned()
        .collect::<Vec<_>>()
}

fn missing_required_input_payload_from_pause(
    state: &EngineRunnerState,
    events: &[EngineEventRecord],
    round: u8,
) -> Option<Value> {
    let paused_reason = state.paused_reason.as_deref()?;
    if !paused_reason.starts_with("need_user_input:")
        && paused_reason != "need_user_input"
        && paused_reason != "missing_required_input"
    {
        return None;
    }

    let event = events
        .iter()
        .rev()
        .find(|record| record.event.event_type == EngineEventType::NeedUserInput)?;
    let reason_code = event
        .event
        .data
        .get("reason_code")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if reason_code != "missing_required_input" {
        return None;
    }

    let message = event.event.data.get("reason").and_then(Value::as_str);
    let details = event
        .event
        .data
        .get("details")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let missing_refs = string_array_field(details.get("missing_refs"));
    let suggested_paths = string_array_field(details.get("suggested_paths"));
    let questions = details
        .get("questions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let issues = details
        .get("issues")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    Some(super::missing_required_input_payload_with_context(
        message,
        questions.as_slice(),
        issues.as_slice(),
        missing_refs.as_slice(),
        suggested_paths.as_slice(),
        round,
    ))
}

fn string_array_field(value: Option<&Value>) -> Vec<String> {
    let Some(items) = value.and_then(Value::as_array) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
}

fn intent_grounding_ready_for_todos(state: &EngineRunnerState) -> bool {
    let Some(grounding) = state.runtime.pointer("/agent/intent_grounding") else {
        return false;
    };
    let ready_flag = grounding
        .get("ready_for_todos")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    if ready_flag {
        return true;
    }
    let has_questions = grounding
        .get("questions")
        .and_then(Value::as_array)
        .is_some_and(|questions| !questions.is_empty());
    if has_questions {
        return false;
    }
    let has_resolved_inputs = grounding
        .get("resolved_inputs")
        .and_then(Value::as_object)
        .is_some_and(|inputs| !inputs.is_empty());
    has_resolved_inputs
}

fn capability_view_ready(state: &EngineRunnerState) -> bool {
    state
        .runtime
        .pointer("/agent/capability_view/ready")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn plan_round<P: LlmProvider>(
    planner: &mut LlmSegmentedIntentPlanner<P>,
    state: &EngineRunnerState,
    context: &mut SegmentedAgentContext,
) -> Result<SegmentDraft, RunnerError> {
    loop {
        if context.planning_rounds >= context.planner_round_limit {
            return Err(RunnerError::Llm(format!(
                "segmented planner round limit reached ({})",
                context.planner_round_limit
            )));
        }
        context.planning_rounds = context.planning_rounds.saturating_add(1);

        let request = SegmentPlanningRequest {
            intent: context.intent.clone(),
            session: context.session.clone(),
            state_summary: context.state_summary.clone(),
            previous_error: context.previous_error.clone(),
            last_segment: context.last_segment.clone(),
        };
        let expected_finalize_tool = if context.previous_error.is_some() {
            "plan.revise_segment"
        } else {
            "plan.propose_segment"
        };
        if let Some(previous_error) = context.previous_error.as_ref() {
            eprintln!(
                "[agent] plan_round={} mode={} previous_error={}",
                context.planning_rounds,
                expected_finalize_tool,
                previous_error_compact(previous_error)
            );
        } else {
            eprintln!(
                "[agent] plan_round={} mode={} previous_error=-",
                context.planning_rounds, expected_finalize_tool
            );
        }
        let draft_result = if context.previous_error.is_some() {
            planner.revise_segment(request)
        } else {
            planner.propose_segment(request)
        };
        refresh_tool_memory_projection(context, planner, state);
        match draft_result {
            Ok(draft) => {
                context.planner_output_retries = 0;
                return Ok(draft);
            }
            Err(error) => {
                if super::should_retry_segmented_planner_output(&error)
                    && context.planner_output_retries < MAX_PLANNER_OUTPUT_REPAIR_RETRIES
                {
                    context.planner_output_retries =
                        context.planner_output_retries.saturating_add(1);
                    eprintln!(
                        "[agent] planner_output_retry retry={}/{} reason={}",
                        context.planner_output_retries, MAX_PLANNER_OUTPUT_REPAIR_RETRIES, error
                    );
                    let last_failed_finalize = planner.take_last_failed_finalize();
                    context.set_previous_error_and_refresh(
                        state,
                        false,
                        super::segmented_planner_output_error_payload(
                            &error,
                            expected_finalize_tool,
                            context.planning_rounds as u8,
                            context.planner_output_retries as u8,
                            last_failed_finalize,
                        ),
                    );
                    continue;
                }
                return Err(error);
            }
        }
    }
}

fn compile_guard(
    planned: &PlannedSegment,
    context: &SegmentedAgentContext,
    candidate_context: &CandidateContext,
    pack: Option<&ais_sdk::PackDocument>,
    chain_scope: &[String],
) -> Result<PlanDocument, Value> {
    super::compile_segment_plan_with_facts(
        context.intent.as_str(),
        &context.session,
        &planned.segment,
        candidate_context,
        pack,
        chain_scope,
        Some(&context.fact_store),
    )
}

fn previous_error_compact(error: &Value) -> String {
    let phase = error.get("phase").and_then(Value::as_str).unwrap_or("-");
    let reason_code = error
        .get("reason_code")
        .and_then(Value::as_str)
        .unwrap_or("-");
    let sub_reason_code = error
        .get("sub_reason_code")
        .and_then(Value::as_str)
        .unwrap_or("-");
    format!("phase={phase},reason={reason_code},sub_reason={sub_reason_code}")
}

fn compile_error_compact(error: &Value) -> String {
    let reason_code = error
        .get("reason_code")
        .and_then(Value::as_str)
        .unwrap_or("-");
    let message = error.get("message").and_then(Value::as_str).unwrap_or("-");
    let first_issue = error
        .get("issues")
        .and_then(Value::as_array)
        .and_then(|issues| issues.first())
        .map(|issue| {
            let kind = issue.get("kind").and_then(Value::as_str).unwrap_or("-");
            let reference = issue
                .get("reference")
                .and_then(Value::as_str)
                .unwrap_or("-");
            let issue_message = issue.get("message").and_then(Value::as_str).unwrap_or("-");
            format!("{kind}/{reference}:{issue_message}")
        })
        .unwrap_or_else(|| "-".to_string());
    format!("reason={reason_code},message={message},first_issue={first_issue}")
}

#[allow(clippy::too_many_arguments)]
fn execute_round(
    command: &AgentCommand,
    run_id: &str,
    config: &RunnerConfig,
    engine_options: &EngineRunnerOptions,
    decision_policy: &mut dyn crate::agent::brain::DecisionPolicy,
    command_builder: &mut CommandBuilder,
    checkpoint_ledger: &mut RunnerCheckpointLedger,
    state: &mut EngineRunnerState,
    active_plan: &mut PlanDocument,
    active_plan_hash: &mut String,
    segment: &PlanSketchSegment,
    segment_plan: &PlanDocument,
    planning_memory: Option<Value>,
    fact_store: &mut FactStore,
    checkpoint_extensions: &checkpoint_ext::AgentCheckpointExtensions,
    total_events: &mut usize,
    todo_id: &str,
) -> Result<ExecuteRoundOutcome, RunnerError> {
    let replacement = super::merge_segment_plan(active_plan, segment_plan)?;
    let replace_reason = format!("segment:{}", segment.segment_id);
    let replace_command = super::build_replace_plan_command(
        command_builder,
        &replacement,
        Some(replace_reason.as_str()),
    )?;
    let processed = process_replace_plan_commands(
        run_id,
        config,
        &[replace_command],
        engine_options,
        state,
        active_plan,
        active_plan_hash,
    )?;
    let mut round_events = Vec::<EngineEventRecord>::new();
    if !processed.events.is_empty() {
        let annotated = annotate_events_with_todo(processed.events.as_slice(), segment, todo_id);
        *total_events = total_events.saturating_add(annotated.len());
        super::write_event_sinks(command, annotated.as_slice())?;
        checkpoint_ledger.absorb_events(annotated.as_slice());
        checkpoint_ledger.mark_approved_nodes(&state.approved_node_ids, "1970-01-01T00:00:00Z");
        super::record_side_effect_lifecycle(&mut state.runtime, checkpoint_ledger);
        round_events.extend(annotated);
        checkpoint_round(
            command,
            run_id,
            active_plan_hash,
            active_plan,
            state,
            checkpoint_ledger,
            planning_memory.clone(),
            fact_store,
            checkpoint_extensions,
        )?;
    }
    if !processed.plan_replaced {
        return Err(RunnerError::Llm(
            "replace_plan failed while applying segment".to_string(),
        ));
    }

    let router = build_router_executor_for_plan(active_plan, config)
        .map_err(RunnerError::ConfigInvalidForPlan)?;
    state.paused_reason = None;

    let max_iterations = command
        .max_iterations
        .unwrap_or_else(|| active_plan.nodes.len().saturating_mul(8).max(16));
    let loop_config = AgentLoopConfig { max_iterations };
    let mut last_iteration_events = Vec::<EngineEventRecord>::new();
    let loop_result = run_agent_loop(
        run_id,
        active_plan,
        state,
        &router,
        &DefaultSolver,
        engine_options,
        &loop_config,
        command_builder,
        decision_policy,
        |state, events| {
            let annotated = annotate_events_with_todo(events, segment, todo_id);
            *total_events = total_events.saturating_add(annotated.len());
            last_iteration_events = annotated.clone();
            round_events.extend(annotated.clone());
            super::write_event_sinks(command, annotated.as_slice())?;
            apply_segment_stores_from_runtime(segment, state, fact_store, command.verbose_llm);
            checkpoint_ledger.absorb_events(annotated.as_slice());
            checkpoint_ledger.mark_approved_nodes(&state.approved_node_ids, "1970-01-01T00:00:00Z");
            checkpoint_round(
                command,
                run_id,
                active_plan_hash,
                active_plan,
                state,
                checkpoint_ledger,
                planning_memory.clone(),
                fact_store,
                checkpoint_extensions,
            )?;
            Ok(())
        },
    )?;
    super::record_side_effect_lifecycle(&mut state.runtime, checkpoint_ledger);

    Ok(ExecuteRoundOutcome {
        status: loop_result.status,
        iterations: loop_result.iterations,
        round_events,
        last_iteration_events,
    })
}

fn bind_segment_todo_id(segment: &mut PlanSketchSegment, todo_id: &str) {
    segment
        .extensions
        .insert("todo_id".to_string(), Value::String(todo_id.to_string()));
}

fn annotate_events_with_todo(
    events: &[EngineEventRecord],
    segment: &PlanSketchSegment,
    todo_id: &str,
) -> Vec<EngineEventRecord> {
    events
        .iter()
        .cloned()
        .map(|mut record| {
            let agent = record
                .event
                .extensions
                .entry("agent".to_string())
                .or_insert_with(|| Value::Object(Map::new()));
            if !agent.is_object() {
                *agent = Value::Object(Map::new());
            }
            if let Some(agent_obj) = agent.as_object_mut() {
                agent_obj.insert("todo_id".to_string(), Value::String(todo_id.to_string()));
                agent_obj.insert(
                    "segment_id".to_string(),
                    Value::String(segment.segment_id.clone()),
                );
                if let Some(step_id) = step_id_for_segment_node(
                    record.event.node_id.as_deref(),
                    segment.segment_id.as_str(),
                ) {
                    agent_obj.insert("step_id".to_string(), Value::String(step_id.to_string()));
                }
            }
            record
        })
        .collect::<Vec<_>>()
}

fn step_id_for_segment_node<'a>(node_id: Option<&'a str>, segment_id: &str) -> Option<&'a str> {
    let node_id = node_id?;
    let prefix = format!("{segment_id}/");
    node_id
        .strip_prefix(prefix.as_str())
        .and_then(|value| (!value.trim().is_empty()).then_some(value))
}

fn build_todo_receipt(
    planned_segment: &PlannedSegment,
    status: EngineRunStatus,
    state: &EngineRunnerState,
    round_events: &[EngineEventRecord],
) -> super::todos::TodoReceipt {
    let node_ids = planned_segment
        .segment
        .steps
        .iter()
        .map(|step| format!("{}/{}", planned_segment.segment.segment_id, step.id))
        .collect::<Vec<_>>();
    let completed_node_set = state
        .completed_node_ids
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let completed_node_ids = node_ids
        .iter()
        .filter(|node_id| completed_node_set.contains((*node_id).as_str()))
        .cloned()
        .collect::<Vec<_>>();
    let tx_hashes = collect_segment_tx_hashes(state, node_ids.as_slice());
    let event_types = round_events
        .iter()
        .map(|record| event_type_name(record))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();

    super::todos::TodoReceipt {
        schema: "ais-agent-todo-receipt/0.0.1".to_string(),
        todo_id: planned_segment.todo_id.clone(),
        segment_id: planned_segment.segment.segment_id.clone(),
        status: run_status_name(status).to_string(),
        paused_reason: state.paused_reason.clone(),
        node_ids,
        completed_node_ids,
        tx_hashes,
        event_types,
        event_count: round_events.len() as u64,
    }
}

fn run_status_name(status: EngineRunStatus) -> &'static str {
    match status {
        EngineRunStatus::Completed => "completed",
        EngineRunStatus::Paused => "paused",
        EngineRunStatus::Stopped => "stopped",
    }
}

fn event_type_name(record: &EngineEventRecord) -> String {
    serde_json::to_value(record.event.event_type)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_else(|| format!("{:?}", record.event.event_type).to_lowercase())
}

fn collect_segment_tx_hashes(state: &EngineRunnerState, node_ids: &[String]) -> Vec<String> {
    let mut tx_hashes = BTreeSet::<String>::new();
    for node_id in node_ids {
        let Some(outputs) = runtime_node_outputs(state, node_id.as_str()) else {
            continue;
        };
        collect_tx_hash_like_strings(outputs, &mut tx_hashes, 0);
    }
    tx_hashes.into_iter().collect::<Vec<_>>()
}

fn collect_tx_hash_like_strings(value: &Value, output: &mut BTreeSet<String>, depth: usize) {
    if depth > 8 {
        return;
    }
    match value {
        Value::Object(map) => {
            for key in ["tx_hash", "signed_tx_hash", "signature"] {
                if let Some(hash) = map.get(key).and_then(Value::as_str) {
                    let trimmed = hash.trim();
                    if !trimmed.is_empty() {
                        output.insert(trimmed.to_string());
                    }
                }
            }
            for nested in map.values() {
                collect_tx_hash_like_strings(nested, output, depth.saturating_add(1));
            }
        }
        Value::Array(items) => {
            for nested in items {
                collect_tx_hash_like_strings(nested, output, depth.saturating_add(1));
            }
        }
        _ => {}
    }
}

fn checkpoint_round(
    command: &AgentCommand,
    run_id: &str,
    active_plan_hash: &str,
    active_plan: &PlanDocument,
    state: &EngineRunnerState,
    checkpoint_ledger: &RunnerCheckpointLedger,
    planning_memory: Option<Value>,
    fact_store: &FactStore,
    checkpoint_extensions: &checkpoint_ext::AgentCheckpointExtensions,
) -> Result<(), RunnerError> {
    let intent_facts = runtime_intent_facts(&state.runtime);
    super::maybe_save_checkpoint(
        command,
        run_id,
        active_plan_hash,
        active_plan,
        state,
        checkpoint_ledger,
        Some(checkpoint_extensions.encode_updated(
            planning_memory,
            fact_store,
            state.runtime.pointer("/agent/todo_progress"),
            intent_facts.as_ref(),
        )),
    )
}

fn runtime_intent_facts(runtime: &Value) -> Option<std::collections::BTreeMap<String, Value>> {
    runtime
        .pointer("/agent/intent_grounding/intent_facts")
        .and_then(Value::as_object)
        .map(|facts| {
            facts
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect::<std::collections::BTreeMap<String, Value>>()
        })
}

fn apply_segment_stores_from_runtime(
    segment: &PlanSketchSegment,
    state: &EngineRunnerState,
    fact_store: &mut FactStore,
    verbose_llm: bool,
) {
    for step in &segment.steps {
        if step.stores.is_empty() {
            continue;
        }
        let node_id = format!("{}/{}", segment.segment_id, step.id);
        let Some(node_outputs) = runtime_node_outputs(state, node_id.as_str()) else {
            continue;
        };
        for (return_field, slot_name) in &step.stores {
            let Some(value) = extract_store_value(node_outputs, return_field.as_str()) else {
                continue;
            };
            let provenance = format!("segment_store.{node_id}.{}", return_field.trim());
            let upsert_result = fact_store.upsert(
                slot_name.clone(),
                value.clone(),
                FactLayer::Observed,
                FactSource::QueryObserved,
                provenance,
            );
            if verbose_llm {
                eprintln!(
                    "[agent] stores mapped node={} field={} -> slot={} upsert={:?}",
                    node_id, return_field, slot_name, upsert_result
                );
            }
        }
    }
}

fn runtime_node_outputs<'a>(state: &'a EngineRunnerState, node_id: &str) -> Option<&'a Value> {
    let escaped = node_id.replace('~', "~0").replace('/', "~1");
    state
        .runtime
        .pointer(format!("/nodes/{escaped}/outputs").as_str())
}

fn extract_store_value(node_outputs: &Value, field: &str) -> Option<Value> {
    let field = field.trim();
    if field.is_empty() {
        return None;
    }
    if let Some(value) = value_at_dot_path(node_outputs, field) {
        return Some(value.clone());
    }
    if let Some(outputs_value) = node_outputs.get("outputs") {
        if let Some(value) = value_at_dot_path(outputs_value, field) {
            return Some(value.clone());
        }
    }
    None
}

fn value_at_dot_path<'a>(value: &'a Value, dot_path: &str) -> Option<&'a Value> {
    let mut current = value;
    for segment in dot_path.split('.').filter(|item| !item.is_empty()) {
        current = current.get(segment)?;
    }
    Some(current)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ais_engine::{EngineEvent, EngineEventType};
    use serde_json::json;

    #[test]
    fn tool_memory_projection_budget_uses_remaining_ratio() {
        let planner_usage = json!({
            "context_soft_limit_tokens": 100_000,
            "context_remaining_tokens": 90_000
        });
        let budget = resolve_tool_memory_projection_token_budget(Some(&planner_usage), None);
        assert_eq!(budget, TOOL_MEMORY_PROJECTION_MAX_TOKENS);

        let planner_usage = json!({
            "context_soft_limit_tokens": 100_000,
            "context_remaining_tokens": 10_000
        });
        let budget = resolve_tool_memory_projection_token_budget(Some(&planner_usage), None);
        assert_eq!(budget, TOOL_MEMORY_PROJECTION_MIN_TOKENS);
    }

    #[test]
    fn tool_memory_projection_budget_falls_back_to_runtime_and_absolute_remaining() {
        let runtime_usage = json!({
            "context_remaining_tokens": 14_000
        });
        let budget = resolve_tool_memory_projection_token_budget(None, Some(&runtime_usage));
        assert!(budget > TOOL_MEMORY_PROJECTION_MIN_TOKENS);
        assert!(budget < TOOL_MEMORY_PROJECTION_MAX_TOKENS);

        let budget = resolve_tool_memory_projection_token_budget(None, None);
        assert_eq!(budget, TOOL_MEMORY_PROJECTION_DEFAULT_TOKENS);
    }

    #[test]
    fn apply_segment_stores_projects_query_and_action_outputs() {
        let segment: PlanSketchSegment = serde_json::from_value(json!({
            "segment_id":"seg_1",
            "cursor_in":"0",
            "cursor_out":"1",
            "done":false,
            "steps":[
                {
                    "id":"q_balance",
                    "kind":"query",
                    "candidate_ref":"demo@0.0.2/quote",
                    "inputs":{},
                    "stores":{"balance":"facts.balance"}
                },
                {
                    "id":"a_transfer",
                    "kind":"action",
                    "candidate_ref":"demo@0.0.2/swap",
                    "inputs":{},
                    "stores":{"tx_hash":"tx.hash","confirmed":"tx.confirmed"}
                }
            ]
        }))
        .expect("segment");
        let state = EngineRunnerState {
            runtime: json!({
                "nodes": {
                    "seg_1/q_balance": {"outputs":{"balance":"100"}},
                    "seg_1/a_transfer": {"outputs":{"outputs":{"tx_hash":"0xabc","confirmed":true}}}
                }
            }),
            ..EngineRunnerState::default()
        };
        let mut fact_store = FactStore::default();

        apply_segment_stores_from_runtime(&segment, &state, &mut fact_store, false);

        assert_eq!(
            fact_store
                .get("facts.balance")
                .and_then(|entry| entry.value.as_str()),
            Some("100")
        );
        assert_eq!(
            fact_store.get("facts.balance").map(|entry| entry.source),
            Some(FactSource::QueryObserved)
        );
        assert_eq!(
            fact_store
                .get("facts.balance")
                .map(|entry| entry.provenance.as_str()),
            Some("segment_store.seg_1/q_balance.balance")
        );
        assert_eq!(
            fact_store
                .get("tx.hash")
                .and_then(|entry| entry.value.as_str()),
            Some("0xabc")
        );
        assert_eq!(
            fact_store
                .get("tx.confirmed")
                .and_then(|entry| entry.value.as_bool()),
            Some(true)
        );
    }

    #[test]
    fn bind_segment_todo_id_writes_segment_extension() {
        let mut segment: PlanSketchSegment = serde_json::from_value(json!({
            "segment_id":"seg_1",
            "cursor_in":"0",
            "cursor_out":"1",
            "done":false,
            "steps":[
                {"id":"q1","kind":"query","candidate_ref":"demo@0.0.2/quote","inputs":{}}
            ]
        }))
        .expect("segment");
        bind_segment_todo_id(&mut segment, "todo_1");
        assert_eq!(segment.extensions.get("todo_id"), Some(&json!("todo_1")));
    }

    #[test]
    fn annotate_events_with_todo_adds_agent_extension() {
        let segment: PlanSketchSegment = serde_json::from_value(json!({
            "segment_id":"seg_1",
            "cursor_in":"0",
            "cursor_out":"1",
            "done":false,
            "steps":[
                {"id":"q1","kind":"query","candidate_ref":"demo@0.0.2/quote","inputs":{}}
            ]
        }))
        .expect("segment");
        let mut node_event = EngineEvent::new(EngineEventType::NodeReady);
        node_event.node_id = Some("seg_1/q1".to_string());
        let events = vec![
            EngineEventRecord::new("run-1", 1, "1970-01-01T00:00:00Z", node_event),
            EngineEventRecord::new(
                "run-1",
                2,
                "1970-01-01T00:00:00Z",
                EngineEvent::new(EngineEventType::PlanReplaced),
            ),
        ];

        let annotated = annotate_events_with_todo(events.as_slice(), &segment, "todo_1");
        let ext0 = Value::Object(annotated[0].event.extensions.clone());
        let ext1 = Value::Object(annotated[1].event.extensions.clone());
        assert_eq!(ext0.pointer("/agent/todo_id"), Some(&json!("todo_1")));
        assert_eq!(ext0.pointer("/agent/segment_id"), Some(&json!("seg_1")));
        assert_eq!(ext0.pointer("/agent/step_id"), Some(&json!("q1")));
        assert_eq!(ext1.pointer("/agent/todo_id"), Some(&json!("todo_1")));
    }

    #[test]
    fn build_todo_receipt_collects_completed_nodes_and_tx_hashes() {
        let segment: PlanSketchSegment = serde_json::from_value(json!({
            "segment_id":"seg_1",
            "cursor_in":"0",
            "cursor_out":"1",
            "done":false,
            "steps":[
                {"id":"q1","kind":"query","candidate_ref":"demo@0.0.2/quote","inputs":{}},
                {"id":"a1","kind":"action","candidate_ref":"demo@0.0.2/swap","inputs":{}}
            ]
        }))
        .expect("segment");
        let planned = PlannedSegment {
            todo_id: "todo_1".to_string(),
            summary: None,
            segment,
            cursor_next: "1".to_string(),
            done: false,
            issues: Vec::new(),
        };
        let state = EngineRunnerState {
            completed_node_ids: vec!["seg_1/q1".to_string()],
            paused_reason: Some("need_user_confirm:seg_1/a1".to_string()),
            runtime: json!({
                "nodes":{
                    "seg_1/a1":{"outputs":{"tx_hash":"0xabc","nested":{"signed_tx_hash":"0xdef"}}}
                }
            }),
            ..EngineRunnerState::default()
        };
        let events = vec![EngineEventRecord::new(
            "run-1",
            3,
            "1970-01-01T00:00:00Z",
            EngineEvent::new(EngineEventType::NeedUserConfirm),
        )];
        let receipt =
            build_todo_receipt(&planned, EngineRunStatus::Paused, &state, events.as_slice());
        assert_eq!(receipt.todo_id, "todo_1");
        assert_eq!(receipt.segment_id, "seg_1");
        assert_eq!(receipt.status, "paused");
        assert_eq!(receipt.completed_node_ids, vec!["seg_1/q1".to_string()]);
        assert_eq!(
            receipt.tx_hashes,
            vec!["0xabc".to_string(), "0xdef".to_string()]
        );
        assert_eq!(receipt.event_types, vec!["need_user_confirm".to_string()]);
        assert_eq!(receipt.event_count, 1);
    }

    #[test]
    fn missing_required_input_payload_from_pause_maps_need_user_input_event() {
        let state = EngineRunnerState {
            paused_reason: Some("need_user_input:seg_1/q_owner".to_string()),
            ..EngineRunnerState::default()
        };
        let mut event = EngineEvent::new(EngineEventType::NeedUserInput);
        event.node_id = Some("seg_1/q_owner".to_string());
        event.data = serde_json::Map::from_iter([
            ("reason_code".to_string(), json!("missing_required_input")),
            (
                "reason".to_string(),
                json!("missing_inputs_or_runtime_refs"),
            ),
            (
                "details".to_string(),
                json!({
                    "missing_refs":["inputs.owner","params.owner"],
                    "suggested_paths":["inputs.owner","params.owner"],
                    "questions":[{"id":"owner","question":"Provide owner","required":true,"options":[]}],
                    "issues":[{"reason_code":"missing_required_input"}]
                }),
            ),
        ]);
        let record = EngineEventRecord::new("run-1", 4, "1970-01-01T00:00:00Z", event);

        let payload =
            missing_required_input_payload_from_pause(&state, std::slice::from_ref(&record), 2)
                .expect("missing payload");
        assert_eq!(
            payload.get("reason_code").and_then(Value::as_str),
            Some("missing_required_input")
        );
        assert_eq!(
            payload.pointer("/missing_refs/0"),
            Some(&json!("inputs.owner"))
        );
        assert_eq!(
            payload.pointer("/suggested_paths/0"),
            Some(&json!("inputs.owner"))
        );
        assert_eq!(payload.pointer("/questions/0/id"), Some(&json!("owner")));
    }

    #[test]
    fn intent_grounding_ready_for_todos_accepts_legacy_false_without_questions() {
        let state = EngineRunnerState {
            runtime: json!({
                "agent": {
                    "intent_grounding": {
                        "ready_for_todos": false,
                        "questions": [],
                        "resolved_inputs": {"owner":"0xabc"}
                    }
                }
            }),
            ..EngineRunnerState::default()
        };
        assert!(intent_grounding_ready_for_todos(&state));
    }

    #[test]
    fn intent_grounding_ready_for_todos_respects_false_with_questions() {
        let state = EngineRunnerState {
            runtime: json!({
                "agent": {
                    "intent_grounding": {
                        "ready_for_todos": false,
                        "questions": [{"id":"owner","question":"owner?"}],
                        "resolved_inputs": {"owner":"0xabc"}
                    }
                }
            }),
            ..EngineRunnerState::default()
        };
        assert!(!intent_grounding_ready_for_todos(&state));
    }
}
