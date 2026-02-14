use super::{
    build_confirmation_summary, confirmation_hash, enrich_need_user_confirm_output,
    ConfirmationSummary,
};
use crate::policy::{PolicyGateInput, PolicyGateOutput};
use serde_json::{json, Map};

fn sample_gate_input() -> PolicyGateInput {
    PolicyGateInput {
        node_id: Some("swap-1".to_string()),
        chain: "eip155:1".to_string(),
        execution_type: Some("evm_call".to_string()),
        action_ref: Some("swap@1.0.0".to_string()),
        risk_level: Some(3),
        risk_tags: vec!["swap".to_string()],
        spend_amount: Some("100".to_string()),
        slippage_bps: Some(100),
        approval_amount: None,
        unlimited_approval: None,
        spender_address: None,
        missing_fields: vec!["slippage_bps".to_string()],
        unknown_fields: vec![],
        hard_block_fields: vec![],
    }
}

#[test]
fn confirmation_hash_is_stable_ignoring_timestamps() {
    let mut details_a = Map::new();
    details_a.insert("scope".to_string(), json!("swap"));
    details_a.insert("timestamp".to_string(), json!("2026-02-13T00:00:00Z"));
    let mut details_b = Map::new();
    details_b.insert("scope".to_string(), json!("swap"));
    details_b.insert("timestamp".to_string(), json!("2026-02-13T00:05:00Z"));

    let summary_a = ConfirmationSummary {
        kind: "need_user_confirm".to_string(),
        reason: "policy gate input is incomplete".to_string(),
        node_id: Some("swap-1".to_string()),
        chain: "eip155:1".to_string(),
        action_ref: Some("swap@1.0.0".to_string()),
        execution_type: Some("evm_call".to_string()),
        risk_level: Some(3),
        risk_tags: vec!["swap".to_string()],
        missing_fields: vec!["slippage_bps".to_string()],
        unknown_fields: vec![],
        hard_block_fields: vec![],
        details: details_a,
    };
    let mut summary_b = summary_a.clone();
    summary_b.details = details_b;

    let hash_a = confirmation_hash(&summary_a).expect("hash");
    let hash_b = confirmation_hash(&summary_b).expect("hash");
    assert_eq!(hash_a, hash_b);
}

#[test]
fn enrich_need_user_confirm_output_contains_summary_and_hash() {
    let input = sample_gate_input();
    let gate_output = PolicyGateOutput::NeedUserConfirm {
        reason: "policy gate input is incomplete".to_string(),
        details: Map::from_iter([
            ("missing_fields".to_string(), json!(["slippage_bps"])),
            ("ts".to_string(), json!("2026-02-13T00:00:00Z")),
        ]),
    };

    let enriched = enrich_need_user_confirm_output(&input, &gate_output).expect("enrich");
    match enriched {
        PolicyGateOutput::NeedUserConfirm { details, .. } => {
            assert!(details.get("confirmation_summary").is_some());
            assert!(details.get("confirmation_hash").is_some());
        }
        _ => panic!("expected need_user_confirm"),
    }
}

#[test]
fn build_confirmation_summary_returns_none_for_non_confirm_output() {
    let input = sample_gate_input();
    let output = PolicyGateOutput::Ok { details: Map::new() };
    assert!(build_confirmation_summary(&input, &output).is_none());
}
