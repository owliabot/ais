use ais_agent_core::{
    action::{
        kinds::{
            actuate::{ActuateAction, ActuateLiveBinding, ActuateMode, EvmActuateLiveBinding},
            observe::{
                EvmObserveLiveBinding, ObserveAction, ObserveLiveBinding, ObserveSourceKind,
            },
            simulate::{EvmSimulateLiveBinding, SimulateAction, SimulateKind, SimulateLiveBinding},
            verify::{EvmVerifyLiveBinding, VerifyAction, VerifyKind, VerifyLiveBinding},
        },
        ActionNode, ActionNodeKind, ActionNodeStatus, ActionOrigin, ActionPayload,
    },
    binding::evm::{
        EvmActuateBinding, EvmCallRequest, EvmObserveBinding, EvmObserveRequest,
        EvmSimulateBinding, EvmVerifyBinding,
    },
};
use alloy::primitives::{address, bytes};

use crate::stepper::{
    resolve_evm_actuate_binding, resolve_evm_observe_binding, resolve_evm_simulate_binding,
    resolve_evm_verify_binding,
};

#[test]
fn runtime_resolves_typed_evm_bindings_without_protocol_specific_dispatch() {
    let observe = ActionNode {
        node_id: "observe-balance".to_owned(),
        kind: ActionNodeKind::Observe,
        origin: ActionOrigin::DriverFragment,
        status: ActionNodeStatus::Pending,
        depends_on: Vec::new(),
        inputs: Vec::new(),
        evidence_refs: Vec::new(),
        payload: ActionPayload::Observe(ObserveAction {
            source_kind: ObserveSourceKind::ChainRead,
            source_hint: "erc20 balance".to_owned(),
            output_key: Some("balance".to_owned()),
            live: Some(ObserveLiveBinding::Evm(EvmObserveLiveBinding {
                connection: None,
                binding: EvmObserveBinding::Erc20BalanceOf,
                request: EvmObserveRequest::Erc20BalanceOf {
                    token: address!("1111111111111111111111111111111111111111"),
                    owner: address!("2222222222222222222222222222222222222222"),
                },
            })),
        }),
        implementation_hint: Some("evm.read.erc20_balance_of".to_owned()),
        expected_effect_ref: None,
    };
    let simulate = ActionNode {
        node_id: "simulate-swap".to_owned(),
        kind: ActionNodeKind::Simulate,
        origin: ActionOrigin::DriverFragment,
        status: ActionNodeStatus::Pending,
        depends_on: Vec::new(),
        inputs: Vec::new(),
        evidence_refs: Vec::new(),
        payload: ActionPayload::Simulate(SimulateAction {
            simulate_kind: SimulateKind::Call,
            simulator_hint: "swap dry run".to_owned(),
            live: Some(SimulateLiveBinding::Evm(EvmSimulateLiveBinding {
                connection: None,
                binding: EvmSimulateBinding::EthCall,
                request: EvmCallRequest {
                    from: None,
                    to: address!("3333333333333333333333333333333333333333"),
                    data: bytes!("1234"),
                    value: None,
                },
            })),
        }),
        implementation_hint: Some("evm.simulate.eth_call".to_owned()),
        expected_effect_ref: None,
    };
    let actuate = ActionNode {
        node_id: "broadcast-swap".to_owned(),
        kind: ActionNodeKind::Actuate,
        origin: ActionOrigin::DriverFragment,
        status: ActionNodeStatus::Pending,
        depends_on: Vec::new(),
        inputs: Vec::new(),
        evidence_refs: Vec::new(),
        payload: ActionPayload::Actuate(ActuateAction {
            mode: ActuateMode::DriverCall,
            actuator_hint: "broadcast swap".to_owned(),
            chain: Some("eip155:1".to_owned()),
            envelope_ref: Some("env.swap".to_owned()),
            requires_effect_contract: true,
            live: Some(ActuateLiveBinding::Evm(EvmActuateLiveBinding {
                connection: None,
                binding: EvmActuateBinding::BroadcastSignedEnvelope,
            })),
        }),
        implementation_hint: Some("evm.broadcast.signed_envelope".to_owned()),
        expected_effect_ref: Some("effect.swap".to_owned()),
    };
    let verify = ActionNode {
        node_id: "verify-swap".to_owned(),
        kind: ActionNodeKind::Verify,
        origin: ActionOrigin::DriverFragment,
        status: ActionNodeStatus::Pending,
        depends_on: Vec::new(),
        inputs: Vec::new(),
        evidence_refs: Vec::new(),
        payload: ActionPayload::Verify(VerifyAction {
            verify_kind: VerifyKind::EffectContract,
            verifier_hint: "effect verify".to_owned(),
            pre_observation_ref: None,
            post_observation_ref: None,
            live: Some(VerifyLiveBinding::Evm(EvmVerifyLiveBinding {
                connection: None,
                binding: EvmVerifyBinding::EffectContractFromReceiptAndPostState,
                post_request: None,
            })),
        }),
        implementation_hint: Some("evm.verify.receipt_and_post_state".to_owned()),
        expected_effect_ref: Some("effect.swap".to_owned()),
    };

    assert_eq!(
        resolve_evm_observe_binding(&observe),
        Some(EvmObserveBinding::Erc20BalanceOf)
    );
    assert_eq!(
        resolve_evm_simulate_binding(&simulate),
        Some(EvmSimulateBinding::EthCall)
    );
    assert_eq!(
        resolve_evm_actuate_binding(&actuate),
        Some(EvmActuateBinding::BroadcastSignedEnvelope)
    );
    assert_eq!(
        resolve_evm_verify_binding(&verify),
        Some(EvmVerifyBinding::EffectContractFromReceiptAndPostState)
    );
}

#[test]
fn runtime_never_resolves_wrong_binding_for_wrong_node_kind() {
    let node = ActionNode {
        node_id: "observe-balance".to_owned(),
        kind: ActionNodeKind::Observe,
        origin: ActionOrigin::DriverFragment,
        status: ActionNodeStatus::Pending,
        depends_on: Vec::new(),
        inputs: Vec::new(),
        evidence_refs: Vec::new(),
        payload: ActionPayload::Observe(ObserveAction {
            source_kind: ObserveSourceKind::ChainRead,
            source_hint: "balance".to_owned(),
            output_key: None,
            live: Some(ObserveLiveBinding::Evm(EvmObserveLiveBinding {
                connection: None,
                binding: EvmObserveBinding::NativeBalance,
                request: EvmObserveRequest::NativeBalance {
                    address: address!("4444444444444444444444444444444444444444"),
                },
            })),
        }),
        implementation_hint: Some("evm.read.native_balance".to_owned()),
        expected_effect_ref: None,
    };

    assert_eq!(
        resolve_evm_observe_binding(&node),
        Some(EvmObserveBinding::NativeBalance)
    );
    assert_eq!(resolve_evm_simulate_binding(&node), None);
    assert_eq!(resolve_evm_actuate_binding(&node), None);
    assert_eq!(resolve_evm_verify_binding(&node), None);
}
