use super::{summarize_pause, PauseKind};
use ais_engine::{EngineEvent, EngineEventRecord, EngineEventType};
use serde_json::json;

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
