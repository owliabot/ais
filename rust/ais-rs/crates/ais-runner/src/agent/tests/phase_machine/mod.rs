use super::*;

#[test]
fn main_flow_runner_completes_and_records_terminal_transition() {
    let output = run_main_flow(false, |_| Ok("ok".to_string())).expect("main flow success");
    assert_eq!(output, "ok");
}

#[test]
fn main_flow_runner_returns_error() {
    let result = run_main_flow(false, |_| Err(RunnerError::Llm("flow_failed".to_string())));
    let error = result.expect_err("main flow must surface error");
    assert!(matches!(error, RunnerError::Llm(_)));
}

#[test]
fn runner_bootstrap_starts_from_init_phase() {
    let runner = PhaseMachineRunner::new();
    assert_eq!(runner.current_phase(), AgentPhase::Init);
    assert!(runner.transitions().is_empty());
}

#[test]
fn main_flow_runner_attributes_failure_to_last_reported_phase() {
    let (result, runner) = run_main_flow_internal(false, |phase_tracker| {
        phase_tracker.transition_to(AgentPhase::PlanSegment, "test_plan_segment");
        phase_tracker.transition_to(AgentPhase::ExecuteSegment, "test_execute_segment");
        Err(RunnerError::Llm("flow_failed".to_string()))
    });
    let error = result.expect_err("main flow must surface error");
    assert!(matches!(error, RunnerError::Llm(_)));
    assert_eq!(runner.current_phase(), AgentPhase::Failed);
    assert!(matches!(
        runner.transitions().last(),
        Some(PhaseTransition::Fail {
            phase: AgentPhase::ExecuteSegment,
            ..
        })
    ));
}
