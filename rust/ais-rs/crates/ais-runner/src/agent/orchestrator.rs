use super::context::budget_policy::{ContextPressureMode, ToolMemoryBudgetPolicy};
use super::context_view::PlanningContextManager;
use super::phase_machine::types::AgentPhase;
use super::*;
use ais_engine::{EngineEventRecord, EngineRunStatus, EngineRunnerOptions, EngineRunnerState};
use ais_sdk::documents::PlanSketchSegment;
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;

const GROUNDING_NON_ACTIONABLE_REPAIR_RETRY_LIMIT: u8 = 1;
const GROUNDING_NON_ACTIONABLE_REASON_CODE: &str = "grounding_non_actionable_pause";

#[derive(Debug, Clone)]
struct PlannedSegment {
    todo_id: String,
    summary: Option<String>,
    segment: PlanSketchSegment,
    cursor_next: String,
    done: bool,
    issues: Vec<Value>,
}

#[derive(Debug)]
pub(super) struct SegmentedAgentContext {
    pub(super) intent: String,
    pub(super) session: intent_segmented::SegmentPlanningSession,
    input_store: InputStore,
    todo_board: TodoBoard,
    pub(super) state_summary: Option<Value>,
    pub(super) previous_error: Option<Value>,
    pub(super) last_segment: Option<PlanSketchSegment>,
    completed_segments: usize,
    final_status: EngineRunStatus,
    pub(super) planning_rounds: usize,
    pub(super) planner_output_retries: usize,
    pub(super) planner_round_limit: usize,
    segment_limit: usize,
    context_manager: PlanningContextManager,
    tool_memory_projection: Option<Value>,
    checkpoint_extensions: checkpoint_ext::AgentCheckpointExtensions,
    compile_autofill_attempted_todos: BTreeSet<String>,
}

impl SegmentedAgentContext {
    fn new(
        intent: String,
        session: intent_segmented::SegmentPlanningSession,
        input_store: InputStore,
        todo_board: TodoBoard,
        planner_round_limit: usize,
        segment_limit: usize,
        planner_context_token_budget: usize,
        checkpoint_extensions: checkpoint_ext::AgentCheckpointExtensions,
    ) -> Self {
        Self {
            intent,
            session,
            input_store,
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
            compile_autofill_attempted_todos: BTreeSet::new(),
        }
    }

    fn can_continue(&self) -> bool {
        self.completed_segments < self.segment_limit
    }

    pub(super) fn refresh_state_summary(&mut self, state: &EngineRunnerState, done: bool) {
        self.state_summary = Some(self.context_manager.next_summary(
            state,
            self.completed_segments,
            done,
            self.previous_error.as_ref(),
            Some(&self.input_store),
            self.tool_memory_projection.as_ref(),
        ));
    }

    fn update_tool_memory_projection(&mut self, projection: Option<Value>) {
        self.tool_memory_projection = projection;
    }

    pub(super) fn set_previous_error_and_refresh(
        &mut self,
        state: &EngineRunnerState,
        done: bool,
        error: Value,
    ) {
        self.previous_error = Some(error);
        self.refresh_state_summary(state, done);
    }

    pub(super) fn clear_previous_error_and_refresh(
        &mut self,
        state: &EngineRunnerState,
        done: bool,
    ) {
        self.previous_error = None;
        self.refresh_state_summary(state, done);
    }

    pub(super) fn intent(&self) -> &str {
        self.intent.as_str()
    }

    pub(super) fn session(&self) -> &intent_segmented::SegmentPlanningSession {
        &self.session
    }

    pub(super) fn state_summary(&self) -> &Option<Value> {
        &self.state_summary
    }

    pub(super) fn completed_segments_u8(&self) -> u8 {
        self.completed_segments as u8
    }

    pub(super) fn has_compile_autofill_attempt(&self, key: &str) -> bool {
        self.compile_autofill_attempted_todos.contains(key)
    }

    pub(super) fn mark_compile_autofill_attempt(&mut self, key: impl Into<String>) {
        self.compile_autofill_attempted_todos.insert(key.into());
    }

    pub(super) fn input_store_mut(&mut self) -> &mut InputStore {
        &mut self.input_store
    }

    pub(super) fn todo_board(&self) -> &TodoBoard {
        &self.todo_board
    }

    pub(super) fn todo_board_mut(&mut self) -> &mut TodoBoard {
        &mut self.todo_board
    }
}

pub(super) fn execute_segmented_intent_agent(
    command: &AgentCommand,
    config: &RunnerConfig,
    pack: Option<&ais_sdk::PackDocument>,
    candidate_context: Option<CandidateContext>,
    prompt_catalog: &PromptCatalog,
) -> Result<String, RunnerError> {
    super::phase_machine::run_main_flow(command.verbose || command.verbose_llm, |phase_tracker| {
        execute_segmented_intent_agent_main(
            command,
            config,
            pack,
            candidate_context,
            prompt_catalog,
            phase_tracker,
        )
    })
}

fn execute_segmented_intent_agent_main(
    command: &AgentCommand,
    config: &RunnerConfig,
    pack: Option<&ais_sdk::PackDocument>,
    candidate_context: Option<CandidateContext>,
    prompt_catalog: &PromptCatalog,
    phase_tracker: &mut super::phase_machine::MainFlowPhaseTracker<'_>,
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
        .with_llm_transcript(
            command.llm_transcript_path.clone(),
            command.llm_transcript_append,
        )
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
        snapshot_hash: snapshot_hash.clone(),
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
    let mut input_store =
        super::build_initial_input_store(&state.runtime, config, chain_scope.as_slice())?;
    if let Some(restored) = checkpoint_extensions.input_store() {
        input_store.merge(restored);
    }
    if let Some(intent_facts) = checkpoint_extensions.intent_facts() {
        for (key, value) in intent_facts {
            super::upsert_store_value_with_source(
                &mut input_store,
                key.clone(),
                value.clone(),
                super::input_store::InputValueLayer::Seed,
                "intent",
                50,
                format!("checkpoint.intent_facts.{key}"),
            );
        }
    }
    super::runtime_store::record_runtime_agent_field(
        &mut state.runtime,
        "capability_view",
        candidate_context.capability_view(),
    );
    let capability_ready = capability_view_ready(&state);
    super::runtime_store::record_runtime_agent_field(
        &mut state.runtime,
        "capability_ready",
        Value::Bool(capability_ready),
    );
    super::record_side_effect_lifecycle(&mut state.runtime, &checkpoint_ledger);
    sync_todo_progress_receipt_tx_hashes_from_ledger(&mut state, &checkpoint_ledger);
    let runtime_has_intent_grounding = state.runtime.pointer("/agent/intent_grounding").is_some();
    let runtime_has_todo_progress = state.runtime.pointer("/agent/todo_progress").is_some();
    let mut todo_board = TodoBoard::restore_or_bootstrap(&state.runtime, intent.as_str());
    todo_board.ensure_current();
    super::runtime_store::record_todo_progress(&mut state.runtime, &todo_board);

    let initial_router = build_router_executor_for_plan(&active_plan, config)
        .map_err(RunnerError::ConfigInvalidForPlan)?;
    let readonly_autofill_router = crate::config::build_router_executor(config).ok();
    if resumed_from_checkpoint {
        if let Some(paused_reason) = super::reconcile_pending_side_effects(
            &mut checkpoint_ledger,
            &initial_router,
            &mut state,
        ) {
            super::record_side_effect_lifecycle(&mut state.runtime, &checkpoint_ledger);
            state.paused_reason = Some(paused_reason);
            super::checkpoint_flow::checkpoint_round(
                command,
                run_id.as_str(),
                &active_plan_hash,
                &active_plan,
                &state,
                &checkpoint_ledger,
                planner.planning_memory_checkpoint_value(),
                &input_store,
                &checkpoint_extensions,
            )?;
            record_planner_llm_usage(&mut state, &planner);
            return super::render_agent_output(
                command,
                &mut state,
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
    let resumed_command_max = command_builder.set_next_index_from_seen_ids(&state.seen_command_ids);
    if resumed_from_checkpoint && (command.verbose || command.verbose_llm) {
        eprintln!(
            "[checkpoint] command_id_resume mode=continue run_id={} seen_ids={} max_suffix={}",
            run_id,
            state.seen_command_ids.len(),
            resumed_command_max
        );
    }
    let trace_enabled = command.verbose || command.verbose_llm;
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
        input_store,
        todo_board,
        usize::from(planner_round_limit),
        segment_limit,
        planner_context_token_budget,
        checkpoint_extensions,
    );
    refresh_tool_memory_projection(&mut context, &mut planner, &state);
    context.refresh_state_summary(&state, false);
    phase_tracker.transition_to(AgentPhase::GroundIntent, "bootstrap_intent_grounding");
    super::trace::emit(
        trace_enabled,
        "grounding",
        "start",
        &[(
            "runtime_has_intent_grounding",
            runtime_has_intent_grounding.to_string(),
        )],
    );
    let mut grounding_repair_retries = 0u8;
    let mut reuse_runtime_grounding = runtime_has_intent_grounding;
    let grounding_ready = loop {
        let grounded = match bootstrap_intent_grounding_if_needed(
            command,
            &mut planner,
            &mut state,
            &mut context,
            &candidate_context,
            readonly_autofill_router.as_ref(),
            reuse_runtime_grounding,
        ) {
            Ok(value) => value,
            Err(error) => {
                if command.verbose {
                    eprintln!(
                        "[agent] grounding_failed entered_execute_round=false reason={}",
                        error
                    );
                }
                return Err(record_planning_failure_preserving_primary_error(
                    command,
                    run_id.as_str(),
                    &active_plan_hash,
                    &active_plan,
                    &mut state,
                    &checkpoint_ledger,
                    planner.planning_memory_checkpoint_value(),
                    &context.input_store,
                    &context.checkpoint_extensions,
                    context.planning_rounds as u64,
                    error,
                ));
            }
        };
        if grounded {
            break true;
        }
        let Some(non_actionable) = detect_grounding_non_actionable_pause(&state) else {
            break false;
        };
        super::trace::emit(
            trace_enabled,
            "grounding",
            "grounding_non_actionable_pause_detected",
            &[
                ("retry", grounding_repair_retries.to_string()),
                (
                    "max_retries",
                    GROUNDING_NON_ACTIONABLE_REPAIR_RETRY_LIMIT.to_string(),
                ),
                ("reason", non_actionable.message.clone()),
            ],
        );
        match grounding_non_actionable_action(grounding_repair_retries) {
            GroundingNonActionableAction::TerminalFallback => {
                apply_grounding_non_actionable_terminal_fallback(
                    &mut state,
                    &mut context,
                    &non_actionable,
                );
                break false;
            }
            GroundingNonActionableAction::Retry => {
                grounding_repair_retries = grounding_repair_retries.saturating_add(1);
                super::trace::emit(
                    trace_enabled,
                    "grounding",
                    "grounding_repair_retry",
                    &[
                        ("retry", grounding_repair_retries.to_string()),
                        (
                            "max_retries",
                            GROUNDING_NON_ACTIONABLE_REPAIR_RETRY_LIMIT.to_string(),
                        ),
                    ],
                );
                seed_grounding_non_actionable_repair_context(
                    &mut state,
                    &mut context,
                    &non_actionable,
                );
                reuse_runtime_grounding = false;
            }
        }
    };
    super::trace::emit(
        trace_enabled,
        "grounding",
        "complete",
        &[("ready_for_todos", grounding_ready.to_string())],
    );
    if !grounding_ready {
        phase_tracker.transition_to(AgentPhase::ResolvePause, "pause_after_grounding");
        super::trace::emit(
            trace_enabled,
            "pause_resolution",
            "paused_missing_required_input",
            &[("phase_hint", "grounding".to_string())],
        );
        super::checkpoint_flow::checkpoint_round(
            command,
            run_id.as_str(),
            &active_plan_hash,
            &active_plan,
            &state,
            &checkpoint_ledger,
            planner.planning_memory_checkpoint_value(),
            &context.input_store,
            &context.checkpoint_extensions,
        )?;
        record_planner_llm_usage(&mut state, &planner);
        return super::render_agent_output(
            command,
            &mut state,
            EngineRunStatus::Paused,
            0,
            0,
            resumed_from_checkpoint,
        );
    }
    phase_tracker.transition_to(AgentPhase::PlanTodos, "bootstrap_todos");
    super::trace::emit(
        trace_enabled,
        "todo",
        "start",
        &[(
            "runtime_has_todo_progress",
            runtime_has_todo_progress.to_string(),
        )],
    );
    if let Err(error) = bootstrap_todos_if_needed(
        command,
        &mut planner,
        &mut state,
        &mut context,
        &candidate_context,
        readonly_autofill_router.as_ref(),
        runtime_has_todo_progress,
    ) {
        if command.verbose {
            eprintln!(
                "[agent] todo_bootstrap_failed entered_execute_round=false reason={}",
                error
            );
        }
        return Err(record_planning_failure_preserving_primary_error(
            command,
            run_id.as_str(),
            &active_plan_hash,
            &active_plan,
            &mut state,
            &checkpoint_ledger,
            planner.planning_memory_checkpoint_value(),
            &context.input_store,
            &context.checkpoint_extensions,
            context.planning_rounds as u64,
            error,
        ));
    }
    super::trace::emit(
        trace_enabled,
        "todo",
        "complete",
        &[(
            "paused_reason",
            state
                .paused_reason
                .clone()
                .unwrap_or_else(|| "-".to_string()),
        )],
    );
    if state.paused_reason.as_deref() == Some("missing_required_input") {
        phase_tracker.transition_to(AgentPhase::ResolvePause, "pause_after_todo");
        super::trace::emit(
            trace_enabled,
            "pause_resolution",
            "paused_missing_required_input",
            &[("phase_hint", "todo".to_string())],
        );
        super::checkpoint_flow::checkpoint_round(
            command,
            run_id.as_str(),
            &active_plan_hash,
            &active_plan,
            &state,
            &checkpoint_ledger,
            planner.planning_memory_checkpoint_value(),
            &context.input_store,
            &context.checkpoint_extensions,
        )?;
        record_planner_llm_usage(&mut state, &planner);
        return super::render_agent_output(
            command,
            &mut state,
            EngineRunStatus::Paused,
            0,
            0,
            resumed_from_checkpoint,
        );
    }
    context.previous_error = None;
    refresh_tool_memory_projection(&mut context, &mut planner, &state);
    context.refresh_state_summary(&state, false);

    while context.can_continue() {
        phase_tracker.transition_to(AgentPhase::PlanSegment, "plan_round");
        context.todo_board.ensure_current();
        super::runtime_store::record_todo_progress(&mut state.runtime, &context.todo_board);
        super::trace::emit(
            trace_enabled,
            "plan_round",
            "start",
            &[(
                "todo_id",
                context
                    .todo_board
                    .current_todo_id()
                    .unwrap_or("-")
                    .to_string(),
            )],
        );
        let current_todo_id = context
            .todo_board
            .current_todo_id()
            .ok_or_else(|| RunnerError::Llm("todo board has no current todo".to_string()))?
            .to_string();
        if context.previous_error.is_none() {
            let precheck_refs =
                precheck_missing_input_refs_for_current_todo(&context, context.state_summary().as_ref());
            if !precheck_refs.is_empty() {
                super::trace::emit(
                    trace_enabled,
                    "plan_precheck",
                    "missing_refs_detected",
                    &[
                        ("todo_id", current_todo_id.clone()),
                        ("missing_refs", precheck_refs.join(",")),
                    ],
                );
                let precheck_payload = precheck_missing_input_payload(
                    precheck_refs.as_slice(),
                    context.completed_segments as u8,
                );
                let recovery_outcome = recover_missing_refs(
                    command,
                    &mut state,
                    &mut context,
                    &precheck_payload,
                    &candidate_context,
                    readonly_autofill_router.as_ref(),
                    current_todo_id.as_str(),
                    false,
                    "plan_precheck",
                );
                if recovery_outcome.should_retry_round() {
                    super::trace::emit(
                        trace_enabled,
                        "plan_precheck",
                        "recovery_retry_scheduled",
                        &[("todo_id", current_todo_id.clone())],
                    );
                    continue;
                }
            }
        }

        let draft = match plan_round(&mut planner, &state, &mut context) {
            Ok(draft) => draft,
            Err(error) => {
                super::trace::emit(
                    trace_enabled,
                    "plan_round",
                    "failed",
                    &[("error", error.to_string())],
                );
                if command.verbose {
                    eprintln!(
                        "[agent] plan_round_failed entered_execute_round=false reason={}",
                        error
                    );
                }
                return Err(record_planning_failure_preserving_primary_error(
                    command,
                    run_id.as_str(),
                    &active_plan_hash,
                    &active_plan,
                    &mut state,
                    &checkpoint_ledger,
                    planner.planning_memory_checkpoint_value(),
                    &context.input_store,
                    &context.checkpoint_extensions,
                    context.planning_rounds as u64,
                    error,
                ));
            }
        };
        let planned_segment = match draft {
            SegmentDraft::Proposed {
                summary,
                segment,
                cursor_next,
                done,
                issues,
            } => {
                super::trace::emit(
                    trace_enabled,
                    "plan_round",
                    "draft_proposed",
                    &[
                        ("todo_id", current_todo_id.clone()),
                        ("segment_id", segment.segment_id.clone()),
                    ],
                );
                PlannedSegment {
                    todo_id: current_todo_id.clone(),
                    summary,
                    segment,
                    cursor_next,
                    done,
                    issues,
                }
            }
            SegmentDraft::Unavailable {
                reason_code,
                message,
                done,
                issues,
                questions,
            } => {
                super::trace::emit(
                    trace_enabled,
                    "plan_round",
                    "draft_unavailable",
                    &[
                        ("todo_id", current_todo_id.clone()),
                        ("reason_code", reason_code.clone()),
                        ("questions", questions.len().to_string()),
                    ],
                );
                if reason_code == "missing_required_input" {
                    let payload = super::missing_input::payload(
                        message.as_deref(),
                        questions.as_slice(),
                        issues.as_slice(),
                        context.completed_segments as u8,
                    );
                    match super::phase_machine::pause::recover_missing_required_input_payload(
                        command,
                        &mut state,
                        &mut context,
                        &candidate_context,
                        readonly_autofill_router.as_ref(),
                        &payload,
                        current_todo_id.as_str(),
                        done,
                        "plan_round",
                        false,
                        true,
                    )? {
                        super::phase_machine::pause::MissingRequiredInputRecoveryBackflow::RetryScheduled => {
                            continue;
                        }
                        super::phase_machine::pause::MissingRequiredInputRecoveryBackflow::ResolvedByUserInput { answers } => {
                            context.todo_board.mark_current_todo();
                            super::runtime_store::record_todo_progress(
                                &mut state.runtime,
                                &context.todo_board,
                            );
                            context.set_previous_error_and_refresh(
                                &state,
                                done,
                                super::missing_input::resolved_payload(
                                    &answers,
                                    context.completed_segments as u8,
                                ),
                            );
                            super::checkpoint_flow::checkpoint_round(
                                command,
                                run_id.as_str(),
                                &active_plan_hash,
                                &active_plan,
                                &state,
                                &checkpoint_ledger,
                                planner.planning_memory_checkpoint_value(),
                                &context.input_store,
                                &context.checkpoint_extensions,
                            )?;
                            if command.verbose {
                                eprintln!(
                                    "[agent] missing_required_input resolved via user answers keys={}",
                                    answers.keys().cloned().collect::<Vec<_>>().join(",")
                                );
                            }
                            super::trace::emit(
                                trace_enabled,
                                "pause_resolution",
                                "resolved_by_user_input",
                                &[("todo_id", current_todo_id.clone())],
                            );
                            continue;
                        }
                        super::phase_machine::pause::MissingRequiredInputRecoveryBackflow::Paused => {
                            context
                                .todo_board
                                .mark_current_blocked("missing_required_input");
                            super::runtime_store::record_todo_progress(
                                &mut state.runtime,
                                &context.todo_board,
                            );
                            super::checkpoint_flow::checkpoint_round(
                                command,
                                run_id.as_str(),
                                &active_plan_hash,
                                &active_plan,
                                &state,
                                &checkpoint_ledger,
                                planner.planning_memory_checkpoint_value(),
                                &context.input_store,
                                &context.checkpoint_extensions,
                            )?;
                            super::trace::emit(
                                trace_enabled,
                                "pause_resolution",
                                "paused_missing_required_input",
                                &[("todo_id", current_todo_id.clone())],
                            );
                            context.final_status = EngineRunStatus::Paused;
                            break;
                        }
                    }
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
                super::trace::emit(
                    trace_enabled,
                    "plan_round",
                    "draft_invalid",
                    &[
                        ("todo_id", current_todo_id.clone()),
                        ("reason_code", reason_code.clone()),
                    ],
                );
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
        super::runtime_store::record_todo_progress(&mut state.runtime, &context.todo_board);
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
            &mut planned_segment,
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
                if let Some(payload) = compile_error_missing_required_input_payload(
                    &error_payload,
                    context.completed_segments as u8,
                ) {
                    if try_schedule_compile_autofill_round(
                        command,
                        &mut state,
                        &mut context,
                        &error_payload,
                        &payload,
                        &candidate_context,
                        current_todo_id.as_str(),
                        planned_segment.done,
                    ) {
                        continue;
                    }
                    let prompt_payload = compile_missing_input_prompt_payload(&state, &payload);
                    if !super::phase_machine::pause::can_prompt_user_missing_input(&prompt_payload)
                    {
                        super::missing_input::pause_with_payload(&mut state, &prompt_payload);
                        context
                            .todo_board
                            .mark_current_blocked("missing_required_input");
                        super::runtime_store::record_todo_progress(
                            &mut state.runtime,
                            &context.todo_board,
                        );
                        super::checkpoint_flow::checkpoint_round(
                            command,
                            run_id.as_str(),
                            &active_plan_hash,
                            &active_plan,
                            &state,
                            &checkpoint_ledger,
                            planner.planning_memory_checkpoint_value(),
                            &context.input_store,
                            &context.checkpoint_extensions,
                        )?;
                        context.final_status = EngineRunStatus::Paused;
                        break;
                    }
                    match super::phase_machine::pause::resolve_missing_required_input_payload(
                        &mut state,
                        context.input_store_mut(),
                        &prompt_payload,
                        false,
                    )? {
                        super::phase_machine::pause::MissingRequiredInputBackflow::ResolvedByUserInput {
                            answers,
                        } => {
                            context.todo_board.mark_current_todo();
                            super::runtime_store::record_todo_progress(
                                &mut state.runtime,
                                &context.todo_board,
                            );
                            context.set_previous_error_and_refresh(
                                &state,
                                planned_segment.done,
                                super::missing_input::resolved_payload(
                                    &answers,
                                    context.completed_segments as u8,
                                ),
                            );
                            super::checkpoint_flow::checkpoint_round(
                                command,
                                run_id.as_str(),
                                &active_plan_hash,
                                &active_plan,
                                &state,
                                &checkpoint_ledger,
                                planner.planning_memory_checkpoint_value(),
                                &context.input_store,
                                &context.checkpoint_extensions,
                            )?;
                            if command.verbose {
                                eprintln!(
                                    "[agent] compile missing_required_input resolved via user answers keys={}",
                                    answers.keys().cloned().collect::<Vec<_>>().join(",")
                                );
                            }
                            super::trace::emit(
                                trace_enabled,
                                "pause_resolution",
                                "resolved_by_user_input",
                                &[("todo_id", current_todo_id.clone())],
                            );
                            continue;
                        }
                        super::phase_machine::pause::MissingRequiredInputBackflow::Paused => {
                            context
                                .todo_board
                                .mark_current_blocked("missing_required_input");
                            super::runtime_store::record_todo_progress(
                                &mut state.runtime,
                                &context.todo_board,
                            );
                            super::checkpoint_flow::checkpoint_round(
                                command,
                                run_id.as_str(),
                                &active_plan_hash,
                                &active_plan,
                                &state,
                                &checkpoint_ledger,
                                planner.planning_memory_checkpoint_value(),
                                &context.input_store,
                                &context.checkpoint_extensions,
                            )?;
                            super::trace::emit(
                                trace_enabled,
                                "pause_resolution",
                                "paused_missing_required_input",
                                &[("todo_id", current_todo_id.clone())],
                            );
                            context.final_status = EngineRunStatus::Paused;
                            break;
                        }
                    }
                }
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
        let execution_precheck_refs = collect_segment_missing_input_refs(
            &planned_segment.segment,
            &context.input_store,
        );
        if !execution_precheck_refs.is_empty() {
            super::trace::emit(
                trace_enabled,
                "execute_precheck",
                "missing_refs_detected",
                &[
                    ("todo_id", planned_segment.todo_id.clone()),
                    ("segment_id", planned_segment.segment.segment_id.clone()),
                    ("missing_refs", execution_precheck_refs.join(",")),
                ],
            );
            let payload = precheck_missing_input_payload(
                execution_precheck_refs.as_slice(),
                context.completed_segments as u8,
            );
            let recovery_outcome = recover_missing_refs(
                command,
                &mut state,
                &mut context,
                &payload,
                &candidate_context,
                readonly_autofill_router.as_ref(),
                planned_segment.todo_id.as_str(),
                planned_segment.done,
                "execute_precheck",
            );
            if recovery_outcome.should_retry_round() {
                super::trace::emit(
                    trace_enabled,
                    "execute_precheck",
                    "recovery_retry_scheduled",
                    &[("todo_id", planned_segment.todo_id.clone())],
                );
                continue;
            }
            context.set_previous_error_and_refresh(&state, planned_segment.done, payload);
            continue;
        }

        phase_tracker.transition_to(AgentPhase::ExecuteSegment, "execute_round");
        super::trace::emit(
            trace_enabled,
            "execute_round",
            "start",
            &[
                ("todo_id", planned_segment.todo_id.clone()),
                ("segment_id", planned_segment.segment.segment_id.clone()),
            ],
        );
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
            &mut context.input_store,
            &context.checkpoint_extensions,
            &mut total_events,
            planned_segment.todo_id.as_str(),
        )?;
        if command.verbose {
            eprintln!(
                "[agent] execute_round_done segment_id={} status={} iterations={} events={}",
                planned_segment.segment.segment_id,
                run_status_name(execute_outcome.status.clone()),
                execute_outcome.iterations,
                execute_outcome.round_events.len()
            );
        }
        super::trace::emit(
            trace_enabled,
            "execute_round",
            "done",
            &[
                ("todo_id", planned_segment.todo_id.clone()),
                ("segment_id", planned_segment.segment.segment_id.clone()),
                (
                    "status",
                    run_status_name(execute_outcome.status.clone()).to_string(),
                ),
            ],
        );
        total_iterations = total_iterations.saturating_add(execute_outcome.iterations);
        let execute_status = execute_outcome.status.clone();
        let todo_receipt = build_todo_receipt(
            &planned_segment,
            execute_status.clone(),
            &mut state,
            &checkpoint_ledger,
            execute_outcome.round_events.as_slice(),
        );
        context
            .todo_board
            .record_receipt_for_todo(planned_segment.todo_id.as_str(), todo_receipt);

        match execute_status {
            EngineRunStatus::Completed | EngineRunStatus::Stopped => {
                context.completed_segments = context.completed_segments.saturating_add(1);
                context.session.cursor = planned_segment.cursor_next;
                let completion_gate_done = advance_todo_after_execute_completion(
                    &mut context.todo_board,
                    context.state_summary.as_ref(),
                    planned_segment.done,
                );
                super::runtime_store::record_todo_progress(&mut state.runtime, &context.todo_board);
                context.previous_error = None;
                context.last_segment = Some(planned_segment.segment);
                context.final_status = execute_outcome.status;
                context.refresh_state_summary(&state, completion_gate_done);
                if completion_gate_done {
                    break;
                }
            }
            EngineRunStatus::Paused => {
                phase_tracker.transition_to(AgentPhase::ResolvePause, "resolve_execution_pause");
                let pause_round = context.completed_segments as u8;
                if let Some(payload) =
                    super::phase_machine::pause::missing_required_input_payload_from_pause(
                        &state,
                        execute_outcome.last_iteration_events.as_slice(),
                        pause_round,
                    )
                {
                    match super::phase_machine::pause::recover_missing_required_input_payload(
                        command,
                        &mut state,
                        &mut context,
                        &candidate_context,
                        readonly_autofill_router.as_ref(),
                        &payload,
                        planned_segment.todo_id.as_str(),
                        planned_segment.done,
                        "pause_resolution",
                        true,
                        true,
                    )? {
                        super::phase_machine::pause::MissingRequiredInputRecoveryBackflow::RetryScheduled => {
                            context.todo_board.mark_current_todo();
                            super::runtime_store::record_todo_progress(
                                &mut state.runtime,
                                &context.todo_board,
                            );
                            super::checkpoint_flow::checkpoint_round(
                                command,
                                run_id.as_str(),
                                &active_plan_hash,
                                &active_plan,
                                &state,
                                &checkpoint_ledger,
                                planner.planning_memory_checkpoint_value(),
                                &context.input_store,
                                &context.checkpoint_extensions,
                            )?;
                            super::trace::emit(
                                trace_enabled,
                                "pause_resolution",
                                "autofill_scheduled",
                                &[
                                    ("todo_id", planned_segment.todo_id.clone()),
                                    ("segment_id", planned_segment.segment.segment_id.clone()),
                                ],
                            );
                            continue;
                        }
                        super::phase_machine::pause::MissingRequiredInputRecoveryBackflow::ResolvedByUserInput { answers } => {
                            context.todo_board.mark_current_todo();
                            super::runtime_store::record_todo_progress(
                                &mut state.runtime,
                                &context.todo_board,
                            );
                            context.set_previous_error_and_refresh(
                                &state,
                                planned_segment.done,
                                super::missing_input::resolved_payload(
                                    &answers,
                                    context.completed_segments as u8,
                                ),
                            );
                            super::checkpoint_flow::checkpoint_round(
                                command,
                                run_id.as_str(),
                                &active_plan_hash,
                                &active_plan,
                                &state,
                                &checkpoint_ledger,
                                planner.planning_memory_checkpoint_value(),
                                &context.input_store,
                                &context.checkpoint_extensions,
                            )?;
                            if command.verbose {
                                eprintln!(
                                    "[agent] execution missing_required_input resolved via user answers keys={}",
                                    answers.keys().cloned().collect::<Vec<_>>().join(",")
                                );
                            }
                            super::trace::emit(
                                trace_enabled,
                                "pause_resolution",
                                "resolved_by_user_input",
                                &[
                                    ("todo_id", planned_segment.todo_id.clone()),
                                    ("segment_id", planned_segment.segment.segment_id.clone()),
                                ],
                            );
                            continue;
                        }
                        super::phase_machine::pause::MissingRequiredInputRecoveryBackflow::Paused => {
                            context
                                .todo_board
                                .mark_current_blocked("missing_required_input");
                            super::runtime_store::record_todo_progress(
                                &mut state.runtime,
                                &context.todo_board,
                            );
                            super::trace::emit(
                                trace_enabled,
                                "pause_resolution",
                                "paused_missing_required_input",
                                &[
                                    ("todo_id", planned_segment.todo_id.clone()),
                                    ("segment_id", planned_segment.segment.segment_id.clone()),
                                ],
                            );
                            context.final_status = EngineRunStatus::Paused;
                            break;
                        }
                    }
                }
                match super::phase_machine::pause::resolve_execution_pause_backflow(
                    &mut state,
                    context.input_store_mut(),
                    execute_outcome.last_iteration_events.as_slice(),
                    pause_round,
                )? {
                    super::phase_machine::pause::ResolvePauseBackflow::MissingRequiredInputResolved {
                        answers,
                    } => {
                        context.todo_board.mark_current_todo();
                        super::runtime_store::record_todo_progress(
                            &mut state.runtime,
                            &context.todo_board,
                        );
                        context.set_previous_error_and_refresh(
                            &state,
                            planned_segment.done,
                            super::missing_input::resolved_payload(
                                &answers,
                                context.completed_segments as u8,
                            ),
                        );
                        super::checkpoint_flow::checkpoint_round(
                            command,
                            run_id.as_str(),
                            &active_plan_hash,
                            &active_plan,
                            &state,
                            &checkpoint_ledger,
                            planner.planning_memory_checkpoint_value(),
                            &context.input_store,
                            &context.checkpoint_extensions,
                        )?;
                        if command.verbose {
                            eprintln!(
                                "[agent] execution missing_required_input resolved via user answers keys={}",
                                answers.keys().cloned().collect::<Vec<_>>().join(",")
                            );
                        }
                        super::trace::emit(
                            trace_enabled,
                            "pause_resolution",
                            "resolved_by_user_input",
                            &[
                                ("todo_id", planned_segment.todo_id.clone()),
                                ("segment_id", planned_segment.segment.segment_id.clone()),
                            ],
                        );
                        continue;
                    }
                    super::phase_machine::pause::ResolvePauseBackflow::MissingRequiredInputPaused => {
                        context
                            .todo_board
                            .mark_current_blocked("missing_required_input");
                        super::runtime_store::record_todo_progress(
                            &mut state.runtime,
                            &context.todo_board,
                        );
                        super::trace::emit(
                            trace_enabled,
                            "pause_resolution",
                            "paused_missing_required_input",
                            &[
                                ("todo_id", planned_segment.todo_id.clone()),
                                ("segment_id", planned_segment.segment.segment_id.clone()),
                            ],
                        );
                        context.final_status = EngineRunStatus::Paused;
                        break;
                    }
                    super::phase_machine::pause::ResolvePauseBackflow::PauseTerminal {
                        blocked_reason,
                    } => {
                        context.final_status = EngineRunStatus::Paused;
                        context.todo_board.mark_current_blocked(blocked_reason);
                        super::runtime_store::record_todo_progress(
                            &mut state.runtime,
                            &context.todo_board,
                        );
                        super::trace::emit(
                            trace_enabled,
                            "pause_resolution",
                            "pause_terminal",
                            &[
                                ("todo_id", planned_segment.todo_id.clone()),
                                ("segment_id", planned_segment.segment.segment_id.clone()),
                                (
                                    "paused_reason",
                                    state
                                        .paused_reason
                                        .clone()
                                        .unwrap_or_else(|| "-".to_string()),
                                ),
                            ],
                        );
                        break;
                    }
                    super::phase_machine::pause::ResolvePauseBackflow::RepairScheduled {
                        previous_error,
                    } => {
                        context.final_status = EngineRunStatus::Paused;
                        super::trace::emit(
                            trace_enabled,
                            "pause_resolution",
                            "pause_repair_scheduled",
                            &[
                                ("todo_id", planned_segment.todo_id.clone()),
                                ("segment_id", planned_segment.segment.segment_id.clone()),
                            ],
                        );
                        context.set_previous_error_and_refresh(
                            &state,
                            planned_segment.done,
                            previous_error,
                        );
                        super::runtime_store::record_todo_progress(
                            &mut state.runtime,
                            &context.todo_board,
                        );
                        context.last_segment = Some(planned_segment.segment);
                    }
                }
            }
        }
    }

    if matches!(
        context.final_status,
        EngineRunStatus::Completed | EngineRunStatus::Stopped
    ) {
        super::checkpoint_flow::checkpoint_round(
            command,
            run_id.as_str(),
            &active_plan_hash,
            &active_plan,
            &state,
            &checkpoint_ledger,
            planner.planning_memory_checkpoint_value(),
            &context.input_store,
            &context.checkpoint_extensions,
        )?;
    }
    record_planner_llm_usage(&mut state, &planner);
    super::render_agent_output(
        command,
        &mut state,
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
    super::runtime_store::record_runtime_agent_field(
        &mut state.runtime,
        "llm_usage",
        planner.llm_usage_value(),
    );
    super::runtime_store::record_runtime_agent_field(
        &mut state.runtime,
        "tool_lifecycle",
        planner.tool_lifecycle_value(),
    );
}

pub(super) fn refresh_tool_memory_projection<P>(
    context: &mut SegmentedAgentContext,
    planner: &mut LlmSegmentedIntentPlanner<P>,
    state: &EngineRunnerState,
) {
    let planner_usage = planner.llm_usage_value();
    let runtime_usage = state.runtime.pointer("/agent/llm_usage");
    let store_budget = resolve_planning_memory_store_budget(Some(&planner_usage), runtime_usage);
    planner.set_planning_memory_budget(store_budget);
    let token_budget =
        resolve_tool_memory_projection_token_budget(Some(&planner_usage), runtime_usage);
    let pressure_mode = resolve_tool_pressure_mode(context, runtime_usage);
    let compress_level = ToolMemoryBudgetPolicy::derive_global_compress_level(pressure_mode);
    let candidates = planner.tool_memory_projection_candidates_value(token_budget);
    let projection = candidates.select_for_level(compress_level);
    let estimated_tokens = projection
        .as_ref()
        .and_then(|value| value.pointer("/estimated_tokens"))
        .and_then(Value::as_u64);
    planner.observe_tool_memory_projection(token_budget, estimated_tokens);
    context.update_tool_memory_projection(projection);
}

fn resolve_tool_pressure_mode(
    context: &SegmentedAgentContext,
    runtime_usage: Option<&Value>,
) -> ContextPressureMode {
    if let Some(mode) = context
        .state_summary
        .as_ref()
        .and_then(|summary| summary.pointer("/context_budget/pressure_mode"))
        .and_then(Value::as_str)
        .and_then(ContextPressureMode::from_str)
    {
        return mode;
    }
    if let Some(remaining) = runtime_usage.and_then(|value| {
        value
            .get("context_remaining_tokens")
            .and_then(Value::as_u64)
    }) {
        let usage_ratio = runtime_usage
            .and_then(|value| {
                value
                    .get("context_soft_limit_tokens")
                    .and_then(Value::as_u64)
            })
            .map(|soft_limit| {
                if soft_limit == 0 {
                    0
                } else {
                    10_000_u64.saturating_sub(remaining.saturating_mul(10_000) / soft_limit)
                }
            });
        return ToolMemoryBudgetPolicy::derive_context_pressure_mode(usage_ratio, Some(remaining));
    }
    ContextPressureMode::Normal
}

fn resolve_tool_memory_projection_token_budget(
    planner_usage: Option<&Value>,
    runtime_usage: Option<&Value>,
) -> usize {
    ToolMemoryBudgetPolicy::derive_tool_memory_projection_token_budget(planner_usage, runtime_usage)
}

fn resolve_planning_memory_store_budget(
    planner_usage: Option<&Value>,
    runtime_usage: Option<&Value>,
) -> planning_memory::PlanningMemoryBudget {
    let budget =
        ToolMemoryBudgetPolicy::derive_planning_memory_store_budget(planner_usage, runtime_usage);
    planning_memory::PlanningMemoryBudget {
        max_entries: budget.max_entries,
        max_entry_chars: budget.max_entry_chars,
        max_total_chars: budget.max_total_chars,
    }
}

fn bootstrap_todos_if_needed<P: LlmProvider>(
    command: &AgentCommand,
    planner: &mut LlmSegmentedIntentPlanner<P>,
    state: &mut EngineRunnerState,
    context: &mut SegmentedAgentContext,
    candidate_context: &CandidateContext,
    readonly_autofill_router: Option<&ais_engine::RouterExecutor>,
    runtime_has_todo_progress: bool,
) -> Result<(), RunnerError> {
    super::phase_machine::todo::bootstrap_todos_if_needed(
        command,
        planner,
        state,
        context,
        candidate_context,
        readonly_autofill_router,
        runtime_has_todo_progress,
    )
}

fn bootstrap_intent_grounding_if_needed<P: LlmProvider>(
    command: &AgentCommand,
    planner: &mut LlmSegmentedIntentPlanner<P>,
    state: &mut EngineRunnerState,
    context: &mut SegmentedAgentContext,
    candidate_context: &CandidateContext,
    readonly_autofill_router: Option<&ais_engine::RouterExecutor>,
    runtime_has_intent_grounding: bool,
) -> Result<bool, RunnerError> {
    super::phase_machine::grounding::bootstrap_intent_grounding_if_needed(
        command,
        planner,
        state,
        context,
        candidate_context,
        readonly_autofill_router,
        runtime_has_intent_grounding,
    )
}

#[cfg(test)]
fn missing_required_input_payload_from_pause(
    state: &EngineRunnerState,
    events: &[EngineEventRecord],
    round: u8,
) -> Option<Value> {
    super::phase_machine::pause::missing_required_input_payload_from_pause(state, events, round)
}

#[cfg(test)]
fn apply_intent_grounding(
    state: &mut EngineRunnerState,
    input_store: &mut InputStore,
    resolved_inputs: &std::collections::BTreeMap<String, Value>,
    intent_facts: &std::collections::BTreeMap<String, Value>,
    confidence: &std::collections::BTreeMap<String, u8>,
    intent_text: &str,
) -> super::phase_machine::grounding::GroundingApplySummary {
    super::phase_machine::grounding::apply_intent_grounding(
        state,
        input_store,
        resolved_inputs,
        intent_facts,
        confidence,
        intent_text,
    )
}

#[cfg(test)]
fn intent_grounding_ready_for_todos(state: &EngineRunnerState) -> bool {
    super::phase_machine::grounding::intent_grounding_ready_for_todos(state)
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
    super::phase_machine::segment_plan::plan_round(planner, state, context)
}

fn compile_guard(
    planned: &mut PlannedSegment,
    context: &SegmentedAgentContext,
    candidate_context: &CandidateContext,
    pack: Option<&ais_sdk::PackDocument>,
    chain_scope: &[String],
) -> Result<PlanDocument, Value> {
    let known_refs = super::known_input_refs_from_state_summary(context.state_summary.as_ref());
    let grounding_fact_keys =
        super::grounding_fact_keys_from_state_summary(context.state_summary.as_ref());
    planned.segment = super::canonicalize_segment_input_refs(
        &planned.segment,
        &known_refs,
        &grounding_fact_keys,
    )?;
    super::validate_segment_todo_scope(
        &planned.segment,
        candidate_context,
        context
            .state_summary
            .as_ref()
            .and_then(|summary| summary.pointer("/todo_state/current_todo")),
    )?;
    super::compile_segment_plan_with_inputs(
        context.intent.as_str(),
        &context.session,
        &planned.segment,
        candidate_context,
        pack,
        chain_scope,
        known_refs.as_slice(),
    )
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

fn compile_error_missing_required_input_payload(error_payload: &Value, round: u8) -> Option<Value> {
    let issues = error_payload
        .get("issues")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut missing_refs = BTreeSet::<String>::new();
    let mut missing_input_issues = Vec::<Value>::new();
    for issue in issues {
        let reference = issue.get("reference").and_then(Value::as_str).unwrap_or("");
        let kind = issue.get("kind").and_then(Value::as_str).unwrap_or("");
        if reference == "unknown_input_ref" {
            missing_input_issues.push(issue.clone());
            if let Some(suggested_ref) = issue.get("suggested_ref").and_then(Value::as_str) {
                collect_missing_input_ref(suggested_ref, Some(&issue), &mut missing_refs);
            }
            for candidate in issue
                .get("candidates")
                .and_then(Value::as_array)
                .into_iter()
                .flat_map(|items| items.iter())
                .filter_map(Value::as_str)
            {
                collect_missing_input_ref(candidate, Some(&issue), &mut missing_refs);
            }
            if let Some(message) = issue.get("message").and_then(Value::as_str) {
                collect_missing_input_refs_from_message(message, &mut missing_refs);
            }
            continue;
        }
        if is_write_gate_missing_input_issue(&issue, kind) {
            missing_input_issues.push(issue.clone());
            if let Some(required_fact) = issue.get("required_fact").and_then(Value::as_str) {
                collect_missing_input_ref(required_fact, Some(&issue), &mut missing_refs);
            }
            if let Some(message) = issue.get("message").and_then(Value::as_str) {
                collect_missing_input_refs_from_message(message, &mut missing_refs);
            }
            continue;
        }
    }

    if missing_input_issues.is_empty() {
        return None;
    }
    if let Some(message) = error_payload.get("message").and_then(Value::as_str) {
        collect_missing_input_refs_from_message(message, &mut missing_refs);
    }
    if missing_refs.is_empty() {
        return None;
    }

    let missing_refs_vec = missing_refs.into_iter().collect::<Vec<_>>();
    let suggested_paths = missing_refs_vec.clone();
    let questions = missing_refs_vec
        .iter()
        .map(|path| {
            let id = path.strip_prefix("inputs.").unwrap_or(path.as_str());
            serde_json::json!({
                "id": id,
                "question": format!("Please provide `{id}`"),
                "required": true,
                "options": [],
            })
        })
        .collect::<Vec<_>>();

    Some(super::missing_input::payload_with_context(
        Some("missing inputs required for plan compile"),
        questions.as_slice(),
        missing_input_issues.as_slice(),
        missing_refs_vec.as_slice(),
        suggested_paths.as_slice(),
        round,
    ))
}

fn compile_missing_input_prompt_payload(state: &EngineRunnerState, payload: &Value) -> Value {
    let status = state
        .runtime
        .pointer("/agent/compile_autofill/status")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let reason = state
        .runtime
        .pointer("/agent/compile_autofill/reason")
        .and_then(Value::as_str)
        .unwrap_or("compile_autofill_unknown");
    let missing_refs = super::missing_ref_recovery::missing_required_input_refs(payload);
    let recovery_status = if status == "unresolved" {
        "compile_autofill_exhausted"
    } else {
        "need_user_input"
    };
    super::phase_machine::pause::attach_missing_input_recovery(
        payload,
        recovery_status,
        reason,
        "compile_autofill",
        "compile_autofill",
        "compile",
        missing_refs.as_slice(),
    )
}

fn is_write_gate_missing_input_issue(issue: &Value, kind: &str) -> bool {
    if kind != "write_gate_missing" {
        return false;
    }
    let reason_code = issue
        .get("reason_code")
        .and_then(Value::as_str)
        .unwrap_or("");
    if reason_code.starts_with("missing_") {
        return true;
    }
    let required_fact = issue
        .get("required_fact")
        .and_then(Value::as_str)
        .unwrap_or("");
    if super::input_normalize::normalize_missing_input_ref(required_fact).is_some() {
        return true;
    }
    let mut message_refs = BTreeSet::new();
    let has_message_refs = issue
        .get("message")
        .and_then(Value::as_str)
        .is_some_and(|message| {
            collect_missing_input_refs_from_message(message, &mut message_refs);
            !message_refs.is_empty()
        });
    has_message_refs || reason_code == "missing_required_input"
}

fn advance_todo_after_execute_completion(
    todo_board: &mut TodoBoard,
    state_summary: Option<&Value>,
    planner_done: bool,
) -> bool {
    todo_board.mark_current_done();
    let acceptance_complete = todo_board.intent_acceptance_complete(state_summary);
    if !planner_done && todo_board.current().is_none() && !acceptance_complete {
        todo_board.open_follow_up_todo();
    }
    planner_done || acceptance_complete
}

fn precheck_missing_input_refs_for_current_todo(
    context: &SegmentedAgentContext,
    state_summary: Option<&Value>,
) -> Vec<String> {
    let Some(current_todo) = context.todo_board.current() else {
        return Vec::new();
    };
    let mut refs = BTreeSet::<String>::new();
    for fact in &current_todo.required_facts {
        let Some(slot) = super::input_normalize::normalize_missing_input_ref(fact) else {
            continue;
        };
        let canonical_ref = format!("inputs.{slot}");
        if !runtime_has_input_ref(state_summary, canonical_ref.as_str()) {
            refs.insert(canonical_ref);
        }
    }
    refs.into_iter().collect::<Vec<_>>()
}

fn precheck_missing_input_payload(missing_refs: &[String], round: u8) -> Value {
    let questions = missing_refs
        .iter()
        .map(|reference| {
            serde_json::json!({
                "id": reference,
                "question": format!("Provide `{reference}`"),
                "required": true,
                "options": [],
            })
        })
        .collect::<Vec<_>>();
    super::missing_input::payload_with_context(
        Some("todo precheck missing required inputs"),
        questions.as_slice(),
        &[],
        missing_refs,
        missing_refs,
        round,
    )
}

pub(super) fn recover_missing_refs(
    command: &AgentCommand,
    state: &mut EngineRunnerState,
    context: &mut SegmentedAgentContext,
    missing_input_payload: &Value,
    candidate_context: &CandidateContext,
    readonly_autofill_router: Option<&ais_engine::RouterExecutor>,
    scope_id: &str,
    done: bool,
    phase_hint: &'static str,
) -> super::missing_ref_recovery::RecoveryOutcome {
    super::missing_ref_recovery::recover_missing_refs(
        command,
        state,
        context,
        missing_input_payload,
        candidate_context,
        readonly_autofill_router,
        scope_id,
        done,
        phase_hint,
    )
}

fn try_schedule_compile_autofill_round(
    command: &AgentCommand,
    state: &mut EngineRunnerState,
    context: &mut SegmentedAgentContext,
    compile_error_payload: &Value,
    missing_input_payload: &Value,
    candidate_context: &CandidateContext,
    todo_id: &str,
    done: bool,
) -> bool {
    let trace_enabled = command.verbose || command.verbose_llm;
    emit_unknown_input_ref_repair_suggested(trace_enabled, compile_error_payload, todo_id);
    let initial_missing_refs =
        super::missing_ref_recovery::missing_required_input_refs(missing_input_payload);
    let static_outcome = apply_static_missing_ref_refill(
        state,
        context,
        initial_missing_refs.as_slice(),
        "compile_autofill",
        todo_id,
    );
    let missing_refs = initial_missing_refs
        .into_iter()
        .filter(|path| !runtime_has_input_ref(context.state_summary.as_ref(), path))
        .collect::<Vec<_>>();
    let ambiguous_bindings = static_outcome
        .ambiguous_bindings
        .iter()
        .filter(|item| {
            missing_refs
                .iter()
                .any(|missing| missing == &item.missing_ref)
        })
        .map(|item| {
            serde_json::json!({
                "missing_ref": item.missing_ref,
                "candidate_refs": item.candidate_refs,
            })
        })
        .collect::<Vec<_>>();
    let adjudicate_retry_key = format!("binding_adjudicate:compile:{todo_id}");
    if !ambiguous_bindings.is_empty()
        && !context
            .compile_autofill_attempted_todos
            .contains(adjudicate_retry_key.as_str())
    {
        context
            .compile_autofill_attempted_todos
            .insert(adjudicate_retry_key);
        let mut previous_error = super::compile_error_state_payload(
            compile_error_payload,
            context.completed_segments_u8(),
        );
        if let Some(object) = previous_error.as_object_mut() {
            object.insert(
                "autofill".to_string(),
                serde_json::json!({
                    "mode": "host_binding_adjudicate_round",
                    "todo_id": todo_id,
                    "resolved_refs": static_outcome.resolved_refs.clone(),
                    "unresolved_refs": missing_refs.clone(),
                    "ambiguous_bindings": ambiguous_bindings,
                    "available_input_refs": available_input_ref_catalog(context.state_summary.as_ref()),
                    "query_candidate_pool": [],
                }),
            );
        }
        context.set_previous_error_and_refresh(state, done, previous_error);
        super::runtime_store::record_runtime_agent_field(
            &mut state.runtime,
            "missing_ref_refill",
            serde_json::json!({
                "status": "adjudicate_scheduled",
                "todo_id": todo_id,
                "attempt": "llm_binding_adjudicate",
                "resolved_refs": static_outcome.resolved_refs.clone(),
                "unresolved_refs": missing_refs.clone(),
            }),
        );
        super::trace::emit(
            trace_enabled,
            "compile_autofill",
            "autofill_attempt_resolved",
            &[
                ("todo_id", todo_id.to_string()),
                ("selected_query_refs", "llm_binding_adjudicate".to_string()),
            ],
        );
        return true;
    }
    if !static_outcome.resolved_refs.is_empty() {
        let mut previous_error = super::compile_error_state_payload(
            compile_error_payload,
            context.completed_segments_u8(),
        );
        if let Some(object) = previous_error.as_object_mut() {
            object.insert(
                "autofill".to_string(),
                serde_json::json!({
                    "mode": "host_static_refill_round",
                    "todo_id": todo_id,
                    "resolved_refs": static_outcome.resolved_refs.clone(),
                    "unresolved_refs": missing_refs.clone(),
                }),
            );
        }
        context.set_previous_error_and_refresh(state, done, previous_error);
        super::runtime_store::record_runtime_agent_field(
            &mut state.runtime,
            "missing_ref_refill",
            serde_json::json!({
                "status": if missing_refs.is_empty() { "resolved" } else { "resolved_partial" },
                "todo_id": todo_id,
                "attempt": "static_intent_config",
                "resolved_refs": static_outcome.resolved_refs.clone(),
                "unresolved_refs": missing_refs.clone(),
            }),
        );
        return true;
    }
    if missing_refs.is_empty() {
        emit_compile_autofill_unresolved(trace_enabled, state, todo_id, &[], "already_resolved");
        return false;
    }
    super::trace::emit(
        trace_enabled,
        "compile_autofill",
        "start",
        &[
            ("todo_id", todo_id.to_string()),
            ("missing_refs", missing_refs.join(",")),
        ],
    );
    if context.compile_autofill_attempted_todos.contains(todo_id) {
        if compile_error_has_unknown_input_ref(compile_error_payload) {
            super::trace::emit(
                trace_enabled,
                "compile_autofill",
                "unknown_ref_repair_exhausted",
                &[("todo_id", todo_id.to_string())],
            );
            super::runtime_store::record_runtime_agent_field(
                &mut state.runtime,
                "unknown_ref_repair",
                serde_json::json!({
                    "status": "exhausted",
                    "reason_code": "unknown_input_ref_exhausted",
                    "todo_id": todo_id,
                    "missing_refs": missing_refs,
                }),
            );
        }
        emit_compile_autofill_unresolved(
            trace_enabled,
            state,
            todo_id,
            missing_refs.as_slice(),
            "retry_limited",
        );
        return false;
    }

    let resolution = super::intent_segmented::resolve_missing_facts_for_refs(
        candidate_context,
        missing_refs.as_slice(),
        3,
    );
    let selected_query_refs =
        super::missing_ref_recovery::selected_query_refs_from_missing_resolution(&resolution);
    if selected_query_refs.is_empty() {
        let available_input_refs = available_input_ref_catalog(context.state_summary.as_ref());
        if !context
            .compile_autofill_attempted_todos
            .contains(adjudicate_retry_key.as_str())
        {
            context
                .compile_autofill_attempted_todos
                .insert(adjudicate_retry_key);
            let mut previous_error = super::compile_error_state_payload(
                compile_error_payload,
                context.completed_segments_u8(),
            );
            if let Some(object) = previous_error.as_object_mut() {
                object.insert(
                    "autofill".to_string(),
                    serde_json::json!({
                        "mode": "host_binding_adjudicate_round",
                        "todo_id": todo_id,
                        "resolved_refs": static_outcome.resolved_refs.clone(),
                        "unresolved_refs": missing_refs.clone(),
                        "ambiguous_bindings": [],
                        "available_input_refs": available_input_refs,
                        "query_candidate_pool": super::missing_ref_recovery::query_candidate_pool_from_missing_resolution(&resolution),
                        "resolver": resolution,
                    }),
                );
            }
            context.set_previous_error_and_refresh(state, done, previous_error);
            super::runtime_store::record_runtime_agent_field(
                &mut state.runtime,
                "missing_ref_refill",
                serde_json::json!({
                    "status": "adjudicate_scheduled",
                    "todo_id": todo_id,
                    "attempt": "llm_binding_adjudicate",
                    "resolved_refs": static_outcome.resolved_refs.clone(),
                    "unresolved_refs": missing_refs.clone(),
                    "reason": "no_query_candidates",
                }),
            );
            super::trace::emit(
                trace_enabled,
                "compile_autofill",
                "autofill_attempt_resolved",
                &[
                    ("todo_id", todo_id.to_string()),
                    ("selected_query_refs", "llm_binding_adjudicate".to_string()),
                ],
            );
            return true;
        }
        emit_compile_autofill_unresolved(
            trace_enabled,
            state,
            todo_id,
            missing_refs.as_slice(),
            "no_query_candidates",
        );
        return false;
    }

    context
        .compile_autofill_attempted_todos
        .insert(todo_id.to_string());

    let resolver_unresolved_refs = resolution
        .get("unresolved_refs")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut previous_error =
        super::compile_error_state_payload(compile_error_payload, context.completed_segments_u8());
    if let Some(object) = previous_error.as_object_mut() {
        object.insert(
            "autofill".to_string(),
            serde_json::json!({
                "mode": "host_compile_round",
                "missing_refs": missing_refs,
                "selected_query_refs": selected_query_refs,
                "resolver_unresolved_refs": resolver_unresolved_refs,
                "resolver": resolution,
            }),
        );
    }
    context.set_previous_error_and_refresh(state, done, previous_error);
    super::runtime_store::record_runtime_agent_field(
        &mut state.runtime,
        "compile_autofill",
        serde_json::json!({
            "status": "scheduled",
            "todo_id": todo_id,
            "missing_refs": super::missing_ref_recovery::missing_required_input_refs(missing_input_payload),
            "selected_query_refs": selected_query_refs,
        }),
    );
    super::runtime_store::record_runtime_agent_field(
        &mut state.runtime,
        "missing_ref_refill",
        serde_json::json!({
            "status": "scheduled",
            "todo_id": todo_id,
            "attempt": "dynamic_query",
            "missing_refs": missing_refs,
            "selected_query_refs": selected_query_refs,
        }),
    );
    super::trace::emit(
        trace_enabled,
        "compile_autofill",
        "resolved",
        &[
            ("todo_id", todo_id.to_string()),
            ("selected_query_refs", selected_query_refs.join(",")),
        ],
    );
    true
}

fn compile_error_has_unknown_input_ref(payload: &Value) -> bool {
    payload
        .get("issues")
        .and_then(Value::as_array)
        .into_iter()
        .flat_map(|issues| issues.iter())
        .any(|issue| issue.get("reference").and_then(Value::as_str) == Some("unknown_input_ref"))
}

#[derive(Debug, Clone)]
struct GroundingNonActionablePause {
    message: String,
    issues: Vec<Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GroundingNonActionableAction {
    Retry,
    TerminalFallback,
}

fn detect_grounding_non_actionable_pause(
    state: &EngineRunnerState,
) -> Option<GroundingNonActionablePause> {
    if state.paused_reason.as_deref() != Some("missing_required_input") {
        return None;
    }
    let payload = state.runtime.pointer("/agent/missing_required_input")?;
    let questions_empty = payload
        .get("questions")
        .and_then(Value::as_array)
        .map_or(true, Vec::is_empty);
    let missing_refs_empty = payload
        .get("missing_refs")
        .and_then(Value::as_array)
        .map_or(true, Vec::is_empty);
    if !questions_empty || !missing_refs_empty {
        return None;
    }
    let message = payload
        .get("message")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            state
                .runtime
                .pointer("/agent/intent_grounding/message")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
        })
        .unwrap_or("intent_grounding_missing_inputs")
        .to_string();
    let issues = payload
        .get("issues")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    Some(GroundingNonActionablePause { message, issues })
}

fn grounding_non_actionable_action(retries: u8) -> GroundingNonActionableAction {
    if retries >= GROUNDING_NON_ACTIONABLE_REPAIR_RETRY_LIMIT {
        GroundingNonActionableAction::TerminalFallback
    } else {
        GroundingNonActionableAction::Retry
    }
}

fn seed_grounding_non_actionable_repair_context(
    state: &mut EngineRunnerState,
    context: &mut SegmentedAgentContext,
    non_actionable: &GroundingNonActionablePause,
) {
    clear_agent_runtime_fields(
        &mut state.runtime,
        &["missing_required_input", "intent_grounding"],
    );
    state.paused_reason = None;
    let repair_issue = json!({
        "reason_code": GROUNDING_NON_ACTIONABLE_REASON_CODE,
        "message": non_actionable.message,
        "action": "return actionable questions or missing_refs",
    });
    let mut issues = non_actionable.issues.clone();
    issues.push(repair_issue);
    context.set_previous_error_and_refresh(
        state,
        false,
        super::grounding_phase_error_payload(
            GROUNDING_NON_ACTIONABLE_REASON_CODE,
            Some(non_actionable.message.as_str()),
            issues.as_slice(),
            &[],
            context.completed_segments_u8(),
        ),
    );
}

fn apply_grounding_non_actionable_terminal_fallback(
    state: &mut EngineRunnerState,
    context: &mut SegmentedAgentContext,
    non_actionable: &GroundingNonActionablePause,
) {
    let question = json!({
        "id": "intent.clarification",
        "question": "Please clarify intent and required inputs (token, amount, recipient, chain).",
        "required": true,
        "options": [],
    });
    let missing_refs = vec!["inputs.intent.clarification".to_string()];
    let mut issues = non_actionable.issues.clone();
    issues.push(json!({
        "reason_code": GROUNDING_NON_ACTIONABLE_REASON_CODE,
        "message": non_actionable.message,
    }));
    let payload = super::missing_input::payload_with_context(
        Some("intent_grounding_non_actionable_pause"),
        std::slice::from_ref(&question),
        issues.as_slice(),
        missing_refs.as_slice(),
        missing_refs.as_slice(),
        context.completed_segments_u8(),
    );
    super::missing_input::pause_with_payload(state, &payload);
    super::runtime_store::record_runtime_agent_field(
        &mut state.runtime,
        "intent_grounding",
        json!({
            "status": "unavailable",
            "ready_for_todos": false,
            "reason_code": GROUNDING_NON_ACTIONABLE_REASON_CODE,
            "message": non_actionable.message,
            "issues": issues,
            "questions": [question.clone()],
            "missing_refs": missing_refs,
        }),
    );
    context.set_previous_error_and_refresh(
        state,
        false,
        super::grounding_phase_error_payload(
            GROUNDING_NON_ACTIONABLE_REASON_CODE,
            Some(non_actionable.message.as_str()),
            issues.as_slice(),
            &[question],
            context.completed_segments_u8(),
        ),
    );
}

fn clear_agent_runtime_fields(runtime: &mut Value, fields: &[&str]) {
    let Some(agent) = runtime.get_mut("agent").and_then(Value::as_object_mut) else {
        return;
    };
    for field in fields {
        agent.remove(*field);
    }
}

fn emit_compile_autofill_unresolved(
    trace_enabled: bool,
    state: &mut EngineRunnerState,
    todo_id: &str,
    missing_refs: &[String],
    reason: &str,
) {
    super::runtime_store::record_runtime_agent_field(
        &mut state.runtime,
        "compile_autofill",
        serde_json::json!({
            "status": "unresolved",
            "todo_id": todo_id,
            "missing_refs": missing_refs,
            "reason": reason,
        }),
    );
    super::trace::emit(
        trace_enabled,
        "compile_autofill",
        "unresolved",
        &[
            ("todo_id", todo_id.to_string()),
            ("reason", reason.to_string()),
            ("missing_refs", missing_refs.join(",")),
        ],
    );
}

fn emit_unknown_input_ref_repair_suggested(
    trace_enabled: bool,
    compile_error_payload: &Value,
    todo_id: &str,
) {
    let Some(issues) = compile_error_payload
        .get("issues")
        .and_then(Value::as_array)
    else {
        return;
    };
    for issue in issues {
        if issue.get("reference").and_then(Value::as_str) != Some("unknown_input_ref") {
            continue;
        }
        let top_candidates = issue
            .get("candidates")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .take(3)
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if top_candidates.is_empty() {
            continue;
        }
        super::trace::emit(
            trace_enabled,
            "compile_autofill",
            "unknown_input_ref_repair_suggested",
            &[
                ("todo_id", todo_id.to_string()),
                (
                    "path",
                    issue
                        .get("path")
                        .and_then(Value::as_str)
                        .unwrap_or("-")
                        .to_string(),
                ),
                (
                    "raw_ref",
                    issue
                        .get("raw_ref")
                        .and_then(Value::as_str)
                        .unwrap_or("-")
                        .to_string(),
                ),
                (
                    "suggested_ref",
                    issue
                        .get("suggested_ref")
                        .and_then(Value::as_str)
                        .unwrap_or("-")
                        .to_string(),
                ),
                ("top_candidates", top_candidates.join(",")),
            ],
        );
    }
}

pub(super) fn available_input_ref_catalog(state_summary: Option<&Value>) -> Vec<Value> {
    let mut refs = Vec::<Value>::new();
    let Some(summary) = state_summary else {
        return refs;
    };
    let mut seen = BTreeSet::<String>::new();
    let meta_map = summary
        .pointer("/input_store/meta")
        .and_then(Value::as_object);
    let facts = summary
        .pointer("/input_store/facts")
        .and_then(Value::as_object);

    // Bindable refs are sourced from InputStore projection only.
    for raw_ref in super::known_input_refs_from_state_summary(Some(summary)) {
        let slot = raw_ref.strip_prefix("inputs.").unwrap_or(raw_ref.as_str());
        let value = facts.and_then(|map| {
            map.get(slot)
                .or_else(|| map.get(raw_ref.as_str()))
                .or_else(|| value_at_dotted_path_object(map, slot))
        });
        let meta = meta_map.and_then(|entries| {
            entries
                .get(slot)
                .or_else(|| entries.get(raw_ref.as_str()))
                .or_else(|| value_at_dotted_path_object(entries, slot))
        });
        refs.push(serde_json::json!({
            "ref": raw_ref,
            "has_value": value.is_some(),
            "value_type": value.map(infer_binding_value_type).unwrap_or("unknown"),
            "source_priority": meta
                .and_then(|item| item.get("source_priority"))
                .and_then(Value::as_u64)
                .unwrap_or(0),
            "source": meta
                .and_then(|item| item.get("source"))
                .and_then(Value::as_str)
                .unwrap_or("unknown"),
        }));
        seen.insert(format!("inputs.{slot}"));
    }
    refs
}

fn infer_binding_value_type(value: &Value) -> &'static str {
    match value {
        Value::Bool(_) => "boolean",
        Value::Number(_) => "numeric",
        Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.starts_with("eip155:") {
                "chain"
            } else if trimmed.len() == 42
                && trimmed.starts_with("0x")
                && trimmed
                    .as_bytes()
                    .iter()
                    .skip(2)
                    .all(|byte| byte.is_ascii_hexdigit())
            {
                "address"
            } else {
                "text"
            }
        }
        _ => "unknown",
    }
}

fn value_at_dotted_path_object<'a>(
    map: &'a serde_json::Map<String, Value>,
    dotted: &str,
) -> Option<&'a Value> {
    let mut segments = dotted.split('.').filter(|part| !part.is_empty());
    let first = segments.next()?;
    let mut current = map.get(first)?;
    for segment in segments {
        current = current.get(segment)?;
    }
    Some(current)
}

pub(super) fn runtime_has_input_ref(state_summary: Option<&Value>, input_ref: &str) -> bool {
    let Some(canonical_slot) = super::input_normalize::normalize_input_slot_key(input_ref) else {
        return false;
    };
    let canonical_ref = format!("inputs.{canonical_slot}");
    if super::known_input_refs_from_state_summary(state_summary)
        .iter()
        .any(|known| known == &canonical_ref)
    {
        return true;
    }
    if state_summary
        .and_then(|summary| summary.pointer("/intent_context/facts"))
        .and_then(|facts| {
            facts
                .as_object()
                .and_then(|object| object.get(canonical_slot.as_str()))
                .or_else(|| value_at_dotted_path(facts, canonical_slot.as_str()))
        })
        .is_some()
    {
        return true;
    }
    matches!(canonical_slot.as_str(), "owner" | "wallet.default")
        && state_summary
            .and_then(|summary| summary.pointer("/intent_context/facts/owner"))
            .is_some()
}

#[derive(Debug, Clone, Default)]
pub(super) struct StaticRefillOutcome {
    pub(super) resolved_refs: Vec<String>,
    pub(super) ambiguous_bindings: Vec<AmbiguousBinding>,
}

#[derive(Debug, Clone)]
pub(super) struct AmbiguousBinding {
    pub(super) missing_ref: String,
    pub(super) candidate_refs: Vec<String>,
}

pub(super) fn apply_static_missing_ref_refill(
    state: &mut EngineRunnerState,
    context: &mut SegmentedAgentContext,
    missing_refs: &[String],
    phase_hint: &str,
    scope_id: &str,
) -> StaticRefillOutcome {
    let summary_snapshot = context.state_summary.clone();
    let mut resolved = BTreeSet::<String>::new();
    let mut ambiguous = Vec::<AmbiguousBinding>::new();
    for raw_ref in missing_refs {
        let Some(slot) = super::input_normalize::normalize_input_slot_key(raw_ref) else {
            continue;
        };
        let canonical_ref = format!("inputs.{slot}");
        if runtime_has_input_ref(summary_snapshot.as_ref(), canonical_ref.as_str()) {
            resolved.insert(canonical_ref);
            continue;
        }
        match resolve_static_input_binding(summary_snapshot.as_ref(), slot.as_str()) {
            StaticInputBindingDecision::Resolved(value) => {
                let provenance = format!("autofill.static.{phase_hint}.{scope_id}.{slot}");
                super::input_normalize::set_runtime_input_value(
                    &mut state.runtime,
                    slot.as_str(),
                    value.clone(),
                );
                let _ = super::upsert_store_value_with_source(
                    context.input_store_mut(),
                    slot.as_str(),
                    value,
                    super::input_store::InputValueLayer::Derived,
                    "autofill_static",
                    85,
                    provenance,
                );
                resolved.insert(canonical_ref);
            }
            StaticInputBindingDecision::Ambiguous(candidate_refs) => {
                ambiguous.push(AmbiguousBinding {
                    missing_ref: canonical_ref,
                    candidate_refs,
                });
            }
            StaticInputBindingDecision::Unresolved => {}
        }
    }
    if !resolved.is_empty() {
        context.refresh_state_summary(state, false);
    }
    StaticRefillOutcome {
        resolved_refs: resolved.into_iter().collect::<Vec<_>>(),
        ambiguous_bindings: ambiguous,
    }
}

fn resolve_static_input_binding(
    state_summary: Option<&Value>,
    slot: &str,
) -> StaticInputBindingDecision {
    let Some(summary) = state_summary else {
        return StaticInputBindingDecision::Unresolved;
    };
    let mut candidates = vec![slot.to_string()];
    for alias in static_alias_slots(slot) {
        if !candidates.contains(&alias) {
            candidates.push(alias);
        }
    }
    for candidate in candidates {
        if let Some(value) = summary
            .pointer("/input_store/facts")
            .and_then(|facts| {
                facts
                    .as_object()
                    .and_then(|object| object.get(candidate.as_str()))
                    .or_else(|| value_at_dotted_path(facts, candidate.as_str()))
            })
            .cloned()
            .map(|value| unwrap_input_value(&value))
        {
            return StaticInputBindingDecision::Resolved(value);
        }
        if let Some(value) = summary
            .pointer("/intent_context/facts")
            .and_then(|facts| {
                facts
                    .as_object()
                    .and_then(|object| object.get(candidate.as_str()))
                    .or_else(|| value_at_dotted_path(facts, candidate.as_str()))
            })
            .cloned()
            .map(|value| unwrap_input_value(&value))
        {
            return StaticInputBindingDecision::Resolved(value);
        }
        if let Some(value) = summary
            .pointer("/intent_slots/resolved_inputs")
            .and_then(Value::as_object)
            .and_then(|resolved_inputs| resolved_inputs.get(candidate.as_str()))
            .cloned()
            .map(|value| unwrap_input_value(&value))
        {
            return StaticInputBindingDecision::Resolved(value);
        }
    }
    resolve_static_input_value_by_semantic_match(summary, slot)
}

pub(super) fn resolve_static_input_value_for_slot(
    state_summary: Option<&Value>,
    slot: &str,
) -> Option<Value> {
    match resolve_static_input_binding(state_summary, slot) {
        StaticInputBindingDecision::Resolved(value) => Some(value),
        StaticInputBindingDecision::Ambiguous(_) | StaticInputBindingDecision::Unresolved => None,
    }
}

enum StaticInputBindingDecision {
    Resolved(Value),
    Ambiguous(Vec<String>),
    Unresolved,
}

fn resolve_static_input_value_by_semantic_match(
    summary: &Value,
    slot: &str,
) -> StaticInputBindingDecision {
    let Some(requirement) = TypedBindingRequirement::from_slot(slot) else {
        return StaticInputBindingDecision::Unresolved;
    };
    let mut scored = typed_binding_candidates(summary)
        .into_iter()
        .filter_map(|candidate| {
            score_typed_binding_candidate(&requirement, &candidate).map(|score| (score, candidate))
        })
        .collect::<Vec<_>>();
    if scored.is_empty() {
        return StaticInputBindingDecision::Unresolved;
    }
    scored.sort_by(|left, right| right.0.cmp(&left.0));
    let Some((best_score, best_candidate)) = scored.first() else {
        return StaticInputBindingDecision::Unresolved;
    };
    if *best_score < 50 {
        return StaticInputBindingDecision::Unresolved;
    }
    let second_score = scored.get(1).map(|item| item.0).unwrap_or_default();
    let confident = *best_score >= 180 || best_score.saturating_sub(second_score) >= 15;
    if confident {
        return StaticInputBindingDecision::Resolved(best_candidate.value.clone());
    }
    let candidate_refs = scored
        .iter()
        .take(3)
        .map(|(_, candidate)| canonicalize_binding_candidate_ref(candidate.key.as_str()))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if candidate_refs.len() < 2 {
        return StaticInputBindingDecision::Unresolved;
    }
    StaticInputBindingDecision::Ambiguous(candidate_refs)
}

fn canonicalize_binding_candidate_ref(raw_key: &str) -> String {
    if let Some(slot) = super::input_normalize::normalize_input_slot_key(raw_key) {
        return format!("inputs.{slot}");
    }
    raw_key.to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BindingValueType {
    Address,
    Boolean,
    Numeric,
    Chain,
    Text,
    Unknown,
}

#[derive(Debug, Clone)]
struct TypedBindingRequirement {
    normalized_slot: String,
    tokens: Vec<String>,
    expected_type: BindingValueType,
}

impl TypedBindingRequirement {
    fn from_slot(slot: &str) -> Option<Self> {
        let tokens = semantic_tokens(slot);
        if tokens.is_empty() {
            return None;
        }
        Some(Self {
            normalized_slot: normalize_semantic_key(slot),
            expected_type: infer_slot_type(slot, tokens.as_slice()),
            tokens,
        })
    }
}

#[derive(Debug, Clone)]
struct TypedBindingCandidate {
    key: String,
    normalized_key: String,
    tokens: Vec<String>,
    value: Value,
    value_type: BindingValueType,
    source_priority: u16,
}

fn typed_binding_candidates(summary: &Value) -> Vec<TypedBindingCandidate> {
    let mut candidates = Vec::<TypedBindingCandidate>::new();
    if let Some(facts) = summary
        .pointer("/input_store/facts")
        .and_then(Value::as_object)
    {
        let meta_map = summary
            .pointer("/input_store/meta")
            .and_then(Value::as_object);
        for (key, value) in facts {
            let source_priority = meta_map
                .and_then(|meta| meta.get(key.as_str()))
                .and_then(|entry| entry.get("source_priority"))
                .and_then(Value::as_u64)
                .unwrap_or(60)
                .min(u16::MAX as u64) as u16;
            push_typed_binding_candidate(
                &mut candidates,
                key.as_str(),
                unwrap_input_value(value),
                source_priority,
            );
        }
    }
    if let Some(facts) = summary
        .pointer("/intent_context/facts")
        .and_then(Value::as_object)
    {
        for (key, value) in facts {
            push_typed_binding_candidate(
                &mut candidates,
                key.as_str(),
                unwrap_input_value(value),
                55,
            );
        }
    }
    if let Some(facts) = summary
        .pointer("/intent_slots/resolved_inputs")
        .and_then(Value::as_object)
    {
        for (key, value) in facts {
            let source_priority = value
                .get("confidence")
                .and_then(Value::as_u64)
                .map(|confidence| 50 + confidence.min(50))
                .unwrap_or(50)
                .min(u16::MAX as u64) as u16;
            push_typed_binding_candidate(
                &mut candidates,
                key.as_str(),
                unwrap_input_value(value),
                source_priority,
            );
        }
    }
    candidates
}

fn push_typed_binding_candidate(
    candidates: &mut Vec<TypedBindingCandidate>,
    key: &str,
    value: Value,
    source_priority: u16,
) {
    let tokens = semantic_tokens(key);
    if tokens.is_empty() {
        return;
    }
    candidates.push(TypedBindingCandidate {
        key: key.to_string(),
        normalized_key: normalize_semantic_key(key),
        value_type: infer_value_type(&value),
        value,
        source_priority,
        tokens,
    });
}

fn score_typed_binding_candidate(
    requirement: &TypedBindingRequirement,
    candidate: &TypedBindingCandidate,
) -> Option<u16> {
    if !binding_type_compatible(requirement.expected_type, candidate.value_type) {
        return None;
    }
    if requirement.normalized_slot == candidate.normalized_key {
        return Some(220 + candidate.source_priority / 5);
    }

    let overlap = semantic_overlap(requirement.tokens.as_slice(), candidate.tokens.as_slice());
    if overlap.shared_total == 0 {
        return None;
    }

    let mut score = 0u16;
    score = score.saturating_add((overlap.shared_non_generic as u16).saturating_mul(35));
    score = score.saturating_add((overlap.shared_total as u16).saturating_mul(8));
    if requirement.expected_type == candidate.value_type {
        score = score.saturating_add(25);
    }
    score = score.saturating_add(candidate.source_priority.min(100) / 4);
    if overlap.slot_has_address && overlap.candidate_has_address {
        score = score.saturating_add(20);
    }
    if overlap.slot_has_decimals && overlap.candidate_has_decimals {
        score = score.saturating_add(20);
    }
    if candidate.key.starts_with(requirement.tokens[0].as_str()) {
        score = score.saturating_add(10);
    }
    Some(score)
}

#[derive(Default)]
struct SemanticOverlap {
    shared_total: usize,
    shared_non_generic: usize,
    slot_has_address: bool,
    candidate_has_address: bool,
    slot_has_decimals: bool,
    candidate_has_decimals: bool,
}

fn semantic_overlap(slot_tokens: &[String], candidate_tokens: &[String]) -> SemanticOverlap {
    let mut overlap = SemanticOverlap::default();
    overlap.slot_has_address = slot_tokens.iter().any(|token| token == "address");
    overlap.candidate_has_address = candidate_tokens.iter().any(|token| token == "address");
    overlap.slot_has_decimals = slot_tokens.iter().any(|token| token == "decimals");
    overlap.candidate_has_decimals = candidate_tokens.iter().any(|token| token == "decimals");

    let candidate_set = candidate_tokens
        .iter()
        .map(|token| token.as_str())
        .collect::<BTreeSet<_>>();
    for token in slot_tokens {
        if !candidate_set.contains(token.as_str()) {
            continue;
        }
        overlap.shared_total = overlap.shared_total.saturating_add(1);
        if !is_generic_semantic_token(token.as_str()) {
            overlap.shared_non_generic = overlap.shared_non_generic.saturating_add(1);
        }
    }
    overlap
}

fn binding_type_compatible(expected: BindingValueType, actual: BindingValueType) -> bool {
    expected == BindingValueType::Unknown
        || actual == BindingValueType::Unknown
        || expected == actual
        || (expected == BindingValueType::Numeric && actual == BindingValueType::Text)
        || (expected == BindingValueType::Boolean && actual == BindingValueType::Text)
}

fn infer_slot_type(slot: &str, tokens: &[String]) -> BindingValueType {
    if is_address_like_slot(slot)
        || has_any_token(
            tokens,
            &[
                "address",
                "owner",
                "recipient",
                "wallet",
                "account",
                "signer",
            ],
        )
    {
        return BindingValueType::Address;
    }
    if has_any_token(
        tokens,
        &[
            "amount",
            "threshold",
            "decimals",
            "bps",
            "nonce",
            "limit",
            "deadline",
            "gas",
            "fee",
            "price",
        ],
    ) {
        return BindingValueType::Numeric;
    }
    if has_any_token(tokens, &["chain", "chainid", "chainref"]) {
        return BindingValueType::Chain;
    }
    if has_any_token(
        tokens,
        &[
            "bool", "enabled", "enable", "disabled", "allow", "should", "is", "has", "use",
        ],
    ) {
        return BindingValueType::Boolean;
    }
    BindingValueType::Unknown
}

fn infer_value_type(value: &Value) -> BindingValueType {
    match value {
        Value::Bool(_) => BindingValueType::Boolean,
        Value::Number(_) => BindingValueType::Numeric,
        Value::String(text) => {
            let trimmed = text.trim();
            if is_evm_address_str(trimmed) {
                return BindingValueType::Address;
            }
            if trimmed.starts_with("eip155:") {
                return BindingValueType::Chain;
            }
            if trimmed.eq_ignore_ascii_case("true") || trimmed.eq_ignore_ascii_case("false") {
                return BindingValueType::Boolean;
            }
            if trimmed.parse::<f64>().is_ok() {
                return BindingValueType::Numeric;
            }
            BindingValueType::Text
        }
        _ => BindingValueType::Unknown,
    }
}

fn has_any_token(tokens: &[String], expected: &[&str]) -> bool {
    tokens
        .iter()
        .any(|token| expected.contains(&token.as_str()))
}

fn is_evm_address_str(value: &str) -> bool {
    value.len() == 42
        && value.starts_with("0x")
        && value
            .as_bytes()
            .iter()
            .skip(2)
            .all(|byte| byte.is_ascii_hexdigit())
}

fn is_generic_semantic_token(token: &str) -> bool {
    matches!(
        token,
        "inputs" | "input" | "value" | "field" | "data" | "ref" | "address" | "amount"
    )
}

fn semantic_tokens(key: &str) -> Vec<String> {
    key.to_ascii_lowercase()
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|segment| !segment.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>()
}

fn normalize_semantic_key(key: &str) -> String {
    semantic_tokens(key).join("")
}

fn is_address_like_slot(slot: &str) -> bool {
    slot.ends_with(".address") || slot.ends_with("_address")
}

fn unwrap_input_value(value: &Value) -> Value {
    value
        .as_object()
        .and_then(|object| object.get("value"))
        .cloned()
        .unwrap_or_else(|| value.clone())
}

fn static_alias_slots(slot: &str) -> Vec<String> {
    let mut aliases = Vec::<String>::new();
    match slot {
        "native_transfer_amount" => aliases.push("native_amount".to_string()),
        "token_transfer_amount" => aliases.push("token_amount".to_string()),
        "recipient" => aliases.push("recipient_address".to_string()),
        "recipient_address" => aliases.push("recipient".to_string()),
        "token.address" => {
            aliases.push("token_address".to_string());
            aliases.push("token".to_string());
        }
        "token_address" => aliases.push("token.address".to_string()),
        "chain_ref" => {
            aliases.push("chain".to_string());
            aliases.push("chain_id".to_string());
        }
        "chain_id" => {
            aliases.push("chain".to_string());
            aliases.push("chain_ref".to_string());
        }
        "chain" => {
            aliases.push("chain_id".to_string());
            aliases.push("chain_ref".to_string());
        }
        _ => {}
    }
    if slot.ends_with(".address") {
        aliases.push(slot.replace(".address", "_address"));
    }
    if slot.ends_with("_address") {
        aliases.push(slot.replace("_address", ".address"));
    }
    aliases
}

fn value_at_dotted_path<'a>(root: &'a Value, dotted: &str) -> Option<&'a Value> {
    let mut current = root;
    for segment in dotted.split('.').filter(|part| !part.is_empty()) {
        current = current.get(segment)?;
    }
    Some(current)
}

fn collect_missing_input_refs_from_message(message: &str, missing_refs: &mut BTreeSet<String>) {
    for (index, chunk) in message.split('`').enumerate() {
        if index % 2 == 1 {
            collect_missing_input_ref(chunk, None, missing_refs);
        }
    }
    if let Some(suffix) = message
        .split_once("suggested_ref=")
        .map(|(_, value)| value.trim())
    {
        let candidate = suffix
            .split(|ch: char| ch.is_whitespace() || matches!(ch, ',' | ';' | ')' | '('))
            .next()
            .unwrap_or_default();
        collect_missing_input_ref(candidate, None, missing_refs);
    }
}

fn collect_missing_input_ref(
    raw: &str,
    metadata: Option<&Value>,
    missing_refs: &mut BTreeSet<String>,
) {
    if let Some(path) = super::input_normalize::normalize_missing_input_ref(raw) {
        for leaf in super::input_normalize::expand_missing_input_slot(path.as_str(), metadata) {
            missing_refs.insert(format!("inputs.{leaf}"));
        }
    }
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
    input_store: &mut InputStore,
    checkpoint_extensions: &checkpoint_ext::AgentCheckpointExtensions,
    total_events: &mut usize,
    todo_id: &str,
) -> Result<super::phase_machine::segment_exec::ExecuteRoundOutcome, RunnerError> {
    super::phase_machine::segment_exec::execute_round(
        command,
        run_id,
        config,
        engine_options,
        decision_policy,
        command_builder,
        checkpoint_ledger,
        state,
        active_plan,
        active_plan_hash,
        segment,
        segment_plan,
        planning_memory,
        input_store,
        checkpoint_extensions,
        total_events,
        todo_id,
    )
}

fn bind_segment_todo_id(segment: &mut PlanSketchSegment, todo_id: &str) {
    super::phase_machine::segment_exec::bind_segment_todo_id(segment, todo_id);
}

fn collect_segment_missing_input_refs(
    segment: &PlanSketchSegment,
    input_store: &InputStore,
) -> Vec<String> {
    super::phase_machine::segment_exec::collect_segment_missing_input_refs(segment, input_store)
}

fn build_todo_receipt(
    planned_segment: &PlannedSegment,
    status: EngineRunStatus,
    state: &mut EngineRunnerState,
    checkpoint_ledger: &RunnerCheckpointLedger,
    round_events: &[EngineEventRecord],
) -> super::todos::TodoReceipt {
    let mut receipt = super::phase_machine::segment_exec::build_todo_receipt(
        planned_segment.todo_id.as_str(),
        &planned_segment.segment,
        status,
        state,
        round_events,
    );
    receipt.tx_hashes = checkpoint_ledger.tx_hashes_for_nodes(receipt.node_ids.as_slice());
    project_todo_node_output_tx_hashes_from_ledger(
        state,
        receipt.node_ids.as_slice(),
        checkpoint_ledger,
    );
    receipt
}

fn sync_todo_progress_receipt_tx_hashes_from_ledger(
    state: &mut EngineRunnerState,
    checkpoint_ledger: &RunnerCheckpointLedger,
) {
    let receipt_nodes = collect_todo_progress_receipt_node_ids(&state.runtime);
    if receipt_nodes.is_empty() {
        return;
    }
    for (_, node_ids) in &receipt_nodes {
        project_todo_node_output_tx_hashes_from_ledger(
            state,
            node_ids.as_slice(),
            checkpoint_ledger,
        );
    }
    for (todo_id, node_ids) in receipt_nodes {
        let tx_hashes = checkpoint_ledger.tx_hashes_for_nodes(node_ids.as_slice());
        overwrite_todo_progress_receipt_tx_hashes(&mut state.runtime, todo_id.as_str(), tx_hashes);
    }
}

fn collect_todo_progress_receipt_node_ids(runtime: &Value) -> Vec<(String, Vec<String>)> {
    let mut out = BTreeSet::<(String, Vec<String>)>::new();
    if let Some(current) = runtime.pointer("/agent/todo_progress/current_todo") {
        if let Some(entry) = todo_receipt_node_ids(current) {
            out.insert(entry);
        }
    }
    if let Some(todos) = runtime
        .pointer("/agent/todo_progress/todos")
        .and_then(Value::as_array)
    {
        for todo in todos {
            if let Some(entry) = todo_receipt_node_ids(todo) {
                out.insert(entry);
            }
        }
    }
    out.into_iter().collect::<Vec<_>>()
}

fn todo_receipt_node_ids(todo: &Value) -> Option<(String, Vec<String>)> {
    let todo_id = todo.get("id").and_then(Value::as_str).map(str::trim)?;
    if todo_id.is_empty() {
        return None;
    }
    let node_ids = todo
        .pointer("/receipt/node_ids")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if node_ids.is_empty() {
        return None;
    }
    Some((todo_id.to_string(), node_ids))
}

fn overwrite_todo_progress_receipt_tx_hashes(
    runtime: &mut Value,
    todo_id: &str,
    tx_hashes: Vec<String>,
) {
    let tx_hashes_value = tx_hashes.into_iter().map(Value::String).collect::<Vec<_>>();
    if let Some(current_todo) = runtime.pointer_mut("/agent/todo_progress/current_todo") {
        overwrite_todo_receipt_tx_hashes(current_todo, todo_id, tx_hashes_value.as_slice());
    }
    if let Some(todos) = runtime
        .pointer_mut("/agent/todo_progress/todos")
        .and_then(Value::as_array_mut)
    {
        for todo in todos {
            overwrite_todo_receipt_tx_hashes(todo, todo_id, tx_hashes_value.as_slice());
        }
    }
}

fn overwrite_todo_receipt_tx_hashes(todo: &mut Value, todo_id: &str, tx_hashes: &[Value]) {
    let Some(todo_obj) = todo.as_object_mut() else {
        return;
    };
    let Some(id) = todo_obj.get("id").and_then(Value::as_str) else {
        return;
    };
    if id != todo_id {
        return;
    }
    let Some(receipt_obj) = todo_obj.get_mut("receipt").and_then(Value::as_object_mut) else {
        return;
    };
    receipt_obj.insert("tx_hashes".to_string(), Value::Array(tx_hashes.to_vec()));
}

fn project_todo_node_output_tx_hashes_from_ledger(
    state: &mut EngineRunnerState,
    node_ids: &[String],
    checkpoint_ledger: &RunnerCheckpointLedger,
) {
    for node_id in node_ids {
        let Some(tx_hash) = checkpoint_ledger.preferred_tx_hash_for_node(node_id.as_str()) else {
            continue;
        };
        set_runtime_node_output_tx_hash(state, node_id.as_str(), tx_hash);
    }
}

fn set_runtime_node_output_tx_hash(state: &mut EngineRunnerState, node_id: &str, tx_hash: String) {
    let Some(runtime_obj) = state.runtime.as_object_mut() else {
        return;
    };
    let nodes = runtime_obj
        .entry("nodes".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if !nodes.is_object() {
        *nodes = Value::Object(Map::new());
    }
    let Some(nodes_obj) = nodes.as_object_mut() else {
        return;
    };
    let node = nodes_obj
        .entry(node_id.to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if !node.is_object() {
        *node = Value::Object(Map::new());
    }
    let Some(node_obj) = node.as_object_mut() else {
        return;
    };
    let outputs = node_obj
        .entry("outputs".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if !outputs.is_object() {
        *outputs = Value::Object(Map::new());
    }
    if let Some(outputs_obj) = outputs.as_object_mut() {
        outputs_obj.insert("tx_hash".to_string(), Value::String(tx_hash));
    }
}

fn run_status_name(status: EngineRunStatus) -> &'static str {
    super::phase_machine::segment_exec::run_status_name(status)
}

#[allow(clippy::too_many_arguments)]
fn record_planning_failure_preserving_primary_error(
    command: &AgentCommand,
    run_id: &str,
    active_plan_hash: &str,
    active_plan: &PlanDocument,
    state: &mut EngineRunnerState,
    checkpoint_ledger: &RunnerCheckpointLedger,
    planning_memory: Option<Value>,
    input_store: &InputStore,
    checkpoint_extensions: &checkpoint_ext::AgentCheckpointExtensions,
    round: u64,
    planning_error: RunnerError,
) -> RunnerError {
    if let Err(checkpoint_error) =
        super::checkpoint_flow::record_planning_failure_event_and_checkpoint(
            command,
            run_id,
            active_plan_hash,
            active_plan,
            state,
            checkpoint_ledger,
            planning_memory,
            input_store,
            checkpoint_extensions,
            &planning_error,
            round,
        )
    {
        if command.verbose || command.verbose_llm {
            eprintln!(
                "[agent] planning_failure_checkpoint_record_failed primary_error={} checkpoint_error={}",
                planning_error, checkpoint_error
            );
        }
    }
    planning_error
}

#[cfg(test)]
#[path = "tests/orchestrator_module.rs"]
mod tests;
