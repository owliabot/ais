use serde_json::json;

use crate::cel::runtime::value::CelValue;
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

#[test]
fn cel_converts_between_atomic_and_unit_values_exactly() {
    let scope = CelScope::new();
    let mut evaluator = CelEvaluator::new();

    assert_eq!(
        evaluator
            .evaluate_value("to_atomic('10', 6)", &scope)
            .expect("to_atomic"),
        CelValue::Integer(10_000_000u64.into())
    );
    assert_eq!(
        evaluator
            .evaluate_value("to_atomic('120.5', 18)", &scope)
            .expect("to_atomic"),
        CelValue::Integer("120500000000000000000".parse().expect("bigint"))
    );
    assert_eq!(
        evaluator
            .evaluate_value("to_unit('10000000', 6)", &scope)
            .expect("to_unit"),
        CelValue::Integer(10u8.into())
    );
    assert_eq!(
        evaluator
            .evaluate_value("to_unit('120500000000000000000', 18)", &scope)
            .expect("to_unit"),
        CelValue::Decimal(crate::cel::runtime::numeric::Decimal::parse("120.5").expect("decimal"))
    );
}

#[test]
fn cel_unit_conversion_supports_asset_maps_and_negative_values() {
    let mut scope = CelScope::new();
    scope.insert_json("asset", json!({"decimals": 18}));

    let mut evaluator = CelEvaluator::new();
    assert_eq!(
        evaluator
            .evaluate_value("to_atomic('-0.5', asset)", &scope)
            .expect("to_atomic"),
        CelValue::Integer("-500000000000000000".parse().expect("bigint"))
    );
    assert_eq!(
        evaluator
            .evaluate_value("string(to_unit('-500000000000000000', asset))", &scope)
            .expect("to_unit"),
        CelValue::String("-0.5".to_owned())
    );
}

#[test]
fn cel_rejects_non_exact_unit_conversion() {
    let scope = CelScope::new();
    let mut evaluator = CelEvaluator::new();

    let error = evaluator
        .evaluate_value("to_atomic('1.1234567', 6)", &scope)
        .expect_err("must reject excess precision");
    assert_eq!(error.to_string(), "numeric error: non exact division");
}

#[test]
fn cel_int_builtin_coerces_exact_integer_inputs() {
    let scope = CelScope::new();
    let mut evaluator = CelEvaluator::new();

    assert_eq!(
        evaluator.evaluate_value("int('42')", &scope).expect("int"),
        CelValue::Integer(42u8.into())
    );
    assert_eq!(
        evaluator.evaluate_value("int(42.0)", &scope).expect("int"),
        CelValue::Integer(42u8.into())
    );
    assert_eq!(
        evaluator.evaluate_value("int(true)", &scope).expect("int"),
        CelValue::Integer(1u8.into())
    );
}

#[test]
fn cel_int_builtin_rejects_fractional_values() {
    let scope = CelScope::new();
    let mut evaluator = CelEvaluator::new();

    let error = evaluator
        .evaluate_value("int(42.5)", &scope)
        .expect_err("fractional decimal must fail");
    assert_eq!(error.to_string(), "numeric error: non exact division");
}
