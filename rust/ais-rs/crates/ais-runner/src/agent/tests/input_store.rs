use super::*;
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
}
