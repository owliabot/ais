use super::*;

#[test]
fn apply_intent_grounding_writes_inputs_namespace_only() {
    let mut state = EngineRunnerState::default();
    let mut fact_store = InputStore::default();
    let resolved_inputs = BTreeMap::from_iter([("owner".to_string(), json!("0xabc"))]);
    let confidence = BTreeMap::from_iter([("owner".to_string(), 90u8)]);

    let summary = apply_intent_grounding(
        &mut state,
        &mut fact_store,
        &resolved_inputs,
        &BTreeMap::new(),
        &confidence,
        "transfer",
    );

    assert!(summary.applied.iter().any(|item| item == "inputs.owner:90"));
    assert_eq!(
        fact_store
            .get("owner")
            .and_then(|entry| entry.value.as_str()),
        Some("0xabc")
    );
    assert_eq!(
        fact_store
            .get("inputs.owner")
            .and_then(|entry| entry.value.as_str()),
        Some("0xabc")
    );
    assert_eq!(
        state.runtime.pointer("/inputs/owner"),
        Some(&json!("0xabc"))
    );
}

#[test]
fn deterministic_balance_threshold_writes_inputs_namespace_only() {
    let mut state = EngineRunnerState::default();
    let mut fact_store = InputStore::default();

    let _ = apply_intent_grounding(
        &mut state,
        &mut fact_store,
        &BTreeMap::new(),
        &BTreeMap::new(),
        &BTreeMap::new(),
        "native_balance > 100",
    );

    assert_eq!(
        fact_store
            .get("balance_threshold")
            .and_then(|entry| entry.value.as_u64()),
        Some(100)
    );
    assert_eq!(
        fact_store
            .get("inputs.balance_threshold")
            .and_then(|entry| entry.value.as_u64()),
        Some(100)
    );
    assert_eq!(
        state.runtime.pointer("/inputs/balance_threshold"),
        Some(&json!(100))
    );
}
