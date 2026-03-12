use std::collections::BTreeMap;

use ais_agent_chain_shared::{
    ChainFamily, ReflectionArtifactKind, ReflectionDriver, ReflectionDriverError,
    ReflectionDriverOutput, ReflectionRequest,
};
use ais_agent_core::{
    action::{
        kinds::{
            actuate::{ActuateAction, ActuateMode},
            simulate::{SimulateAction, SimulateKind},
            verify::{EvmVerifyLiveBinding, VerifyAction, VerifyKind, VerifyLiveBinding},
        },
        ActionNode, ActionNodeKind, ActionNodeStatus, ActionOrigin, ActionPayload,
    },
    binding::evm::EvmVerifyBinding,
    binding::evm::{EvmActuateBinding, EvmCallRequest, EvmSimulateBinding},
    driver::{
        ActionGraphFragment, DriverBuildOutput, DriverEvmActuateHint, DriverEvmSimulateHint,
        DriverEvmVerifyHint, DriverNodeLiveBindingHint,
    },
    effect::{EffectAssertion, EffectContract, EffectContractKind},
};
use alloy::primitives::{address, bytes};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EvmAbiReflectionAdapter;

impl ReflectionDriver for EvmAbiReflectionAdapter {
    fn driver_id(&self) -> &'static str {
        "reflect.evm_abi"
    }

    fn family(&self) -> ChainFamily {
        ChainFamily::Evm
    }

    fn build(
        &self,
        request: &ReflectionRequest,
    ) -> Result<ReflectionDriverOutput, ReflectionDriverError> {
        if request.chain_family != ChainFamily::Evm {
            return Err(ReflectionDriverError::UnsupportedFamily(
                request.chain_family.clone(),
            ));
        }
        if request.artifact_kind != ReflectionArtifactKind::EvmAbi {
            return Err(ReflectionDriverError::UnsupportedArtifact);
        }

        let chain = request
            .mission
            .allowed_chains
            .first()
            .cloned()
            .unwrap_or_else(|| "eip155:unknown".to_owned());
        let node_id = format!("reflect.evm.{}", request.action_selector);
        let simulate_node_id = format!("{node_id}.simulate");
        let verify_node_id = format!("{node_id}.verify");

        let mut nodes = BTreeMap::new();
        nodes.insert(
            simulate_node_id.clone(),
            ActionNode {
                node_id: simulate_node_id.clone(),
                kind: ActionNodeKind::Simulate,
                origin: ActionOrigin::ReflectionPath,
                status: ActionNodeStatus::Pending,
                depends_on: Vec::new(),
                inputs: Vec::new(),
                evidence_refs: Vec::new(),
                payload: ActionPayload::Simulate(SimulateAction {
                    simulate_kind: SimulateKind::Call,
                    simulator_hint: format!(
                        "simulate reflected evm abi call {}",
                        request.action_selector
                    ),
                    live: None,
                }),
                implementation_hint: Some(self.driver_id().to_owned()),
                expected_effect_ref: None,
            },
        );
        nodes.insert(
            node_id.clone(),
            ActionNode {
                node_id: node_id.clone(),
                kind: ActionNodeKind::Actuate,
                origin: ActionOrigin::ReflectionPath,
                status: ActionNodeStatus::Pending,
                depends_on: vec![simulate_node_id.clone()],
                inputs: Vec::new(),
                evidence_refs: Vec::new(),
                payload: ActionPayload::Actuate(ActuateAction {
                    mode: ActuateMode::ReflectedCall,
                    actuator_hint: format!("reflect evm abi call {}", request.action_selector),
                    chain: Some(chain.clone()),
                    envelope_ref: None,
                    requires_effect_contract: true,
                    live: None,
                }),
                implementation_hint: Some(self.driver_id().to_owned()),
                expected_effect_ref: Some(format!("effects.{node_id}")),
            },
        );
        nodes.insert(
            verify_node_id.clone(),
            ActionNode {
                node_id: verify_node_id.clone(),
                kind: ActionNodeKind::Verify,
                origin: ActionOrigin::ReflectionPath,
                status: ActionNodeStatus::Pending,
                depends_on: vec![node_id.clone()],
                inputs: Vec::new(),
                evidence_refs: Vec::new(),
                payload: ActionPayload::Verify(VerifyAction {
                    verify_kind: VerifyKind::EffectContract,
                    verifier_hint: format!(
                        "verify reflected evm abi call {}",
                        request.action_selector
                    ),
                    pre_observation_ref: None,
                    post_observation_ref: Some(format!("post.{verify_node_id}")),
                    live: Some(VerifyLiveBinding::Evm(EvmVerifyLiveBinding {
                        connection: None,
                        binding: EvmVerifyBinding::EffectContractFromReceipt,
                        post_request: Some(
                            ais_agent_core::binding::evm::EvmObserveRequest::Erc20BalanceOf {
                                token: address!("3333333333333333333333333333333333333333"),
                                owner: address!("4444444444444444444444444444444444444444"),
                            },
                        ),
                    })),
                }),
                implementation_hint: Some(self.driver_id().to_owned()),
                expected_effect_ref: Some(format!("effects.{node_id}")),
            },
        );

        Ok(DriverBuildOutput {
            fragment: ActionGraphFragment {
                roots: vec![simulate_node_id.clone()],
                terminals: vec![verify_node_id.clone()],
                nodes,
                live_binding_hints: BTreeMap::from([
                    (
                        simulate_node_id.clone(),
                        DriverNodeLiveBindingHint::EvmSimulate(DriverEvmSimulateHint {
                            binding: EvmSimulateBinding::EthCall,
                            request: EvmCallRequest {
                                from: None,
                                to: address!("1111111111111111111111111111111111111111"),
                                data: bytes!("00"),
                                value: None,
                            },
                        }),
                    ),
                    (
                        node_id.clone(),
                        DriverNodeLiveBindingHint::EvmActuate(DriverEvmActuateHint {
                            binding: EvmActuateBinding::BroadcastTypedTransaction,
                        }),
                    ),
                    (
                        verify_node_id.clone(),
                        DriverNodeLiveBindingHint::EvmVerify(DriverEvmVerifyHint {
                            binding: EvmVerifyBinding::EffectContractFromReceipt,
                            post_evm_request: None,
                        }),
                    ),
                ]),
            },
            evidence_requirements: Vec::new(),
            effect_contracts: vec![EffectContract {
                effect_id: format!("effects.{node_id}"),
                kind: EffectContractKind::StateTransition,
                assertions: vec![EffectAssertion {
                    expression: "post.decoded_u256 == \"1\"".to_owned(),
                    description: "reflected EVM call should yield expected post-state".to_owned(),
                }],
                tolerance_hint: Some("receipt_required".to_owned()),
            }],
        })
    }
}
