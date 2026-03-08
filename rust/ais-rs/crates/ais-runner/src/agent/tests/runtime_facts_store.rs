use super::super::runtime_facts_store::{normalize_runtime_fact_key, RuntimeFactsStore};
use crate::agent::input_store::{InputValueLayer, InputValueMeta, InputValueStability};
use serde_json::json;

#[test]
fn runtime_facts_store_normalizes_fact_refs_only() {
    assert_eq!(normalize_runtime_fact_key("token.decimals"), None);
    assert_eq!(normalize_runtime_fact_key("inputs.token.decimals"), None);
    assert_eq!(
        normalize_runtime_fact_key("facts.balance"),
        Some("facts.balance".to_string())
    );
    assert_eq!(
        normalize_runtime_fact_key("fact:quote.price"),
        Some("facts.quote.price".to_string())
    );
}

#[test]
fn runtime_facts_store_keeps_query_derived_values_outside_input_store_namespace_rules() {
    let mut store = RuntimeFactsStore::default();
    let result = store.upsert(
        "facts.balance",
        json!("100"),
        InputValueMeta {
            source: "query".to_string(),
            source_priority: 90,
            provenance: Some("segment_store.seg_1/q_balance.balance".to_string()),
            confidence: None,
            layer: InputValueLayer::Observed,
            stability: InputValueStability::Volatile,
            observed_at_ms: Some(42),
        },
    );
    assert!(matches!(
        result,
        crate::agent::input_store::InputStoreUpsertResult::Inserted
    ));
    assert_eq!(
        store.get("facts.balance").map(|entry| entry.value.clone()),
        Some(json!("100"))
    );
}

#[test]
fn runtime_facts_store_refreshes_equal_priority_newer_observation() {
    let mut store = RuntimeFactsStore::default();
    assert!(matches!(
        store.upsert(
            "facts.native_balance",
            json!("100"),
            InputValueMeta {
                source: "query".to_string(),
                source_priority: 90,
                provenance: Some("segment_store.seg_1/q_balance.balance".to_string()),
                confidence: None,
                layer: InputValueLayer::Observed,
                stability: InputValueStability::Volatile,
                observed_at_ms: Some(42),
            },
        ),
        crate::agent::input_store::InputStoreUpsertResult::Inserted
    ));
    assert!(matches!(
        store.upsert(
            "facts.native_balance",
            json!("100"),
            InputValueMeta {
                source: "query".to_string(),
                source_priority: 90,
                provenance: Some("segment_store.seg_2/q_balance.balance".to_string()),
                confidence: None,
                layer: InputValueLayer::Observed,
                stability: InputValueStability::Volatile,
                observed_at_ms: Some(84),
            },
        ),
        crate::agent::input_store::InputStoreUpsertResult::Replaced
    ));
    let entry = store
        .get("facts.native_balance")
        .expect("refreshed runtime fact");
    assert_eq!(entry.meta.observed_at_ms, Some(84));
    assert_eq!(
        entry.meta.provenance.as_deref(),
        Some("segment_store.seg_2/q_balance.balance")
    );
}

#[test]
fn runtime_facts_store_ignores_equal_priority_older_observation() {
    let mut store = RuntimeFactsStore::default();
    let _ = store.upsert(
        "facts.allowance",
        json!("15"),
        InputValueMeta {
            source: "query".to_string(),
            source_priority: 90,
            provenance: Some("segment_store.seg_2/q_allowance.allowance".to_string()),
            confidence: None,
            layer: InputValueLayer::Observed,
            stability: InputValueStability::Volatile,
            observed_at_ms: Some(84),
        },
    );

    assert!(matches!(
        store.upsert(
            "facts.allowance",
            json!("12"),
            InputValueMeta {
                source: "query".to_string(),
                source_priority: 90,
                provenance: Some("segment_store.seg_1/q_allowance.allowance".to_string()),
                confidence: None,
                layer: InputValueLayer::Observed,
                stability: InputValueStability::Volatile,
                observed_at_ms: Some(42),
            },
        ),
        crate::agent::input_store::InputStoreUpsertResult::Ignored
    ));
    let entry = store
        .get("facts.allowance")
        .expect("existing fact preserved");
    assert_eq!(entry.value, json!("15"));
    assert_eq!(entry.meta.observed_at_ms, Some(84));
}

#[test]
fn runtime_facts_store_stamps_query_balance_metadata_when_missing() {
    let mut store = RuntimeFactsStore::default();
    let result = store.upsert(
        "facts.native_balance",
        json!("100"),
        InputValueMeta {
            source: "query".to_string(),
            source_priority: 90,
            provenance: Some("segment_store.seg_1/q_balance.balance".to_string()),
            confidence: None,
            layer: InputValueLayer::Observed,
            stability: InputValueStability::Unknown,
            observed_at_ms: None,
        },
    );

    assert!(matches!(
        result,
        crate::agent::input_store::InputStoreUpsertResult::Inserted
    ));
    let entry = store
        .get("facts.native_balance")
        .expect("query-derived runtime fact");
    assert_eq!(entry.meta.stability, InputValueStability::Volatile);
    assert!(entry.meta.observed_at_ms.is_some());
}

#[test]
fn runtime_facts_store_invalidate_volatile_signals_clears_only_matching_query_observations() {
    let mut store = RuntimeFactsStore::default();
    let _ = store.upsert(
        "facts.native_balance",
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
    let _ = store.upsert(
        "facts.allowance",
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

    let invalidated =
        store.invalidate_volatile_signals(&[super::super::VolatileInputSignal::Balance]);
    assert_eq!(invalidated, vec!["facts.native_balance".to_string()]);
    assert_eq!(
        store
            .get("facts.native_balance")
            .and_then(|entry| entry.meta.observed_at_ms),
        None
    );
    assert_eq!(
        store
            .get("facts.allowance")
            .and_then(|entry| entry.meta.observed_at_ms),
        Some(456)
    );
}
