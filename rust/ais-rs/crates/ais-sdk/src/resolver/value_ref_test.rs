use super::{
    evaluate_value_ref, evaluate_value_ref_async, DetectResolver, DetectSpec, ValueRef,
    ValueRefEvalError, ValueRefEvalOptions,
};
use crate::resolver::ResolverContext;
use futures::executor::block_on;
use futures::FutureExt;
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
            ("network".to_string(), ValueRef::Ref { ref_path: "ctx.chain".to_string() }),
            (
                "list".to_string(),
                ValueRef::Array {
                    array: vec![ValueRef::Lit { lit: json!(1) }, ValueRef::Lit { lit: json!(2) }],
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

struct StaticDetectResolver;

impl DetectResolver for StaticDetectResolver {
    fn resolve<'a>(
        &'a self,
        detect: &'a DetectSpec,
        _context: &'a ResolverContext,
        _options: &'a ValueRefEvalOptions,
    ) -> futures::future::LocalBoxFuture<'a, Result<serde_json::Value, ValueRefEvalError>> {
        async move { Ok(json!({"kind": detect.kind, "provider": detect.provider})) }.boxed_local()
    }
}

#[test]
fn evaluate_detect_async_calls_resolver() {
    let context = ResolverContext::new();
    let value_ref = ValueRef::Detect {
        detect: DetectSpec {
            kind: "choose_one".to_string(),
            provider: Some("mock".to_string()),
            candidates: vec![],
            constraints: BTreeMap::new(),
        },
    };
    let options = ValueRefEvalOptions::default();
    let resolver = StaticDetectResolver;

    let value = block_on(evaluate_value_ref_async(
        &value_ref,
        &context,
        &options,
        Some(&resolver),
    ))
    .expect("must evaluate");
    assert_eq!(value, json!({"kind": "choose_one", "provider": "mock"}));
}

#[test]
fn evaluate_detect_async_without_resolver_fails() {
    let context = ResolverContext::new();
    let value_ref = ValueRef::Detect {
        detect: DetectSpec {
            kind: "choose_one".to_string(),
            provider: None,
            candidates: vec![],
            constraints: BTreeMap::new(),
        },
    };

    let err = block_on(evaluate_value_ref_async(
        &value_ref,
        &context,
        &ValueRefEvalOptions::default(),
        None,
    ))
    .expect_err("must fail");
    assert!(matches!(err, ValueRefEvalError::NeedDetect { .. }));
}
