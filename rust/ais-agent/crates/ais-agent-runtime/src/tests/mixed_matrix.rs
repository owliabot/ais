use std::collections::BTreeMap;

use ais_agent_chain_shared::{
    ChainFamily, ReflectionArtifactKind, ReflectionDriver, ReflectionRequest,
};
use ais_agent_control::ids::RunId;
use ais_agent_core::{
    action::{
        kinds::{
            actuate::{ActuateAction, ActuateLiveBinding, ActuateMode},
            simulate::{EvmSimulateLiveBinding, SimulateAction, SimulateKind, SimulateLiveBinding},
            verify::{VerifyAction, VerifyKind, VerifyLiveBinding},
        },
        ActionGraph, ActionNode, ActionNodeKind, ActionNodeStatus, ActionOrigin, ActionPayload,
    },
    binding::evm::{
        EvmActuateBinding, EvmCallRequest, EvmConnectionSpec, EvmSimulateBinding, EvmVerifyBinding,
    },
    checkpoint::{CheckpointSnapshot, PendingRequestsSnapshot},
    driver::{
        ActionGraphFragment, DriverBuildOutput, DriverEvmActuateHint, DriverEvmSimulateHint,
        DriverEvmVerifyHint, DriverNodeLiveBindingHint,
    },
    effect::{EffectAssertion, EffectContract, EffectContractKind},
    envelope::{RuntimeEnvelope, RuntimeEnvelopeKind},
    evidence::EvidenceGraph,
    mission::{Mission, MissionBudget, MissionPolicy},
    runtime::{RunLifecycleState, RunPhase},
};
use ais_agent_drivers::api_native::{
    ApiNativeAdapter, ApiNativeProviderKind, ApiNativeRequest, DirectEnvelopeApiAdapter,
    DirectEnvelopePayload, EvmNativeEnvelope,
};
use ais_agent_evm::reflect::EvmAbiReflectionAdapter;
use alloy::primitives::{address, bytes, Bytes, U256};
use serde_json::json;

use crate::runtime::{
    ActiveRun, DriverBindingContext, RawEnvelopeBindingRequest, RuntimeDriverBinder,
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct GuardedPathSignature {
    has_simulate: bool,
    has_actuate: bool,
    has_verify: bool,
    simulate_precedes_actuate: bool,
    verify_depends_on_actuate: bool,
    actuate_requires_effect_contract: bool,
    actuate_has_evm_binding: bool,
    actuate_has_connection: bool,
    verify_is_effect_contract: bool,
    verify_has_evm_binding: bool,
    verify_has_connection: bool,
    effect_contract_attached: bool,
    terminal_is_verify: bool,
}

#[test]
fn standard_reflection_api_native_and_raw_envelope_paths_share_guarded_runtime_signature() {
    let standard = bind_standard_runtime();
    let reflection = bind_reflection_runtime();
    let api_native = bind_api_native_runtime();
    let raw = bind_raw_runtime();

    let standard_sig = extract_signature(&standard, "std.matrix");
    let reflection_sig = extract_signature(&reflection, "reflect.evm.swapExactIn");
    let api_native_sig = extract_signature(&api_native, "api_native.matrix-direct");
    let raw_sig = extract_signature(&raw, "raw.matrix");

    assert_eq!(standard_sig, reflection_sig);
    assert_eq!(standard_sig, api_native_sig);
    assert_eq!(standard_sig, raw_sig);
}

fn bind_standard_runtime() -> ActiveRun {
    let mut runtime = base_runtime();
    runtime.envelopes.insert(
        "env.standard".to_owned(),
        RuntimeEnvelope {
            envelope_id: "env.standard".to_owned(),
            kind: RuntimeEnvelopeKind::EvmEnvelope,
            chain: "eip155:1".to_owned(),
            payload: json!({"raw_tx":"0xfeedbeef"}),
            provenance: Some("standard".to_owned()),
        },
    );

    RuntimeDriverBinder::bind_output(
        &mut runtime,
        standard_like_output(),
        &binding_ctx().with_envelope_ref("std.matrix.actuate", "env.standard"),
    )
    .expect("standard output should bind");

    runtime
}

fn bind_reflection_runtime() -> ActiveRun {
    let mut runtime = base_runtime();
    runtime.envelopes.insert(
        "env.reflect".to_owned(),
        RuntimeEnvelope {
            envelope_id: "env.reflect".to_owned(),
            kind: RuntimeEnvelopeKind::EvmEnvelope,
            chain: "eip155:1".to_owned(),
            payload: json!({"raw_tx":"0xfeedbeef"}),
            provenance: Some("reflection".to_owned()),
        },
    );

    let mut output = EvmAbiReflectionAdapter
        .build(&ReflectionRequest {
            mission: sample_mission(),
            evidence: EvidenceGraph::default(),
            chain_family: ChainFamily::Evm,
            artifact_kind: ReflectionArtifactKind::EvmAbi,
            artifact: json!({"name":"Router","methods":["swapExactIn"]}),
            action_selector: "swapExactIn".to_owned(),
        })
        .expect("reflection output");
    output.fragment.terminals = vec!["reflect.evm.swapExactIn.verify".to_owned()];

    RuntimeDriverBinder::bind_output(
        &mut runtime,
        output,
        &binding_ctx().with_envelope_ref("reflect.evm.swapExactIn", "env.reflect"),
    )
    .expect("reflection output should bind");

    runtime
}

fn bind_api_native_runtime() -> ActiveRun {
    let mut runtime = base_runtime();
    let output = DirectEnvelopeApiAdapter
        .build(&ApiNativeRequest {
            mission: sample_mission(),
            evidence: EvidenceGraph::default(),
            provider_id: "matrix-direct".to_owned(),
            provider_kind: ApiNativeProviderKind::DirectEnvelopeProvider,
            chain: Some("eip155:1".to_owned()),
            payload: json!({"provider":"direct"}),
            direct_envelope: Some(DirectEnvelopePayload::Evm(EvmNativeEnvelope {
                to: address!("1111111111111111111111111111111111111111"),
                data: Bytes::from_static(b"\x12\x34"),
                value: U256::from(0u64),
            })),
        })
        .expect("api-native output");

    RuntimeDriverBinder::bind_api_native_output(&mut runtime, output, &binding_ctx())
        .expect("api-native output should bind");

    runtime
}

fn bind_raw_runtime() -> ActiveRun {
    let mut runtime = base_runtime();
    runtime.envelopes.insert(
        "env.raw".to_owned(),
        RuntimeEnvelope {
            envelope_id: "env.raw".to_owned(),
            kind: RuntimeEnvelopeKind::EvmEnvelope,
            chain: "eip155:1".to_owned(),
            payload: json!({"raw_tx":"0xfeedbeef"}),
            provenance: Some("raw".to_owned()),
        },
    );
    runtime.checkpoint.effect_contracts.insert(
        "effects.raw.matrix".to_owned(),
        EffectContract {
            effect_id: "effects.raw.matrix".to_owned(),
            kind: EffectContractKind::StateTransition,
            assertions: vec![EffectAssertion {
                expression: "receipt != null".to_owned(),
                description: "raw matrix path should yield a receipt".to_owned(),
            }],
            tolerance_hint: Some("receipt_required".to_owned()),
        },
    );

    RuntimeDriverBinder::bind_raw_envelope_path(
        &mut runtime,
        RawEnvelopeBindingRequest {
            node_prefix: "raw.matrix".to_owned(),
            envelope_ref: "env.raw".to_owned(),
            effect_contract_ref: Some("effects.raw.matrix".to_owned()),
            actuator_hint: "raw matrix broadcast".to_owned(),
            depends_on: vec!["raw.matrix.simulate".to_owned()],
        },
        &binding_ctx(),
    )
    .expect("raw envelope path should bind");

    runtime.checkpoint.action_graph.nodes.insert(
        "raw.matrix.simulate".to_owned(),
        raw_simulate_node("raw.matrix.simulate"),
    );
    runtime
        .checkpoint
        .action_graph
        .roots
        .push("raw.matrix.simulate".to_owned());

    runtime
}

fn extract_signature(runtime: &ActiveRun, prefix: &str) -> GuardedPathSignature {
    let simulate_id = format!("{prefix}.simulate");
    let actuate_id = if runtime
        .checkpoint
        .action_graph
        .nodes
        .contains_key(&format!("{prefix}.actuate"))
    {
        format!("{prefix}.actuate")
    } else {
        prefix.to_owned()
    };
    let verify_id = format!("{prefix}.verify");

    let simulate = runtime.checkpoint.action_graph.nodes.get(&simulate_id);
    let actuate = runtime
        .checkpoint
        .action_graph
        .nodes
        .get(&actuate_id)
        .expect("actuate node");
    let verify = runtime
        .checkpoint
        .action_graph
        .nodes
        .get(&verify_id)
        .expect("verify node");

    let ActionPayload::Actuate(actuate_payload) = &actuate.payload else {
        panic!("expected actuate payload");
    };
    let ActionPayload::Verify(verify_payload) = &verify.payload else {
        panic!("expected verify payload");
    };

    GuardedPathSignature {
        has_simulate: simulate.is_some(),
        has_actuate: true,
        has_verify: true,
        simulate_precedes_actuate: actuate.depends_on.contains(&simulate_id),
        verify_depends_on_actuate: verify.depends_on == vec![actuate_id],
        actuate_requires_effect_contract: actuate_payload.requires_effect_contract,
        actuate_has_evm_binding: matches!(&actuate_payload.live, Some(ActuateLiveBinding::Evm(_))),
        actuate_has_connection: matches!(
            &actuate_payload.live,
            Some(ActuateLiveBinding::Evm(live)) if live.connection.is_some()
        ),
        verify_is_effect_contract: verify_payload.verify_kind == VerifyKind::EffectContract,
        verify_has_evm_binding: matches!(&verify_payload.live, Some(VerifyLiveBinding::Evm(_))),
        verify_has_connection: matches!(
            &verify_payload.live,
            Some(VerifyLiveBinding::Evm(live)) if live.connection.is_some()
        ),
        effect_contract_attached: actuate.expected_effect_ref.is_some()
            && verify.expected_effect_ref.is_some(),
        terminal_is_verify: runtime.checkpoint.action_graph.terminals == vec![verify_id],
    }
}

fn standard_like_output() -> DriverBuildOutput {
    let mut nodes = BTreeMap::new();
    nodes.insert(
        "std.matrix.simulate".to_owned(),
        ActionNode {
            node_id: "std.matrix.simulate".to_owned(),
            kind: ActionNodeKind::Simulate,
            origin: ActionOrigin::DriverFragment,
            status: ActionNodeStatus::Pending,
            depends_on: Vec::new(),
            inputs: Vec::new(),
            evidence_refs: Vec::new(),
            payload: ActionPayload::Simulate(SimulateAction {
                simulate_kind: SimulateKind::Call,
                simulator_hint: "standard matrix simulate".to_owned(),
                live: None,
            }),
            implementation_hint: Some("standard.matrix".to_owned()),
            expected_effect_ref: None,
        },
    );
    nodes.insert(
        "std.matrix.actuate".to_owned(),
        ActionNode {
            node_id: "std.matrix.actuate".to_owned(),
            kind: ActionNodeKind::Actuate,
            origin: ActionOrigin::DriverFragment,
            status: ActionNodeStatus::Pending,
            depends_on: vec!["std.matrix.simulate".to_owned()],
            inputs: Vec::new(),
            evidence_refs: Vec::new(),
            payload: ActionPayload::Actuate(ActuateAction {
                mode: ActuateMode::DriverCall,
                actuator_hint: "standard matrix broadcast".to_owned(),
                chain: Some("eip155:1".to_owned()),
                envelope_ref: None,
                requires_effect_contract: true,
                live: None,
            }),
            implementation_hint: Some("standard.matrix".to_owned()),
            expected_effect_ref: Some("effects.std.matrix".to_owned()),
        },
    );
    nodes.insert(
        "std.matrix.verify".to_owned(),
        ActionNode {
            node_id: "std.matrix.verify".to_owned(),
            kind: ActionNodeKind::Verify,
            origin: ActionOrigin::DriverFragment,
            status: ActionNodeStatus::Pending,
            depends_on: vec!["std.matrix.actuate".to_owned()],
            inputs: Vec::new(),
            evidence_refs: Vec::new(),
            payload: ActionPayload::Verify(VerifyAction {
                verify_kind: VerifyKind::EffectContract,
                verifier_hint: "standard matrix verify".to_owned(),
                pre_observation_ref: None,
                post_observation_ref: None,
                live: None,
            }),
            implementation_hint: Some("standard.matrix".to_owned()),
            expected_effect_ref: Some("effects.std.matrix".to_owned()),
        },
    );

    DriverBuildOutput {
        fragment: ActionGraphFragment {
            roots: vec!["std.matrix.simulate".to_owned()],
            terminals: vec!["std.matrix.verify".to_owned()],
            nodes,
            live_binding_hints: BTreeMap::from([
                (
                    "std.matrix.simulate".to_owned(),
                    DriverNodeLiveBindingHint::EvmSimulate(DriverEvmSimulateHint {
                        binding: EvmSimulateBinding::EthCall,
                        request: EvmCallRequest {
                            from: None,
                            to: address!("1111111111111111111111111111111111111111"),
                            data: bytes!("1234"),
                            value: None,
                        },
                    }),
                ),
                (
                    "std.matrix.actuate".to_owned(),
                    DriverNodeLiveBindingHint::EvmActuate(DriverEvmActuateHint {
                        binding: EvmActuateBinding::BroadcastSignedEnvelope,
                    }),
                ),
                (
                    "std.matrix.verify".to_owned(),
                    DriverNodeLiveBindingHint::EvmVerify(DriverEvmVerifyHint {
                        binding: EvmVerifyBinding::EffectContractFromReceipt,
                        post_evm_request: None,
                    }),
                ),
            ]),
        },
        evidence_requirements: Vec::new(),
        effect_contracts: vec![EffectContract {
            effect_id: "effects.std.matrix".to_owned(),
            kind: EffectContractKind::StateTransition,
            assertions: vec![EffectAssertion {
                expression: "receipt != null".to_owned(),
                description: "standard matrix path should yield receipt".to_owned(),
            }],
            tolerance_hint: Some("receipt_required".to_owned()),
        }],
    }
}

fn raw_simulate_node(node_id: &str) -> ActionNode {
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
            simulator_hint: "raw matrix simulate".to_owned(),
            live: Some(SimulateLiveBinding::Evm(EvmSimulateLiveBinding {
                connection: Some(EvmConnectionSpec {
                    rpc_url: "http://example.invalid".to_owned(),
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
        implementation_hint: Some("raw.matrix".to_owned()),
        expected_effect_ref: None,
    }
}

fn binding_ctx() -> DriverBindingContext {
    DriverBindingContext::default().with_evm_connection(
        "eip155:1",
        EvmConnectionSpec {
            rpc_url: "http://example.invalid".to_owned(),
        },
    )
}

fn base_runtime() -> ActiveRun {
    ActiveRun::new(sample_mission(), empty_checkpoint())
}

fn sample_mission() -> Mission {
    Mission {
        mission_id: "mission-matrix".to_owned(),
        goal: "mixed path matrix".to_owned(),
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
    let mut lifecycle = RunLifecycleState::new(RunId("run-matrix".to_owned()), "mission-matrix");
    lifecycle.mark_running(RunPhase::Planning);

    CheckpointSnapshot {
        run_id: "run-matrix".to_owned(),
        mission_id: "mission-matrix".to_owned(),
        checkpoint_seq: 0,
        plan_epoch: 0,
        lifecycle,
        action_graph: ActionGraph {
            graph_id: Some("graph-matrix".to_owned()),
            roots: Vec::new(),
            terminals: Vec::new(),
            nodes: BTreeMap::new(),
        },
        evidence_graph: EvidenceGraph::default(),
        effect_contracts: BTreeMap::new(),
        pending_requests: PendingRequestsSnapshot::default(),
        last_completed_node_id: None,
        actuation_records: Vec::new(),
    }
}
