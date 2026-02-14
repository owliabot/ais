use super::{evaluate_expression, CelContext, CelValue, CELEvaluator};
use crate::numeric::Decimal;
use num_bigint::BigInt;
use std::collections::BTreeMap;

fn bi(value: i64) -> BigInt {
    BigInt::from(value)
}

#[test]
fn evaluates_arithmetic_expression() {
    let context = CelContext::new();
    let value = evaluate_expression("1 + 2 * 3", &context).expect("eval");
    assert_eq!(value, CelValue::Integer(bi(7)));
}

#[test]
fn evaluates_member_and_index() {
    let mut context = CelContext::new();
    context.insert(
        "inputs".to_string(),
        CelValue::Map(BTreeMap::from([(
            "items".to_string(),
            CelValue::List(vec![CelValue::Integer(bi(10)), CelValue::Integer(bi(11))]),
        )])),
    );

    let value = evaluate_expression("inputs.items[1]", &context).expect("eval");
    assert_eq!(value, CelValue::Integer(bi(11)));
}

#[test]
fn evaluates_ternary_expression() {
    let mut context = CelContext::new();
    context.insert("ok".to_string(), CelValue::Bool(true));
    let value = evaluate_expression("ok ? 1 : 2", &context).expect("eval");
    assert_eq!(value, CelValue::Integer(bi(1)));
}

#[test]
fn caches_parsed_expressions() {
    let context = CelContext::new();
    let mut evaluator = CELEvaluator::new();
    evaluator.evaluate("1 + 1", &context).expect("eval");
    evaluator.evaluate("1 + 1", &context).expect("eval");
    assert_eq!(evaluator.cached_expressions(), 1);
}

#[test]
fn builtin_string_functions_work() {
    let context = CelContext::new();
    assert_eq!(
        evaluate_expression("size('abc')", &context).expect("eval"),
        CelValue::Integer(bi(3))
    );
    assert_eq!(
        evaluate_expression("contains('hello', 'ell')", &context).expect("eval"),
        CelValue::Bool(true)
    );
    assert_eq!(
        evaluate_expression("startsWith('hello', 'he')", &context).expect("eval"),
        CelValue::Bool(true)
    );
    assert_eq!(
        evaluate_expression("endsWith('hello', 'lo')", &context).expect("eval"),
        CelValue::Bool(true)
    );
    assert_eq!(
        evaluate_expression("matches('hello', '^h.*o$')", &context).expect("eval"),
        CelValue::Bool(true)
    );
    assert_eq!(
        evaluate_expression("lower('HELLO')", &context).expect("eval"),
        CelValue::String("hello".to_string())
    );
    assert_eq!(
        evaluate_expression("upper('hello')", &context).expect("eval"),
        CelValue::String("HELLO".to_string())
    );
    assert_eq!(
        evaluate_expression("trim('  hi  ')", &context).expect("eval"),
        CelValue::String("hi".to_string())
    );
}

#[test]
fn builtin_math_functions_work() {
    let context = CelContext::new();
    assert_eq!(evaluate_expression("abs(-5)", &context).expect("eval"), CelValue::Integer(bi(5)));
    assert_eq!(evaluate_expression("min(3,1,2)", &context).expect("eval"), CelValue::Integer(bi(1)));
    assert_eq!(evaluate_expression("max(3,1,2)", &context).expect("eval"), CelValue::Integer(bi(3)));
    assert_eq!(evaluate_expression("ceil(1.2)", &context).expect("eval"), CelValue::Integer(bi(2)));
    assert_eq!(evaluate_expression("floor(1.8)", &context).expect("eval"), CelValue::Integer(bi(1)));
    assert_eq!(evaluate_expression("round(1.5)", &context).expect("eval"), CelValue::Integer(bi(2)));
    assert_eq!(
        evaluate_expression("mul_div(1000, 9950, 10000)", &context).expect("eval"),
        CelValue::Integer(bi(995))
    );
}

#[test]
fn builtin_type_functions_work() {
    let context = CelContext::new();
    assert_eq!(evaluate_expression("int('42')", &context).expect("eval"), CelValue::Integer(bi(42)));
    assert_eq!(evaluate_expression("uint(-5)", &context).expect("eval"), CelValue::Integer(bi(5)));
    assert_eq!(
        evaluate_expression("double('3.14')", &context).expect("eval"),
        CelValue::Decimal(Decimal::parse("3.14").expect("decimal"))
    );
    assert_eq!(
        evaluate_expression("string(42)", &context).expect("eval"),
        CelValue::String("42".to_string())
    );
    assert_eq!(evaluate_expression("bool(1)", &context).expect("eval"), CelValue::Bool(true));
    assert_eq!(
        evaluate_expression("type([1,2])", &context).expect("eval"),
        CelValue::String("list".to_string())
    );
}

#[test]
fn builtin_collection_functions_work() {
    let context = CelContext::new();
    assert_eq!(evaluate_expression("exists([0, 1])", &context).expect("eval"), CelValue::Bool(true));
    assert_eq!(evaluate_expression("all([1, 1])", &context).expect("eval"), CelValue::Bool(true));
}

#[test]
fn builtin_ais_functions_work() {
    let context = CelContext::new();
    assert_eq!(
        evaluate_expression("to_atomic('1.5', 6)", &context).expect("eval"),
        CelValue::Integer(BigInt::from(1_500_000u64))
    );
    assert_eq!(
        evaluate_expression("to_human(1500000, 6)", &context).expect("eval"),
        CelValue::String("1.5".to_string())
    );
}

#[test]
fn numeric_string_can_compare_with_numeric_value() {
    let mut context = CelContext::new();
    context.insert("balance".to_string(), CelValue::String("20000000000000000000".to_string()));
    assert_eq!(
        evaluate_expression("balance > to_atomic('15', 18)", &context).expect("eval"),
        CelValue::Bool(true)
    );
}

#[test]
fn numeric_string_can_do_arithmetic_with_numeric_value() {
    let mut context = CelContext::new();
    context.insert("balance".to_string(), CelValue::String("20000000000000000000".to_string()));
    assert_eq!(
        evaluate_expression("balance - to_atomic('15', 18)", &context).expect("eval"),
        CelValue::Integer(BigInt::from(5_000_000_000_000_000_000u64))
    );
}

#[test]
fn numeric_string_compare_is_numeric_when_both_sides_are_numeric_strings() {
    let context = CelContext::new();
    assert_eq!(
        evaluate_expression("'10' > '2'", &context).expect("eval"),
        CelValue::Bool(true)
    );
}

#[test]
fn huge_integer_string_can_compare_without_decimal_overflow() {
    let mut context = CelContext::new();
    context.insert(
        "balance".to_string(),
        CelValue::String("999999999949999065895326171875".to_string()),
    );
    assert_eq!(
        evaluate_expression("balance > to_atomic('15', 18)", &context).expect("eval"),
        CelValue::Bool(true)
    );
}

#[test]
fn to_human_huge_integer_returns_error_instead_of_panic() {
    let context = CelContext::new();
    let result = evaluate_expression("to_human('999999999949999065895326171875', 18)", &context)
        .expect("eval");
    assert_eq!(
        result,
        CelValue::String("999999999949.999065895326171875".to_string())
    );
}
