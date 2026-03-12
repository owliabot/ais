use super::{build_solver_event, DefaultSolver, Solver, SolverContext, SolverDecision};
use crate::events::EngineEventType;
use ais_sdk::{NodeReadinessResult, NodeRunState};
use serde_json::{json, Map};

fn blocked_readiness(missing_refs: Vec<&str>) -> NodeReadinessResult {
    NodeReadinessResult {
        state: NodeRunState::Blocked,
        missing_refs: missing_refs.into_iter().map(str::to_string).collect(),
        errors: Vec::new(),
        resolved_params: Some(Map::new()),
    }
}

#[test]
fn blocked_contract_ref_returns_need_user_confirm_instead_of_patch_autofill() {
    let solver = DefaultSolver;
    let readiness = blocked_readiness(vec!["contracts.router"]);
    let context = SolverContext::default();

    let decision = solver.solve(&json!({"id": "n1"}), &readiness, &context);
    match &decision {
        SolverDecision::NeedUserConfirm { reason, details } => {
            assert_eq!(reason, "unresolved_system_refs");
            assert_eq!(
                details.get("system_missing_refs"),
                Some(&json!(["contracts.router"]))
            );
        }
        _ => panic!("expected need_user_confirm"),
    }

    let event = build_solver_event(Some("n1"), &decision).expect("event expected");
    assert_eq!(event.event_type, EngineEventType::NeedUserConfirm);
}

#[test]
fn blocked_input_missing_returns_need_user_input() {
    let solver = DefaultSolver;
    let readiness = blocked_readiness(vec!["inputs.amount"]);
    let context = SolverContext::default();

    let decision = solver.solve(&json!({"id": "n2"}), &readiness, &context);
    match &decision {
        SolverDecision::NeedUserInput { reason, details } => {
            assert_eq!(reason, "missing_inputs_or_runtime_refs");
            assert!(details.get("missing_refs").is_some());
        }
        _ => panic!("expected need_user_input"),
    }

    let event = build_solver_event(Some("n2"), &decision).expect("event expected");
    assert_eq!(event.event_type, EngineEventType::NeedUserInput);
}
