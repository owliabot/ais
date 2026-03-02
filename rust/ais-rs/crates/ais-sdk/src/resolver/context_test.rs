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
    assert_eq!(
        error,
        ResolverError::InvalidPath("nodes[0].outputs".to_string())
    );
}

#[test]
fn get_ref_address_suffix_accepts_string_asset_slot() {
    let context = ResolverContext::with_runtime(json!({
        "inputs": {
            "erc20_token": "0x8464135c8F25Da09e49BC8782676a84730C318bC"
        },
        "params": {
            "token": "0x8464135c8F25Da09e49BC8782676a84730C318bC"
        }
    }));

    let input_token_address = context
        .get_ref("inputs.erc20_token.address")
        .expect("string token should satisfy .address read");
    assert_eq!(
        input_token_address,
        json!("0x8464135c8F25Da09e49BC8782676a84730C318bC")
    );

    let param_token_address = context
        .get_ref("params.token.address")
        .expect("string token should satisfy .address read");
    assert_eq!(
        param_token_address,
        json!("0x8464135c8F25Da09e49BC8782676a84730C318bC")
    );
}

#[test]
fn get_ref_non_address_suffix_still_rejects_string_slot() {
    let context = ResolverContext::with_runtime(json!({
        "params": {"token": "0xabc"}
    }));

    let error = context
        .get_ref("params.token.chain_id")
        .expect_err("non-address suffix on string should stay invalid");
    assert_eq!(
        error,
        ResolverError::NotFound("params.token.chain_id".to_string())
    );
}
