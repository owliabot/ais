use super::{build_solver_event, DefaultSolver, Solver, SolverContext, SolverDecision};
use crate::events::EngineEventType;
use ais_sdk::{NodeReadinessResult, NodeRunState};
use serde_json::{json, Map};
use std::collections::BTreeMap;

fn blocked_readiness(missing_refs: Vec<&str>, needs_detect: bool) -> NodeReadinessResult {
    NodeReadinessResult {
        state: NodeRunState::Blocked,
        missing_refs: missing_refs.into_iter().map(str::to_string).collect(),
        needs_detect,
        errors: Vec::new(),
        resolved_params: Some(Map::new()),
    }
}

#[test]
fn blocked_contract_ref_with_single_candidate_returns_solver_applied() {
    let solver = DefaultSolver;
    let readiness = blocked_readiness(vec!["contracts.router"], false);
    let context = SolverContext {
        contract_candidates: BTreeMap::from([(
            "contracts.router".to_string(),
            vec![json!("0x0000000000000000000000000000000000000001")],
        )]),
        detect_provider_candidates: Vec::new(),
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
fn blocked_input_missing_returns_need_user_confirm() {
    let solver = DefaultSolver;
    let readiness = blocked_readiness(vec!["inputs.amount"], false);
    let context = SolverContext::default();

    let decision = solver.solve(&json!({"id": "n2"}), &readiness, &context);
    match &decision {
        SolverDecision::NeedUserConfirm { reason, details } => {
            assert_eq!(reason, "missing_inputs_or_runtime_refs");
            assert!(details.get("missing_refs").is_some());
        }
        _ => panic!("expected need_user_confirm"),
    }

    let event = build_solver_event(Some("n2"), &decision).expect("event expected");
    assert_eq!(event.event_type, EngineEventType::NeedUserConfirm);
}

#[test]
fn blocked_detect_with_single_provider_selects_provider() {
    let solver = DefaultSolver;
    let readiness = blocked_readiness(Vec::new(), true);
    let context = SolverContext {
        contract_candidates: BTreeMap::new(),
        detect_provider_candidates: vec!["provider-a".to_string()],
    };

    let decision = solver.solve(&json!({"id": "n3"}), &readiness, &context);
    match decision {
        SolverDecision::SelectProvider { provider, .. } => assert_eq!(provider, "provider-a"),
        _ => panic!("expected select_provider"),
    }
}
