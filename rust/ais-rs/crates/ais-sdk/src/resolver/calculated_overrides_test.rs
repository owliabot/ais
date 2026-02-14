use super::{calculated_override_order_from_map, CalculatedOverrideError};
use serde_json::json;

#[test]
fn override_order_supports_chained_dependencies() {
    let overrides = json!({
        "amount_out_min": { "expr": { "lit": "100" } },
        "slippage_bps": { "expr": { "lit": 50 } },
        "amount_out_limit": { "expr": { "cel": "int(calculated.amount_out_min) - calculated.slippage_bps" } }
    });
    let order = calculated_override_order_from_map(overrides.as_object().expect("object"))
        .expect("must order");
    assert_eq!(order, vec!["amount_out_min", "slippage_bps", "amount_out_limit"]);
}

#[test]
fn override_order_reports_missing_dependency() {
    let overrides = json!({
        "amount_out_limit": { "expr": { "ref": "calculated.missing_field" } }
    });
    let errors = calculated_override_order_from_map(overrides.as_object().expect("object"))
        .expect_err("must fail");
    assert!(errors.iter().any(|error| {
        matches!(
            error,
            CalculatedOverrideError::MissingDependency { key, dependency }
            if key == "amount_out_limit" && dependency == "missing_field"
        )
    }));
}

#[test]
fn override_order_reports_cycle() {
    let overrides = json!({
        "a": { "expr": { "ref": "calculated.b" } },
        "b": { "expr": { "ref": "calculated.a" } }
    });
    let errors = calculated_override_order_from_map(overrides.as_object().expect("object"))
        .expect_err("must fail");
    assert!(errors
        .iter()
        .any(|error| matches!(error, CalculatedOverrideError::DependencyCycle { .. })));
}
