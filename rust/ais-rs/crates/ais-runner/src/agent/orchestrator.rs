use super::context::budget_policy::{ContextPressureMode, ToolMemoryBudgetPolicy};
use super::context_view::PlanningContextManager;
use super::phase_machine::types::AgentPhase;
use super::runtime_facts_store::RuntimeFactsStore;
use super::state_summary::StateSummary;
use super::*;
use crate::policy::{volatile_facts_policy_from_pack, VolatileFactsPolicy};
use ais_engine::{EngineRunStatus, EngineRunnerOptions, EngineRunnerState};
use ais_sdk::documents::PlanSketchSegment;
use serde_json::{json, Value};
use std::collections::BTreeSet;

const GROUNDING_NON_ACTIONABLE_REPAIR_RETRY_LIMIT: u8 = 1;
const GROUNDING_NON_ACTIONABLE_REASON_CODE: &str = "grounding_non_actionable_pause";

const CONSECUTIVE_SAME_ERROR_TERMINAL_LIMIT: u32 = 3;
const INFRA_ERROR_RAW_RETRY_LIMIT: u32 = 2;

#[derive(Debug, Clone, Default)]
struct ExecutionRetryTracker {
    last_error_signature: Option<String>,
    consecutive_same_error_count: u32,
}

impl ExecutionRetryTracker {
    fn observe_error(&mut self, paused_reason: Option<&str>) -> ExecutionRetryAction {
        let signature = super::extract_executor_error_signature(paused_reason);
        let Some(ref sig) = signature else {
            self.last_error_signature = None;
            self.consecutive_same_error_count = 0;
            return ExecutionRetryAction::RepairViaLlm;
        };

        if self.last_error_signature.as_ref() == Some(sig) {
            self.consecutive_same_error_count = self.consecutive_same_error_count.saturating_add(1);
        } else {
            self.last_error_signature = Some(sig.clone());
            self.consecutive_same_error_count = 1;
        }

        let severity = super::classify_executor_error_severity(paused_reason.unwrap_or_default());

        match severity {
            super::error_state::ExecutorErrorSeverity::InfrastructureUnavailable => {
                if self.consecutive_same_error_count >= INFRA_ERROR_RAW_RETRY_LIMIT {
                    return ExecutionRetryAction::Terminal;
                }
                ExecutionRetryAction::RawRetry
            }
            _ => {
                if self.consecutive_same_error_count >= CONSECUTIVE_SAME_ERROR_TERMINAL_LIMIT {
                    return ExecutionRetryAction::Terminal;
                }
                if self.consecutive_same_error_count == 2 {
                    return ExecutionRetryAction::RawRetry;
                }
                ExecutionRetryAction::RepairViaLlm
            }
        }
    }

    fn reset(&mut self) {
        self.last_error_signature = None;
        self.consecutive_same_error_count = 0;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExecutionRetryAction {
    RepairViaLlm,
    RawRetry,
    Terminal,
}

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
struct AgentRunConfig {
    planner_round_limit: usize,
    segment_limit: usize,
}

#[derive(Debug)]
struct AgentRunState {
    completed_segments: usize,
    final_status: EngineRunStatus,
    planning_rounds: usize,
    planner_output_retries: usize,
    execution_retry_tracker: ExecutionRetryTracker,
}

#[derive(Debug)]
struct AgentStoreContext {
    input_store: InputStore,
    runtime_facts_store: RuntimeFactsStore,
    todo_board: TodoBoard,
    context_manager: PlanningContextManager,
    tool_memory_projection: Option<Value>,
    packed_summary: Option<Value>,
    typed_summary: Option<StateSummary>,
}

#[derive(Debug)]
pub(super) struct SegmentedAgentContext {
    pub(super) intent: String,
    pub(super) session: intent_segmented::SegmentPlanningSession,
    pub(super) previous_error: Option<Value>,
    pub(super) last_segment: Option<PlanSketchSegment>,
    run_config: AgentRunConfig,
    run_state: AgentRunState,
    store_context: AgentStoreContext,
    checkpoint_extensions: checkpoint_ext::AgentCheckpointExtensions,
    compile_autofill_attempted_todos: BTreeSet<String>,
}

impl SegmentedAgentContext {
    fn new(
        intent: String,
        session: intent_segmented::SegmentPlanningSession,
        input_store: InputStore,
        runtime_facts_store: RuntimeFactsStore,
        todo_board: TodoBoard,
        planner_round_limit: usize,
        segment_limit: usize,
        planner_context_token_budget: usize,
        checkpoint_extensions: checkpoint_ext::AgentCheckpointExtensions,
    ) -> Self {
        Self {
            intent,
            session,
            previous_error: None,
            last_segment: None,
            run_config: AgentRunConfig {
                planner_round_limit,
                segment_limit,
            },
            run_state: AgentRunState {
                completed_segments: 0,
                final_status: EngineRunStatus::Completed,
                planning_rounds: 0,
                planner_output_retries: 0,
                execution_retry_tracker: ExecutionRetryTracker::default(),
            },
            store_context: AgentStoreContext {
                input_store,
                runtime_facts_store,
                todo_board,
                context_manager: PlanningContextManager::with_token_budget(
                    planner_context_token_budget,
                ),
                tool_memory_projection: None,
                packed_summary: None,
                typed_summary: None,
            },
            checkpoint_extensions,
            compile_autofill_attempted_todos: BTreeSet::new(),
        }
    }

    fn can_continue(&self) -> bool {
        self.run_state.completed_segments < self.run_config.segment_limit
    }

    pub(super) fn refresh_state_summary(&mut self, state: &EngineRunnerState, done: bool) {
        let next = self
            .store_context
            .context_manager
            .next_summary_result_with_runtime_facts(
                state,
                self.run_state.completed_segments,
                done,
                self.previous_error.as_ref(),
                Some(&self.store_context.input_store),
                Some(&self.store_context.runtime_facts_store),
                self.store_context.tool_memory_projection.as_ref(),
            );
        self.store_context.packed_summary = Some(next.packed);
        self.store_context.typed_summary = Some(next.typed);
    }

    fn update_tool_memory_projection(&mut self, projection: Option<Value>) {
        self.store_context.tool_memory_projection = projection;
    }

    pub(super) fn set_previous_error_and_refresh(
        &mut self,
        state: &EngineRunnerState,
        done: bool,
        mut error: Value,
    ) {
        Self::merge_autofill_history(self.previous_error.as_ref(), &mut error);
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

    pub(super) fn packed_summary(&self) -> &Option<Value> {
        &self.store_context.packed_summary
    }

    #[cfg(test)]
    pub(super) fn packed_summary_mut(&mut self) -> &mut Option<Value> {
        &mut self.store_context.packed_summary
    }

    pub(super) fn typed_summary(&self) -> Option<&StateSummary> {
        self.store_context.typed_summary.as_ref()
    }

    #[cfg(test)]
    pub(super) fn typed_summary_mut(&mut self) -> &mut Option<StateSummary> {
        &mut self.store_context.typed_summary
    }

    pub(super) fn completed_segments_u8(&self) -> u8 {
        self.run_state.completed_segments as u8
    }

    pub(super) fn completed_segments(&self) -> usize {
        self.run_state.completed_segments
    }

    pub(super) fn increment_completed_segments(&mut self) {
        self.run_state.completed_segments = self.run_state.completed_segments.saturating_add(1);
    }

    pub(super) fn has_compile_autofill_attempt(&self, key: &str) -> bool {
        self.compile_autofill_attempted_todos.contains(key)
    }

    pub(super) fn mark_compile_autofill_attempt(&mut self, key: impl Into<String>) {
        self.compile_autofill_attempted_todos.insert(key.into());
    }

    fn merge_autofill_history(previous_error: Option<&Value>, error: &mut Value) {
        fn collect_attempt_keys(source: &Value, out: &mut BTreeSet<String>) {
            if let Some(existing) = source
                .pointer("/autofill_history/attempt_keys")
                .and_then(Value::as_array)
            {
                for key in existing.iter().filter_map(Value::as_str) {
                    let key = key.trim();
                    if !key.is_empty() {
                        out.insert(key.to_string());
                    }
                }
            }
            if let Some(mode) = source
                .pointer("/autofill/mode")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                out.insert(format!("mode:{mode}"));
            }
            if let Some(refs) = source
                .pointer("/autofill/selected_query_refs")
                .and_then(Value::as_array)
            {
                for reference in refs.iter().filter_map(Value::as_str) {
                    let reference = reference.trim();
                    if !reference.is_empty() {
                        out.insert(format!("query_ref:{reference}"));
                    }
                }
            }
            if let Some(pool) = source
                .pointer("/autofill/query_candidate_pool")
                .and_then(Value::as_array)
            {
                for candidate in pool {
                    if let Some(reference) = candidate
                        .get("query_ref")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                    {
                        out.insert(format!("query_ref:{reference}"));
                    }
                }
            }
        }

        let mut attempt_keys = BTreeSet::<String>::new();
        if let Some(previous) = previous_error {
            collect_attempt_keys(previous, &mut attempt_keys);
        }
        collect_attempt_keys(error, &mut attempt_keys);

        if attempt_keys.is_empty() {
            return;
        }
        if let Some(object) = error.as_object_mut() {
            object.insert(
                "autofill_history".to_string(),
                json!({
                    "attempt_keys": attempt_keys.into_iter().collect::<Vec<_>>(),
                }),
            );
        }
    }
    pub(super) fn input_store_mut(&mut self) -> &mut InputStore {
        &mut self.store_context.input_store
    }

    pub(super) fn input_store(&self) -> &InputStore {
        &self.store_context.input_store
    }

    pub(super) fn runtime_facts_store(&self) -> &RuntimeFactsStore {
        &self.store_context.runtime_facts_store
    }

    pub(super) fn todo_board(&self) -> &TodoBoard {
        &self.store_context.todo_board
    }

    pub(super) fn todo_board_mut(&mut self) -> &mut TodoBoard {
        &mut self.store_context.todo_board
    }

    #[cfg(test)]
    pub(super) fn tool_memory_projection(&self) -> &Option<Value> {
        &self.store_context.tool_memory_projection
    }

    pub(super) fn planner_round_limit(&self) -> usize {
        self.run_config.planner_round_limit
    }

    pub(super) fn planning_rounds(&self) -> usize {
        self.run_state.planning_rounds
    }

    pub(super) fn increment_planning_rounds(&mut self) {
        self.run_state.planning_rounds = self.run_state.planning_rounds.saturating_add(1);
    }

    pub(super) fn planner_output_retries(&self) -> usize {
        self.run_state.planner_output_retries
    }

    pub(super) fn reset_planner_output_retries(&mut self) {
        self.run_state.planner_output_retries = 0;
    }

    pub(super) fn increment_planner_output_retries(&mut self) {
        self.run_state.planner_output_retries =
            self.run_state.planner_output_retries.saturating_add(1);
    }

    pub(super) fn final_status(&self) -> EngineRunStatus {
        self.run_state.final_status.clone()
    }

    pub(super) fn set_final_status(&mut self, status: EngineRunStatus) {
        self.run_state.final_status = status;
    }

    fn execution_retry_tracker_mut(&mut self) -> &mut ExecutionRetryTracker {
        &mut self.run_state.execution_retry_tracker
    }
}

pub(super) fn execute_segmented_intent_agent(
    command: &AgentCommand,
    config: &RunnerConfig,
    pack: Option<&ais_sdk::PackDocument>,
    candidate_context: Option<CandidateContext>,
    prompt_catalog: &PromptCatalog,
) -> Result<String, RunnerError> {
    let result = super::phase_machine::run_main_flow(
        command.verbose || command.verbose_llm,
        |phase_tracker| {
            execute_segmented_intent_agent_main(
                command,
                config,
                pack,
                candidate_context,
                prompt_catalog,
                phase_tracker,
            )
        },
    );
    super::trace::ensure_sink_healthy()?;
    result
}

fn execute_segmented_intent_agent_main(
    command: &AgentCommand,
    config: &RunnerConfig,
    pack: Option<&ais_sdk::PackDocument>,
    candidate_context: Option<CandidateContext>,
    prompt_catalog: &PromptCatalog,
    phase_tracker: &mut super::phase_machine::MainFlowPhaseTracker<'_>,
) -> Result<String, RunnerError> {
    // ── Phase 1: Initialization ─────────────────────────────────────────
    // Provider setup, checkpoint restore, input store init, session bootstrap,
    // decision policy, engine options, command builder, context creation.

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
    let volatile_facts_policy = volatile_facts_policy_from_pack(pack)
        .map_err(|error| RunnerError::WorkspaceValidate(error.to_string()))?;
    let segmented_prompt_overrides = super::load_segmented_prompt_overrides(prompt_catalog);
    let llm_context_limit_tokens = super::resolve_llm_context_limit_tokens(config);
    let segmented_max_tool_rounds = super::resolve_segmented_max_tool_rounds(command, config);
    let mut planner = LlmSegmentedIntentPlanner::new(provider)
        .with_candidate_context(Some(candidate_context.clone()))
        .with_prompt_overrides(segmented_prompt_overrides)
        .with_max_tool_rounds(segmented_max_tool_rounds)
        .with_volatile_facts_policy(volatile_facts_policy)
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
        pending_resume_traces,
    ) = super::load_or_init_state(command, &active_plan_hash, runtime)?;
    let mut audit_attempt = if resumed_from_checkpoint {
        crate::audit_contract::next_attempt_from_extensions(checkpoint_extensions.as_ref())
    } else {
        crate::audit_contract::AuditStreamAttempt::fresh()
    };
    let _agent_trace_guard = super::trace::install_jsonl_sink(
        command.agent_trace_jsonl.as_deref(),
        run_id.as_str(),
        &audit_attempt,
    )?;
    super::trace::flush_pending(
        command.verbose || command.verbose_llm,
        pending_resume_traces.as_slice(),
    );
    super::trace::ensure_sink_healthy()?;
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
    let mut runtime_facts_store = RuntimeFactsStore::default();
    if let Some(restored) = checkpoint_extensions.input_store() {
        input_store.merge(restored);
    }
    if let Some(restored) = checkpoint_extensions.runtime_facts_store() {
        runtime_facts_store.merge(restored);
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
    super::receipt_view::project_todo_progress_receipts_from_ledger(
        &mut state.runtime,
        &checkpoint_ledger,
    );
    let runtime_has_intent_grounding = state.runtime.pointer("/agent/intent_grounding").is_some();
    let runtime_has_todo_progress = state.runtime.pointer("/agent/todo_progress").is_some();
    let mut todo_board = TodoBoard::restore_or_bootstrap(&state.runtime, intent.as_str());
    todo_board.ensure_current();
    super::runtime_store::record_todo_progress(&mut state.runtime, &todo_board);

    let initial_router = build_router_executor_for_plan(&active_plan, config)
        .map_err(RunnerError::ConfigInvalidForPlan)?;
    let readonly_autofill_router = crate::config::build_router_executor(config).ok();
    let ckpt = super::checkpoint_flow::CheckpointGuard {
        command,
        run_id: run_id.as_str(),
        active_plan_hash: &active_plan_hash,
        active_plan: &active_plan,
    };
    if resumed_from_checkpoint {
        if let Some(paused_reason) = super::reconcile_pending_side_effects(
            &mut checkpoint_ledger,
            &initial_router,
            &mut state,
        ) {
            super::record_side_effect_lifecycle(&mut state.runtime, &checkpoint_ledger);
            state.paused_reason = Some(paused_reason);
            ckpt.save(
                &state,
                &checkpoint_ledger,
                planner.planning_memory_checkpoint_value(),
                &input_store,
                &runtime_facts_store,
                &checkpoint_extensions,
                &audit_attempt,
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
        runtime_facts_store,
        todo_board,
        usize::from(planner_round_limit),
        segment_limit,
        planner_context_token_budget,
        checkpoint_extensions,
    );
    refresh_tool_memory_projection(&mut context, &mut planner, &state);
    context.refresh_state_summary(&state, false);
    // ── Phase 2: Grounding ─────────────────────────────────────────────
    // Attempt to ground the intent via LLM, with non-actionable pause
    // retry/fallback logic. Returns true when grounded, false when paused.

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
    let grounding_ready = match run_grounding_loop(
        command,
        &mut planner,
        &mut state,
        &mut context,
        &candidate_context,
        readonly_autofill_router.as_ref(),
        runtime_has_intent_grounding,
        trace_enabled,
    ) {
        Ok(ready) => ready,
        Err(error) => {
            if command.verbose {
                eprintln!(
                    "[agent] grounding_failed entered_execute_round=false reason={}",
                    error
                );
            }
            return Err(record_planning_failure_preserving_primary_error(
                &ckpt,
                &mut state,
                &mut checkpoint_ledger,
                planner.planning_memory_checkpoint_value(),
                &context.input_store(),
                &context.runtime_facts_store(),
                &context.checkpoint_extensions,
                context.planning_rounds() as u64,
                error,
                &mut audit_attempt,
            ));
        }
    };
    super::trace::emit(
        trace_enabled,
        "grounding",
        "complete",
        &[("ready_for_todos", grounding_ready.to_string())],
    );
    if matches!(context.final_status(), EngineRunStatus::Stopped) {
        ckpt.save(
            &state,
            &checkpoint_ledger,
            planner.planning_memory_checkpoint_value(),
            &context.input_store(),
            &context.runtime_facts_store(),
            &context.checkpoint_extensions,
            &audit_attempt,
        )?;
        record_planner_llm_usage(&mut state, &planner);
        return super::render_agent_output(
            command,
            &mut state,
            EngineRunStatus::Stopped,
            0,
            0,
            resumed_from_checkpoint,
        );
    }
    if !grounding_ready {
        phase_tracker.transition_to(AgentPhase::ResolvePause, "pause_after_grounding");
        super::trace::emit(
            trace_enabled,
            "pause_resolution",
            "paused_missing_required_input",
            &[("phase_hint", "grounding".to_string())],
        );
        ckpt.save(
            &state,
            &checkpoint_ledger,
            planner.planning_memory_checkpoint_value(),
            &context.input_store(),
            &context.runtime_facts_store(),
            &context.checkpoint_extensions,
            &audit_attempt,
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
    // ── Phase 3: Todo Bootstrap ─────────────────────────────────────────
    // Bootstrap the todo board via LLM if not already present.
    // Pauses when missing required input is detected.

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
            &ckpt,
            &mut state,
            &mut checkpoint_ledger,
            planner.planning_memory_checkpoint_value(),
            &context.input_store(),
            &context.runtime_facts_store(),
            &context.checkpoint_extensions,
            context.planning_rounds() as u64,
            error,
            &mut audit_attempt,
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
    if matches!(context.final_status(), EngineRunStatus::Stopped) {
        ckpt.save(
            &state,
            &checkpoint_ledger,
            planner.planning_memory_checkpoint_value(),
            &context.input_store(),
            &context.runtime_facts_store(),
            &context.checkpoint_extensions,
            &audit_attempt,
        )?;
        record_planner_llm_usage(&mut state, &planner);
        return super::render_agent_output(
            command,
            &mut state,
            EngineRunStatus::Stopped,
            0,
            0,
            resumed_from_checkpoint,
        );
    }
    if state.paused_reason.as_deref() == Some("missing_required_input") {
        phase_tracker.transition_to(AgentPhase::ResolvePause, "pause_after_todo");
        super::trace::emit(
            trace_enabled,
            "pause_resolution",
            "paused_missing_required_input",
            &[("phase_hint", "todo".to_string())],
        );
        ckpt.save(
            &state,
            &checkpoint_ledger,
            planner.planning_memory_checkpoint_value(),
            &context.input_store(),
            &context.runtime_facts_store(),
            &context.checkpoint_extensions,
            &audit_attempt,
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
    drop(ckpt);

    // ── Phase 4: Segment Loop (plan → compile → execute → pause) ────
    // Iterates over the todo board: plans a segment via LLM, compiles it
    // into an executable plan, runs the engine, then handles pause/retry
    // logic. Kept inline due to heavy mutable-state interdependencies
    // (state, context, planner, checkpoint_ledger, active_plan, etc.).

    while context.can_continue() {
        let ckpt = super::checkpoint_flow::CheckpointGuard {
            command,
            run_id: run_id.as_str(),
            active_plan_hash: &active_plan_hash,
            active_plan: &active_plan,
        };
        phase_tracker.transition_to(AgentPhase::PlanSegment, "plan_round");
        context.todo_board_mut().ensure_current();
        super::runtime_store::record_todo_progress(&mut state.runtime, &context.todo_board());
        super::trace::emit(
            trace_enabled,
            "plan_round",
            "start",
            &[(
                "todo_id",
                context
                    .todo_board()
                    .current_todo_id()
                    .unwrap_or("-")
                    .to_string(),
            )],
        );
        let current_todo_id = context
            .todo_board()
            .current_todo_id()
            .ok_or_else(|| RunnerError::Llm("todo board has no current todo".to_string()))?
            .to_string();
        if context.previous_error.is_none() {
            let precheck_refs =
                super::missing_resolution::precheck_missing_input_refs_for_current_todo(
                    &context,
                    context.typed_summary(),
                );
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
                let precheck_payload = super::missing_resolution::precheck_missing_input_payload(
                    precheck_refs.as_slice(),
                    context.completed_segments() as u8,
                );
                match super::phase_machine::pause::recover_missing_required_input_payload(
                    command,
                    &mut state,
                    &mut context,
                    &candidate_context,
                    readonly_autofill_router.as_ref(),
                    &precheck_payload,
                    current_todo_id.as_str(),
                    false,
                    "plan_precheck",
                    false,
                    true,
                )? {
                    super::phase_machine::pause::MissingRequiredInputRecoveryBackflow::Retry {
                        answers,
                        ..
                    } => {
                        super::trace::emit(
                            trace_enabled,
                            "plan_precheck",
                            if answers.is_some() {
                                "resolved_by_user_input"
                            } else {
                                "recovery_retry_scheduled"
                            },
                            &[("todo_id", current_todo_id.clone())],
                        );
                        continue;
                    }
                    super::phase_machine::pause::MissingRequiredInputRecoveryBackflow::Paused => {
                        super::trace::emit(
                            trace_enabled,
                            "plan_precheck",
                            "paused_missing_required_input",
                            &[("todo_id", current_todo_id.clone())],
                        );
                        break;
                    }
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
                    &ckpt,
                    &mut state,
                    &mut checkpoint_ledger,
                    planner.planning_memory_checkpoint_value(),
                    &context.input_store(),
                    &context.runtime_facts_store(),
                    &context.checkpoint_extensions,
                    context.planning_rounds() as u64,
                    error,
                    &mut audit_attempt,
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
                error_details,
            } => {
                if reason_code == "intent_aborted" {
                    super::trace::emit(
                        trace_enabled,
                        "plan_round",
                        "abort_intent",
                        &[("todo_id", current_todo_id.clone())],
                    );
                    super::runtime_store::record_runtime_agent_field(
                        &mut state.runtime,
                        "abort_intent",
                        json!({
                            "accepted": true,
                            "phase": "plan_round",
                            "reason_code": reason_code,
                            "summary": message,
                            "evidence": error_details.as_ref().and_then(|value| value.get("evidence")).cloned().unwrap_or_else(|| json!({})),
                            "user_fix_hint": error_details.as_ref().and_then(|value| value.get("user_fix_hint")).cloned().unwrap_or(Value::Null),
                        }),
                    );
                    state.paused_reason = None;
                    context.set_final_status(EngineRunStatus::Stopped);
                    context.clear_previous_error_and_refresh(&state, true);
                    break;
                }
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
                    let payload = super::missing_input::payload_with_error_details(
                        message.as_deref(),
                        questions.as_slice(),
                        issues.as_slice(),
                        error_details.as_ref(),
                        context.completed_segments() as u8,
                    );
                    let trace_extra = [("todo_id", current_todo_id.clone())];
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
                        super::phase_machine::pause::MissingRequiredInputRecoveryBackflow::Retry { answers, .. } => {
                            if let Some(answers) = answers {
                                handle_resolved_by_user_input(command, &mut state, &mut context, &ckpt, &checkpoint_ledger, planner.planning_memory_checkpoint_value(), done, trace_enabled, "plan_round", &trace_extra, answers, &audit_attempt)?;
                            }
                            continue;
                        }
                        super::phase_machine::pause::MissingRequiredInputRecoveryBackflow::Paused => {
                            handle_paused_missing_input(&mut state, &mut context, Some(&ckpt), &checkpoint_ledger, planner.planning_memory_checkpoint_value(), trace_enabled, &trace_extra, Some(&audit_attempt))?;
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

        context.todo_board_mut().mark_current_in_progress(
            planned_segment.summary.as_deref(),
            planned_segment.segment.segment_id.as_str(),
        );
        super::runtime_store::record_todo_progress(&mut state.runtime, &context.todo_board());
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
            volatile_facts_policy,
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
                    context.completed_segments() as u8,
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
                            .todo_board_mut()
                            .mark_current_blocked("missing_required_input");
                        super::runtime_store::record_todo_progress(
                            &mut state.runtime,
                            &context.todo_board(),
                        );
                        ckpt.save(
                            &state,
                            &checkpoint_ledger,
                            planner.planning_memory_checkpoint_value(),
                            &context.input_store(),
                            &context.runtime_facts_store(),
                            &context.checkpoint_extensions,
                            &audit_attempt,
                        )?;
                        context.set_final_status(EngineRunStatus::Paused);
                        break;
                    }
                    let trace_extra = [("todo_id", current_todo_id.clone())];
                    match super::phase_machine::pause::resolve_missing_required_input_payload(
                        &mut state,
                        context.input_store_mut(),
                        &prompt_payload,
                        false,
                    )? {
                        super::phase_machine::pause::MissingRequiredInputBackflow::ResolvedByUserInput { answers } => {
                            handle_resolved_by_user_input(command, &mut state, &mut context, &ckpt, &checkpoint_ledger, planner.planning_memory_checkpoint_value(), planned_segment.done, trace_enabled, "compile", &trace_extra, answers, &audit_attempt)?;
                            continue;
                        }
                        super::phase_machine::pause::MissingRequiredInputBackflow::Paused => {
                            handle_paused_missing_input(&mut state, &mut context, Some(&ckpt), &checkpoint_ledger, planner.planning_memory_checkpoint_value(), trace_enabled, &trace_extra, Some(&audit_attempt))?;
                            break;
                        }
                    }
                }
                context.set_previous_error_and_refresh(
                    &state,
                    planned_segment.done,
                    super::compile_error_state_payload(
                        &error_payload,
                        context.completed_segments() as u8,
                    ),
                );
                continue;
            }
        };
        let execution_precheck_refs =
            collect_segment_missing_refs(&planned_segment.segment, context.typed_summary());
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
            let payload = super::missing_resolution::precheck_missing_input_payload(
                execution_precheck_refs.as_slice(),
                context.completed_segments() as u8,
            );
            let recovery_outcome =
                super::missing_resolution::missing_resolution_recover_missing_refs(
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
        drop(ckpt);
        let checkpoint_extensions = context.checkpoint_extensions.clone();
        let AgentStoreContext {
            runtime_facts_store,
            input_store,
            ..
        } = &mut context.store_context;
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
            &candidate_context,
            planner.planning_memory_checkpoint_value(),
            runtime_facts_store,
            input_store,
            &checkpoint_extensions,
            &mut audit_attempt,
            &mut total_events,
            planned_segment.todo_id.as_str(),
        )?;
        let ckpt = super::checkpoint_flow::CheckpointGuard {
            command,
            run_id: run_id.as_str(),
            active_plan_hash: &active_plan_hash,
            active_plan: &active_plan,
        };
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
        let todo_receipt = super::receipt_view::build_segment_todo_receipt(
            planned_segment.todo_id.as_str(),
            &planned_segment.segment,
            execute_status.clone(),
            &state,
            execute_outcome.round_events.as_slice(),
            Some(&checkpoint_ledger),
        );
        context
            .todo_board_mut()
            .record_receipt_for_todo(planned_segment.todo_id.as_str(), todo_receipt);

        match execute_status {
            EngineRunStatus::Completed | EngineRunStatus::Stopped => {
                context.execution_retry_tracker_mut().reset();
                context.increment_completed_segments();
                context.session.cursor = planned_segment.cursor_next;
                let typed_summary_snapshot = context.typed_summary().cloned();
                let completion_gate_done = advance_todo_after_execute_completion(
                    context.todo_board_mut(),
                    typed_summary_snapshot.as_ref(),
                    planned_segment.done,
                );
                super::runtime_store::record_todo_progress(
                    &mut state.runtime,
                    &context.todo_board(),
                );
                context.previous_error = None;
                context.last_segment = Some(planned_segment.segment);
                context.set_final_status(execute_outcome.status);
                context.refresh_state_summary(&state, completion_gate_done);
                if completion_gate_done {
                    break;
                }
            }
            EngineRunStatus::Paused => {
                phase_tracker.transition_to(AgentPhase::ResolvePause, "resolve_execution_pause");
                let pause_round = context.completed_segments() as u8;
                if let Some(payload) =
                    super::phase_machine::pause::missing_required_input_payload_from_pause(
                        &state,
                        execute_outcome.last_iteration_events.as_slice(),
                        pause_round,
                    )
                {
                    let trace_extra: [(&str, String); 2] = [
                        ("todo_id", planned_segment.todo_id.clone()),
                        ("segment_id", planned_segment.segment.segment_id.clone()),
                    ];
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
                        super::phase_machine::pause::MissingRequiredInputRecoveryBackflow::Retry { answers, .. } => {
                            context.todo_board_mut().mark_current_todo();
                            super::runtime_store::record_todo_progress(
                                &mut state.runtime,
                                &context.todo_board(),
                            );
                            ckpt.save(
                                &state,
                                &checkpoint_ledger,
                                planner.planning_memory_checkpoint_value(),
                                &context.input_store(),
                                &context.runtime_facts_store(),
                                &context.checkpoint_extensions,
                                &audit_attempt,
                            )?;
                            super::trace::emit(
                                trace_enabled,
                                "pause_resolution",
                                if answers.is_some() {
                                    "resolved_by_user_input"
                                } else {
                                    "autofill_scheduled"
                                },
                                &trace_extra,
                            );
                            if let Some(answers) = answers {
                                handle_resolved_by_user_input(command, &mut state, &mut context, &ckpt, &checkpoint_ledger, planner.planning_memory_checkpoint_value(), planned_segment.done, trace_enabled, "execution", &trace_extra, answers, &audit_attempt)?;
                            }
                            continue;
                        }
                        super::phase_machine::pause::MissingRequiredInputRecoveryBackflow::Paused => {
                            handle_paused_missing_input(&mut state, &mut context, None, &checkpoint_ledger, None, trace_enabled, &trace_extra, None)?;
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
                        context.todo_board_mut().mark_current_todo();
                        super::runtime_store::record_todo_progress(
                            &mut state.runtime,
                            &context.todo_board(),
                        );
                        context.set_previous_error_and_refresh(
                            &state,
                            planned_segment.done,
                            super::missing_input::resolved_payload(
                                &answers,
                                context.completed_segments() as u8,
                            ),
                        );
                        ckpt.save(
                            &state,
                            &checkpoint_ledger,
                            planner.planning_memory_checkpoint_value(),
                            &context.input_store(),
                            &context.runtime_facts_store(),
                            &context.checkpoint_extensions,
                            &audit_attempt,
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
                            .todo_board_mut()
                            .mark_current_blocked("missing_required_input");
                        super::runtime_store::record_todo_progress(
                            &mut state.runtime,
                            &context.todo_board(),
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
                        context.set_final_status(EngineRunStatus::Paused);
                        break;
                    }
                    super::phase_machine::pause::ResolvePauseBackflow::PauseTerminal {
                        blocked_reason,
                    } => {
                        context.set_final_status(EngineRunStatus::Paused);
                        context.todo_board_mut().mark_current_blocked(blocked_reason);
                        super::runtime_store::record_todo_progress(
                            &mut state.runtime,
                            &context.todo_board(),
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
                        let retry_action = context
                            .execution_retry_tracker_mut()
                            .observe_error(state.paused_reason.as_deref());
                        let consecutive_same_error = context
                            .execution_retry_tracker_mut()
                            .consecutive_same_error_count;
                        let last_error_signature = context
                            .execution_retry_tracker_mut()
                            .last_error_signature
                            .clone();
                        super::trace::emit(
                            trace_enabled,
                            "pause_resolution",
                            "pause_repair_scheduled",
                            &[
                                ("todo_id", planned_segment.todo_id.clone()),
                                ("segment_id", planned_segment.segment.segment_id.clone()),
                                ("retry_action", format!("{:?}", retry_action)),
                                (
                                    "consecutive_same_error",
                                    consecutive_same_error.to_string(),
                                ),
                            ],
                        );
                        match retry_action {
                            ExecutionRetryAction::Terminal => {
                                let blocked_reason = format!(
                                    "repeated_executor_error:{}",
                                    last_error_signature
                                        .as_deref()
                                        .unwrap_or("unknown")
                                );
                                context.set_final_status(EngineRunStatus::Paused);
                                context.todo_board_mut().mark_current_blocked(&blocked_reason);
                                super::runtime_store::record_todo_progress(
                                    &mut state.runtime,
                                    &context.todo_board(),
                                );
                                super::trace::emit(
                                    trace_enabled,
                                    "pause_resolution",
                                    "pause_terminal_repeated_error",
                                    &[
                                        ("todo_id", planned_segment.todo_id.clone()),
                                        ("segment_id", planned_segment.segment.segment_id.clone()),
                                        (
                                            "consecutive_failures",
                                            consecutive_same_error.to_string(),
                                        ),
                                    ],
                                );
                                break;
                            }
                            ExecutionRetryAction::RawRetry => {
                                context.set_final_status(EngineRunStatus::Paused);
                                context.set_previous_error_and_refresh(
                                    &state,
                                    planned_segment.done,
                                    previous_error,
                                );
                                super::runtime_store::record_todo_progress(
                                    &mut state.runtime,
                                    &context.todo_board(),
                                );
                                context.last_segment = Some(planned_segment.segment);
                            }
                            ExecutionRetryAction::RepairViaLlm => {
                                context.set_final_status(EngineRunStatus::Paused);
                                context.set_previous_error_and_refresh(
                                    &state,
                                    planned_segment.done,
                                    previous_error,
                                );
                                super::runtime_store::record_todo_progress(
                                    &mut state.runtime,
                                    &context.todo_board(),
                                );
                                context.last_segment = Some(planned_segment.segment);
                            }
                        }
                    }
                }
            }
        }
    }

    let ckpt = super::checkpoint_flow::CheckpointGuard {
        command,
        run_id: run_id.as_str(),
        active_plan_hash: &active_plan_hash,
        active_plan: &active_plan,
    };
    let (normalized_final_status, normalized_paused_reason) =
        super::normalize_agent_terminal_contract(context.final_status(), &state);
    state.paused_reason = normalized_paused_reason;
    if matches!(
        normalized_final_status,
        EngineRunStatus::Completed | EngineRunStatus::Stopped
    ) {
        ckpt.save(
            &state,
            &checkpoint_ledger,
            planner.planning_memory_checkpoint_value(),
            &context.input_store(),
            &context.runtime_facts_store(),
            &context.checkpoint_extensions,
            &audit_attempt,
        )?;
    }
    record_planner_llm_usage(&mut state, &planner);
    super::render_agent_output(
        command,
        &mut state,
        normalized_final_status,
        total_iterations,
        total_events,
        resumed_from_checkpoint,
    )
}

/// Runs the grounding phase loop: attempts to ground the intent, handling
/// non-actionable pause retries and terminal fallbacks.
///
/// Returns `Ok(true)` when grounding succeeds and the agent can proceed to todos.
/// Returns `Ok(false)` when grounding pauses (missing input or terminal fallback).
/// Returns `Err` on fatal grounding failures (caller wraps with checkpoint save).
fn run_grounding_loop<P: LlmProvider>(
    command: &AgentCommand,
    planner: &mut LlmSegmentedIntentPlanner<P>,
    state: &mut EngineRunnerState,
    context: &mut SegmentedAgentContext,
    candidate_context: &CandidateContext,
    readonly_autofill_router: Option<&ais_engine::RouterExecutor>,
    runtime_has_intent_grounding: bool,
    trace_enabled: bool,
) -> Result<bool, RunnerError> {
    let mut grounding_repair_retries = 0u8;
    let mut reuse_runtime_grounding = runtime_has_intent_grounding;
    loop {
        let grounded = bootstrap_intent_grounding_if_needed(
            command,
            planner,
            state,
            context,
            candidate_context,
            readonly_autofill_router,
            reuse_runtime_grounding,
        )?;
        if grounded {
            return Ok(true);
        }
        if matches!(context.final_status(), EngineRunStatus::Stopped) {
            return Ok(false);
        }
        let Some(non_actionable) = detect_grounding_non_actionable_pause(state) else {
            return Ok(false);
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
                apply_grounding_non_actionable_terminal_fallback(state, context, &non_actionable);
                return Ok(false);
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
                seed_grounding_non_actionable_repair_context(state, context, &non_actionable);
                reuse_runtime_grounding = false;
            }
        }
    }
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
        .packed_summary()
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
    volatile_facts_policy: VolatileFactsPolicy,
) -> Result<PlanDocument, Value> {
    let known_refs = super::known_input_refs_from_typed_summary(context.typed_summary())
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let grounding_fact_keys =
        super::grounding_fact_keys_from_typed_summary(context.typed_summary())
            .into_iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
    planned.segment = super::canonicalize_segment_input_refs(
        &planned.segment,
        &known_refs,
        &grounding_fact_keys,
    )?;
    super::validate_segment_todo_scope_with_runtime_facts_and_policy(
        &planned.segment,
        candidate_context,
        context
            .typed_summary()
            .and_then(|summary| summary.todo_state_view().current_todo_execution_scope()),
        context.typed_summary(),
        Some(context.runtime_facts_store()),
        Some(context.input_store()),
        volatile_facts_policy,
    )?;
    super::compile_segment_plan_with_inputs_and_policy(
        context.intent.as_str(),
        &context.session,
        &planned.segment,
        candidate_context,
        pack,
        chain_scope,
        known_refs.as_slice(),
        volatile_facts_policy,
        context.runtime_facts_store(),
        context.input_store(),
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
    let collected = super::missing_registry::collect_compile_missing_input(error_payload);
    if collected.issues.is_empty() {
        return None;
    }
    if collected.missing_refs.is_empty() {
        return None;
    }

    let missing_refs_vec = collected.missing_refs;
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
        collected.issues.as_slice(),
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
    let missing_refs = super::missing_resolution::missing_required_refs(payload);
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

fn advance_todo_after_execute_completion(
    todo_board: &mut TodoBoard,
    typed_summary: Option<&StateSummary>,
    planner_done: bool,
) -> bool {
    todo_board.mark_current_done();
    let acceptance_complete = todo_board.intent_acceptance_complete(typed_summary);
    if !planner_done && todo_board.current().is_none() && !acceptance_complete {
        todo_board.open_follow_up_todo();
    }
    planner_done || acceptance_complete
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
        super::missing_resolution::missing_required_refs(missing_input_payload);
    let static_outcome = super::missing_resolution::apply_static_missing_ref_refill(
        state,
        context,
        initial_missing_refs.as_slice(),
        "compile_autofill",
        todo_id,
    );
    let missing_refs = initial_missing_refs
        .into_iter()
        .filter(|path| {
            !super::missing_resolution::runtime_has_ref_typed(context.typed_summary(), path)
        })
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
                    "available_input_refs": available_input_ref_catalog(context.typed_summary()),
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
        super::missing_resolution::selected_query_refs_from_missing_resolution(&resolution);
    if selected_query_refs.is_empty() {
        let available_input_refs = available_input_ref_catalog(context.typed_summary());
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
                        "query_candidate_pool": super::missing_resolution::query_candidate_pool_from_missing_resolution(&resolution),
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
            "missing_refs": super::missing_resolution::missing_required_input_refs(missing_input_payload),
            "missing_refs_all": super::missing_resolution::missing_required_refs(missing_input_payload),
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

pub(super) fn available_input_ref_catalog(typed_summary: Option<&StateSummary>) -> Vec<Value> {
    super::ref_catalog::available_input_ref_catalog_typed(typed_summary)
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
    candidate_context: &CandidateContext,
    planning_memory: Option<Value>,
    runtime_facts_store: &mut RuntimeFactsStore,
    input_store: &mut InputStore,
    checkpoint_extensions: &checkpoint_ext::AgentCheckpointExtensions,
    audit_attempt: &mut crate::audit_contract::AuditStreamAttempt,
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
        candidate_context,
        planning_memory,
        runtime_facts_store,
        input_store,
        checkpoint_extensions,
        audit_attempt,
        total_events,
        todo_id,
    )
}

fn bind_segment_todo_id(segment: &mut PlanSketchSegment, todo_id: &str) {
    super::phase_machine::segment_exec::bind_segment_todo_id(segment, todo_id);
}

fn collect_segment_missing_refs(
    segment: &PlanSketchSegment,
    typed_summary: Option<&StateSummary>,
) -> Vec<String> {
    super::phase_machine::segment_exec::collect_segment_missing_refs(segment, |reference| {
        super::missing_resolution::runtime_has_ref_typed(typed_summary, reference)
    })
}

fn run_status_name(status: EngineRunStatus) -> &'static str {
    super::phase_machine::segment_exec::run_status_name(status)
}

fn handle_resolved_by_user_input(
    command: &AgentCommand,
    state: &mut EngineRunnerState,
    context: &mut SegmentedAgentContext,
    ckpt: &super::checkpoint_flow::CheckpointGuard<'_>,
    checkpoint_ledger: &RunnerCheckpointLedger,
    planning_memory: Option<Value>,
    done: bool,
    trace_enabled: bool,
    phase_label: &str,
    trace_extra: &[(&str, String)],
    answers: serde_json::Map<String, Value>,
    audit_attempt: &crate::audit_contract::AuditStreamAttempt,
) -> Result<(), RunnerError> {
    context.todo_board_mut().mark_current_todo();
    super::runtime_store::record_todo_progress(&mut state.runtime, &context.todo_board());
    context.set_previous_error_and_refresh(
        state,
        done,
        super::missing_input::resolved_payload(&answers, context.completed_segments() as u8),
    );
    ckpt.save(
        state,
        checkpoint_ledger,
        planning_memory,
        &context.input_store(),
        &context.runtime_facts_store(),
        &context.checkpoint_extensions,
        audit_attempt,
    )?;
    if command.verbose {
        eprintln!(
            "[agent] {phase_label} missing_required_input resolved via user answers keys={}",
            answers.keys().cloned().collect::<Vec<_>>().join(",")
        );
    }
    super::trace::emit(
        trace_enabled,
        "pause_resolution",
        "resolved_by_user_input",
        trace_extra,
    );
    Ok(())
}

fn handle_paused_missing_input(
    state: &mut EngineRunnerState,
    context: &mut SegmentedAgentContext,
    ckpt: Option<&super::checkpoint_flow::CheckpointGuard<'_>>,
    checkpoint_ledger: &RunnerCheckpointLedger,
    planning_memory: Option<Value>,
    trace_enabled: bool,
    trace_extra: &[(&str, String)],
    audit_attempt: Option<&crate::audit_contract::AuditStreamAttempt>,
) -> Result<(), RunnerError> {
    context
        .todo_board_mut()
        .mark_current_blocked("missing_required_input");
    super::runtime_store::record_todo_progress(&mut state.runtime, &context.todo_board());
    if let (Some(ckpt), Some(audit_attempt)) = (ckpt, audit_attempt) {
        ckpt.save(
            state,
            checkpoint_ledger,
            planning_memory,
            &context.input_store(),
            &context.runtime_facts_store(),
            &context.checkpoint_extensions,
            audit_attempt,
        )?;
    }
    super::trace::emit(
        trace_enabled,
        "pause_resolution",
        "paused_missing_required_input",
        trace_extra,
    );
    context.set_final_status(EngineRunStatus::Paused);
    Ok(())
}

fn record_planning_failure_preserving_primary_error(
    ckpt: &super::checkpoint_flow::CheckpointGuard<'_>,
    state: &mut EngineRunnerState,
    checkpoint_ledger: &mut RunnerCheckpointLedger,
    planning_memory: Option<Value>,
    input_store: &InputStore,
    runtime_facts_store: &RuntimeFactsStore,
    checkpoint_extensions: &checkpoint_ext::AgentCheckpointExtensions,
    round: u64,
    planning_error: RunnerError,
    audit_attempt: &mut crate::audit_contract::AuditStreamAttempt,
) -> RunnerError {
    if let Err(checkpoint_error) = ckpt.save_with_planning_failure(
        state,
        checkpoint_ledger,
        planning_memory,
        input_store,
        runtime_facts_store,
        checkpoint_extensions,
        &planning_error,
        round,
        audit_attempt,
    ) {
        if ckpt.command.verbose || ckpt.command.verbose_llm {
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
