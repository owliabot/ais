use super::validate_schema_instance;
use crate::versions::SCHEMA_PLAN_0_0_3;
use serde_json::json;

#[test]
fn unknown_schema_returns_error_issue() {
    let issues = validate_schema_instance("ais-unknown/0.0.1", &json!({}));
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].kind, "schema_error");
}

#[test]
fn valid_plan_schema_passes_validation() {
    let plan = json!({
        "schema": "ais-plan/0.0.3",
        "nodes": []
    });
    let issues = validate_schema_instance(SCHEMA_PLAN_0_0_3, &plan);
    assert!(issues.is_empty());
}

#[test]
fn invalid_plan_schema_returns_error_issue() {
    let plan = json!({
        "schema": "ais-plan/0.0.3"
    });
    let issues = validate_schema_instance(SCHEMA_PLAN_0_0_3, &plan);
    assert!(!issues.is_empty());
}
