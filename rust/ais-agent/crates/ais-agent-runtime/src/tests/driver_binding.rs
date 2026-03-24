use std::collections::BTreeMap;

use ais_agent_chain_shared::{
    ChainFamily, ReflectionArtifactKind, ReflectionDriver, ReflectionRequest,
};
use ais_agent_control::ids::RunId;
use ais_agent_core::{
    action::{
        kinds::{
            actuate::{ActuateAction, ActuateMode},
            observe::{ObserveAction, ObserveSourceKind},
            simulate::{EvmSimulateLiveBinding, SimulateAction, SimulateKind, SimulateLiveBinding},
            verify::{EvmVerifyLiveBinding, VerifyAction, VerifyKind, VerifyLiveBinding},
        },
        ActionGraph, ActionNode, ActionNodeKind, ActionNodeStatus, ActionOrigin, ActionPayload,
    },
    binding::evm::{
        EvmActuateBinding, EvmCallRequest, EvmConnectionSpec, EvmObserveBinding, EvmObserveRequest,
        EvmSimulateBinding, EvmVerifyBinding,
    },
    checkpoint::{CheckpointSnapshot, PendingRequestsSnapshot},
    driver::{
        ActionGraphFragment, DriverBuildOutput, DriverEvmActuateHint, DriverEvmObserveHint,
        DriverEvmSimulateHint, DriverEvmVerifyHint, DriverNodeLiveBindingHint,
    },
    effect::{EffectAssertion, EffectContract, EffectContractKind},
    envelope::{RuntimeEnvelope, RuntimeEnvelopeKind},
    evidence::{EvidenceGraph, EvidenceRequirement},
    mission::{Mission, MissionBudget, MissionPolicy},
    runtime::{RunLifecycleState, RunPhase, RunStatus, SignerResolution, SignerResolutionKind},
};
use ais_agent_drivers::api_native::{
    ApiNativeAdapter, ApiNativeProviderKind, ApiNativeRequest, DirectEnvelopeApiAdapter,
    DirectEnvelopePayload, EvmNativeEnvelope,
};
use ais_agent_evm::reflect::EvmAbiReflectionAdapter;
use alloy::{
    consensus::{Receipt, ReceiptEnvelope},
    primitives::{address, b256, bytes, Bytes, U256},
    providers::ProviderBuilder,
    rpc::types::TransactionReceipt,
    transports::mock::Asserter,
};
use serde_json::json;

use crate::{
    runtime::{ActiveRun, DriverBindingContext, RawEnvelopeBindingRequest, RuntimeDriverBinder},
    stepper::{
        apply_live_evm_broadcast_with_provider, apply_live_evm_observe_with_provider,
        apply_live_evm_simulate_with_provider, apply_live_evm_verify_with_provider, StepOnce,
        StepTransitionKind,
    },
};

#[tokio::test]
async fn standard_like_driver_output_binds_into_runtime_and_executes_live_path() {
    let mission = sample_mission();
    let checkpoint = empty_checkpoint();
    let mut runtime = ActiveRun::new(mission, checkpoint);
    runtime.envelopes.insert(
        "env.swap".to_owned(),
        RuntimeEnvelope {
            envelope_id: "env.swap".to_owned(),
            kind: RuntimeEnvelopeKind::EvmEnvelope,
            chain: "eip155:1".to_owned(),
            payload: json!({"raw_tx":"0x0102"}),
            provenance: Some("test".to_owned()),
        },
    );

    RuntimeDriverBinder::bind_output(
        &mut runtime,
        standard_like_driver_output(),
        &DriverBindingContext::default()
            .with_evm_connection(
                "eip155:1",
                EvmConnectionSpec {
                    http_url: "http://example.invalid".to_owned(),
                    ws_url: None,
                },
            )
            .with_envelope_ref("driver.actuate.swap", "env.swap"),
    )
    .expect("driver output should bind");

    match &runtime.checkpoint.action_graph.nodes["driver.verify.swap"].payload {
        ActionPayload::Verify(action) => match &action.live {
            Some(VerifyLiveBinding::Evm(live)) => {
                assert_eq!(live.binding, EvmVerifyBinding::EffectContractFromReceipt);
                assert!(live.connection.is_some());
            }
            other => panic!("unexpected live binding: {other:?}"),
        },
        other => panic!("unexpected payload: {other:?}"),
    }

    let asserter = Asserter::new();
    let provider = ProviderBuilder::new().connect_mocked_client(asserter.clone());
    let tx_hash = b256!("abababababababababababababababababababababababababababababababab");
    asserter.push_success(&80u64);
    asserter.push_success(&bytes!(
        "0000000000000000000000000000000000000000000000000000000000000001"
    ));
    asserter.push_success(&tx_hash);
    asserter.push_success(&sample_receipt(tx_hash, 77));
    asserter.push_success(&80u64);
    asserter.push_success(&encode_u256_return(120));

    let observe = apply_live_evm_observe_with_provider(&mut runtime, &provider)
        .await
        .expect("observe");
    assert_eq!(observe.kind, StepTransitionKind::Observe);

    let simulate = apply_live_evm_simulate_with_provider(&mut runtime, &provider)
        .await
        .expect("simulate");
    assert_eq!(simulate.kind, StepTransitionKind::Simulate);

    let govern = StepOnce::apply(&mut runtime).await;
    assert_eq!(
        govern.applied_transition.as_ref().map(|step| step.kind),
        Some(StepTransitionKind::Govern)
    );
    assert_eq!(
        runtime.checkpoint.lifecycle.status,
        RunStatus::AwaitingSigner
    );

    let request_id = runtime
        .checkpoint
        .pending_requests
        .pending_signer_request_id
        .clone()
        .expect("pending signer request");
    runtime
        .pending_signer_state
        .as_mut()
        .expect("pending signer")
        .apply_resolution(SignerResolution {
            request_id: ais_agent_control::ids::SignerRequestId(request_id),
            kind: SignerResolutionKind::Signed,
            resolved_at_ms: None,
            submission_id: None,
            signed_payload: Some(json!({"raw_tx":"0x0102"})),
        });

    let signer = StepOnce::apply(&mut runtime).await;
    assert_eq!(
        signer.applied_transition.as_ref().map(|step| step.kind),
        Some(StepTransitionKind::Signer)
    );

    let broadcast = apply_live_evm_broadcast_with_provider(&mut runtime, &provider)
        .await
        .expect("broadcast");
    assert_eq!(broadcast.kind, StepTransitionKind::Broadcast);

    let verify = apply_live_evm_verify_with_provider(&mut runtime, &provider)
        .await
        .expect("verify");
    assert_eq!(verify.kind, StepTransitionKind::Verify);
    assert_eq!(
        runtime
            .checkpoint
            .action_graph
            .nodes
            .get("driver.verify.swap")
            .map(|node| node.status.clone()),
        Some(ActionNodeStatus::Succeeded),
        "effect evidence: {:?}, lifecycle: {:?}",
        runtime
            .checkpoint
            .evidence_graph
            .records
            .get("effect.driver.verify.swap")
            .map(|record| record.payload.clone()),
        runtime.checkpoint.lifecycle
    );

    let complete = StepOnce::apply(&mut runtime).await;
    assert_eq!(
        complete.applied_transition.as_ref().map(|step| step.kind),
        Some(StepTransitionKind::Complete)
    );
    assert_eq!(runtime.checkpoint.lifecycle.status, RunStatus::Completed);
}

#[tokio::test]
async fn reflection_output_binds_into_runtime_and_executes_same_actuate_verify_path() {
    let mission = sample_mission();
    let checkpoint = empty_checkpoint();
    let mut runtime = ActiveRun::new(mission.clone(), checkpoint);
    runtime.envelopes.insert(
        "env.reflect.swap".to_owned(),
        RuntimeEnvelope {
            envelope_id: "env.reflect.swap".to_owned(),
            kind: RuntimeEnvelopeKind::EvmEnvelope,
            chain: "eip155:1".to_owned(),
            payload: json!({"raw_tx":"0x0102"}),
            provenance: Some("reflect.test".to_owned()),
        },
    );

    let mut output = EvmAbiReflectionAdapter
        .build(&ReflectionRequest {
            mission,
            evidence: EvidenceGraph::default(),
            chain_family: ChainFamily::Evm,
            artifact_kind: ReflectionArtifactKind::EvmAbi,
            artifact: json!({"name":"Router","methods":["swapExactIn"]}),
            action_selector: "swapExactIn".to_owned(),
        })
        .expect("reflection output");

    // Force verify terminal on the fragment so runtime completion can be observed.
    output.fragment.terminals = vec!["reflect.evm.swapExactIn.verify".to_owned()];

    RuntimeDriverBinder::bind_output(
        &mut runtime,
        output,
        &DriverBindingContext::default()
            .with_evm_connection(
                "eip155:1",
                EvmConnectionSpec {
                    http_url: "http://example.invalid".to_owned(),
                    ws_url: None,
                },
            )
            .with_envelope_ref("reflect.evm.swapExactIn", "env.reflect.swap"),
    )
    .expect("reflection output should bind");

    match &runtime.checkpoint.action_graph.nodes["reflect.evm.swapExactIn.verify"].payload {
        ActionPayload::Verify(action) => match &action.live {
            Some(VerifyLiveBinding::Evm(live)) => {
                assert_eq!(live.binding, EvmVerifyBinding::EffectContractFromReceipt);
                assert!(live.connection.is_some());
            }
            other => panic!("unexpected live binding: {other:?}"),
        },
        other => panic!("unexpected payload: {other:?}"),
    }

    let asserter = Asserter::new();
    let provider = ProviderBuilder::new().connect_mocked_client(asserter.clone());
    let tx_hash = b256!("cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd");
    asserter.push_success(&bytes!(
        "0000000000000000000000000000000000000000000000000000000000000001"
    ));
    asserter.push_success(&tx_hash);
    asserter.push_success(&sample_receipt(tx_hash, 88));
    asserter.push_success(&92u64);
    asserter.push_success(&encode_u256_return(1));

    let simulate = apply_live_evm_simulate_with_provider(&mut runtime, &provider)
        .await
        .expect("simulate");
    assert_eq!(simulate.kind, StepTransitionKind::Simulate);

    let govern = StepOnce::apply(&mut runtime).await;
    assert_eq!(
        govern.applied_transition.as_ref().map(|step| step.kind),
        Some(StepTransitionKind::Govern)
    );
    assert_eq!(
        runtime.checkpoint.lifecycle.status,
        RunStatus::AwaitingSigner
    );

    let request_id = runtime
        .checkpoint
        .pending_requests
        .pending_signer_request_id
        .clone()
        .expect("pending signer request");
    runtime
        .pending_signer_state
        .as_mut()
        .expect("pending signer")
        .apply_resolution(SignerResolution {
            request_id: ais_agent_control::ids::SignerRequestId(request_id),
            kind: SignerResolutionKind::Signed,
            resolved_at_ms: None,
            submission_id: None,
            signed_payload: Some(json!({"raw_tx":"0x0102"})),
        });

    let signer = StepOnce::apply(&mut runtime).await;
    assert_eq!(
        signer.applied_transition.as_ref().map(|step| step.kind),
        Some(StepTransitionKind::Signer)
    );

    let broadcast = apply_live_evm_broadcast_with_provider(&mut runtime, &provider)
        .await
        .expect("broadcast");
    assert_eq!(broadcast.kind, StepTransitionKind::Broadcast);

    let verify = apply_live_evm_verify_with_provider(&mut runtime, &provider)
        .await
        .expect("verify");
    assert_eq!(verify.kind, StepTransitionKind::Verify);
    assert_eq!(
        runtime
            .checkpoint
            .action_graph
            .nodes
            .get("reflect.evm.swapExactIn.verify")
            .map(|node| node.status.clone()),
        Some(ActionNodeStatus::Succeeded),
        "effect evidence: {:?}, lifecycle: {:?}",
        runtime
            .checkpoint
            .evidence_graph
            .records
            .get("effect.reflect.evm.swapExactIn.verify")
            .map(|record| record.payload.clone()),
        runtime.checkpoint.lifecycle
    );
    assert_eq!(
        runtime
            .checkpoint
            .evidence_graph
            .records
            .get("effect.reflect.evm.swapExactIn.verify")
            .and_then(|record| record.payload.get("final_status"))
            .and_then(|value| value.as_str()),
        Some("satisfied")
    );

    let complete = StepOnce::apply(&mut runtime).await;
    assert_eq!(
        complete.applied_transition.as_ref().map(|step| step.kind),
        Some(StepTransitionKind::Complete)
    );
    assert_eq!(runtime.checkpoint.lifecycle.status, RunStatus::Completed);
}

#[tokio::test]
async fn api_native_direct_envelope_output_binds_into_same_guarded_path() {
    let mission = sample_mission();
    let checkpoint = empty_checkpoint();
    let mut runtime = ActiveRun::new(mission.clone(), checkpoint);

    let output = DirectEnvelopeApiAdapter
        .build(&ApiNativeRequest {
            mission,
            evidence: EvidenceGraph::default(),
            provider_id: "api-direct".to_owned(),
            provider_kind: ApiNativeProviderKind::DirectEnvelopeProvider,
            chain: Some("eip155:1".to_owned()),
            payload: json!({"provider":"direct"}),
            direct_envelope: Some(DirectEnvelopePayload::Evm(EvmNativeEnvelope {
                to: address!("1111111111111111111111111111111111111111"),
                data: Bytes::from_static(b"\x12\x34"),
                value: U256::from(0u64),
            })),
        })
        .expect("api-native direct envelope output");

    RuntimeDriverBinder::bind_api_native_output(
        &mut runtime,
        output,
        &DriverBindingContext::default().with_evm_connection(
            "eip155:1",
            EvmConnectionSpec {
                http_url: "http://example.invalid".to_owned(),
                ws_url: None,
            },
        ),
    )
    .expect("api-native output should bind");
    match &runtime.checkpoint.action_graph.nodes["api_native.api-direct.verify"].payload {
        ActionPayload::Verify(action) => match &action.live {
            Some(VerifyLiveBinding::Evm(live)) => {
                assert_eq!(live.binding, EvmVerifyBinding::EffectContractFromReceipt);
                assert!(live.connection.is_some());
            }
            other => panic!("unexpected live binding: {other:?}"),
        },
        other => panic!("unexpected payload: {other:?}"),
    }

    let asserter = Asserter::new();
    let provider = ProviderBuilder::new().connect_mocked_client(asserter.clone());
    let tx_hash = b256!("edededededededededededededededededededededededededededededededed");
    asserter.push_success(&bytes!(
        "0000000000000000000000000000000000000000000000000000000000000001"
    ));
    asserter.push_success(&tx_hash);
    asserter.push_success(&sample_receipt(tx_hash, 99));
    asserter.push_success(&100u64);

    let simulate = apply_live_evm_simulate_with_provider(&mut runtime, &provider)
        .await
        .expect("simulate");
    assert_eq!(simulate.kind, StepTransitionKind::Simulate);

    let govern = StepOnce::apply(&mut runtime).await;
    assert_eq!(
        govern.applied_transition.as_ref().map(|step| step.kind),
        Some(StepTransitionKind::Govern)
    );
    assert_eq!(
        runtime.checkpoint.lifecycle.status,
        RunStatus::AwaitingSigner
    );

    let request_id = runtime
        .checkpoint
        .pending_requests
        .pending_signer_request_id
        .clone()
        .expect("pending signer request");
    runtime
        .pending_signer_state
        .as_mut()
        .expect("pending signer")
        .apply_resolution(SignerResolution {
            request_id: ais_agent_control::ids::SignerRequestId(request_id),
            kind: SignerResolutionKind::Signed,
            resolved_at_ms: None,
            submission_id: None,
            signed_payload: Some(json!({"raw_tx":"0x0102"})),
        });

    let signer = StepOnce::apply(&mut runtime).await;
    assert_eq!(
        signer.applied_transition.as_ref().map(|step| step.kind),
        Some(StepTransitionKind::Signer)
    );

    let broadcast = apply_live_evm_broadcast_with_provider(&mut runtime, &provider)
        .await
        .expect("broadcast");
    assert_eq!(broadcast.kind, StepTransitionKind::Broadcast);

    let verify = apply_live_evm_verify_with_provider(&mut runtime, &provider)
        .await
        .expect("verify");
    assert_eq!(verify.kind, StepTransitionKind::Verify);
    assert_eq!(
        runtime
            .checkpoint
            .evidence_graph
            .records
            .get("effect.api_native.api-direct.verify")
            .and_then(|record| record.payload.get("final_status"))
            .and_then(|value| value.as_str()),
        Some("satisfied"),
        "effect payload: {:?}, node: {:?}, effects: {:?}, lifecycle: {:?}",
        runtime
            .checkpoint
            .evidence_graph
            .records
            .get("effect.api_native.api-direct.verify")
            .map(|record| record.payload.clone()),
        runtime
            .checkpoint
            .action_graph
            .nodes
            .get("api_native.api-direct.verify"),
        runtime
            .checkpoint
            .effect_contracts
            .keys()
            .collect::<Vec<_>>(),
        runtime.checkpoint.lifecycle
    );

    let complete = StepOnce::apply(&mut runtime).await;
    assert_eq!(
        complete.applied_transition.as_ref().map(|step| step.kind),
        Some(StepTransitionKind::Complete)
    );
    assert_eq!(runtime.checkpoint.lifecycle.status, RunStatus::Completed);
}

#[test]
fn raw_envelope_binding_requires_effect_contract() {
    let mission = sample_mission();
    let checkpoint = empty_checkpoint();
    let mut runtime = ActiveRun::new(mission, checkpoint);
    runtime.envelopes.insert(
        "env.raw".to_owned(),
        RuntimeEnvelope {
            envelope_id: "env.raw".to_owned(),
            kind: RuntimeEnvelopeKind::EvmEnvelope,
            chain: "eip155:1".to_owned(),
            payload: json!({"raw_tx":"0xfeedbeef"}),
            provenance: Some("raw.test".to_owned()),
        },
    );

    let error = RuntimeDriverBinder::bind_raw_envelope_path(
        &mut runtime,
        RawEnvelopeBindingRequest {
            node_prefix: "raw.swap".to_owned(),
            envelope_ref: "env.raw".to_owned(),
            effect_contract_ref: None,
            actuator_hint: "raw broadcast".to_owned(),
            depends_on: vec!["simulate.raw".to_owned()],
        },
        &DriverBindingContext::default().with_evm_connection(
            "eip155:1",
            EvmConnectionSpec {
                http_url: "http://example.invalid".to_owned(),
                ws_url: None,
            },
        ),
    )
    .expect_err("raw envelope binding should require effect contract");

    assert!(matches!(
        error,
        crate::runtime::RuntimeDriverBindingError::RawEnvelope(_)
    ));
}

#[tokio::test]
async fn raw_envelope_binding_executes_under_same_guarded_contract() {
    let mission = sample_mission();
    let checkpoint = empty_checkpoint();
    let mut runtime = ActiveRun::new(mission, checkpoint);
    runtime.envelopes.insert(
        "env.raw".to_owned(),
        RuntimeEnvelope {
            envelope_id: "env.raw".to_owned(),
            kind: RuntimeEnvelopeKind::EvmEnvelope,
            chain: "eip155:1".to_owned(),
            payload: json!({"raw_tx":"0xfeedbeef"}),
            provenance: Some("raw.test".to_owned()),
        },
    );
    runtime.checkpoint.effect_contracts.insert(
        "effects.raw.swap".to_owned(),
        EffectContract {
            effect_id: "effects.raw.swap".to_owned(),
            kind: EffectContractKind::StateTransition,
            assertions: vec![EffectAssertion {
                expression: "receipt != null".to_owned(),
                description: "raw envelope should yield a receipt".to_owned(),
            }],
            tolerance_hint: Some("receipt_required".to_owned()),
        },
    );
    runtime.checkpoint.action_graph.nodes.insert(
        "simulate.raw".to_owned(),
        raw_simulate_eth_call_node("simulate.raw"),
    );
    runtime
        .checkpoint
        .action_graph
        .roots
        .push("simulate.raw".to_owned());

    RuntimeDriverBinder::bind_raw_envelope_path(
        &mut runtime,
        RawEnvelopeBindingRequest {
            node_prefix: "raw.swap".to_owned(),
            envelope_ref: "env.raw".to_owned(),
            effect_contract_ref: Some("effects.raw.swap".to_owned()),
            actuator_hint: "raw broadcast".to_owned(),
            depends_on: vec!["simulate.raw".to_owned()],
        },
        &DriverBindingContext::default().with_evm_connection(
            "eip155:1",
            EvmConnectionSpec {
                http_url: "http://example.invalid".to_owned(),
                ws_url: None,
            },
        ),
    )
    .expect("raw envelope binding should bind");
    match &runtime.checkpoint.action_graph.nodes["raw.swap.verify"].payload {
        ActionPayload::Verify(action) => match &action.live {
            Some(VerifyLiveBinding::Evm(live)) => {
                assert_eq!(live.binding, EvmVerifyBinding::EffectContractFromReceipt);
                assert!(live.connection.is_some());
            }
            other => panic!("unexpected live binding: {other:?}"),
        },
        other => panic!("unexpected payload: {other:?}"),
    }

    let asserter = Asserter::new();
    let provider = ProviderBuilder::new().connect_mocked_client(asserter.clone());
    let tx_hash = b256!("efefefefefefefefefefefefefefefefefefefefefefefefefefefefefefefef");
    asserter.push_success(&bytes!(
        "0000000000000000000000000000000000000000000000000000000000000001"
    ));
    asserter.push_success(&tx_hash);
    asserter.push_success(&sample_receipt(tx_hash, 111));
    asserter.push_success(&112u64);

    let simulate = apply_live_evm_simulate_with_provider(&mut runtime, &provider)
        .await
        .expect("simulate");
    assert_eq!(simulate.kind, StepTransitionKind::Simulate);

    let govern = StepOnce::apply(&mut runtime).await;
    assert_eq!(
        govern.applied_transition.as_ref().map(|step| step.kind),
        Some(StepTransitionKind::Govern)
    );

    let request_id = runtime
        .checkpoint
        .pending_requests
        .pending_signer_request_id
        .clone()
        .expect("pending signer request");
    runtime
        .pending_signer_state
        .as_mut()
        .expect("pending signer")
        .apply_resolution(SignerResolution {
            request_id: ais_agent_control::ids::SignerRequestId(request_id),
            kind: SignerResolutionKind::Signed,
            resolved_at_ms: None,
            submission_id: None,
            signed_payload: Some(json!({"raw_tx":"0xfeedbeef"})),
        });
    StepOnce::apply(&mut runtime).await;

    let broadcast = apply_live_evm_broadcast_with_provider(&mut runtime, &provider)
        .await
        .expect("broadcast");
    assert_eq!(broadcast.kind, StepTransitionKind::Broadcast);

    let verify = apply_live_evm_verify_with_provider(&mut runtime, &provider)
        .await
        .expect("verify");
    assert_eq!(verify.kind, StepTransitionKind::Verify);
    assert_eq!(
        runtime
            .checkpoint
            .evidence_graph
            .records
            .get("effect.raw.swap.verify")
            .and_then(|record| record.payload.get("final_status"))
            .and_then(|value| value.as_str()),
        Some("satisfied"),
        "effect payload: {:?}, node: {:?}, effects: {:?}, lifecycle: {:?}",
        runtime
            .checkpoint
            .evidence_graph
            .records
            .get("effect.raw.swap.verify")
            .map(|record| record.payload.clone()),
        runtime.checkpoint.action_graph.nodes.get("raw.swap.verify"),
        runtime
            .checkpoint
            .effect_contracts
            .keys()
            .collect::<Vec<_>>(),
        runtime.checkpoint.lifecycle
    );
}

fn standard_like_driver_output() -> DriverBuildOutput {
    let mut nodes = BTreeMap::new();
    nodes.insert(
        "driver.observe.pre".to_owned(),
        ActionNode {
            node_id: "driver.observe.pre".to_owned(),
            kind: ActionNodeKind::Observe,
            origin: ActionOrigin::DriverFragment,
            status: ActionNodeStatus::Pending,
            depends_on: Vec::new(),
            inputs: Vec::new(),
            evidence_refs: Vec::new(),
            payload: ActionPayload::Observe(ObserveAction {
                source_kind: ObserveSourceKind::ChainRead,
                source_hint: "driver observe pre-balance".to_owned(),
                output_key: Some("driver.pre.balance".to_owned()),
                live: None,
            }),
            implementation_hint: Some("driver.mock.observe".to_owned()),
            expected_effect_ref: None,
        },
    );
    nodes.insert(
        "driver.simulate.swap".to_owned(),
        ActionNode {
            node_id: "driver.simulate.swap".to_owned(),
            kind: ActionNodeKind::Simulate,
            origin: ActionOrigin::DriverFragment,
            status: ActionNodeStatus::Pending,
            depends_on: vec!["driver.observe.pre".to_owned()],
            inputs: Vec::new(),
            evidence_refs: Vec::new(),
            payload: ActionPayload::Simulate(SimulateAction {
                simulate_kind: SimulateKind::Call,
                simulator_hint: "driver simulate swap".to_owned(),
                live: None,
            }),
            implementation_hint: Some("driver.mock.simulate".to_owned()),
            expected_effect_ref: None,
        },
    );
    nodes.insert(
        "driver.actuate.swap".to_owned(),
        ActionNode {
            node_id: "driver.actuate.swap".to_owned(),
            kind: ActionNodeKind::Actuate,
            origin: ActionOrigin::DriverFragment,
            status: ActionNodeStatus::Pending,
            depends_on: vec!["driver.simulate.swap".to_owned()],
            inputs: Vec::new(),
            evidence_refs: Vec::new(),
            payload: ActionPayload::Actuate(ActuateAction {
                mode: ActuateMode::DriverCall,
                actuator_hint: "driver broadcast swap".to_owned(),
                chain: Some("eip155:1".to_owned()),
                envelope_ref: None,
                requires_effect_contract: true,
                live: None,
            }),
            implementation_hint: Some("driver.mock.broadcast".to_owned()),
            expected_effect_ref: Some("effects.driver.swap".to_owned()),
        },
    );
    nodes.insert(
        "driver.verify.swap".to_owned(),
        ActionNode {
            node_id: "driver.verify.swap".to_owned(),
            kind: ActionNodeKind::Verify,
            origin: ActionOrigin::DriverFragment,
            status: ActionNodeStatus::Pending,
            depends_on: vec!["driver.actuate.swap".to_owned()],
            inputs: Vec::new(),
            evidence_refs: Vec::new(),
            payload: ActionPayload::Verify(VerifyAction {
                verify_kind: VerifyKind::EffectContract,
                verifier_hint: "driver verify swap effect".to_owned(),
                pre_observation_ref: None,
                post_observation_ref: Some("post.driver.swap".to_owned()),
                live: Some(VerifyLiveBinding::Evm(EvmVerifyLiveBinding {
                    connection: None,
                    binding: EvmVerifyBinding::ReceiptStatus,
                    post_request: Some(EvmObserveRequest::Erc20BalanceOf {
                        token: address!("3333333333333333333333333333333333333333"),
                        owner: address!("4444444444444444444444444444444444444444"),
                    }),
                })),
            }),
            implementation_hint: Some("driver.mock.verify".to_owned()),
            expected_effect_ref: Some("effects.driver.swap".to_owned()),
        },
    );

    DriverBuildOutput {
        fragment: ActionGraphFragment {
            roots: vec!["driver.observe.pre".to_owned()],
            terminals: vec!["driver.verify.swap".to_owned()],
            nodes,
            live_binding_hints: BTreeMap::from([
                (
                    "driver.observe.pre".to_owned(),
                    DriverNodeLiveBindingHint::EvmObserve(DriverEvmObserveHint {
                        binding: EvmObserveBinding::NativeBalance,
                        request: EvmObserveRequest::NativeBalance {
                            address: address!("1111111111111111111111111111111111111111"),
                        },
                    }),
                ),
                (
                    "driver.simulate.swap".to_owned(),
                    DriverNodeLiveBindingHint::EvmSimulate(DriverEvmSimulateHint {
                        binding: EvmSimulateBinding::EthCall,
                        request: EvmCallRequest {
                            from: None,
                            to: address!("2222222222222222222222222222222222222222"),
                            data: bytes!("06fdde03"),
                            value: None,
                        },
                    }),
                ),
                (
                    "driver.actuate.swap".to_owned(),
                    DriverNodeLiveBindingHint::EvmActuate(DriverEvmActuateHint {
                        binding: EvmActuateBinding::BroadcastSignedEnvelope,
                    }),
                ),
                (
                    "driver.verify.swap".to_owned(),
                    DriverNodeLiveBindingHint::EvmVerify(DriverEvmVerifyHint {
                        binding: EvmVerifyBinding::EffectContractFromReceipt,
                        post_evm_request: None,
                    }),
                ),
            ]),
        },
        evidence_requirements: vec![EvidenceRequirement {
            requirement_id: "quote".to_owned(),
            reference: "evidence.quote".to_owned(),
            reason: "driver requires quote evidence".to_owned(),
            required_by_node_id: Some("driver.simulate.swap".to_owned()),
            satisfied_by_evidence_id: None,
        }],
        effect_contracts: vec![EffectContract {
            effect_id: "effects.driver.swap".to_owned(),
            kind: EffectContractKind::StateTransition,
            assertions: vec![EffectAssertion {
                expression: "post.decoded_u256 == \"120\"".to_owned(),
                description: "broadcasted swap should yield expected post-state".to_owned(),
            }],
            tolerance_hint: Some("receipt_required".to_owned()),
        }],
    }
}

fn raw_simulate_eth_call_node(node_id: &str) -> ActionNode {
    ActionNode {
        node_id: node_id.to_owned(),
        kind: ActionNodeKind::Simulate,
        origin: ActionOrigin::RawEnvelopePath,
        status: ActionNodeStatus::Pending,
        depends_on: Vec::new(),
        inputs: Vec::new(),
        evidence_refs: Vec::new(),
        payload: ActionPayload::Simulate(SimulateAction {
            simulate_kind: SimulateKind::Call,
            simulator_hint: "raw envelope preflight".to_owned(),
            live: Some(SimulateLiveBinding::Evm(EvmSimulateLiveBinding {
                connection: Some(EvmConnectionSpec {
                    http_url: "http://example.invalid".to_owned(),
                    ws_url: None,
                }),
                binding: EvmSimulateBinding::EthCall,
                request: EvmCallRequest {
                    from: None,
                    to: address!("1111111111111111111111111111111111111111"),
                    data: bytes!("1234"),
                    value: None,
                },
            })),
        }),
        implementation_hint: Some("raw.preflight".to_owned()),
        expected_effect_ref: None,
    }
}

fn sample_mission() -> Mission {
    Mission {
        mission_id: "mission-1".to_owned(),
        goal: "driver binding".to_owned(),
        allowed_chains: vec!["eip155:1".to_owned()],
        budget: MissionBudget {
            max_steps: Some(8),
            max_signer_requests: Some(1),
            max_wall_clock_ms: Some(30_000),
        },
        policy: MissionPolicy {
            policy_mode: Some("guarded".to_owned()),
            allow_raw_envelopes: true,
            require_effect_contract_for_writes: true,
        },
        constraints: BTreeMap::new(),
        metadata: BTreeMap::new(),
    }
}

fn empty_checkpoint() -> CheckpointSnapshot {
    let mut lifecycle = RunLifecycleState::new(RunId("run-driver".to_owned()), "mission-1");
    lifecycle.mark_running(RunPhase::Planning);

    CheckpointSnapshot {
        run_id: "run-driver".to_owned(),
        mission_id: "mission-1".to_owned(),
        checkpoint_seq: 0,
        plan_epoch: 0,
        lifecycle,
        action_graph: ActionGraph {
            graph_id: Some("graph-driver".to_owned()),
            roots: Vec::new(),
            terminals: Vec::new(),
            nodes: BTreeMap::new(),
        },
        evidence_graph: EvidenceGraph::default(),
        effect_contracts: BTreeMap::new(),
        pending_requests: PendingRequestsSnapshot::default(),
        last_completed_node_id: None,
        actuation_records: Vec::new(),
        execution_artifact: None,
    }
}

fn sample_receipt(tx_hash: alloy::primitives::B256, block_number: u64) -> TransactionReceipt {
    TransactionReceipt {
        inner: ReceiptEnvelope::Eip1559(
            Receipt {
                status: true.into(),
                cumulative_gas_used: 21_000,
                logs: Vec::new(),
            }
            .with_bloom(),
        ),
        transaction_hash: tx_hash,
        transaction_index: Some(0),
        block_hash: Some(b256!(
            "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"
        )),
        block_number: Some(block_number),
        gas_used: 21_000,
        effective_gas_price: 1,
        blob_gas_used: None,
        blob_gas_price: None,
        from: address!("1111111111111111111111111111111111111111"),
        to: Some(address!("2222222222222222222222222222222222222222")),
        contract_address: None,
    }
}

fn encode_u256_return(value: u64) -> alloy::primitives::Bytes {
    let mut word = [0u8; 32];
    word[24..].copy_from_slice(&value.to_be_bytes());
    alloy::primitives::Bytes::from(word.to_vec())
}
