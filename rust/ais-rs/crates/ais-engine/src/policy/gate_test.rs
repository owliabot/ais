use super::{
    enforce_policy_gate, extract_policy_gate_input, PolicyEnforcementOptions, PolicyGateOutput,
    PolicyGateReasonCode, PolicyPackAllowlist, PolicyThresholdRules,
};
use serde_json::{json, Map};

#[test]
fn policy_gate_ok_branch() {
    let node = json!({
        "id": "n-ok",
        "chain": "eip155:1",
        "execution": {
            "type": "evm_call",
            "method": "swapExactTokensForTokens"
        }
    });
    let params = Map::from_iter([
        ("spend_amount".to_string(), json!("100")),
        ("slippage_bps".to_string(), json!(100)),
    ]);
    let input = extract_policy_gate_input(
        &node,
        Some(&params),
        Some("action:swap@1.0.0".to_string()),
        Some(2),
        vec!["swap".to_string()],
    );
    let options = PolicyEnforcementOptions {
        strict_allowlist: false,
        hard_block_on_missing: false,
        enforce_plugin_execution_allowlist: false,
        allowlist: PolicyPackAllowlist {
            chains: vec!["eip155:1".to_string()],
            execution_types: vec!["evm_call".to_string()],
            action_refs: vec!["action:swap@1.0.0".to_string()],
        },
        thresholds: PolicyThresholdRules {
            max_risk_level: Some(3),
        },
    };

    let output = enforce_policy_gate(&input, &options);
    assert!(matches!(output, PolicyGateOutput::Ok { .. }));
}

#[test]
fn policy_gate_need_user_confirm_branch() {
    let node = json!({
        "id": "n-confirm",
        "chain": "eip155:1",
        "extensions": {
            "policy": {
                "required_fields": ["slippage_bps"],
                "param_roles": { "slippage_bps": "slippage_bps" }
            }
        },
        "execution": {
            "type": "evm_call",
            "method": "swapExactTokensForTokens"
        }
    });
    let params = Map::from_iter([("spend_amount".to_string(), json!("100"))]);
    let input = extract_policy_gate_input(
        &node,
        Some(&params),
        Some("action:swap@1.0.0".to_string()),
        Some(2),
        vec!["swap".to_string()],
    );

    let output = enforce_policy_gate(&input, &PolicyEnforcementOptions::default());
    match output {
        PolicyGateOutput::NeedUserConfirm {
            reason_code,
            reason,
            details,
        } => {
            assert_eq!(reason_code, PolicyGateReasonCode::MissingFields);
            assert_eq!(reason, "policy gate input is incomplete");
            assert!(details.get("missing_fields").is_some());
        }
        _ => panic!("expected need_user_confirm"),
    }
}

#[test]
fn policy_gate_hard_block_branch() {
    let node = json!({
        "id": "n-block",
        "chain": "eip155:137",
        "execution": {
            "type": "evm_call",
            "method": "swapExactTokensForTokens"
        }
    });
    let params = Map::from_iter([
        ("spend_amount".to_string(), json!("100")),
        ("slippage_bps".to_string(), json!(100)),
    ]);
    let input = extract_policy_gate_input(
        &node,
        Some(&params),
        Some("action:swap@1.0.0".to_string()),
        Some(2),
        vec!["swap".to_string()],
    );
    let options = PolicyEnforcementOptions {
        strict_allowlist: false,
        hard_block_on_missing: false,
        enforce_plugin_execution_allowlist: false,
        allowlist: PolicyPackAllowlist {
            chains: vec!["eip155:1".to_string()],
            execution_types: vec![],
            action_refs: vec![],
        },
        thresholds: PolicyThresholdRules::default(),
    };

    let output = enforce_policy_gate(&input, &options);
    match output {
        PolicyGateOutput::HardBlock {
            reason_code,
            reason,
            ..
        } => {
            assert_eq!(reason_code, PolicyGateReasonCode::AllowlistChainNotAllowed);
            assert_eq!(reason, "chain is not allowlisted by pack");
        }
        _ => panic!("expected hard_block"),
    }
}

#[test]
fn policy_gate_constraint_template_max_spend_requires_confirmation() {
    let node = json!({
        "id": "n-template-confirm",
        "chain": "eip155:1",
        "extensions": {
            "policy": {
                "param_roles": { "spend_amount": "amount" },
                "constraint_templates": [
                    { "name": "max_spend", "params": { "amount_atomic": "100" } }
                ]
            }
        },
        "execution": {
            "type": "evm_call",
            "method": "swapExactTokensForTokens"
        }
    });
    let params = Map::from_iter([("amount".to_string(), json!("101"))]);
    let input = extract_policy_gate_input(
        &node,
        Some(&params),
        Some("action:swap@1.0.0".to_string()),
        Some(2),
        vec!["swap".to_string()],
    );

    let output = enforce_policy_gate(&input, &PolicyEnforcementOptions::default());
    match output {
        PolicyGateOutput::NeedUserConfirm {
            reason_code,
            details,
            ..
        } => {
            let details_json = json!(details);
            assert_eq!(
                reason_code,
                PolicyGateReasonCode::ConstraintTemplateViolated
            );
            assert_eq!(
                details_json.pointer("/matched_constraints/0"),
                Some(&json!("max_spend"))
            );
            assert_eq!(
                details_json.pointer("/violations/0/effect"),
                Some(&json!("need_user_confirm"))
            );
        }
        _ => panic!("expected need_user_confirm"),
    }
}

#[test]
fn policy_gate_constraint_template_disallow_unlimited_approval_hard_blocks() {
    let node = json!({
        "id": "n-template-block",
        "chain": "eip155:1",
        "extensions": {
            "policy": {
                "constraint_templates": [
                    { "name": "disallow_unlimited_approval" }
                ]
            }
        },
        "execution": {
            "type": "evm_call",
            "method": "approve"
        }
    });
    let params = Map::from_iter([("unlimited_approval".to_string(), json!(true))]);
    let input = extract_policy_gate_input(
        &node,
        Some(&params),
        Some("action:approve@1.0.0".to_string()),
        Some(4),
        vec!["approval".to_string()],
    );

    let output = enforce_policy_gate(&input, &PolicyEnforcementOptions::default());
    match output {
        PolicyGateOutput::HardBlock {
            reason_code,
            details,
            ..
        } => {
            let details_json = json!(details);
            assert_eq!(
                reason_code,
                PolicyGateReasonCode::ConstraintTemplateViolated
            );
            assert_eq!(
                details_json.pointer("/matched_constraints/0"),
                Some(&json!("disallow_unlimited_approval"))
            );
            assert_eq!(
                details_json.pointer("/violations/0/effect"),
                Some(&json!("hard_block"))
            );
        }
        _ => panic!("expected hard_block"),
    }
}
