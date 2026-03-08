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
    assert_eq!(
        payload.pointer("/questions/0/id"),
        Some(&json!("inputs.owner"))
    );
}

#[test]
fn payload_with_error_details_surfaces_decisions_and_recovery_exhaustion() {
    let payload = payload_with_error_details(
        Some("need owner"),
        &[json!({"id":"inputs.owner","question":"owner?"})],
        &[json!({"reason_code":"missing_required_input"})],
        Some(&json!({
            "decisions": [
                {
                    "kind":"run_producer",
                    "target":"inputs.owner",
                    "query_ref":"wallet@0.0.1/defaultOwner"
                }
            ],
            "recovery_exhaustion": {
                "unresolved_refs": ["inputs.owner"],
                "reasons": ["no_query_candidates"],
                "attempt_trace_id": "missing_resolution:todo:todo_1:user_input:1"
            }
        })),
        1,
    );
    assert_eq!(
        payload.pointer("/decisions/0/query_ref"),
        Some(&json!("wallet@0.0.1/defaultOwner"))
    );
    assert_eq!(
        payload.pointer("/recovery_exhaustion/unresolved_refs/0"),
        Some(&json!("inputs.owner"))
    );
    assert_eq!(
        payload.pointer("/error_details/decisions/0/query_ref"),
        Some(&json!("wallet@0.0.1/defaultOwner"))
    );
}

#[test]
fn payload_from_pause_rewrites_params_token_address_to_inputs_source_ref() {
    let state = EngineRunnerState {
        paused_reason: Some("need_user_input:seg_1/q_erc20_balance".to_string()),
        ..EngineRunnerState::default()
    };
    let mut event = ais_engine::EngineEvent::new(EngineEventType::NeedUserInput);
    event.node_id = Some("seg_1/q_erc20_balance".to_string());
    event.data = serde_json::Map::from_iter([
        ("reason_code".to_string(), json!("missing_required_input")),
        (
            "reason".to_string(),
            json!("missing_inputs_or_runtime_refs"),
        ),
        (
            "details".to_string(),
            json!({
                "missing_refs":["inputs.tst_token_address","params.token.address"],
                "suggested_paths":["inputs.tst_token_address","params.token.address"],
                "questions":[
                    {"id":"token.address","question":"Provide token address","required":true,"options":[]},
                    {"id":"params.token.address","question":"Provide params token address","required":true,"options":[]}
                ],
                "issues":[{"reason_code":"missing_required_input"}]
            }),
        ),
    ]);
    let record = EngineEventRecord::new("run-1", 9, "1970-01-01T00:00:00Z", event);

    let payload =
        payload_from_pause(&state, std::slice::from_ref(&record), 2).expect("missing payload");
    assert_eq!(
        payload.get("missing_refs"),
        Some(&json!(["inputs.tst_token_address"]))
    );
    assert_eq!(
        payload.get("suggested_paths"),
        Some(&json!(["inputs.tst_token_address"]))
    );
    assert_eq!(
        payload.pointer("/questions/0/id"),
        Some(&json!("inputs.tst_token_address"))
    );
    assert!(payload
        .get("questions")
        .and_then(Value::as_array)
        .is_some_and(|items| items.iter().all(|q| {
            q.get("id")
                .and_then(Value::as_str)
                .map(|id| !id.starts_with("params."))
                .unwrap_or(true)
        })));
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
        fact_store
            .get("wallet.default")
            .and_then(|entry| entry.value.as_str()),
        Some("0xabc")
    );
}

#[test]
fn payload_from_pause_keeps_recovery_exhaustion_evidence() {
    let state = EngineRunnerState {
        paused_reason: Some("need_user_input:seg_1/q_token".to_string()),
        ..EngineRunnerState::default()
    };
    let mut event = ais_engine::EngineEvent::new(EngineEventType::NeedUserInput);
    event.node_id = Some("seg_1/q_token".to_string());
    event.data = serde_json::Map::from_iter([
        ("reason_code".to_string(), json!("missing_required_input")),
        (
            "reason".to_string(),
            json!("missing_inputs_or_runtime_refs"),
        ),
        (
            "details".to_string(),
            json!({
                "missing_refs":["inputs.tst_token_address","params.token.address"],
                "questions":[{"id":"params.token.address","question":"Provide token address","required":true,"options":[]}],
                "recovery_exhaustion":{
                    "unresolved_refs":["params.token.address"],
                    "reasons":["host_recovery_exhausted"],
                    "attempt_trace_id":"missing_resolution:todo:todo_1:user_input:2"
                }
            }),
        ),
    ]);
    let record = EngineEventRecord::new("run-1", 10, "1970-01-01T00:00:00Z", event);

    let payload =
        payload_from_pause(&state, std::slice::from_ref(&record), 2).expect("missing payload");
    assert_eq!(
        payload.pointer("/recovery_exhaustion/unresolved_refs/0"),
        Some(&json!("inputs.tst_token_address"))
    );
    assert_eq!(
        payload.pointer("/recovery_exhaustion/reasons/0"),
        Some(&json!("host_recovery_exhausted"))
    );
    assert_eq!(
        payload.pointer("/recovery_exhaustion/attempt_trace_id"),
        Some(&json!("missing_resolution:todo:todo_1:user_input:2"))
    );
}

#[test]
fn normalize_missing_required_input_payload_preserves_recovery_status_and_source() {
    let payload = normalize_missing_required_input_payload(&json!({
        "missing_refs": ["inputs.token.decimals"],
        "recovery_exhaustion": {
            "status": "need_user_input",
            "source": "missing_resolution",
            "unresolved_refs": ["inputs.token.decimals"],
            "reasons": ["query_exec_failed"],
            "attempt_trace_id": "missing_resolution:grounding:grounding:need_user_input"
        }
    }));
    assert_eq!(
        payload.pointer("/recovery_exhaustion/status"),
        Some(&json!("need_user_input"))
    );
    assert_eq!(
        payload.pointer("/recovery_exhaustion/source"),
        Some(&json!("missing_resolution"))
    );
}

#[test]
fn normalize_missing_required_input_payload_infers_recovery_status_from_attempt_trace() {
    let payload = normalize_missing_required_input_payload(&json!({
        "missing_refs": ["inputs.token.decimals"],
        "recovery_exhaustion": {
            "unresolved_refs": ["inputs.token.decimals"],
            "reasons": ["query_exec_failed"],
            "attempt_trace_id": "missing_resolution:grounding:grounding:exhausted_unavailable"
        }
    }));
    assert_eq!(
        payload.pointer("/recovery_exhaustion/status"),
        Some(&json!("exhausted_unavailable"))
    );
    assert_eq!(
        payload.pointer("/recovery_exhaustion/source"),
        Some(&json!("missing_resolution"))
    );
}

#[test]
fn normalize_missing_required_input_payload_keeps_non_input_refs() {
    let payload = normalize_missing_required_input_payload(&json!({
        "missing_refs": ["facts.quote.price", "nodes.q_balance.outputs.balance"],
        "suggested_paths": ["facts.quote.price", "nodes.q_balance.outputs.balance"],
        "questions": [
            {"id":"facts.quote.price","question":"Need quote"},
            {"id":"nodes.q_balance.outputs.balance","question":"Need balance"}
        ]
    }));
    assert_eq!(
        payload.get("missing_refs"),
        Some(&json!([
            "facts.quote.price",
            "nodes.q_balance.outputs.balance"
        ]))
    );
    assert_eq!(
        payload.pointer("/questions/0/id"),
        Some(&json!("facts.quote.price"))
    );
    assert_eq!(
        payload.pointer("/questions/1/id"),
        Some(&json!("nodes.q_balance.outputs.balance"))
    );
}

#[test]
fn render_missing_input_recovery_summary_requires_complete_evidence() {
    assert!(super::render_missing_input_recovery_summary(None).is_none());
    assert!(super::render_missing_input_recovery_summary(Some(&json!({
        "unresolved_refs":["inputs.token.decimals"],
        "reasons":[]
    })))
    .is_none());
}

#[test]
fn render_missing_input_recovery_summary_formats_compact_line() {
    let summary = super::render_missing_input_recovery_summary(Some(&json!({
        "unresolved_refs":["inputs.token.decimals"],
        "reasons":["query_autofill_exhausted"],
        "attempt_trace_id":"missing_resolution:todo:todo_1:user_input:1"
    })))
    .expect("summary line");
    assert!(summary.contains("attempt_trace_id=missing_resolution:todo:todo_1:user_input:1"));
    assert!(summary.contains("unresolved_refs=inputs.token.decimals"));
    assert!(summary.contains("reasons=query_autofill_exhausted"));
}
