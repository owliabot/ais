use super::super::orchestrator::SegmentedAgentContext;
use super::super::*;
use serde_json::Value;

pub(crate) fn bootstrap_todos_if_needed<P: LlmProvider>(
    command: &AgentCommand,
    planner: &mut LlmSegmentedIntentPlanner<P>,
    state: &mut EngineRunnerState,
    context: &mut SegmentedAgentContext,
    candidate_context: &CandidateContext,
    readonly_autofill_router: Option<&ais_engine::RouterExecutor>,
    runtime_has_todo_progress: bool,
) -> Result<(), RunnerError> {
    let trace_enabled = command.verbose || command.verbose_llm;
    if runtime_has_todo_progress {
        super::super::trace::emit(trace_enabled, "todo", "reuse_runtime_progress", &[]);
        return Ok(());
    }
    if !super::grounding::intent_grounding_ready_for_todos(state) || !capability_view_ready(state) {
        let questions = state
            .runtime
            .pointer("/agent/intent_grounding/questions")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let payload = super::super::missing_input::payload(
            Some("planning_readiness_not_met"),
            questions.as_slice(),
            &[],
            context.completed_segments_u8(),
        );
        super::super::missing_input::pause_with_payload(state, &payload);
        context.set_previous_error_and_refresh(
            state,
            false,
            super::super::todo_phase_error_payload(
                "planning_readiness_not_met",
                Some("planning_readiness_not_met"),
                &[],
                questions.as_slice(),
                context.completed_segments_u8(),
            ),
        );
        super::super::trace::emit(
            trace_enabled,
            "todo",
            "planning_readiness_not_met",
            &[("questions", questions.len().to_string())],
        );
        return Ok(());
    }
    let draft = planner.propose_todos(TodoPlanningRequest {
        intent: context.intent().to_string(),
        session: context.session().clone(),
        state_summary: context.state_summary().clone(),
    });
    super::super::orchestrator::refresh_tool_memory_projection(context, planner, state);
    match draft {
        Ok(TodoDraft::Proposed {
            summary,
            todos,
            issues,
        }) => {
            let intent = context.intent().to_string();
            {
                let todo_board = context.todo_board_mut();
                let rejected_tail_count = todo_board.replace_from_specs(intent.as_str(), &todos);
                todo_board.ensure_current();
                if rejected_tail_count > 0 {
                    super::super::trace::emit(
                        trace_enabled,
                        "todo",
                        "todo_tail_rejected",
                        &[("count", rejected_tail_count.to_string())],
                    );
                }
            }
            super::super::runtime_store::record_todo_progress(
                &mut state.runtime,
                context.todo_board(),
            );
            context.clear_previous_error_and_refresh(state, false);
            super::super::trace::emit(
                trace_enabled,
                "todo",
                "todos_proposed",
                &[("count", todos.len().to_string())],
            );
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
            super::super::trace::emit(
                trace_enabled,
                "todo",
                "todos_unavailable",
                &[
                    ("reason_code", reason_code.clone()),
                    ("questions", questions.len().to_string()),
                ],
            );
            if reason_code == "missing_required_input" {
                let payload = super::super::missing_input::payload(
                    message.as_deref(),
                    questions.as_slice(),
                    issues.as_slice(),
                    context.completed_segments_u8(),
                );
                match super::super::phase_machine::pause::recover_missing_required_input_payload(
                    command,
                    state,
                    context,
                    candidate_context,
                    readonly_autofill_router,
                    &payload,
                    "todo",
                    false,
                    "todo",
                    false,
                    true,
                )? {
                    super::super::phase_machine::pause::MissingRequiredInputRecoveryBackflow::RetryScheduled => {
                        return Ok(());
                    }
                    super::super::phase_machine::pause::MissingRequiredInputRecoveryBackflow::ResolvedByUserInput { answers } => {
                        context.clear_previous_error_and_refresh(state, false);
                        if command.verbose_llm {
                            eprintln!(
                                "[agent] todo plan missing_required_input resolved via user answers keys={}",
                                answers.keys().cloned().collect::<Vec<_>>().join(",")
                            );
                        }
                        super::super::trace::emit(
                            trace_enabled,
                            "pause_resolution",
                            "resolved_by_user_input",
                            &[("phase_hint", "todo".to_string())],
                        );
                        return Ok(());
                    }
                    super::super::phase_machine::pause::MissingRequiredInputRecoveryBackflow::Paused => {
                        context.set_previous_error_and_refresh(
                            state,
                            false,
                            super::super::todo_phase_error_payload(
                                "missing_required_input",
                                message.as_deref(),
                                issues.as_slice(),
                                questions.as_slice(),
                                context.completed_segments_u8(),
                            ),
                        );
                        super::super::trace::emit(
                            trace_enabled,
                            "pause_resolution",
                            "paused_missing_required_input",
                            &[("phase_hint", "todo".to_string())],
                        );
                        return Ok(());
                    }
                }
            }
            context.set_previous_error_and_refresh(
                state,
                false,
                super::super::todo_phase_error_payload(
                    "unavailable",
                    message.as_deref(),
                    issues.as_slice(),
                    questions.as_slice(),
                    context.completed_segments_u8(),
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
                super::super::todo_phase_error_payload(
                    "invalid",
                    message.as_deref(),
                    issues.as_slice(),
                    &[],
                    context.completed_segments_u8(),
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
            super::super::trace::emit(
                trace_enabled,
                "todo",
                "todos_invalid",
                &[("reason_code", reason_code)],
            );
        }
        Err(error) => {
            let error_message = error.to_string();
            context.set_previous_error_and_refresh(
                state,
                false,
                super::super::todo_phase_error_payload(
                    "planner_call_failed",
                    Some(error_message.as_str()),
                    &[],
                    &[],
                    context.completed_segments_u8(),
                ),
            );
            if command.verbose_llm {
                eprintln!(
                    "[agent] todo plan failed error={} (fallback bootstrap todo)",
                    error
                );
            }
            super::super::trace::emit(
                trace_enabled,
                "todo",
                "planner_call_failed",
                &[("error", error.to_string())],
            );
        }
    }
    Ok(())
}

fn capability_view_ready(state: &EngineRunnerState) -> bool {
    state
        .runtime
        .pointer("/agent/capability_view/ready")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}
