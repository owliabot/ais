use super::{AgentPhase, PhaseTransition};

#[test]
fn all_phases_cover_expected_state_machine_nodes() {
    assert_eq!(AgentPhase::ALL.len(), 8);
    assert!(AgentPhase::ALL.contains(&AgentPhase::Init));
    assert!(AgentPhase::ALL.contains(&AgentPhase::GroundIntent));
    assert!(AgentPhase::ALL.contains(&AgentPhase::PlanTodos));
    assert!(AgentPhase::ALL.contains(&AgentPhase::PlanSegment));
    assert!(AgentPhase::ALL.contains(&AgentPhase::ExecuteSegment));
    assert!(AgentPhase::ALL.contains(&AgentPhase::ResolvePause));
    assert!(AgentPhase::ALL.contains(&AgentPhase::Completed));
    assert!(AgentPhase::ALL.contains(&AgentPhase::Failed));
}

#[test]
fn phase_transition_maps_pause_and_terminal_states() {
    let paused = PhaseTransition::Pause {
        phase: AgentPhase::ExecuteSegment,
        reason: "need_user_confirm".to_string(),
    };
    assert_eq!(paused.current_phase(), Some(AgentPhase::ExecuteSegment));
    assert_eq!(paused.next_phase(), AgentPhase::ResolvePause);

    let completed = PhaseTransition::Complete;
    assert_eq!(completed.current_phase(), Some(AgentPhase::Completed));
    assert_eq!(completed.next_phase(), AgentPhase::Completed);

    let failed = PhaseTransition::Fail {
        phase: AgentPhase::PlanSegment,
        reason: "compile_guard_failed".to_string(),
    };
    assert_eq!(failed.current_phase(), Some(AgentPhase::PlanSegment));
    assert_eq!(failed.next_phase(), AgentPhase::Failed);
}

#[test]
fn terminal_phase_flag_is_consistent() {
    assert!(!AgentPhase::Init.is_terminal());
    assert!(!AgentPhase::ResolvePause.is_terminal());
    assert!(AgentPhase::Completed.is_terminal());
    assert!(AgentPhase::Failed.is_terminal());
}
