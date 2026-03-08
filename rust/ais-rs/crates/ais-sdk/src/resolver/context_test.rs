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

#[test]
fn get_ref_unwraps_value_sentinel_from_leaf_subtree_collision() {
    let context = ResolverContext::with_runtime(json!({
        "inputs": {
            "owner": {
                "_value": "0x70997970c51812dc3a010c7d01b50e0d17dc79c8",
                "balance": {
                    "erc20": "999999"
                }
            }
        }
    }));
    let owner = context.get_ref("inputs.owner").expect("must resolve");
    assert_eq!(owner, json!("0x70997970c51812dc3a010c7d01b50e0d17dc79c8"));
    let balance = context
        .get_ref("inputs.owner.balance.erc20")
        .expect("must resolve");
    assert_eq!(balance, json!("999999"));
}

#[test]
fn get_ref_returns_plain_object_when_no_value_sentinel() {
    let context = ResolverContext::with_runtime(json!({
        "inputs": {
            "owner": {
                "balance": { "native": "100" }
            }
        }
    }));
    let owner = context.get_ref("inputs.owner").expect("must resolve");
    assert_eq!(owner, json!({"balance": {"native": "100"}}));
}

#[test]
fn get_ref_value_sentinel_with_address_bridge_still_works() {
    let context = ResolverContext::with_runtime(json!({
        "inputs": {
            "token": {
                "_value": "0x8464135c8F25Da09e49BC8782676a84730C318bC",
                "decimals": "18"
            }
        }
    }));
    let token = context.get_ref("inputs.token").expect("must resolve");
    assert_eq!(token, json!("0x8464135c8F25Da09e49BC8782676a84730C318bC"));
    let token_address = context
        .get_ref("inputs.token.address")
        .expect("address bridge");
    assert_eq!(
        token_address,
        json!("0x8464135c8F25Da09e49BC8782676a84730C318bC")
    );
    let decimals = context
        .get_ref("inputs.token.decimals")
        .expect("must resolve");
    assert_eq!(decimals, json!("18"));
}
