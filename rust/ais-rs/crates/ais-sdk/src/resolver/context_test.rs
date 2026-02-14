use super::{ResolverContext, ResolverError};
use serde_json::json;

#[test]
fn set_and_get_ref_roundtrip() {
    let mut context = ResolverContext::new();
    context
        .set_ref("inputs.amount", json!("100"))
        .expect("set must work");

    let value = context.get_ref("inputs.amount").expect("get must work");
    assert_eq!(value, json!("100"));
}

#[test]
fn get_ref_supports_array_indexes() {
    let context = ResolverContext::with_runtime(json!({
        "nodes": [{"outputs": {"value": 1}}]
    }));

    let value = context
        .get_ref("nodes[0].outputs.value")
        .expect("get must work");
    assert_eq!(value, json!(1));
}

#[test]
fn set_ref_rejects_index_path() {
    let mut context = ResolverContext::new();
    let error = context
        .set_ref("nodes[0].outputs", json!(1))
        .expect_err("must reject");
    assert_eq!(error, ResolverError::InvalidPath("nodes[0].outputs".to_string()));
}
