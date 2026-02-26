use super::validate_schema_instance;
use crate::versions::{
    SCHEMA_PLAN_0_0_3, SCHEMA_PLAN_SKETCH_0_1_0, SCHEMA_SIDE_EFFECT_RECORD_0_1_0,
};
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

#[test]
fn valid_plan_sketch_schema_passes_validation() {
    let sketch = json!({
        "schema": "ais-plan-sketch/0.1.0",
        "intent": "check and transfer",
        "pack_snapshot": { "hash": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" },
        "catalog_snapshot": {
          "schema": "ais-catalog/0.0.1",
          "hash": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
        },
        "segments": [{
          "segment_id": "s1",
          "cursor_in": "c0",
          "cursor_out": "c1",
          "done": false,
          "steps": [{
            "id": "step1",
            "kind": "query",
            "candidate_ref": "erc20@0.0.2/balance-of",
            "inputs": {}
          }]
        }]
    });
    let issues = validate_schema_instance(SCHEMA_PLAN_SKETCH_0_1_0, &sketch);
    assert!(issues.is_empty());
}

#[test]
fn valid_side_effect_record_schema_passes_validation() {
    let record = json!({
        "schema":"ais-side-effect-record/0.1.0",
        "effect_type":"tx",
        "idempotency_key":"tx:swap-1:0xabc",
        "node_id":"swap-1",
        "chain":"eip155:1",
        "execution_type":"evm_call",
        "status":"sent",
        "observed_at":"2026-02-24T00:00:00Z",
        "tx_hash":"0xabc"
    });
    let issues = validate_schema_instance(SCHEMA_SIDE_EFFECT_RECORD_0_1_0, &record);
    assert!(issues.is_empty());
}
