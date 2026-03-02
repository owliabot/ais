#![allow(dead_code)]

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum AgentPhase {
    Init,
    GroundIntent,
    PlanTodos,
    PlanSegment,
    ExecuteSegment,
    ResolvePause,
    Completed,
    Failed,
}

impl AgentPhase {
    pub(crate) const ALL: [AgentPhase; 8] = [
        AgentPhase::Init,
        AgentPhase::GroundIntent,
        AgentPhase::PlanTodos,
        AgentPhase::PlanSegment,
        AgentPhase::ExecuteSegment,
        AgentPhase::ResolvePause,
        AgentPhase::Completed,
        AgentPhase::Failed,
    ];

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            AgentPhase::Init => "init",
            AgentPhase::GroundIntent => "ground_intent",
            AgentPhase::PlanTodos => "plan_todos",
            AgentPhase::PlanSegment => "plan_segment",
            AgentPhase::ExecuteSegment => "execute_segment",
            AgentPhase::ResolvePause => "resolve_pause",
            AgentPhase::Completed => "completed",
            AgentPhase::Failed => "failed",
        }
    }

    pub(crate) fn is_terminal(self) -> bool {
        matches!(self, AgentPhase::Completed | AgentPhase::Failed)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PhaseTransition {
    Advance {
        from: AgentPhase,
        to: AgentPhase,
    },
    Stay {
        phase: AgentPhase,
        reason: &'static str,
    },
    Pause {
        phase: AgentPhase,
        reason: String,
    },
    Complete,
    Fail {
        phase: AgentPhase,
        reason: String,
    },
}

impl PhaseTransition {
    pub(crate) fn current_phase(&self) -> Option<AgentPhase> {
        match self {
            PhaseTransition::Advance { from, .. } => Some(*from),
            PhaseTransition::Stay { phase, .. } => Some(*phase),
            PhaseTransition::Pause { phase, .. } => Some(*phase),
            PhaseTransition::Complete => Some(AgentPhase::Completed),
            PhaseTransition::Fail { phase, .. } => Some(*phase),
        }
    }

    pub(crate) fn next_phase(&self) -> AgentPhase {
        match self {
            PhaseTransition::Advance { to, .. } => *to,
            PhaseTransition::Stay { phase, .. } => *phase,
            PhaseTransition::Pause { .. } => AgentPhase::ResolvePause,
            PhaseTransition::Complete => AgentPhase::Completed,
            PhaseTransition::Fail { .. } => AgentPhase::Failed,
        }
    }
}

#[cfg(test)]
#[path = "../tests/phase_machine/types.rs"]
mod tests;
