use super::{evaluate_value_ref, evaluate_value_ref_async, ValueRef, ValueRefEvalOptions};
use crate::resolver::ResolverContext;
use futures::executor::block_on;
use serde_json::json;
use std::collections::BTreeMap;

#[test]
fn evaluate_lit_returns_value() {
    let context = ResolverContext::new();
    let value_ref = ValueRef::Lit { lit: json!(123) };
    let value = evaluate_value_ref(&value_ref, &context).expect("must evaluate");
    assert_eq!(value, json!(123));
}

#[test]
fn evaluate_ref_reads_from_context() {
    let context = ResolverContext::with_runtime(json!({"inputs": {"amount": "100"}}));
    let value_ref = ValueRef::Ref {
        ref_path: "inputs.amount".to_string(),
    };
    let value = evaluate_value_ref(&value_ref, &context).expect("must evaluate");
    assert_eq!(value, json!("100"));
}

#[test]
fn evaluate_object_and_array_walks_recursively() {
    let context = ResolverContext::with_runtime(json!({"ctx": {"chain": "eip155:1"}}));
    let value_ref = ValueRef::Object {
        object: BTreeMap::from([
            (
                "network".to_string(),
                ValueRef::Ref {
                    ref_path: "ctx.chain".to_string(),
                },
            ),
            (
                "list".to_string(),
                ValueRef::Array {
                    array: vec![
                        ValueRef::Lit { lit: json!(1) },
                        ValueRef::Lit { lit: json!(2) },
                    ],
                },
            ),
        ]),
    };

    let value = evaluate_value_ref(&value_ref, &context).expect("must evaluate");
    assert_eq!(value, json!({"network": "eip155:1", "list": [1, 2]}));
}

#[test]
fn evaluate_cel_runs_with_runtime_context() {
    let context = ResolverContext::with_runtime(json!({"inputs": {"amount": 10}}));
    let value_ref = ValueRef::Cel {
        cel: "inputs.amount > 0".to_string(),
    };
    let value = evaluate_value_ref(&value_ref, &context).expect("must evaluate");
    assert_eq!(value, json!(true));
}

#[test]
fn evaluate_ref_uses_root_override() {
    let context = ResolverContext::with_runtime(json!({"params": {"amount": "runtime"}}));
    let value_ref = ValueRef::Ref {
        ref_path: "params.amount".to_string(),
    };
    let options = ValueRefEvalOptions {
        root_overrides: BTreeMap::from([("params".to_string(), json!({"amount": "override"}))]),
    };

    let value = super::evaluate_value_ref_with_options(&value_ref, &context, &options)
        .expect("must evaluate");
    assert_eq!(value, json!("override"));
}

#[test]
fn evaluate_async_matches_sync_semantics() {
    let context = ResolverContext::with_runtime(json!({"inputs": {"amount": 10}}));
    let value_ref = ValueRef::Cel {
        cel: "inputs.amount > 0".to_string(),
    };
    let value = block_on(evaluate_value_ref_async(
        &value_ref,
        &context,
        &ValueRefEvalOptions::default(),
    ))
    .expect("must evaluate");
    assert_eq!(value, json!(true));
}
