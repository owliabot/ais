use serde_json::json;

use crate::cel::{typing::CelExpressionKind, CelEvaluator, CelScope, CelTypeChecker};

#[test]
fn cel_evaluates_policy_predicate_with_member_access() {
    let mut scope = CelScope::new();
    scope.insert_json("mission", json!({"max_slippage_bps": 100}));
    scope.insert_json("params", json!({"slippage_bps": 50}));

    let mut evaluator = CelEvaluator::new();
    let result = evaluator
        .evaluate_bool("params.slippage_bps <= mission.max_slippage_bps", &scope)
        .expect("predicate");

    assert!(result);
}

#[test]
fn cel_evaluates_mul_div_with_numeric_strings() {
    let mut scope = CelScope::new();
    scope.insert_json("quote", json!({"amount_out_atomic": "1000000"}));
    scope.insert_json("params", json!({"slippage_bps": 50}));

    let mut evaluator = CelEvaluator::new();
    let value = evaluator
        .evaluate_value(
            "mul_div(quote.amount_out_atomic, (10000 - params.slippage_bps), 10000)",
            &scope,
        )
        .expect("mul_div");

    let rendered = evaluator
        .evaluate_value(
            "string(mul_div(quote.amount_out_atomic, (10000 - params.slippage_bps), 10000))",
            &scope,
        )
        .expect("string");

    assert!(matches!(
        value,
        crate::cel::runtime::value::CelValue::Decimal(_)
    ));
    assert_eq!(
        rendered,
        crate::cel::runtime::value::CelValue::String("995000".to_owned())
    );
}

#[test]
fn cel_type_checker_accepts_local_boundary_expression() {
    let checker = CelTypeChecker;
    checker
        .validate(
            CelExpressionKind::BoundaryPredicate,
            "size(required_inputs) == 0 || params.force == true",
        )
        .expect("valid expression");
}
