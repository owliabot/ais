use crate::error::RunnerError;
use serde_json::Value;

use super::super::intent_segmented::PlannerRoundPhase;
use super::guide::normalize_guide_get_tool_args;
use super::names::{
    TOOL_PLAN_BEGIN, TOOL_PLAN_GROUND_INTENT, TOOL_PLAN_PROPOSE_SEGMENT, TOOL_PLAN_PROPOSE_TODOS,
    TOOL_PLAN_REVISE_SEGMENT,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ToolArgsNormalization {
    pub(crate) arguments: Value,
    pub(crate) normalized_fields: Vec<&'static str>,
}

impl ToolArgsNormalization {
    pub(crate) fn changed(&self) -> bool {
        !self.normalized_fields.is_empty()
    }
}

pub(crate) fn normalize_tool_args_for_validation(
    tool_name: &str,
    arguments: &Value,
) -> ToolArgsNormalization {
    match tool_name {
        "guide.get" => normalize_guide_get_tool_args(arguments),
        _ => ToolArgsNormalization {
            arguments: arguments.clone(),
            normalized_fields: vec![],
        },
    }
}

pub(crate) fn phase_from_finalize_tool(
    finalize_tool: &str,
) -> Result<PlannerRoundPhase, RunnerError> {
    match finalize_tool {
        TOOL_PLAN_BEGIN => Ok(PlannerRoundPhase::Begin),
        TOOL_PLAN_GROUND_INTENT => Ok(PlannerRoundPhase::GroundIntent),
        TOOL_PLAN_PROPOSE_TODOS => Ok(PlannerRoundPhase::ProposeTodos),
        TOOL_PLAN_PROPOSE_SEGMENT => Ok(PlannerRoundPhase::ProposeSegment),
        TOOL_PLAN_REVISE_SEGMENT => Ok(PlannerRoundPhase::ReviseSegment),
        other => Err(RunnerError::Llm(format!(
            "unsupported segmented planner finalize tool `{other}`"
        ))),
    }
}
