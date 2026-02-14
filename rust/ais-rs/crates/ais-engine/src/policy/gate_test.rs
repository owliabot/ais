use super::{
    enforce_policy_gate, extract_policy_gate_input, PolicyEnforcementOptions, PolicyGateOutput,
    PolicyPackAllowlist, PolicyThresholdRules,
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
        Some("swap@1.0.0".to_string()),
        Some(2),
        vec!["swap".to_string()],
    );
    let options = PolicyEnforcementOptions {
        strict_allowlist: false,
        hard_block_on_missing: false,
        allowlist: PolicyPackAllowlist {
            chains: vec!["eip155:1".to_string()],
            execution_types: vec!["evm_call".to_string()],
            action_refs: vec!["swap@1.0.0".to_string()],
        },
        thresholds: PolicyThresholdRules {
            max_risk_level: Some(3),
            max_spend_amount: Some("1000".to_string()),
            max_slippage_bps: Some(500),
            forbid_unlimited_approval: true,
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
        "execution": {
            "type": "evm_call",
            "method": "swapExactTokensForTokens"
        }
    });
    let params = Map::from_iter([("spend_amount".to_string(), json!("100"))]);
    let input = extract_policy_gate_input(
        &node,
        Some(&params),
        Some("swap@1.0.0".to_string()),
        Some(2),
        vec!["swap".to_string()],
    );

    let output = enforce_policy_gate(&input, &PolicyEnforcementOptions::default());
    match output {
        PolicyGateOutput::NeedUserConfirm { reason, details } => {
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
        Some("swap@1.0.0".to_string()),
        Some(2),
        vec!["swap".to_string()],
    );
    let options = PolicyEnforcementOptions {
        strict_allowlist: false,
        hard_block_on_missing: false,
        allowlist: PolicyPackAllowlist {
            chains: vec!["eip155:1".to_string()],
            execution_types: vec![],
            action_refs: vec![],
        },
        thresholds: PolicyThresholdRules::default(),
    };

    let output = enforce_policy_gate(&input, &options);
    match output {
        PolicyGateOutput::HardBlock { reason, .. } => {
            assert_eq!(reason, "chain is not allowlisted by pack");
        }
        _ => panic!("expected hard_block"),
    }
}
