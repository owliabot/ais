use serde_json::json;

use crate::effect::{
    verify_effect_contract, EffectAssertion, EffectContract, EffectContractKind, EffectDeltaStatus,
    EffectObservationBundle,
};
use crate::{
    driver::{ActionGraphFragment, DriverBuildOutput},
    envelope::{bind_raw_envelope_action, RuntimeEnvelope, RuntimeEnvelopeKind},
};

#[test]
fn effect_verifier_satisfies_pre_post_asset_delta_assertions() {
    let contract = EffectContract {
        effect_id: "swap-effect".to_owned(),
        kind: EffectContractKind::AssetDelta,
        assertions: vec![
            EffectAssertion {
                expression: "post.usdc < pre.usdc".to_owned(),
                description: "input asset should decrease".to_owned(),
            },
            EffectAssertion {
                expression: "post.eth > pre.eth".to_owned(),
                description: "output asset should increase".to_owned(),
            },
        ],
        tolerance_hint: Some("tight".to_owned()),
    };

    let result = verify_effect_contract(
        &contract,
        &EffectObservationBundle {
            pre: Some(json!({"usdc": "1000", "eth": "1"})),
            post: Some(json!({"usdc": "900", "eth": "2"})),
            receipt: None,
            expected: None,
            context: None,
        },
    );

    assert_eq!(result.final_status, EffectDeltaStatus::Satisfied);
    assert_eq!(result.deltas.len(), 2);
    assert!(result
        .deltas
        .iter()
        .all(|delta| delta.status == EffectDeltaStatus::Satisfied));
}

#[test]
fn effect_verifier_marks_violated_assertions_as_violated() {
    let contract = EffectContract {
        effect_id: "swap-effect".to_owned(),
        kind: EffectContractKind::AssetDelta,
        assertions: vec![EffectAssertion {
            expression: "post.amount_out >= expected.min_out".to_owned(),
            description: "min out should be honored".to_owned(),
        }],
        tolerance_hint: None,
    };

    let result = verify_effect_contract(
        &contract,
        &EffectObservationBundle {
            pre: None,
            post: Some(json!({"amount_out": "95"})),
            receipt: None,
            expected: Some(json!({"min_out": "100"})),
            context: None,
        },
    );

    assert_eq!(result.final_status, EffectDeltaStatus::Violated);
    assert_eq!(result.deltas[0].status, EffectDeltaStatus::Violated);
}

#[test]
fn effect_verifier_reports_unknown_when_required_observation_is_missing() {
    let contract = EffectContract {
        effect_id: "swap-effect".to_owned(),
        kind: EffectContractKind::StateTransition,
        assertions: vec![EffectAssertion {
            expression: "receipt.status == true && post.balance > pre.balance".to_owned(),
            description: "receipt and balance transition should both hold".to_owned(),
        }],
        tolerance_hint: None,
    };

    let result = verify_effect_contract(
        &contract,
        &EffectObservationBundle {
            pre: Some(json!({"balance": "1"})),
            post: Some(json!({"balance": "2"})),
            receipt: None,
            expected: None,
            context: None,
        },
    );

    assert_eq!(
        result.final_status,
        EffectDeltaStatus::UnknownDueToMissingObservation
    );
    assert_eq!(
        result.deltas[0].status,
        EffectDeltaStatus::UnknownDueToMissingObservation
    );
    assert!(!result.deltas[0].missing_bindings.is_empty());
}

#[test]
fn driver_backed_effect_contract_can_be_verified_as_satisfied() {
    let output = DriverBuildOutput {
        fragment: ActionGraphFragment::default(),
        evidence_requirements: Vec::new(),
        effect_contracts: vec![EffectContract {
            effect_id: "effects.driver.swap".to_owned(),
            kind: EffectContractKind::AssetDelta,
            assertions: vec![
                EffectAssertion {
                    expression: "post.output_atomic >= expected.min_out_atomic".to_owned(),
                    description: "driver swap must honor min out".to_owned(),
                },
                EffectAssertion {
                    expression: "receipt.status == true".to_owned(),
                    description: "driver swap receipt must succeed".to_owned(),
                },
            ],
            tolerance_hint: Some("driver".to_owned()),
        }],
    };

    let result = verify_effect_contract(
        &output.effect_contracts[0],
        &EffectObservationBundle {
            pre: None,
            post: Some(json!({ "output_atomic": "105" })),
            receipt: Some(json!({ "status": true })),
            expected: Some(json!({ "min_out_atomic": "100" })),
            context: Some(json!({ "path": "driver" })),
        },
    );

    assert_eq!(result.final_status, EffectDeltaStatus::Satisfied);
    assert_eq!(result.deltas.len(), 2);
}

#[test]
fn driver_backed_effect_contract_can_be_verified_as_violated() {
    let output = DriverBuildOutput {
        fragment: ActionGraphFragment::default(),
        evidence_requirements: Vec::new(),
        effect_contracts: vec![EffectContract {
            effect_id: "effects.driver.swap".to_owned(),
            kind: EffectContractKind::AssetDelta,
            assertions: vec![EffectAssertion {
                expression: "post.output_atomic >= expected.min_out_atomic".to_owned(),
                description: "driver swap must honor min out".to_owned(),
            }],
            tolerance_hint: Some("driver".to_owned()),
        }],
    };

    let result = verify_effect_contract(
        &output.effect_contracts[0],
        &EffectObservationBundle {
            pre: None,
            post: Some(json!({ "output_atomic": "95" })),
            receipt: Some(json!({ "status": true })),
            expected: Some(json!({ "min_out_atomic": "100" })),
            context: Some(json!({ "path": "driver" })),
        },
    );

    assert_eq!(result.final_status, EffectDeltaStatus::Violated);
    assert_eq!(result.deltas[0].status, EffectDeltaStatus::Violated);
}

#[test]
fn raw_envelope_effect_contract_can_be_verified_as_satisfied() {
    let envelope = RuntimeEnvelope {
        envelope_id: "env-1".to_owned(),
        kind: RuntimeEnvelopeKind::EvmEnvelope,
        chain: "eip155:1".to_owned(),
        payload: json!({"to":"0xabc","data":"0xdeadbeef","value":"0"}),
        provenance: Some("host.route_api".to_owned()),
    };
    let action = bind_raw_envelope_action(
        "swap",
        &envelope,
        Some("effects.raw.swap".to_owned()),
        "broadcast raw swap",
    )
    .expect("raw envelope action");

    let contract = EffectContract {
        effect_id: action
            .expected_effect_ref
            .clone()
            .expect("effect contract ref"),
        kind: EffectContractKind::StateTransition,
        assertions: vec![
            EffectAssertion {
                expression: "receipt.status == true".to_owned(),
                description: "raw envelope receipt must succeed".to_owned(),
            },
            EffectAssertion {
                expression: "post.received_atomic >= expected.min_received_atomic".to_owned(),
                description: "raw envelope must deliver min received amount".to_owned(),
            },
        ],
        tolerance_hint: Some("raw-envelope".to_owned()),
    };

    let result = verify_effect_contract(
        &contract,
        &EffectObservationBundle {
            pre: Some(json!({ "received_atomic": "0" })),
            post: Some(json!({ "received_atomic": "120" })),
            receipt: Some(json!({ "status": true, "tx_hash": "0xdeadbeef" })),
            expected: Some(json!({ "min_received_atomic": "100" })),
            context: Some(json!({ "path": "raw-envelope" })),
        },
    );

    assert_eq!(result.final_status, EffectDeltaStatus::Satisfied);
}

#[test]
fn raw_envelope_effect_contract_reports_unknown_when_receipt_observation_is_missing() {
    let envelope = RuntimeEnvelope {
        envelope_id: "env-2".to_owned(),
        kind: RuntimeEnvelopeKind::EvmEnvelope,
        chain: "eip155:1".to_owned(),
        payload: json!({"to":"0xabc","data":"0xdeadbeef","value":"0"}),
        provenance: Some("host.route_api".to_owned()),
    };
    let action = bind_raw_envelope_action(
        "swap",
        &envelope,
        Some("effects.raw.swap".to_owned()),
        "broadcast raw swap",
    )
    .expect("raw envelope action");

    let contract = EffectContract {
        effect_id: action
            .expected_effect_ref
            .clone()
            .expect("effect contract ref"),
        kind: EffectContractKind::StateTransition,
        assertions: vec![EffectAssertion {
            expression:
                "receipt.status == true && post.received_atomic >= expected.min_received_atomic"
                    .to_owned(),
            description: "raw envelope needs receipt and output observation".to_owned(),
        }],
        tolerance_hint: Some("raw-envelope".to_owned()),
    };

    let result = verify_effect_contract(
        &contract,
        &EffectObservationBundle {
            pre: Some(json!({ "received_atomic": "0" })),
            post: Some(json!({ "received_atomic": "120" })),
            receipt: None,
            expected: Some(json!({ "min_received_atomic": "100" })),
            context: Some(json!({ "path": "raw-envelope" })),
        },
    );

    assert_eq!(
        result.final_status,
        EffectDeltaStatus::UnknownDueToMissingObservation
    );
    assert!(!result.deltas[0].missing_bindings.is_empty());
}
