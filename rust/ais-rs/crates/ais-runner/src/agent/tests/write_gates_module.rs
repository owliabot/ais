use super::*;
use crate::agent::{InputValueLayer, InputValueMeta, InputValueStability};
use serde_json::{json, Value};

fn candidate_context_with_demo_refs() -> CandidateContext {
    let mut context = CandidateContext::default();
    context.detail_by_ref.insert(
        "demo-bank@0.0.1/native-balance".to_string(),
        json!({
            "ref": "demo-bank@0.0.1/native-balance",
            "kind": "query"
        }),
    );
    context.detail_by_ref.insert(
        "demo-bank@0.0.1/native-transfer".to_string(),
        json!({
            "ref": "demo-bank@0.0.1/native-transfer",
            "kind": "action",
            "risk_tags": ["transfer"]
        }),
    );
    context
}

#[test]
fn write_gate_accepts_recursive_query_assert_branch_action_chain() {
    let segment: PlanSketchSegment = serde_json::from_value(json!({
            "segment_id": "seg-1",
            "cursor_in": "0",
            "cursor_out": "1",
            "done": false,
            "steps": [
                {"id":"q_balance","kind":"query","candidate_ref":"demo-bank@0.0.1/native-balance","inputs":{}},
                {"id":"g_assert","kind":"assert","depends_on":["q_balance"],"inputs":{"condition":{"cel":"nodes.q_balance.outputs.balance != null"}}},
                {"id":"g_branch","kind":"branch","depends_on":["g_assert"],"inputs":{"condition":{"cel":"nodes.q_balance.outputs.balance > 0"}}},
                {"id":"a_transfer","kind":"action","candidate_ref":"demo-bank@0.0.1/native-transfer","depends_on":["g_branch"],"inputs":{}}
            ],
            "extensions": {}
        }))
        .expect("segment");
    let context = candidate_context_with_demo_refs();

    let result = validate_segment_write_gates(&segment, &context, None, None);
    assert!(
        result.is_ok(),
        "recursive gate chain should pass: {result:?}"
    );
}

#[test]
fn write_gate_rejects_chain_without_data_backing() {
    let segment: PlanSketchSegment = serde_json::from_value(json!({
            "segment_id": "seg-1",
            "cursor_in": "0",
            "cursor_out": "1",
            "done": false,
            "steps": [
                {"id":"g_assert","kind":"assert","inputs":{"condition":{"cel":"true"}}},
                {"id":"g_branch","kind":"branch","depends_on":["g_assert"],"inputs":{"condition":{"cel":"true"}}},
                {"id":"a_transfer","kind":"action","candidate_ref":"demo-bank@0.0.1/native-transfer","depends_on":["g_branch"],"inputs":{}}
            ],
            "extensions": {}
        }))
        .expect("segment");
    let context = candidate_context_with_demo_refs();

    let error = validate_segment_write_gates(&segment, &context, None, None)
        .expect_err("missing data-backed gate chain must fail");
    assert_eq!(
        error.pointer("/reason_code").and_then(Value::as_str),
        Some("write_gate_missing")
    );
    assert!(error
        .pointer("/issues/0/reason_code")
        .and_then(Value::as_str)
        .is_some_and(|code| code == "missing_gate_data_backing"));
    assert!(error
        .pointer("/issues/0/family_reason_code")
        .and_then(Value::as_str)
        .is_some_and(|code| code == "missing_query_assert_branch_chain"));
    assert_eq!(
        error.pointer("/issues/0/gate_step_ids"),
        Some(&json!(["g_assert", "g_branch"]))
    );
    assert_eq!(
        error.pointer("/issues/0/missing_gate_step_ids"),
        Some(&json!(["g_assert", "g_branch"]))
    );
    assert_eq!(
        error.pointer("/issues/0/accepted_backing_modes"),
        Some(&json!(["same_segment_query", "historical_node_output"]))
    );
}

#[test]
fn write_gate_accepts_historical_node_output_backing_without_same_segment_query_dep() {
    let segment: PlanSketchSegment = serde_json::from_value(json!({
            "segment_id": "seg-1",
            "cursor_in": "0",
            "cursor_out": "1",
            "done": false,
            "steps": [
                {"id":"g_assert","kind":"assert","inputs":{"condition":{"cel":"nodes.prev_q_balance.outputs.balance > 0"}}},
                {"id":"g_branch","kind":"branch","depends_on":["g_assert"],"inputs":{"condition":{"cel":"nodes.prev_q_balance.outputs.balance > 10"}}},
                {"id":"a_transfer","kind":"action","candidate_ref":"demo-bank@0.0.1/native-transfer","depends_on":["g_branch"],"inputs":{}}
            ],
            "extensions": {}
        }))
        .expect("segment");
    let context = candidate_context_with_demo_refs();

    let result = validate_segment_write_gates(&segment, &context, None, None);
    assert!(
        result.is_ok(),
        "historical node outputs should count as gate backing without same-segment query deps: {result:?}"
    );
}

#[test]
fn write_gate_stale_diagnostic_uses_pack_threshold_and_reports_observation_age() {
    let segment: PlanSketchSegment = serde_json::from_value(json!({
            "segment_id": "seg-1",
            "cursor_in": "0",
            "cursor_out": "1",
            "done": false,
            "steps": [
                {"id":"g_assert","kind":"assert","inputs":{"condition":{"cel":"nodes.prev_quote.outputs.price > 0"}}},
                {"id":"a_transfer","kind":"action","candidate_ref":"demo-bank@0.0.1/native-transfer","depends_on":["g_assert"],"inputs":{}}
            ],
            "extensions": {}
        }))
        .expect("segment");
    let context = candidate_context_with_demo_refs();
    let mut input_store = InputStore::default();
    input_store.upsert(
        "inputs.native_balance",
        json!("100"),
        InputValueMeta {
            source: "query".to_string(),
            source_priority: 90,
            provenance: Some("segment_store.seg_old/q_balance.balance".to_string()),
            confidence: None,
            layer: InputValueLayer::Observed,
            stability: InputValueStability::Volatile,
            observed_at_ms: Some(0),
        },
    );

    let error = validate_segment_write_gates_with_policy(
        &segment,
        &context,
        None,
        Some(&input_store),
        crate::policy::VolatileFactsPolicy { max_age_ms: 1 },
    )
    .expect_err("stale volatile observation should fail");
    assert_eq!(
        error.pointer("/issues/0/reason_code"),
        Some(&json!("stale_volatile_fact"))
    );
    assert_eq!(error.pointer("/issues/0/max_age_ms"), Some(&json!(1)));
    assert_eq!(error.pointer("/issues/0/observed_at_ms"), Some(&json!(0)));
    assert!(error
        .pointer("/issues/0/age_ms")
        .and_then(Value::as_u64)
        .is_some_and(|age_ms| age_ms > 0));
}

#[test]
fn write_gate_rejects_action_without_gate_dependency() {
    let segment: PlanSketchSegment = serde_json::from_value(json!({
            "segment_id": "seg-1",
            "cursor_in": "0",
            "cursor_out": "1",
            "done": false,
            "steps": [
                {"id":"q_balance","kind":"query","candidate_ref":"demo-bank@0.0.1/native-balance","inputs":{}},
                {"id":"a_transfer","kind":"action","candidate_ref":"demo-bank@0.0.1/native-transfer","inputs":{}}
            ],
            "extensions": {}
        }))
        .expect("segment");
    let context = candidate_context_with_demo_refs();

    let error = validate_segment_write_gates(&segment, &context, None, None)
        .expect_err("missing gate dependency must fail");
    assert!(error
        .pointer("/issues/0/reason_code")
        .and_then(Value::as_str)
        .is_some_and(|code| code == "missing_action_gate_dep"));
    assert_eq!(
        error.pointer("/issues/0/family_reason_code"),
        Some(&json!("missing_query_assert_branch_chain"))
    );
    assert_eq!(
        error.pointer("/issues/0/action_depends_on"),
        Some(&json!([]))
    );
    assert_eq!(
        error.pointer("/issues/0/missing_depends_on"),
        Some(&json!(true))
    );
}

#[test]
fn write_gate_accepts_token_leaf_inputs_from_fact_store() {
    let segment: PlanSketchSegment = serde_json::from_value(json!({
            "segment_id": "seg-1",
            "cursor_in": "0",
            "cursor_out": "1",
            "done": false,
            "steps": [
                {"id":"q_balance","kind":"query","candidate_ref":"demo-bank@0.0.1/native-balance","inputs":{}},
                {"id":"g_assert","kind":"assert","depends_on":["q_balance"],"inputs":{"condition":{"cel":"nodes.q_balance.outputs.balance != null"}}},
                {
                    "id":"a_transfer",
                    "kind":"action",
                    "candidate_ref":"demo-bank@0.0.1/token-transfer",
                    "depends_on":["g_assert"],
                    "inputs":{"token":{"ref":"inputs.token.address"}}
                }
            ],
            "extensions": {}
        }))
        .expect("segment");
    let mut context = candidate_context_with_demo_refs();
    context.detail_by_ref.insert(
        "demo-bank@0.0.1/token-transfer".to_string(),
        json!({
            "ref":"demo-bank@0.0.1/token-transfer",
            "kind":"action",
            "risk_tags":["transfer"],
            "params":[{"name":"token","type":"asset"}]
        }),
    );
    let mut input_store = InputStore::default();
    input_store.upsert_user(
        "inputs.token.address",
        json!("0x2222222222222222222222222222222222222222"),
        "user.prompt.token.address",
    );
    input_store.upsert_user(
        "inputs.token.decimals",
        json!(6),
        "user.prompt.token.decimals",
    );

    let result = validate_segment_write_gates(&segment, &context, None, Some(&input_store));
    assert!(
        result.is_ok(),
        "token.address + token.decimals leaf refs should satisfy write gate: {result:?}"
    );
}

#[test]
fn write_gate_accepts_query_observed_input_store_decimals_without_runtime_facts() {
    let segment: PlanSketchSegment = serde_json::from_value(json!({
            "segment_id": "seg-1",
            "cursor_in": "0",
            "cursor_out": "1",
            "done": false,
            "steps": [
                {"id":"q_balance","kind":"query","candidate_ref":"demo-bank@0.0.1/native-balance","inputs":{}},
                {"id":"g_assert","kind":"assert","depends_on":["q_balance"],"inputs":{"condition":{"cel":"nodes.q_balance.outputs.balance != null"}}},
                {
                    "id":"a_transfer",
                    "kind":"action",
                    "candidate_ref":"demo-bank@0.0.1/token-transfer",
                    "depends_on":["g_assert"],
                    "inputs":{"token":{"ref":"inputs.token.address"}}
                }
            ],
            "extensions": {}
        }))
        .expect("segment");
    let mut context = candidate_context_with_demo_refs();
    context.detail_by_ref.insert(
        "demo-bank@0.0.1/token-transfer".to_string(),
        json!({
            "ref":"demo-bank@0.0.1/token-transfer",
            "kind":"action",
            "risk_tags":["transfer"],
            "params":[{"name":"token","type":"asset"}]
        }),
    );
    let mut input_store = InputStore::default();
    input_store.upsert(
        "token.decimals",
        json!(6),
        InputValueMeta {
            source: "query".to_string(),
            source_priority: 90,
            provenance: Some("query:erc20-decimals".to_string()),
            confidence: None,
            layer: InputValueLayer::Observed,
            stability: InputValueStability::Stable,
            observed_at_ms: Some(123),
        },
    );

    let result = validate_segment_write_gates(&segment, &context, None, Some(&input_store));
    assert!(
        result.is_ok(),
        "query-observed input store decimals should satisfy write gate without runtime facts: {result:?}"
    );
}
