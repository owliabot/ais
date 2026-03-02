use super::*;
use serde_json::json;

#[test]
fn pause_with_payload_sets_reason_and_runtime_payload() {
    let payload = payload(
        Some("need_owner"),
        &[json!({"id":"owner","question":"owner?"})],
        &[],
        1,
    );
    let mut state = EngineRunnerState::default();
    pause_with_payload(&mut state, &payload);
    assert_eq!(
        state.paused_reason.as_deref(),
        Some("missing_required_input")
    );
    assert_eq!(
        state
            .runtime
            .pointer("/agent/missing_required_input/reason_code"),
        Some(&json!("missing_required_input"))
    );
}

#[test]
fn payload_from_pause_maps_need_user_input_event() {
    let state = EngineRunnerState {
        paused_reason: Some("need_user_input:seg_1/q_owner".to_string()),
        ..EngineRunnerState::default()
    };
    let mut event = ais_engine::EngineEvent::new(EngineEventType::NeedUserInput);
    event.node_id = Some("seg_1/q_owner".to_string());
    event.data = serde_json::Map::from_iter([
        ("reason_code".to_string(), json!("missing_required_input")),
        (
            "reason".to_string(),
            json!("missing_inputs_or_runtime_refs"),
        ),
        (
            "details".to_string(),
            json!({
                "missing_refs":["inputs.owner","params.owner"],
                "suggested_paths":["inputs.owner","params.owner"],
                "questions":[{"id":"owner","question":"Provide owner","required":true,"options":[]}],
                "issues":[{"reason_code":"missing_required_input"}]
            }),
        ),
    ]);
    let record = EngineEventRecord::new("run-1", 4, "1970-01-01T00:00:00Z", event);

    let payload =
        payload_from_pause(&state, std::slice::from_ref(&record), 2).expect("missing payload");
    assert_eq!(
        payload.get("reason_code").and_then(Value::as_str),
        Some("missing_required_input")
    );
    assert_eq!(
        payload.pointer("/missing_refs/0"),
        Some(&json!("inputs.owner"))
    );
    assert_eq!(
        payload.pointer("/suggested_paths/0"),
        Some(&json!("inputs.owner"))
    );
    assert_eq!(payload.pointer("/questions/0/id"), Some(&json!("owner")));
}

#[test]
fn apply_answers_writes_inputs_namespace_only() {
    let mut state = EngineRunnerState::default();
    let mut fact_store = InputStore::default();
    let answers = Map::from_iter([("owner".to_string(), json!("0xabc"))]);

    apply_answers(&mut state, &mut fact_store, &answers);

    assert_eq!(
        state.runtime.pointer("/inputs/owner"),
        Some(&json!("0xabc"))
    );
    assert!(fact_store.get("owner").is_none());
    assert_eq!(
        fact_store
            .get("inputs.owner")
            .and_then(|entry| entry.value.as_str()),
        Some("0xabc")
    );
    assert_eq!(
        fact_store
            .get("wallet.default")
            .and_then(|entry| entry.value.as_str()),
        Some("0xabc")
    );
}
