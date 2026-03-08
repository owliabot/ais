use super::super::orchestrator::SegmentedAgentContext;
use super::super::*;
use serde_json::Value;

const MAX_PLANNER_OUTPUT_REPAIR_RETRIES: usize = 2;

pub(crate) fn plan_round<P: LlmProvider>(
    planner: &mut LlmSegmentedIntentPlanner<P>,
    state: &EngineRunnerState,
    context: &mut SegmentedAgentContext,
) -> Result<SegmentDraft, RunnerError> {
    loop {
        if context.planning_rounds() >= context.planner_round_limit() {
            return Err(RunnerError::Llm(format!(
                "segmented planner round limit reached ({})",
                context.planner_round_limit()
            )));
        }
        context.increment_planning_rounds();

        let request = SegmentPlanningRequest {
            intent: context.intent.clone(),
            session: context.session.clone(),
            state_summary: context.packed_summary().clone(),
            typed_summary: context.typed_summary().cloned(),
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
                context.planning_rounds(),
                expected_finalize_tool,
                previous_error_compact(previous_error)
            );
        } else {
            eprintln!(
                "[agent] plan_round={} mode={} previous_error=-",
                context.planning_rounds(),
                expected_finalize_tool
            );
        }
        let draft_result = if context.previous_error.is_some() {
            planner.revise_segment(request)
        } else {
            planner.propose_segment(request)
        };
        super::super::orchestrator::refresh_tool_memory_projection(context, planner, state);
        match draft_result {
            Ok(draft) => {
                context.reset_planner_output_retries();
                return Ok(draft);
            }
            Err(error) => {
                if super::super::should_retry_segmented_planner_output(&error)
                    && context.planner_output_retries() < MAX_PLANNER_OUTPUT_REPAIR_RETRIES
                {
                    context.increment_planner_output_retries();
                    eprintln!(
                        "[agent] planner_output_retry retry={}/{} reason={}",
                        context.planner_output_retries(),
                        MAX_PLANNER_OUTPUT_REPAIR_RETRIES,
                        error
                    );
                    let last_failed_finalize = planner.take_last_failed_finalize();
                    let mut payload = super::super::segmented_planner_output_error_payload(
                        &error,
                        expected_finalize_tool,
                        context.planning_rounds() as u8,
                        context.planner_output_retries() as u8,
                        last_failed_finalize,
                    );
                    super::super::missing_resolution::preserve_autofill_context(
                        context.previous_error.as_ref(),
                        &mut payload,
                    );
                    context.set_previous_error_and_refresh(state, false, payload);
                    continue;
                }
                return Err(error);
            }
        }
    }
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
    let autofill_selected = error
        .pointer("/autofill/selected_query_refs")
        .and_then(Value::as_array)
        .map(|items| items.len().to_string())
        .unwrap_or_else(|| "0".to_string());
    format!(
        "phase={phase},reason={reason_code},sub_reason={sub_reason_code},autofill_selected={autofill_selected}"
    )
}
