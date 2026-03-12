use std::collections::BTreeMap;

use alloy::primitives::Address;

use crate::{
    action::{
        kinds::{
            actuate::{ActuateAction, ActuateLiveBinding, ActuateMode, EvmActuateLiveBinding},
            observe::{
                EvmObserveLiveBinding, ObserveAction, ObserveLiveBinding, ObserveSourceKind,
            },
        },
        ActionNode, ActionNodeKind, ActionNodeStatus, ActionOrigin, ActionPayload,
    },
    binding::evm::{EvmActuateBinding, EvmObserveBinding, EvmObserveRequest},
    driver::{
        ActionGraphFragment, DriverEvmActuateHint, DriverEvmObserveHint,
        DriverFragmentBindingError, DriverNodeLiveBindingHint,
    },
};

#[test]
fn driver_fragment_applies_evm_live_binding_hints_into_node_payloads() {
    let mut fragment = ActionGraphFragment {
        roots: vec!["observe.balance".to_owned()],
        terminals: vec!["actuate.swap".to_owned()],
        nodes: BTreeMap::from([
            (
                "observe.balance".to_owned(),
                ActionNode {
                    node_id: "observe.balance".to_owned(),
                    kind: ActionNodeKind::Observe,
                    origin: ActionOrigin::DriverFragment,
                    status: ActionNodeStatus::Pending,
                    depends_on: Vec::new(),
                    inputs: Vec::new(),
                    evidence_refs: Vec::new(),
                    payload: ActionPayload::Observe(ObserveAction {
                        source_kind: ObserveSourceKind::ChainRead,
                        source_hint: "observe owner balance".to_owned(),
                        output_key: Some("state.owner_balance".to_owned()),
                        live: None,
                    }),
                    implementation_hint: Some("mock.driver".to_owned()),
                    expected_effect_ref: None,
                },
            ),
            (
                "actuate.swap".to_owned(),
                ActionNode {
                    node_id: "actuate.swap".to_owned(),
                    kind: ActionNodeKind::Actuate,
                    origin: ActionOrigin::DriverFragment,
                    status: ActionNodeStatus::Pending,
                    depends_on: vec!["observe.balance".to_owned()],
                    inputs: Vec::new(),
                    evidence_refs: Vec::new(),
                    payload: ActionPayload::Actuate(ActuateAction {
                        mode: ActuateMode::DriverCall,
                        actuator_hint: "broadcast typed swap".to_owned(),
                        chain: Some("eip155:1".to_owned()),
                        envelope_ref: Some("env.swap".to_owned()),
                        requires_effect_contract: true,
                        live: None,
                    }),
                    implementation_hint: Some("mock.driver".to_owned()),
                    expected_effect_ref: Some("effects.swap".to_owned()),
                },
            ),
        ]),
        live_binding_hints: BTreeMap::from([
            (
                "observe.balance".to_owned(),
                DriverNodeLiveBindingHint::EvmObserve(DriverEvmObserveHint {
                    binding: EvmObserveBinding::NativeBalance,
                    request: EvmObserveRequest::NativeBalance {
                        address: Address::repeat_byte(0x11),
                    },
                }),
            ),
            (
                "actuate.swap".to_owned(),
                DriverNodeLiveBindingHint::EvmActuate(DriverEvmActuateHint {
                    binding: EvmActuateBinding::BroadcastSignedEnvelope,
                }),
            ),
        ]),
    };

    fragment
        .apply_live_binding_hints()
        .expect("binding hints apply");

    match &fragment.nodes["observe.balance"].payload {
        ActionPayload::Observe(action) => {
            assert_eq!(
                action.live,
                Some(ObserveLiveBinding::Evm(EvmObserveLiveBinding {
                    connection: None,
                    binding: EvmObserveBinding::NativeBalance,
                    request: EvmObserveRequest::NativeBalance {
                        address: Address::repeat_byte(0x11),
                    },
                }))
            );
        }
        other => panic!("unexpected payload: {other:?}"),
    }

    match &fragment.nodes["actuate.swap"].payload {
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

#[test]
fn driver_fragment_rejects_kind_mismatch_between_hint_and_node() {
    let mut fragment = ActionGraphFragment {
        roots: vec!["observe.balance".to_owned()],
        terminals: vec!["observe.balance".to_owned()],
        nodes: BTreeMap::from([(
            "observe.balance".to_owned(),
            ActionNode {
                node_id: "observe.balance".to_owned(),
                kind: ActionNodeKind::Observe,
                origin: ActionOrigin::DriverFragment,
                status: ActionNodeStatus::Pending,
                depends_on: Vec::new(),
                inputs: Vec::new(),
                evidence_refs: Vec::new(),
                payload: ActionPayload::Observe(ObserveAction {
                    source_kind: ObserveSourceKind::ChainRead,
                    source_hint: "observe owner balance".to_owned(),
                    output_key: Some("state.owner_balance".to_owned()),
                    live: None,
                }),
                implementation_hint: None,
                expected_effect_ref: None,
            },
        )]),
        live_binding_hints: BTreeMap::from([(
            "observe.balance".to_owned(),
            DriverNodeLiveBindingHint::EvmActuate(DriverEvmActuateHint {
                binding: EvmActuateBinding::BroadcastRawTransaction,
            }),
        )]),
    };

    let error = fragment
        .apply_live_binding_hints()
        .expect_err("mismatched hint should fail");

    assert_eq!(
        error,
        DriverFragmentBindingError::KindMismatch {
            node_id: "observe.balance".to_owned(),
            node_kind: ActionNodeKind::Observe,
            hint_kind: "evm_actuate".to_owned(),
        }
    );
}
