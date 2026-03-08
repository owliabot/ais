use super::{
    approvals_mode_from_pack, llm_may_approve_max_risk_level_from_pack, policy_from_pack,
    volatile_facts_policy_from_pack, DEFAULT_VOLATILE_FACT_MAX_AGE_MS,
};
use crate::cli::ApprovalsMode;
use ais_sdk::PackDocument;
use serde_json::json;

#[test]
fn approvals_mode_can_be_derived_from_pack() {
    let pack = PackDocument {
        schema: "ais-pack/0.0.2".to_string(),
        name: None,
        version: None,
        description: None,
        meta: None,
        includes: vec![],
        policy: Some(json!({"approvals": {"mode": "yolo"}})),
        token_policy: None,
        providers: None,
        plugins: None,
        overrides: None,
        extensions: serde_json::Map::new(),
    };

    assert_eq!(approvals_mode_from_pack(&pack), Some(ApprovalsMode::Yolo));
}

#[test]
fn llm_assist_threshold_can_be_derived_from_pack() {
    let pack = PackDocument {
        schema: "ais-pack/0.0.2".to_string(),
        name: None,
        version: None,
        description: None,
        meta: None,
        includes: vec![],
        policy: Some(json!({
            "approvals": {
                "mode": "assist",
                "llm_may_approve_max_risk_level": 2
            }
        })),
        token_policy: None,
        providers: None,
        plugins: None,
        overrides: None,
        extensions: serde_json::Map::new(),
    };

    assert_eq!(llm_may_approve_max_risk_level_from_pack(&pack), Some(2));
}

#[test]
fn policy_from_pack_maps_approval_risk_gate_and_chain_scope() {
    let pack = PackDocument {
        schema: "ais-pack/0.0.2".to_string(),
        name: None,
        version: None,
        description: None,
        meta: None,
        includes: vec![
            json!({"protocol":"uniswap-v3","version":"0.0.2","source":"registry","chain_scope":["eip155:8453"]}),
            json!({"protocol":"aave-v3","version":"0.0.2","source":"registry","chain_scope":["eip155:1","eip155:8453"]}),
        ],
        policy: Some(json!({
            "approvals": {
                "auto_execute_max_risk_level": 2,
                "require_approval_min_risk_level": 4
            }
        })),
        token_policy: None,
        providers: None,
        plugins: None,
        overrides: None,
        extensions: serde_json::Map::new(),
    };

    let options = policy_from_pack(&pack).expect("policy must map");
    assert_eq!(options.thresholds.max_risk_level, Some(2));
    assert_eq!(
        options.allowlist.chains,
        vec!["eip155:1".to_string(), "eip155:8453".to_string()]
    );
}

#[test]
fn policy_from_pack_rejects_invalid_approvals_thresholds() {
    let pack = PackDocument {
        schema: "ais-pack/0.0.2".to_string(),
        name: None,
        version: None,
        description: None,
        meta: None,
        includes: vec![],
        policy: Some(json!({
            "approvals": {
                "auto_execute_max_risk_level": 3,
                "require_approval_min_risk_level": 3
            }
        })),
        token_policy: None,
        providers: None,
        plugins: None,
        overrides: None,
        extensions: serde_json::Map::new(),
    };

    let error = policy_from_pack(&pack).expect_err("must reject invalid thresholds");
    assert!(error.to_string().contains("invalid approvals thresholds"));
}

#[test]
fn policy_from_pack_maps_plugin_execution_allowlist() {
    let pack = PackDocument {
        schema: "ais-pack/0.0.2".to_string(),
        name: None,
        version: None,
        description: None,
        meta: None,
        includes: vec![
            json!({"protocol":"demo","version":"0.0.2","source":"registry","chain_scope":["eip155:1"]}),
        ],
        policy: None,
        token_policy: None,
        providers: None,
        plugins: Some(json!({
            "execution": {
                "enabled": [
                    {"type": "offchain_apy_query", "chains": ["eip155:1", "eip155:8453"]}
                ]
            }
        })),
        overrides: None,
        extensions: serde_json::Map::new(),
    };

    let options = policy_from_pack(&pack).expect("policy must map");
    assert_eq!(
        options.allowlist.execution_types,
        vec!["offchain_apy_query".to_string()]
    );
    assert_eq!(
        options.allowlist.chains,
        vec!["eip155:1".to_string(), "eip155:8453".to_string()]
    );
}

#[test]
fn volatile_facts_policy_from_pack_defaults_when_not_configured() {
    let policy = volatile_facts_policy_from_pack(None).expect("default policy");
    assert_eq!(policy.max_age_ms, DEFAULT_VOLATILE_FACT_MAX_AGE_MS);
}

#[test]
fn volatile_facts_policy_from_pack_reads_pack_execution_policy() {
    let pack = PackDocument {
        schema: "ais-pack/0.0.2".to_string(),
        name: None,
        version: None,
        description: None,
        meta: None,
        includes: vec![],
        policy: Some(json!({
            "execution": {
                "volatile_facts": {
                    "max_age_ms": 45_000
                }
            }
        })),
        token_policy: None,
        providers: None,
        plugins: None,
        overrides: None,
        extensions: serde_json::Map::new(),
    };

    let policy = volatile_facts_policy_from_pack(Some(&pack)).expect("pack policy");
    assert_eq!(policy.max_age_ms, 45_000);
}

#[test]
fn volatile_facts_policy_from_pack_rejects_invalid_max_age_ms() {
    let pack = PackDocument {
        schema: "ais-pack/0.0.2".to_string(),
        name: None,
        version: None,
        description: None,
        meta: None,
        includes: vec![],
        policy: Some(json!({
            "execution": {
                "volatile_facts": {
                    "max_age_ms": 0
                }
            }
        })),
        token_policy: None,
        providers: None,
        plugins: None,
        overrides: None,
        extensions: serde_json::Map::new(),
    };

    let error = volatile_facts_policy_from_pack(Some(&pack)).expect_err("must reject zero");
    assert!(error
        .to_string()
        .contains("policy.execution.volatile_facts.max_age_ms"));
}
