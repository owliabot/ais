use super::*;
use ais_engine::{EngineEvent, EngineEventType};
use serde_json::json;

#[test]
fn classify_pause_reason_splits_missing_input_and_confirm() {
    assert_eq!(
        classify_pause_reason(Some("missing_required_input")),
        PauseReasonKind::MissingRequiredInput
    );
    assert_eq!(
        classify_pause_reason(Some("need_user_input:seg_1/q_owner")),
        PauseReasonKind::MissingRequiredInput
    );
    assert_eq!(
        classify_pause_reason(Some("need_user_confirm:seg_1/a1")),
        PauseReasonKind::NeedUserConfirm
    );
}

#[test]
fn resolve_execution_pause_backflow_keeps_need_user_confirm_terminal() {
    let mut state = EngineRunnerState {
        paused_reason: Some("need_user_confirm:seg_1/a1".to_string()),
        ..EngineRunnerState::default()
    };
    let mut fact_store = InputStore::default();

    let result = resolve_execution_pause_backflow(&mut state, &mut fact_store, &[], 0)
        .expect("pause resolution");
    match result {
        ResolvePauseBackflow::PauseTerminal { blocked_reason } => {
            assert_eq!(
                classify_pause_reason(state.paused_reason.as_deref()),
                PauseReasonKind::NeedUserConfirm
            );
            assert_eq!(blocked_reason, "need_user_confirm:seg_1/a1");
        }
        other => panic!("unexpected result: {other:?}"),
    }
}

#[test]
fn resolve_execution_pause_backflow_normalizes_missing_required_input_pause() {
    let mut state = EngineRunnerState {
        paused_reason: Some("need_user_input:seg_1/q_owner".to_string()),
        ..EngineRunnerState::default()
    };
    let mut fact_store = InputStore::default();
    let mut event = EngineEvent::new(EngineEventType::NeedUserInput);
    event.node_id = Some("seg_1/q_owner".to_string());
    event.data = serde_json::Map::from_iter([
        ("reason_code".to_string(), json!("missing_required_input")),
        (
            "details".to_string(),
            json!({
                "questions":[{"id":"owner","question":"Provide owner","required":true,"options":[]}]
            }),
        ),
    ]);
    let record = EngineEventRecord::new("run-1", 1, "1970-01-01T00:00:00Z", event);

    let result = resolve_execution_pause_backflow(
        &mut state,
        &mut fact_store,
        std::slice::from_ref(&record),
        1,
    )
    .expect("pause resolution");
    assert!(matches!(
        result,
        ResolvePauseBackflow::MissingRequiredInputPaused
    ));
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
fn missing_required_input_payload_from_pause_prefers_latest_event_payload() {
    let state = EngineRunnerState {
        paused_reason: Some("need_user_input:seg_1/q_owner".to_string()),
        runtime: json!({
            "agent": {
                "missing_required_input": {
                    "reason_code": "missing_required_input",
                    "message": "runtime-source",
                    "questions": [{"id":"owner","question":"owner?"}]
                }
            }
        }),
        ..EngineRunnerState::default()
    };
    let mut event = EngineEvent::new(EngineEventType::NeedUserInput);
    event.node_id = Some("seg_1/q_owner".to_string());
    event.data = serde_json::Map::from_iter([
        ("reason_code".to_string(), json!("missing_required_input")),
        ("reason".to_string(), json!("event-source")),
        (
            "details".to_string(),
            json!({
                "questions":[{"id":"event_owner","question":"event owner?"}]
            }),
        ),
    ]);
    let record = EngineEventRecord::new("run-1", 1, "1970-01-01T00:00:00Z", event);

    let payload =
        missing_required_input_payload_from_pause(&state, std::slice::from_ref(&record), 1)
            .expect("payload");
    assert_eq!(
        payload.get("message").and_then(Value::as_str),
        Some("event-source")
    );
    assert_eq!(
        payload.pointer("/questions/0/id"),
        Some(&json!("event_owner"))
    );
}

#[test]
fn missing_required_input_payload_from_pause_uses_runtime_payload_when_events_missing() {
    let state = EngineRunnerState {
        paused_reason: Some("missing_required_input".to_string()),
        runtime: json!({
            "agent": {
                "missing_required_input": {
                    "reason_code": "missing_required_input",
                    "message": "runtime-source",
                    "questions": [{"id":"owner","question":"owner?"}]
                }
            }
        }),
        ..EngineRunnerState::default()
    };
    let payload = missing_required_input_payload_from_pause(&state, &[], 1).expect("payload");
    assert_eq!(
        payload.get("message").and_then(Value::as_str),
        Some("runtime-source")
    );
    assert_eq!(payload.pointer("/questions/0/id"), Some(&json!("owner")));
}
