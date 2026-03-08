use super::*;
use ais_sdk::{evaluate_value_ref, ResolverContext, ValueRef};
use serde_json::json;

#[test]
fn input_ref_canonicalizes_known_prefixes() {
    assert_eq!(InputRef::new("inputs.owner").unwrap().as_str(), "owner");
    assert_eq!(
        InputRef::new("runtime.inputs.token.address")
            .unwrap()
            .as_str(),
        "token.address"
    );
    assert_eq!(
        InputRef::new(" owner.wallet ").unwrap().as_str(),
        "owner.wallet"
    );
    assert!(InputRef::new("nodes.balance").is_none());
}

#[test]
fn upsert_applies_meta_priority_and_runtime_projection() {
    let mut store = InputStore::default();

    assert_eq!(
        store.upsert_user("inputs.owner", json!("0xfirst"), "prompt"),
        InputStoreUpsertResult::Inserted
    );
    assert_eq!(store.get("owner").unwrap().value, json!("0xfirst"));

    assert_eq!(
        store.upsert_seed("owner", json!("0xseed"), "runtime"),
        InputStoreUpsertResult::Ignored
    );
    assert_eq!(store.get("owner").unwrap().value, json!("0xfirst"));

    assert_eq!(
        store.upsert(
            "owner",
            json!("0xquery"),
            InputValueMeta {
                source: "query".to_string(),
                source_priority: 200,
                provenance: Some("query.intend".to_string()),
                confidence: Some(0.8),
                layer: InputValueLayer::Observed,
                stability: InputValueStability::Volatile,
                observed_at_ms: Some(123),
            }
        ),
        InputStoreUpsertResult::Replaced
    );
    assert_eq!(
        store
            .get("owner")
            .and_then(|entry| entry.meta.observed_at_ms),
        Some(123)
    );

    let projection = store.to_runtime_projection();
    assert_eq!(projection.pointer("/inputs/owner"), Some(&json!("0xquery")));
}

#[test]
fn upsert_rejects_non_input_scoped_keys() {
    let mut store = InputStore::default();
    assert_eq!(
        store.upsert("facts.owner", json!(1), InputValueMeta::default()),
        InputStoreUpsertResult::Rejected
    );
    assert!(!store.has("facts.owner"));
}

#[test]
fn to_runtime_projection_builds_nested_slots() {
    let mut store = InputStore::default();
    store.upsert(
        "inputs.token.address",
        json!("0xabc"),
        InputValueMeta::default(),
    );
    store.upsert(
        "inputs.token.decimals",
        json!(18),
        InputValueMeta::default(),
    );

    let runtime = store.to_runtime_projection();
    assert_eq!(
        runtime.pointer("/inputs/token/address"),
        Some(&json!("0xabc"))
    );
    assert_eq!(runtime.pointer("/inputs/token/decimals"), Some(&json!(18)));
}

#[test]
fn list_refs_reflects_sorted_keys() {
    let mut store = InputStore::default();
    store.upsert("inputs.token.decimals", json!(6), InputValueMeta::default());
    store.upsert("inputs.owner", json!("0xowner"), InputValueMeta::default());

    assert_eq!(
        store.list_ref_strings(),
        vec!["owner".to_string(), "token.decimals".to_string()]
    );
    assert_eq!(
        store.list_projected_ref_strings(),
        vec![
            "owner".to_string(),
            "token".to_string(),
            "token.decimals".to_string()
        ]
    );
}

#[test]
fn asset_object_upsert_normalizes_into_leaf_semantics_and_projected_root() {
    let mut store = InputStore::default();
    store.upsert(
        "inputs.token",
        json!({
            "address": "0xabc",
            "decimals": "6"
        }),
        InputValueMeta {
            source: "query".to_string(),
            source_priority: 90,
            ..Default::default()
        },
    );

    assert!(store.get_semantic("token").is_none());
    assert_eq!(
        store
            .get_semantic("token.address")
            .and_then(|entry| entry.value.as_str()),
        Some("0xabc")
    );
    let decimals = store
        .get_semantic("token.decimals")
        .expect("token.decimals semantic leaf");
    assert_eq!(decimals.value.as_i64(), Some(6));
    assert_eq!(decimals.meta.stability, InputValueStability::Stable);

    let projected = store
        .get_projected("inputs.token")
        .expect("projected token root");
    assert_eq!(projected.value.pointer("/address"), Some(&json!("0xabc")));
    assert_eq!(projected.value.pointer("/decimals"), Some(&json!(6)));
    assert_eq!(projected.meta.layer, InputValueLayer::Derived);
    assert_eq!(projected.meta.source, "derived");
}

#[test]
fn integer_like_input_slots_normalize_without_coercing_decimal_text() {
    let mut store = InputStore::default();
    store.upsert(
        "inputs.native.decimals",
        json!("18"),
        InputValueMeta::default(),
    );
    store.upsert(
        "inputs.slippage_bps",
        json!("75"),
        InputValueMeta::default(),
    );
    store.upsert("inputs.retry_limit", json!("3"), InputValueMeta::default());
    store.upsert(
        "inputs.price_limit",
        json!("1.01"),
        InputValueMeta::default(),
    );

    assert_eq!(
        store
            .get_semantic("native.decimals")
            .and_then(|entry| entry.value.as_i64()),
        Some(18)
    );
    assert_eq!(
        store
            .get_semantic("slippage_bps")
            .and_then(|entry| entry.value.as_i64()),
        Some(75)
    );
    assert_eq!(
        store
            .get_semantic("retry_limit")
            .and_then(|entry| entry.value.as_i64()),
        Some(3)
    );
    assert_eq!(
        store
            .get_semantic("price_limit")
            .and_then(|entry| entry.value.as_str()),
        Some("1.01")
    );
}

#[test]
fn runtime_projection_exposes_integer_typed_decimals_to_cel() {
    let mut store = InputStore::default();
    store.upsert("inputs.amount", json!("1"), InputValueMeta::default());
    store.upsert(
        "inputs.native.decimals",
        json!("18"),
        InputValueMeta::default(),
    );
    store.upsert(
        "inputs.token.decimals",
        json!("6"),
        InputValueMeta::default(),
    );

    let runtime = store.to_runtime_projection();
    let context = ResolverContext::with_runtime(runtime);

    let native_atomic = evaluate_value_ref(
        &ValueRef::Cel {
            cel: "to_atomic(inputs.amount, inputs.native.decimals)".to_string(),
        },
        &context,
    )
    .expect("native decimals should be integer-typed");
    assert_eq!(native_atomic, json!(1_000_000_000_000_000_000u64));

    let token_atomic = evaluate_value_ref(
        &ValueRef::Cel {
            cel: "to_atomic(inputs.amount, inputs.token.decimals)".to_string(),
        },
        &context,
    )
    .expect("token decimals should be integer-typed");
    assert_eq!(token_atomic, json!(1_000_000u64));
}

#[test]
fn query_balance_upsert_stamps_volatile_observation_metadata() {
    let mut store = InputStore::default();
    store.upsert(
        "inputs.native_balance",
        json!("100"),
        InputValueMeta {
            source: "query.auto_project".to_string(),
            source_priority: 80,
            provenance: Some("segment_store.seg_prev.q_native_balance.balance".to_string()),
            confidence: None,
            layer: InputValueLayer::Observed,
            stability: InputValueStability::Unknown,
            observed_at_ms: None,
        },
    );

    let entry = store
        .get_semantic("native_balance")
        .expect("query-derived balance should be stored");
    assert_eq!(entry.meta.stability, InputValueStability::Volatile);
    assert!(entry.meta.observed_at_ms.is_some());
}

#[test]
fn invalidate_volatile_signals_clears_query_observation_freshness_only_for_matching_signals() {
    let mut store = InputStore::default();
    store.upsert(
        "inputs.native_balance",
        json!("100"),
        InputValueMeta {
            source: "query".to_string(),
            source_priority: 80,
            provenance: Some("segment_store.seg_1/q_balance.balance".to_string()),
            confidence: None,
            layer: InputValueLayer::Observed,
            stability: InputValueStability::Volatile,
            observed_at_ms: Some(123),
        },
    );
    store.upsert(
        "inputs.owner",
        json!("0xowner"),
        InputValueMeta {
            source: "user".to_string(),
            source_priority: 100,
            provenance: Some("prompt".to_string()),
            confidence: None,
            layer: InputValueLayer::Observed,
            stability: InputValueStability::Unknown,
            observed_at_ms: None,
        },
    );
    store.upsert(
        "inputs.allowance",
        json!("15"),
        InputValueMeta {
            source: "query".to_string(),
            source_priority: 80,
            provenance: Some("segment_store.seg_1/q_allowance.allowance".to_string()),
            confidence: None,
            layer: InputValueLayer::Observed,
            stability: InputValueStability::Volatile,
            observed_at_ms: Some(456),
        },
    );

    let invalidated = store.invalidate_volatile_signals(&[VolatileInputSignal::Balance]);
    assert_eq!(invalidated, vec!["native_balance".to_string()]);
    assert_eq!(
        store
            .get_semantic("native_balance")
            .and_then(|entry| entry.meta.observed_at_ms),
        None
    );
    assert_eq!(
        store
            .get_semantic("allowance")
            .and_then(|entry| entry.meta.observed_at_ms),
        Some(456)
    );
    assert_eq!(
        store
            .get_semantic("owner")
            .and_then(|entry| entry.meta.observed_at_ms),
        None
    );
}
