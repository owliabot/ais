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
        Some(&json!("inputs.event_owner"))
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
    assert_eq!(payload.pointer("/questions/0/id"), Some(&json!("inputs.owner")));
}

#[test]
fn resolve_missing_required_input_payload_normalizes_params_refs_before_pause() {
    let mut state = EngineRunnerState::default();
    let mut fact_store = InputStore::default();
    let payload = json!({
        "reason_code":"missing_required_input",
        "questions":[
            {"id":"params.token.address","question":"Provide params token address","required":true,"options":[]}
        ],
        "missing_refs":["params.token.address"],
        "suggested_paths":["params.token.address"]
    });

    let result = resolve_missing_required_input_payload(
        &mut state,
        &mut fact_store,
        &payload,
        true,
    )
    .expect("pause resolution");
    assert!(matches!(result, MissingRequiredInputBackflow::Paused));
    assert_eq!(state.paused_reason.as_deref(), Some("missing_required_input"));
    assert_eq!(
        state.runtime.pointer("/agent/missing_required_input/missing_refs"),
        Some(&json!(["inputs.token.address"]))
    );
    assert!(
        state
            .runtime
            .pointer("/agent/missing_required_input/questions/0/id")
            .and_then(Value::as_str)
            .is_some_and(|id| !id.starts_with("params."))
    );
}

#[test]
fn attach_missing_input_recovery_adds_recovery_exhaustion_evidence() {
    let payload = json!({
        "reason_code": "missing_required_input",
        "questions": [{"id":"inputs.owner","question":"owner?"}]
    });
    let attached = attach_missing_input_recovery(
        &payload,
        "need_user_input",
        "no_query_candidates",
        "missing_ref_recovery",
        "grounding",
        "todo_1",
        &["inputs.owner".to_string()],
    );
    assert_eq!(
        attached.pointer("/recovery_exhaustion/unresolved_refs/0"),
        Some(&json!("inputs.owner"))
    );
    assert_eq!(
        attached.pointer("/recovery_exhaustion/reasons/0"),
        Some(&json!("no_query_candidates"))
    );
    assert_eq!(
        attached
            .pointer("/recovery_exhaustion/attempt_trace_id")
            .and_then(Value::as_str),
        Some("missing_ref_recovery:grounding:todo_1:need_user_input")
    );
}

#[test]
fn can_prompt_user_missing_input_requires_recovery_exhaustion_evidence() {
    let with_evidence = json!({
        "recovery": {
            "status": "need_user_input",
            "reason": "no_query_candidates",
            "missing_refs": ["inputs.owner"]
        },
        "recovery_exhaustion": {
            "unresolved_refs": ["inputs.owner"],
            "reasons": ["no_query_candidates"],
            "attempt_trace_id": "missing_ref_recovery:grounding:todo_1:need_user_input"
        },
        "questions": [{"id":"inputs.owner","question":"owner?"}]
    });
    assert!(can_prompt_user_missing_input(&with_evidence));

    let missing_evidence = json!({
        "recovery": {
            "status": "need_user_input",
            "reason": "no_query_candidates",
            "missing_refs": ["inputs.owner"]
        },
        "questions": [{"id":"inputs.owner","question":"owner?"}]
    });
    assert!(!can_prompt_user_missing_input(&missing_evidence));
}
