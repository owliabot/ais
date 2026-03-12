use std::collections::BTreeMap;

use ais_agent_core::{
    action::{
        kinds::{
            actuate::{ActuateAction, ActuateLiveBinding, ActuateMode, EvmActuateLiveBinding},
            derive::{DeriveAction, DeriveKind},
        },
        ActionNode, ActionNodeKind, ActionNodeStatus, ActionOrigin, ActionPayload,
    },
    binding::evm::EvmActuateBinding,
    driver::{DriverEvmActuateHint, DriverNodeLiveBindingHint},
    effect::{EffectAssertion, EffectContract, EffectContractKind},
    evidence::{EvidenceGraph, EvidenceRequirement},
    mission::{Mission, MissionBudget, MissionPolicy},
};

use crate::standard::{
    ActionGraphFragment, StandardDriver, StandardDriverError, StandardDriverOutput,
    StandardDriverRequest,
};

#[test]
fn standard_driver_builds_action_fragment_evidence_and_effects() {
    let driver = MockSwapDriver;
    let output = driver
        .build(&StandardDriverRequest {
            mission: sample_mission(),
            evidence: EvidenceGraph::default(),
            action_selector: "swap_exact_in".to_owned(),
        })
        .expect("driver output");

    assert_eq!(output.fragment.roots, vec!["derive.min_out".to_owned()]);
    assert_eq!(output.fragment.terminals, vec!["derive.min_out".to_owned()]);
    assert!(output.fragment.nodes.contains_key("derive.min_out"));
    assert_eq!(output.evidence_requirements.len(), 1);
    assert_eq!(output.effect_contracts.len(), 1);
}

#[test]
fn standard_driver_reports_unsupported_action() {
    let driver = MockSwapDriver;
    let error = driver
        .build(&StandardDriverRequest {
            mission: sample_mission(),
            evidence: EvidenceGraph::default(),
            action_selector: "borrow".to_owned(),
        })
        .expect_err("unsupported action should fail");

    assert_eq!(
        error,
        StandardDriverError::UnsupportedAction("borrow".to_owned())
    );
}

#[test]
fn standard_driver_can_emit_fragment_level_live_binding_hints() {
    let driver = MockLiveSwapDriver;
    let mut output = driver
        .build(&StandardDriverRequest {
            mission: sample_mission(),
            evidence: EvidenceGraph::default(),
            action_selector: "swap_exact_in".to_owned(),
        })
        .expect("driver output");

    assert!(output
        .fragment
        .live_binding_hints
        .contains_key("actuate.swap"));
    output
        .apply_live_binding_hints()
        .expect("live binding hints should apply");

    match &output.fragment.nodes["actuate.swap"].payload {
        ActionPayload::Actuate(action) => {
            assert_eq!(
                action.live,
                Some(ActuateLiveBinding::Evm(EvmActuateLiveBinding {
                    connection: None,
                    binding: EvmActuateBinding::BroadcastSignedEnvelope,
                }))
            );
        }
        other => panic!("unexpected payload: {other:?}"),
    }
}

struct MockSwapDriver;
struct MockLiveSwapDriver;

impl StandardDriver for MockSwapDriver {
    fn driver_id(&self) -> &'static str {
        "mock.swap"
    }

    fn supports_action(&self, action_selector: &str) -> bool {
        action_selector == "swap_exact_in"
    }

    fn build(
        &self,
        request: &StandardDriverRequest,
    ) -> Result<StandardDriverOutput, StandardDriverError> {
        if !self.supports_action(request.action_selector.as_str()) {
            return Err(StandardDriverError::UnsupportedAction(
                request.action_selector.clone(),
            ));
        }

        let mut nodes = BTreeMap::new();
        nodes.insert(
            "derive.min_out".to_owned(),
            ActionNode {
                node_id: "derive.min_out".to_owned(),
                kind: ActionNodeKind::Derive,
                origin: ActionOrigin::DriverFragment,
                status: ActionNodeStatus::Pending,
                depends_on: Vec::new(),
                inputs: Vec::new(),
                evidence_refs: vec!["quote".to_owned()],
                payload: ActionPayload::Derive(DeriveAction {
                    derive_kind: DeriveKind::SlippageBound,
                    derivation_hint: "derive min out from quote".to_owned(),
                    output_key: Some("derived.min_out".to_owned()),
                }),
                implementation_hint: Some("mock.swap".to_owned()),
                expected_effect_ref: Some("effects.swap".to_owned()),
            },
        );

        Ok(StandardDriverOutput {
            fragment: ActionGraphFragment {
                roots: vec!["derive.min_out".to_owned()],
                terminals: vec!["derive.min_out".to_owned()],
                nodes,
                live_binding_hints: BTreeMap::new(),
            },
            evidence_requirements: vec![EvidenceRequirement {
                requirement_id: "quote".to_owned(),
                reference: "evidence.quote".to_owned(),
                reason: "quote required for swap".to_owned(),
                required_by_node_id: Some("derive.min_out".to_owned()),
                satisfied_by_evidence_id: None,
            }],
            effect_contracts: vec![EffectContract {
                effect_id: "effects.swap".to_owned(),
                kind: EffectContractKind::AssetDelta,
                assertions: vec![EffectAssertion {
                    expression: "post.amount_out >= expected.min_out".to_owned(),
                    description: "minimum output must hold".to_owned(),
                }],
                tolerance_hint: Some("tight".to_owned()),
            }],
        })
    }
}

impl StandardDriver for MockLiveSwapDriver {
    fn driver_id(&self) -> &'static str {
        "mock.live_swap"
    }

    fn supports_action(&self, action_selector: &str) -> bool {
        action_selector == "swap_exact_in"
    }

    fn build(
        &self,
        request: &StandardDriverRequest,
    ) -> Result<StandardDriverOutput, StandardDriverError> {
        if !self.supports_action(request.action_selector.as_str()) {
            return Err(StandardDriverError::UnsupportedAction(
                request.action_selector.clone(),
            ));
        }

        let mut nodes = BTreeMap::new();
        nodes.insert(
            "actuate.swap".to_owned(),
            ActionNode {
                node_id: "actuate.swap".to_owned(),
                kind: ActionNodeKind::Actuate,
                origin: ActionOrigin::DriverFragment,
                status: ActionNodeStatus::Pending,
                depends_on: Vec::new(),
                inputs: Vec::new(),
                evidence_refs: Vec::new(),
                payload: ActionPayload::Actuate(ActuateAction {
                    mode: ActuateMode::DriverCall,
                    actuator_hint: "swap through live EVM binding".to_owned(),
                    chain: Some("eip155:1".to_owned()),
                    envelope_ref: Some("env.swap".to_owned()),
                    requires_effect_contract: true,
                    live: None,
                }),
                implementation_hint: Some(self.driver_id().to_owned()),
                expected_effect_ref: Some("effects.swap".to_owned()),
            },
        );

        Ok(StandardDriverOutput {
            fragment: ActionGraphFragment {
                roots: vec!["actuate.swap".to_owned()],
                terminals: vec!["actuate.swap".to_owned()],
                nodes,
                live_binding_hints: BTreeMap::from([(
                    "actuate.swap".to_owned(),
                    DriverNodeLiveBindingHint::EvmActuate(DriverEvmActuateHint {
                        binding: EvmActuateBinding::BroadcastSignedEnvelope,
                    }),
                )]),
            },
            evidence_requirements: Vec::new(),
            effect_contracts: vec![EffectContract {
                effect_id: "effects.swap".to_owned(),
                kind: EffectContractKind::StateTransition,
                assertions: vec![EffectAssertion {
                    expression: "receipt != null".to_owned(),
                    description: "signed swap should yield a receipt".to_owned(),
                }],
                tolerance_hint: Some("receipt_required".to_owned()),
            }],
        })
    }
}

fn sample_mission() -> Mission {
    Mission {
        mission_id: "mission-1".to_owned(),
        goal: "swap usdc to eth".to_owned(),
        allowed_chains: vec!["eip155:1".to_owned()],
        budget: MissionBudget::default(),
        policy: MissionPolicy::default(),
        constraints: BTreeMap::new(),
        metadata: BTreeMap::new(),
    }
}
