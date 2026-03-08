use super::{summarize_pause, summarize_pause_with_context, PauseKind};
use ais_engine::{EngineEvent, EngineEventRecord, EngineEventType, EngineRunnerState};
use ais_sdk::PlanDocument;
use serde_json::json;
use serde_json::Map;

#[test]
fn render_for_humans_includes_confirmation_highlights() {
    let mut event = EngineEvent::new(EngineEventType::NeedUserConfirm);
    event.node_id = Some("transfer-1".to_string());
    event.data = serde_json::Map::from_iter([
        (
            "reason_code".to_string(),
            json!("threshold_risk_level_exceeded"),
        ),
        ("reason".to_string(), json!("manual review required")),
        (
            "details".to_string(),
            json!({
                "confirmation_hash":"0xabc",
                "confirmation_summary":{
                    "chain":"eip155:1",
                    "action_ref":"action:erc20/transfer",
                    "execution_type":"evm_call",
                    "risk_level":3,
                    "details":{
                        "spend_amount":"10",
                        "token":"USDC",
                        "to":"0xB"
                    }
                }
            }),
        ),
    ]);
    let record = EngineEventRecord::new("run-test", 1, "1970-01-01T00:00:00Z", event);
    let summary = summarize_pause(Some("need_user_confirm:transfer-1"), &[record]);
    assert_eq!(summary.kind, PauseKind::NeedUserConfirm);
    let rendered = summary.render_for_humans();
    assert!(rendered.contains("chain: eip155:1"));
    assert!(rendered.contains("action_ref: action:erc20/transfer"));
    assert!(rendered.contains("risk_level: 3"));
    assert!(rendered.contains("amount: 10"));
    assert!(rendered.contains("asset: USDC"));
    assert!(rendered.contains("target: 0xB"));
}

#[test]
fn summarize_pause_recognizes_need_user_input_kind() {
    let summary = summarize_pause(Some("need_user_input:command"), &[]);
    assert_eq!(summary.kind, PauseKind::NeedUserInput);
    assert_eq!(summary.node_id.as_deref(), Some("command"));

    let missing = summarize_pause(Some("missing_required_input"), &[]);
    assert_eq!(missing.kind, PauseKind::NeedUserInput);
    assert!(missing.node_id.is_none());
}

#[test]
fn summarize_pause_with_context_renders_bundle_params_from_runtime_inputs() {
    let mut event = EngineEvent::new(EngineEventType::NeedUserConfirm);
    event.node_id = Some("seg_3__a_native_transfer".to_string());
    event.data = serde_json::Map::from_iter([
        (
            "reason_code".to_string(),
            json!("threshold_risk_level_exceeded"),
        ),
        ("reason".to_string(), json!("manual review required")),
        (
            "details".to_string(),
            json!({
                "confirmation_hash":"0xabc",
                "confirmation_summary":{
                    "node_id":"seg_3__a_native_transfer",
                    "chain":"eip155:31338",
                    "action_ref":"action:evm-native-utils@0.0.1/native-transfer",
                    "execution_type":"evm_call",
                    "risk_level":3
                }
            }),
        ),
    ]);
    let record = EngineEventRecord::new("run-test", 1, "1970-01-01T00:00:00Z", event);
    let plan = PlanDocument {
        schema: "ais-plan/0.0.3".to_string(),
        meta: None,
        nodes: vec![
            json!({
                "id":"seg_3__a_native_transfer",
                "kind":"action_ref",
                "chain":"eip155:31338",
                "execution":{"type":"evm_call"},
                "bindings":{"params":{
                    "amount":{"lit":"5"},
                    "to":{"ref":"inputs.recipient"}
                }},
                "extensions":{
                    "risk_level":3,
                    "plan_sketch":{"candidate_ref":"evm-native-utils@0.0.1/native-transfer"}
                }
            }),
            json!({
                "id":"seg_3__a_token_transfer",
                "kind":"action_ref",
                "chain":"eip155:31338",
                "execution":{"type":"evm_call"},
                "bindings":{"params":{
                    "amount":{"lit":"10"},
                    "to":{"ref":"inputs.recipient"},
                    "token":{"ref":"inputs.token.address"}
                }},
                "extensions":{
                    "risk_level":3,
                    "plan_sketch":{"candidate_ref":"erc20@0.0.2/transfer"}
                }
            }),
        ],
        extensions: Map::new(),
    };
    let state = EngineRunnerState {
        runtime: json!({
            "inputs": {
                "recipient": "0xrecipient",
                "token": {
                    "address": "0xtoken"
                }
            }
        }),
        ..EngineRunnerState::default()
    };

    let summary = summarize_pause_with_context(
        Some("need_user_confirm:seg_3__a_native_transfer"),
        &[record],
        Some(&plan),
        Some(&state),
    );
    let rendered = summary.render_for_humans();
    assert!(rendered.contains("to=0xrecipient"));
    assert!(rendered.contains("token=0xtoken"));
    assert!(!rendered.contains("ref:inputs.recipient"));
    assert!(!rendered.contains("ref:inputs.token.address"));
}

#[test]
fn summarize_pause_with_context_keeps_bundle_id_stable_when_current_node_advances() {
    let plan = PlanDocument {
        schema: "ais-plan/0.0.3".to_string(),
        meta: None,
        nodes: vec![
            json!({
                "id":"seg_3__a_native_transfer",
                "kind":"action_ref",
                "chain":"eip155:31338",
                "execution":{"type":"evm_call"},
                "bindings":{"params":{
                    "amount":{"lit":"5"},
                    "to":{"ref":"inputs.recipient"}
                }},
                "extensions":{
                    "risk_level":3,
                    "plan_sketch":{"candidate_ref":"evm-native-utils@0.0.1/native-transfer"}
                }
            }),
            json!({
                "id":"seg_3__a_token_transfer",
                "kind":"action_ref",
                "chain":"eip155:31338",
                "execution":{"type":"evm_call"},
                "bindings":{"params":{
                    "amount":{"lit":"10"},
                    "to":{"ref":"inputs.recipient"},
                    "token":{"ref":"inputs.token.address"}
                }},
                "extensions":{
                    "risk_level":3,
                    "plan_sketch":{"candidate_ref":"erc20@0.0.2/transfer"}
                }
            }),
        ],
        extensions: Map::new(),
    };
    let state = EngineRunnerState {
        runtime: json!({
            "inputs": {
                "recipient": "0xrecipient",
                "token": {
                    "address": "0xtoken"
                }
            }
        }),
        ..EngineRunnerState::default()
    };
    let mut native_event = EngineEvent::new(EngineEventType::NeedUserConfirm);
    native_event.node_id = Some("seg_3__a_native_transfer".to_string());
    native_event.data = serde_json::Map::from_iter([
        (
            "reason_code".to_string(),
            json!("threshold_risk_level_exceeded"),
        ),
        ("reason".to_string(), json!("manual review required")),
        (
            "details".to_string(),
            json!({
                "confirmation_hash":"0xnative",
                "confirmation_summary":{
                    "node_id":"seg_3__a_native_transfer",
                    "chain":"eip155:31338",
                    "action_ref":"action:evm-native-utils@0.0.1/native-transfer",
                    "execution_type":"evm_call",
                    "risk_level":3
                }
            }),
        ),
    ]);
    let native_record = EngineEventRecord::new("run-test", 1, "1970-01-01T00:00:00Z", native_event);
    let mut token_event = EngineEvent::new(EngineEventType::NeedUserConfirm);
    token_event.node_id = Some("seg_3__a_token_transfer".to_string());
    token_event.data = serde_json::Map::from_iter([
        (
            "reason_code".to_string(),
            json!("threshold_risk_level_exceeded"),
        ),
        ("reason".to_string(), json!("manual review required")),
        (
            "details".to_string(),
            json!({
                "confirmation_hash":"0xtoken",
                "confirmation_summary":{
                    "node_id":"seg_3__a_token_transfer",
                    "chain":"eip155:31338",
                    "action_ref":"action:erc20@0.0.2/transfer",
                    "execution_type":"evm_call",
                    "risk_level":3
                }
            }),
        ),
    ]);
    let token_record = EngineEventRecord::new("run-test", 2, "1970-01-01T00:00:01Z", token_event);

    let native_summary = summarize_pause_with_context(
        Some("need_user_confirm:seg_3__a_native_transfer"),
        &[native_record],
        Some(&plan),
        Some(&state),
    );
    let token_summary = summarize_pause_with_context(
        Some("need_user_confirm:seg_3__a_token_transfer"),
        &[token_record],
        Some(&plan),
        Some(&state),
    );

    let native_bundle = native_summary
        .need_user_confirm
        .as_ref()
        .and_then(|need| need.confirmation_bundle.as_ref())
        .expect("native bundle");
    let token_bundle = token_summary
        .need_user_confirm
        .as_ref()
        .and_then(|need| need.confirmation_bundle.as_ref())
        .expect("token bundle");

    assert_eq!(native_bundle.current_node_id, "seg_3__a_native_transfer");
    assert_eq!(token_bundle.current_node_id, "seg_3__a_token_transfer");
    assert_eq!(native_bundle.bundle_id, token_bundle.bundle_id);
}

#[test]
fn summarize_pause_with_context_keeps_input_refs_unresolved_when_only_fact_value_exists() {
    let mut event = EngineEvent::new(EngineEventType::NeedUserConfirm);
    event.node_id = Some("seg_3__a_native_transfer".to_string());
    event.data = serde_json::Map::from_iter([
        (
            "reason_code".to_string(),
            json!("threshold_risk_level_exceeded"),
        ),
        ("reason".to_string(), json!("manual review required")),
        (
            "details".to_string(),
            json!({
                "confirmation_hash":"0xabc",
                "confirmation_summary":{
                    "node_id":"seg_3__a_native_transfer",
                    "chain":"eip155:31338",
                    "action_ref":"action:evm-native-utils@0.0.1/native-transfer",
                    "execution_type":"evm_call",
                    "risk_level":3
                }
            }),
        ),
    ]);
    let record = EngineEventRecord::new("run-test", 1, "1970-01-01T00:00:00Z", event);
    let plan = PlanDocument {
        schema: "ais-plan/0.0.3".to_string(),
        meta: None,
        nodes: vec![json!({
            "id":"seg_3__a_native_transfer",
            "kind":"action_ref",
            "chain":"eip155:31338",
            "execution":{"type":"evm_call"},
            "bindings":{"params":{
                "to":{"ref":"inputs.recipient"}
            }},
            "extensions":{
                "risk_level":3,
                "plan_sketch":{"candidate_ref":"evm-native-utils@0.0.1/native-transfer"}
            }
        })],
        extensions: Map::new(),
    };
    let state = EngineRunnerState {
        runtime: json!({
            "agent": {
                "state_summary": {
                    "intent_context": {
                        "facts": {
                            "recipient": "0xfact-only"
                        }
                    }
                }
            }
        }),
        ..EngineRunnerState::default()
    };

    let summary = summarize_pause_with_context(
        Some("need_user_confirm:seg_3__a_native_transfer"),
        &[record],
        Some(&plan),
        Some(&state),
    );
    let rendered = summary.render_for_humans();
    assert!(rendered.contains("to=ref:inputs.recipient"));
    assert!(!rendered.contains("to=0xfact-only"));
}
