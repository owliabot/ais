use super::{build_solver_event, DefaultSolver, Solver, SolverContext, SolverDecision};
use crate::events::EngineEventType;
use ais_sdk::{NodeReadinessResult, NodeRunState};
use serde_json::{json, Map};
use std::collections::BTreeMap;

fn blocked_readiness(missing_refs: Vec<&str>) -> NodeReadinessResult {
    NodeReadinessResult {
        state: NodeRunState::Blocked,
        missing_refs: missing_refs.into_iter().map(str::to_string).collect(),
        errors: Vec::new(),
        resolved_params: Some(Map::new()),
    }
}

#[test]
fn blocked_contract_ref_with_single_candidate_returns_solver_applied() {
    let solver = DefaultSolver;
    let readiness = blocked_readiness(vec!["contracts.router"]);
    let context = SolverContext {
        contract_candidates: BTreeMap::from([(
            "contracts.router".to_string(),
            vec![json!("0x0000000000000000000000000000000000000001")],
        )]),
    };

    let decision = solver.solve(&json!({"id": "n1"}), &readiness, &context);
    match &decision {
        SolverDecision::ApplyPatches { patches, .. } => {
            assert_eq!(patches.len(), 1);
            assert_eq!(patches[0].path, "contracts.router");
        }
        _ => panic!("expected apply_patches"),
    }

    let event = build_solver_event(Some("n1"), &decision).expect("event expected");
    assert_eq!(event.event_type, EngineEventType::SolverApplied);
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
