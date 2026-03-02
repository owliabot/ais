use crate::error::RunnerError;
use ais_llm::ToolCall;

use super::super::intent_segmented::PlannerRoundPhase;
use super::names::{
    TOOL_CATALOG_RESOLVE_MISSING_FACTS, TOOL_CATALOG_SEARCH, TOOL_GET_CANDIDATE_DETAIL,
    TOOL_GUIDE_GET, TOOL_LIST_CANDIDATES, TOOL_PLAN_BEGIN, TOOL_PLAN_CHECK_SEGMENT,
    TOOL_PLAN_GROUND_INTENT, TOOL_PLAN_PROPOSE_SEGMENT, TOOL_PLAN_PROPOSE_TODOS,
    TOOL_PLAN_REVISE_SEGMENT,
};

pub(crate) fn phase_name(phase: PlannerRoundPhase) -> &'static str {
    match phase {
        PlannerRoundPhase::Begin => "begin",
        PlannerRoundPhase::GroundIntent => "ground_intent",
        PlannerRoundPhase::ProposeTodos => "propose_todos",
        PlannerRoundPhase::ProposeSegment => "propose_segment",
        PlannerRoundPhase::ReviseSegment => "revise_segment",
    }
}

pub(crate) fn finalize_tool_for_phase(phase: PlannerRoundPhase) -> &'static str {
    match phase {
        PlannerRoundPhase::Begin => TOOL_PLAN_BEGIN,
        PlannerRoundPhase::GroundIntent => TOOL_PLAN_GROUND_INTENT,
        PlannerRoundPhase::ProposeTodos => TOOL_PLAN_PROPOSE_TODOS,
        PlannerRoundPhase::ProposeSegment => TOOL_PLAN_PROPOSE_SEGMENT,
        PlannerRoundPhase::ReviseSegment => TOOL_PLAN_REVISE_SEGMENT,
    }
}

pub(crate) fn ensure_tool_allowed_for_phase(
    tool_name: &str,
    phase: PlannerRoundPhase,
) -> Result<(), RunnerError> {
    let allowed = match phase {
        PlannerRoundPhase::Begin => matches!(tool_name, TOOL_PLAN_BEGIN),
        PlannerRoundPhase::GroundIntent => matches!(
            tool_name,
            TOOL_LIST_CANDIDATES
                | TOOL_CATALOG_SEARCH
                | TOOL_CATALOG_RESOLVE_MISSING_FACTS
                | TOOL_GET_CANDIDATE_DETAIL
                | TOOL_GUIDE_GET
                | TOOL_PLAN_GROUND_INTENT
        ),
        PlannerRoundPhase::ProposeTodos => matches!(
            tool_name,
            TOOL_LIST_CANDIDATES
                | TOOL_CATALOG_SEARCH
                | TOOL_CATALOG_RESOLVE_MISSING_FACTS
                | TOOL_GET_CANDIDATE_DETAIL
                | TOOL_GUIDE_GET
                | TOOL_PLAN_PROPOSE_TODOS
        ),
        PlannerRoundPhase::ProposeSegment => matches!(
            tool_name,
            TOOL_LIST_CANDIDATES
                | TOOL_CATALOG_SEARCH
                | TOOL_CATALOG_RESOLVE_MISSING_FACTS
                | TOOL_GET_CANDIDATE_DETAIL
                | TOOL_GUIDE_GET
                | TOOL_PLAN_CHECK_SEGMENT
                | TOOL_PLAN_PROPOSE_SEGMENT
        ),
        PlannerRoundPhase::ReviseSegment => matches!(
            tool_name,
            TOOL_LIST_CANDIDATES
                | TOOL_CATALOG_SEARCH
                | TOOL_CATALOG_RESOLVE_MISSING_FACTS
                | TOOL_GET_CANDIDATE_DETAIL
                | TOOL_GUIDE_GET
                | TOOL_PLAN_CHECK_SEGMENT
                | TOOL_PLAN_REVISE_SEGMENT
        ),
    };
    if allowed {
        Ok(())
    } else {
        Err(RunnerError::Llm(format!(
            "tool `{tool_name}` is not allowed in planner phase `{}`",
            phase_name(phase)
        )))
    }
}

pub(crate) fn validate_tool_calls_for_phase(
    tool_calls: &[ToolCall],
    phase: PlannerRoundPhase,
) -> Result<(), RunnerError> {
    for call in tool_calls {
        ensure_tool_allowed_for_phase(call.name.as_str(), phase)?;
    }
    let finalize_tool = finalize_tool_for_phase(phase);
    let finalize_indexes = tool_calls
        .iter()
        .enumerate()
        .filter_map(|(index, call)| (call.name == finalize_tool).then_some(index))
        .collect::<Vec<_>>();

    if phase == PlannerRoundPhase::Begin {
        if tool_calls.len() != 1 || finalize_indexes.len() != 1 {
            return Err(RunnerError::Llm(format!(
                "{} phase requires exactly one tool call: `{finalize_tool}`",
                phase_name(phase)
            )));
        }
        return Ok(());
    }

    if finalize_indexes.len() > 1 {
        return Err(RunnerError::Llm(format!(
            "planner phase `{}` allows at most one finalize tool `{finalize_tool}` per round",
            phase_name(phase)
        )));
    }
    if let Some(index) = finalize_indexes.first() {
        if *index != tool_calls.len().saturating_sub(1) {
            return Err(RunnerError::Llm(format!(
                "finalize tool `{finalize_tool}` must be the last tool call in this round"
            )));
        }
    }
    Ok(())
}
