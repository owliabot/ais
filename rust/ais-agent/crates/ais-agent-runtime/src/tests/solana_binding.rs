use ais_agent_core::{
    action::{
        kinds::{
            actuate::{ActuateAction, ActuateLiveBinding, ActuateMode, SolanaActuateLiveBinding},
            observe::{
                ObserveAction, ObserveLiveBinding, ObserveSourceKind, SolanaObserveLiveBinding,
            },
            simulate::{
                SimulateAction, SimulateKind, SimulateLiveBinding, SolanaSimulateLiveBinding,
            },
            verify::{SolanaVerifyLiveBinding, VerifyAction, VerifyKind, VerifyLiveBinding},
        },
        ActionNode, ActionNodeKind, ActionNodeStatus, ActionOrigin, ActionPayload,
    },
    binding::solana::{
        SolanaActuateBinding, SolanaConnectionSpec, SolanaObserveBinding, SolanaObserveRequest,
        SolanaSimulateBinding, SolanaTransactionRequest, SolanaVerifyBinding,
    },
};
use solana_sdk::{
    instruction::Instruction, message::AddressLookupTableAccount, pubkey::Pubkey,
    signature::Signature,
};

use crate::stepper::{
    resolve_solana_actuate_binding, resolve_solana_observe_binding,
    resolve_solana_simulate_binding, resolve_solana_verify_binding,
};

#[test]
fn runtime_resolves_typed_solana_bindings_without_protocol_specific_dispatch() {
    let observe = ActionNode {
        node_id: "observe.sol".to_owned(),
        kind: ActionNodeKind::Observe,
        origin: ActionOrigin::DriverFragment,
        status: ActionNodeStatus::Pending,
        depends_on: Vec::new(),
        inputs: Vec::new(),
        evidence_refs: Vec::new(),
        payload: ActionPayload::Observe(ObserveAction {
            source_kind: ObserveSourceKind::ChainRead,
            source_hint: "solana rpc get account".to_owned(),
            output_key: Some("solana.account".to_owned()),
            live: Some(ObserveLiveBinding::Solana(SolanaObserveLiveBinding {
                connection: Some(SolanaConnectionSpec {
                    http_url: "http://localhost:8899".to_owned(),
                    ws_url: Some("ws://localhost:8900".to_owned()),
                }),
                binding: SolanaObserveBinding::AccountLamports,
                request: SolanaObserveRequest::AccountLamports {
                    address: Pubkey::new_from_array([1u8; 32]),
                },
            })),
        }),
        implementation_hint: Some("solana.observe".to_owned()),
        expected_effect_ref: None,
    };

    let simulate = ActionNode {
        node_id: "simulate.sol".to_owned(),
        kind: ActionNodeKind::Simulate,
        origin: ActionOrigin::DriverFragment,
        status: ActionNodeStatus::Pending,
        depends_on: Vec::new(),
        inputs: Vec::new(),
        evidence_refs: Vec::new(),
        payload: ActionPayload::Simulate(SimulateAction {
            simulate_kind: SimulateKind::Call,
            simulator_hint: "solana simulate transaction".to_owned(),
            live: Some(SimulateLiveBinding::Solana(SolanaSimulateLiveBinding {
                connection: Some(SolanaConnectionSpec {
                    http_url: "http://localhost:8899".to_owned(),
                    ws_url: None,
                }),
                binding: SolanaSimulateBinding::SimulateTransaction,
                request: SolanaTransactionRequest::Legacy {
                    recent_blockhash: None,
                    payer: Some(Pubkey::new_from_array([2u8; 32])),
                    instructions: vec![Instruction {
                        program_id: Pubkey::new_from_array([3u8; 32]),
                        accounts: Vec::new(),
                        data: vec![1, 2, 3],
                    }],
                },
            })),
        }),
        implementation_hint: Some("solana.simulate".to_owned()),
        expected_effect_ref: None,
    };

    let actuate = ActionNode {
        node_id: "actuate.sol".to_owned(),
        kind: ActionNodeKind::Actuate,
        origin: ActionOrigin::DriverFragment,
        status: ActionNodeStatus::Pending,
        depends_on: Vec::new(),
        inputs: Vec::new(),
        evidence_refs: Vec::new(),
        payload: ActionPayload::Actuate(ActuateAction {
            mode: ActuateMode::DriverCall,
            actuator_hint: "solana broadcast".to_owned(),
            chain: Some("solana:mainnet".to_owned()),
            envelope_ref: Some("env.sol".to_owned()),
            requires_effect_contract: true,
            live: Some(ActuateLiveBinding::Solana(SolanaActuateLiveBinding {
                connection: Some(SolanaConnectionSpec {
                    http_url: "http://localhost:8899".to_owned(),
                    ws_url: None,
                }),
                binding: SolanaActuateBinding::BroadcastSignedTransaction,
            })),
        }),
        implementation_hint: Some("solana.broadcast".to_owned()),
        expected_effect_ref: Some("effects.sol".to_owned()),
    };

    let verify = ActionNode {
        node_id: "verify.sol".to_owned(),
        kind: ActionNodeKind::Verify,
        origin: ActionOrigin::DriverFragment,
        status: ActionNodeStatus::Pending,
        depends_on: Vec::new(),
        inputs: Vec::new(),
        evidence_refs: Vec::new(),
        payload: ActionPayload::Verify(VerifyAction {
            verify_kind: VerifyKind::EffectContract,
            verifier_hint: "solana signature status".to_owned(),
            pre_observation_ref: None,
            post_observation_ref: None,
            live: Some(VerifyLiveBinding::Solana(SolanaVerifyLiveBinding {
                connection: Some(SolanaConnectionSpec {
                    http_url: "http://localhost:8899".to_owned(),
                    ws_url: Some("ws://localhost:8900".to_owned()),
                }),
                binding: SolanaVerifyBinding::EffectContractFromSignatureStatus,
                post_request: Some(SolanaObserveRequest::SignatureStatus {
                    signature: Signature::new_unique(),
                }),
            })),
        }),
        implementation_hint: Some("solana.verify".to_owned()),
        expected_effect_ref: Some("effects.sol".to_owned()),
    };

    assert_eq!(
        resolve_solana_observe_binding(&observe),
        Some(SolanaObserveBinding::AccountLamports)
    );
    assert_eq!(
        resolve_solana_simulate_binding(&simulate),
        Some(SolanaSimulateBinding::SimulateTransaction)
    );
    assert_eq!(
        resolve_solana_actuate_binding(&actuate),
        Some(SolanaActuateBinding::BroadcastSignedTransaction)
    );
    assert_eq!(
        resolve_solana_verify_binding(&verify),
        Some(SolanaVerifyBinding::EffectContractFromSignatureStatus)
    );
}

#[test]
fn runtime_never_resolves_wrong_solana_binding_for_wrong_node_kind() {
    let derive_like = ActionNode {
        node_id: "not.sol".to_owned(),
        kind: ActionNodeKind::Recover,
        origin: ActionOrigin::RecoveryRuntime,
        status: ActionNodeStatus::Pending,
        depends_on: Vec::new(),
        inputs: Vec::new(),
        evidence_refs: Vec::new(),
        payload: ActionPayload::Verify(VerifyAction {
            verify_kind: VerifyKind::ReceiptObserved,
            verifier_hint: "mismatched".to_owned(),
            pre_observation_ref: None,
            post_observation_ref: None,
            live: Some(VerifyLiveBinding::Solana(SolanaVerifyLiveBinding {
                connection: Some(SolanaConnectionSpec {
                    http_url: "http://localhost:8899".to_owned(),
                    ws_url: None,
                }),
                binding: SolanaVerifyBinding::SignatureStatus,
                post_request: None,
            })),
        }),
        implementation_hint: None,
        expected_effect_ref: None,
    };

    assert_eq!(resolve_solana_observe_binding(&derive_like), None);
    assert_eq!(resolve_solana_simulate_binding(&derive_like), None);
    assert_eq!(resolve_solana_actuate_binding(&derive_like), None);
    assert_eq!(resolve_solana_verify_binding(&derive_like), None);
}

#[test]
fn runtime_accepts_v0_solana_transaction_request_with_lookup_tables() {
    let simulate = ActionNode {
        node_id: "simulate.sol.v0".to_owned(),
        kind: ActionNodeKind::Simulate,
        origin: ActionOrigin::DriverFragment,
        status: ActionNodeStatus::Pending,
        depends_on: Vec::new(),
        inputs: Vec::new(),
        evidence_refs: Vec::new(),
        payload: ActionPayload::Simulate(SimulateAction {
            simulate_kind: SimulateKind::Call,
            simulator_hint: "solana simulate v0 transaction".to_owned(),
            live: Some(SimulateLiveBinding::Solana(SolanaSimulateLiveBinding {
                connection: Some(SolanaConnectionSpec {
                    http_url: "http://localhost:8899".to_owned(),
                    ws_url: None,
                }),
                binding: SolanaSimulateBinding::SimulateTransaction,
                request: SolanaTransactionRequest::V0 {
                    recent_blockhash: None,
                    payer: Some(Pubkey::new_from_array([9u8; 32])),
                    instructions: vec![Instruction {
                        program_id: Pubkey::new_from_array([8u8; 32]),
                        accounts: Vec::new(),
                        data: vec![4, 5, 6],
                    }],
                    address_lookup_tables: vec![AddressLookupTableAccount {
                        key: Pubkey::new_from_array([7u8; 32]),
                        addresses: vec![
                            Pubkey::new_from_array([6u8; 32]),
                            Pubkey::new_from_array([5u8; 32]),
                        ],
                    }],
                },
            })),
        }),
        implementation_hint: Some("solana.simulate.v0".to_owned()),
        expected_effect_ref: None,
    };

    assert_eq!(
        resolve_solana_simulate_binding(&simulate),
        Some(SolanaSimulateBinding::SimulateTransaction)
    );

    let ActionPayload::Simulate(action) = &simulate.payload else {
        panic!("expected simulate payload");
    };
    match action.live.as_ref() {
        Some(SimulateLiveBinding::Solana(SolanaSimulateLiveBinding {
            request:
                SolanaTransactionRequest::V0 {
                    address_lookup_tables,
                    ..
                },
            ..
        })) => {
            assert_eq!(address_lookup_tables.len(), 1);
            assert_eq!(address_lookup_tables[0].addresses.len(), 2);
        }
        other => panic!("unexpected live binding: {other:?}"),
    }
}
