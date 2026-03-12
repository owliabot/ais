use std::collections::BTreeMap;

use ais_agent_control::ids::RunId;
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
        ActionGraph, ActionNode, ActionNodeKind, ActionNodeStatus, ActionOrigin, ActionPayload,
    },
    actuation::ActuationKind,
    binding::evm::{
        EvmActuateBinding, EvmCallRequest, EvmConnectionSpec, EvmObserveBinding, EvmObserveRequest,
        EvmSimulateBinding, EvmVerifyBinding,
    },
    checkpoint::{CheckpointSnapshot, PendingRequestsSnapshot},
    effect::{EffectAssertion, EffectContract, EffectContractKind},
    envelope::{RuntimeEnvelope, RuntimeEnvelopeKind},
    evidence::{
        EvidenceFreshness, EvidenceGraph, EvidenceKind, EvidenceProvenance, EvidenceRecord,
    },
    mission::{Mission, MissionBudget, MissionPolicy},
    runtime::{RunLifecycleState, RunPhase},
};
use alloy::{
    consensus::{Receipt, ReceiptEnvelope},
    primitives::{address, b256, bytes},
    providers::ProviderBuilder,
    rpc::types::TransactionReceipt,
    transports::mock::Asserter,
};
use serde_json::json;

use crate::{
    persistence::{
        persist_side_effect_checkpoint, restore_active_run, restore_active_run_from_parts,
        InMemoryCheckpointRepository, InMemoryMissionRepository, InMemorySignerStateArchive,
        MissionRepository,
    },
    runtime::ActiveRun,
    stepper::{
        apply_live_evm_broadcast_with_provider, apply_live_evm_observe_with_provider,
        apply_live_evm_simulate_with_provider, apply_live_evm_verify_with_provider, StepOnce,
        StepTransitionKind,
    },
};

#[tokio::test]
async fn runtime_observe_node_can_emit_machine_readable_evidence_via_alloy_provider() {
    let asserter = Asserter::new();
    let provider = ProviderBuilder::new().connect_mocked_client(asserter.clone());
    asserter.push_success(&777u64);

    let checkpoint = checkpoint_with_nodes(vec![observe_block_number_node("observe-block")]);
    let mission = sample_mission();
    let mut runtime = ActiveRun::new(mission, checkpoint);

    let transition = apply_live_evm_observe_with_provider(&mut runtime, &provider)
        .await
        .expect("observe transition");

    assert_eq!(transition.node_id.as_deref(), Some("observe-block"));
    assert_eq!(
        runtime
            .checkpoint
            .action_graph
            .nodes
            .get("observe-block")
            .map(|node| node.status.clone()),
        Some(ActionNodeStatus::Succeeded)
    );

    let payload = runtime
        .checkpoint
        .evidence_graph
        .records
        .get("observed.block_number")
        .expect("evidence")
        .payload
        .clone();
    assert_eq!(payload["block_number"], 777);
    assert_eq!(payload["source_hint"], "alloy_provider:get_block_number");
}

#[tokio::test]
async fn runtime_simulate_node_can_emit_machine_readable_report_via_alloy_provider() {
    let asserter = Asserter::new();
    let provider = ProviderBuilder::new().connect_mocked_client(asserter.clone());
    let return_data = bytes!("0000000000000000000000000000000000000000000000000000000000000042");
    asserter.push_success(&return_data);

    let checkpoint = checkpoint_with_nodes(vec![simulate_eth_call_node("simulate-call")]);
    let mission = sample_mission();
    let mut runtime = ActiveRun::new(mission, checkpoint);

    let transition = apply_live_evm_simulate_with_provider(&mut runtime, &provider)
        .await
        .expect("simulate transition");

    assert_eq!(transition.node_id.as_deref(), Some("simulate-call"));
    assert_eq!(
        runtime
            .checkpoint
            .action_graph
            .nodes
            .get("simulate-call")
            .map(|node| node.status.clone()),
        Some(ActionNodeStatus::Succeeded)
    );

    let payload = runtime
        .checkpoint
        .evidence_graph
        .records
        .get("simulation.simulate-call")
        .expect("simulation record")
        .payload
        .clone();
    assert_eq!(payload["accepted"], true);
    assert_eq!(payload["source_hint"], "alloy_provider:eth_call");
}

#[tokio::test]
async fn runtime_broadcast_node_can_submit_live_tx_hash_and_enter_confirmation_wait() {
    let asserter = Asserter::new();
    let provider = ProviderBuilder::new().connect_mocked_client(asserter.clone());
    let tx_hash = b256!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    asserter.push_success(&tx_hash);

    let checkpoint = checkpoint_with_nodes(vec![broadcast_swap_node("broadcast-swap")]);
    let mission = sample_mission();
    let mut runtime = ActiveRun::new(mission, checkpoint);
    runtime.envelopes.insert(
        "env.swap".to_owned(),
        RuntimeEnvelope {
            envelope_id: "env.swap".to_owned(),
            kind: RuntimeEnvelopeKind::EvmEnvelope,
            chain: "eip155:1".to_owned(),
            payload: serde_json::json!({"raw_tx":"0x0102"}),
            provenance: Some("test".to_owned()),
        },
    );

    let transition = apply_live_evm_broadcast_with_provider(&mut runtime, &provider)
        .await
        .expect("broadcast transition");

    assert_eq!(transition.node_id.as_deref(), Some("broadcast-swap"));
    assert_eq!(
        runtime.checkpoint.lifecycle.status,
        ais_agent_core::runtime::RunStatus::AwaitingConfirmation
    );
    assert_eq!(
        runtime
            .checkpoint
            .pending_requests
            .pending_confirmation_id
            .as_deref(),
        Some("0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
    );
    assert!(runtime.checkpoint.actuation_records.iter().any(|record| {
        matches!(record.kind, ActuationKind::BroadcastSubmitted)
            && record.tx_hash.as_deref()
                == Some("0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
    }));
}

#[tokio::test]
async fn runtime_can_resume_after_broadcast_and_observe_live_receipt() {
    let broadcast_asserter = Asserter::new();
    let broadcast_provider =
        ProviderBuilder::new().connect_mocked_client(broadcast_asserter.clone());
    let tx_hash = b256!("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
    broadcast_asserter.push_success(&tx_hash);

    let checkpoint = checkpoint_with_nodes(vec![
        broadcast_swap_node("broadcast-swap"),
        verify_receipt_node("verify-swap", vec!["broadcast-swap"]),
    ]);
    let mission = sample_mission();
    let mut runtime = ActiveRun::new(mission.clone(), checkpoint);
    runtime.envelopes.insert(
        "env.swap".to_owned(),
        RuntimeEnvelope {
            envelope_id: "env.swap".to_owned(),
            kind: RuntimeEnvelopeKind::EvmEnvelope,
            chain: "eip155:1".to_owned(),
            payload: serde_json::json!({"raw_tx":"0x0102"}),
            provenance: Some("test".to_owned()),
        },
    );

    apply_live_evm_broadcast_with_provider(&mut runtime, &broadcast_provider)
        .await
        .expect("broadcast transition");

    let mut restored = restore_active_run_from_parts(mission, runtime.checkpoint.clone(), None)
        .expect("restore runtime after broadcast");

    let receipt_asserter = Asserter::new();
    let receipt_provider = ProviderBuilder::new().connect_mocked_client(receipt_asserter.clone());
    receipt_asserter.push_success(&sample_receipt(tx_hash, 100));
    receipt_asserter.push_success(&104u64);

    let transition = apply_live_evm_verify_with_provider(&mut restored, &receipt_provider)
        .await
        .expect("verify transition");

    assert_eq!(transition.node_id.as_deref(), Some("verify-swap"));
    assert_eq!(
        restored.checkpoint.pending_requests.pending_confirmation_id,
        None
    );
    assert_eq!(
        restored
            .checkpoint
            .action_graph
            .nodes
            .get("verify-swap")
            .map(|node| node.status.clone()),
        Some(ActionNodeStatus::Succeeded)
    );
    assert!(restored.checkpoint.actuation_records.iter().any(|record| {
        matches!(record.kind, ActuationKind::ReceiptObserved)
            && record.tx_hash.as_deref()
                == Some("0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
    }));
    assert!(restored
        .checkpoint
        .evidence_graph
        .records
        .contains_key("receipt.verify-swap"));
}

#[tokio::test]
async fn runtime_can_restart_from_durable_side_effect_cut_after_evm_broadcast_success() {
    let broadcast_asserter = Asserter::new();
    let broadcast_provider =
        ProviderBuilder::new().connect_mocked_client(broadcast_asserter.clone());
    let tx_hash = b256!("bcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbc");
    broadcast_asserter.push_success(&tx_hash);

    let mut checkpoint = checkpoint_with_nodes(vec![
        broadcast_swap_node("broadcast-swap"),
        verify_effect_node("verify-swap", vec!["broadcast-swap"], Some("state.pre.out")),
    ]);
    checkpoint.effect_contracts.insert(
        "effect.swap".to_owned(),
        swap_effect_contract("post.decoded_u256 == \"120\""),
    );
    checkpoint.evidence_graph.records.insert(
        "state.pre.out".to_owned(),
        pre_balance_evidence("state.pre.out", "90"),
    );

    let mission = sample_mission();
    let run_id = RunId("run-1".to_owned());
    let mut runtime = ActiveRun::new(mission.clone(), checkpoint);
    runtime.envelopes.insert(
        "env.swap".to_owned(),
        RuntimeEnvelope {
            envelope_id: "env.swap".to_owned(),
            kind: RuntimeEnvelopeKind::EvmEnvelope,
            chain: "eip155:1".to_owned(),
            payload: serde_json::json!({"raw_tx":"0x0102"}),
            provenance: Some("test".to_owned()),
        },
    );

    apply_live_evm_broadcast_with_provider(&mut runtime, &broadcast_provider)
        .await
        .expect("broadcast transition");

    let mut checkpoint_repo = InMemoryCheckpointRepository::default();
    persist_side_effect_checkpoint(&mut checkpoint_repo, &runtime)
        .expect("persist side-effect checkpoint");
    let mut mission_repo = InMemoryMissionRepository::default();
    mission_repo
        .insert(run_id.clone(), mission)
        .expect("insert mission");

    let mut restored = restore_active_run(
        &run_id,
        &mission_repo,
        &checkpoint_repo,
        &InMemorySignerStateArchive::default(),
    )
    .expect("restore from durable side-effect cut");

    let receipt_asserter = Asserter::new();
    let receipt_provider = ProviderBuilder::new().connect_mocked_client(receipt_asserter.clone());
    receipt_asserter.push_success(&sample_receipt(tx_hash, 100));
    receipt_asserter.push_success(&104u64);
    receipt_asserter.push_success(&encode_u256_return(120));

    let transition = apply_live_evm_verify_with_provider(&mut restored, &receipt_provider)
        .await
        .expect("verify transition after restart");

    assert_eq!(transition.node_id.as_deref(), Some("verify-swap"));
    assert_eq!(
        restored.checkpoint.pending_requests.pending_confirmation_id,
        None
    );
    assert_eq!(
        restored
            .checkpoint
            .action_graph
            .nodes
            .get("verify-swap")
            .map(|node| node.status.clone()),
        Some(ActionNodeStatus::Succeeded)
    );
    assert!(restored
        .checkpoint
        .evidence_graph
        .records
        .contains_key("receipt.verify-swap"));
    assert!(restored
        .checkpoint
        .evidence_graph
        .records
        .contains_key("effect.verify-swap"));
}

#[tokio::test]
async fn runtime_effect_verify_can_reach_satisfied_from_live_receipt_and_post_state() {
    let tx_hash = b256!("cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc");
    let mut checkpoint = checkpoint_with_nodes(vec![
        succeeded_broadcast_swap_node("broadcast-swap"),
        verify_effect_node("verify-swap", vec!["broadcast-swap"], Some("state.pre.out")),
    ]);
    checkpoint.effect_contracts.insert(
        "effect.swap".to_owned(),
        swap_effect_contract("post.decoded_u256 == \"120\""),
    );
    checkpoint.evidence_graph.records.insert(
        "state.pre.out".to_owned(),
        pre_balance_evidence("state.pre.out", "90"),
    );

    let mission = sample_mission();
    let mut runtime = ActiveRun::new(mission, checkpoint);
    runtime.checkpoint.pending_requests.pending_confirmation_id = Some(format!("{:#x}", tx_hash));

    let asserter = Asserter::new();
    let provider = ProviderBuilder::new().connect_mocked_client(asserter.clone());
    asserter.push_success(&sample_receipt(tx_hash, 100));
    asserter.push_success(&104u64);
    asserter.push_success(&encode_u256_return(120));

    let transition = apply_live_evm_verify_with_provider(&mut runtime, &provider)
        .await
        .expect("verify transition");

    assert_eq!(transition.node_id.as_deref(), Some("verify-swap"));
    assert_eq!(
        runtime
            .checkpoint
            .action_graph
            .nodes
            .get("verify-swap")
            .map(|node| node.status.clone()),
        Some(ActionNodeStatus::Succeeded)
    );
    assert_eq!(
        runtime.checkpoint.pending_requests.pending_confirmation_id,
        None
    );
    assert!(runtime
        .checkpoint
        .evidence_graph
        .records
        .contains_key("post.verify-swap"));
    assert_eq!(
        runtime
            .checkpoint
            .evidence_graph
            .records
            .get("effect.verify-swap")
            .and_then(|record| record.payload.get("final_status"))
            .and_then(|value| value.as_str()),
        Some("satisfied")
    );
}

#[tokio::test]
async fn runtime_effect_verify_mismatch_pauses_with_recovery_context() {
    let tx_hash = b256!("dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd");
    let mut checkpoint = checkpoint_with_nodes(vec![
        succeeded_broadcast_swap_node("broadcast-swap"),
        verify_effect_node("verify-swap", vec!["broadcast-swap"], Some("state.pre.out")),
    ]);
    checkpoint.effect_contracts.insert(
        "effect.swap".to_owned(),
        swap_effect_contract("post.decoded_u256 == \"120\""),
    );
    checkpoint.evidence_graph.records.insert(
        "state.pre.out".to_owned(),
        pre_balance_evidence("state.pre.out", "90"),
    );

    let mission = sample_mission();
    let mut runtime = ActiveRun::new(mission, checkpoint);
    runtime.checkpoint.pending_requests.pending_confirmation_id = Some(format!("{:#x}", tx_hash));

    let asserter = Asserter::new();
    let provider = ProviderBuilder::new().connect_mocked_client(asserter.clone());
    asserter.push_success(&sample_receipt(tx_hash, 100));
    asserter.push_success(&104u64);
    asserter.push_success(&encode_u256_return(80));

    let transition = apply_live_evm_verify_with_provider(&mut runtime, &provider)
        .await
        .expect("verify transition");

    assert_eq!(transition.node_id.as_deref(), Some("verify-swap"));
    assert_eq!(
        runtime.checkpoint.lifecycle.status,
        ais_agent_core::runtime::RunStatus::Paused
    );
    assert_eq!(
        runtime
            .checkpoint
            .action_graph
            .nodes
            .get("verify-swap")
            .map(|node| node.status.clone()),
        Some(ActionNodeStatus::Failed)
    );
    assert_eq!(
        runtime
            .checkpoint
            .lifecycle
            .failure
            .as_ref()
            .map(|failure| &failure.code),
        Some(&ais_agent_control::recovery::RunFailureCode::VerifyMismatch)
    );
    let failure = runtime
        .checkpoint
        .lifecycle
        .failure
        .as_ref()
        .expect("verify mismatch failure");
    assert_eq!(failure.effect_refs, vec!["effect.swap".to_owned()]);
    assert_eq!(failure.confirmation_refs, vec![format!("{:#x}", tx_hash)]);
    assert!(!failure.actuation_refs.is_empty());
    assert!(failure
        .evidence_refs
        .contains(&"effect.verify-swap".to_owned()));
    assert!(failure
        .evidence_refs
        .contains(&"receipt.verify-swap".to_owned()));
    assert_eq!(
        runtime
            .checkpoint
            .evidence_graph
            .records
            .get("effect.verify-swap")
            .and_then(|record| record.payload.get("final_status"))
            .and_then(|value| value.as_str()),
        Some("violated")
    );
}

#[tokio::test]
async fn runtime_effect_verify_can_reach_pending_when_required_pre_state_is_missing() {
    let tx_hash = b256!("eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee");
    let mut checkpoint = checkpoint_with_nodes(vec![
        succeeded_broadcast_swap_node("broadcast-swap"),
        verify_effect_node(
            "verify-swap",
            vec!["broadcast-swap"],
            Some("state.pre.missing"),
        ),
    ]);
    checkpoint.effect_contracts.insert(
        "effect.swap".to_owned(),
        swap_effect_contract("pre.decoded_u256 != null && post.decoded_u256 == \"120\""),
    );

    let mission = sample_mission();
    let mut runtime = ActiveRun::new(mission, checkpoint);
    runtime.checkpoint.pending_requests.pending_confirmation_id = Some(format!("{:#x}", tx_hash));

    let asserter = Asserter::new();
    let provider = ProviderBuilder::new().connect_mocked_client(asserter.clone());
    asserter.push_success(&sample_receipt(tx_hash, 100));
    asserter.push_success(&104u64);
    asserter.push_success(&encode_u256_return(120));

    let transition = apply_live_evm_verify_with_provider(&mut runtime, &provider)
        .await
        .expect("verify transition");

    assert_eq!(transition.node_id.as_deref(), Some("verify-swap"));
    assert_eq!(
        runtime.checkpoint.lifecycle.status,
        ais_agent_core::runtime::RunStatus::AwaitingEvidence
    );
    assert_eq!(
        runtime.checkpoint.pending_requests.pending_evidence_refs,
        vec!["pre".to_owned()]
    );
    assert_eq!(
        runtime
            .checkpoint
            .evidence_graph
            .records
            .get("effect.verify-swap")
            .and_then(|record| record.payload.get("final_status"))
            .and_then(|value| value.as_str()),
        Some("unknown_due_to_missing_observation")
    );
}

#[tokio::test]
async fn runtime_evm_guarded_execution_can_complete_end_to_end_with_live_ports() {
    let tx_hash = b256!("ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff");
    let mut checkpoint = checkpoint_with_nodes(vec![
        observe_pre_balance_node("observe-pre"),
        simulate_swap_call_node("simulate-swap", vec!["observe-pre"]),
        broadcast_swap_with_simulation_dep_node("broadcast-swap", vec!["simulate-swap"]),
        verify_effect_node("verify-swap", vec!["broadcast-swap"], Some("state.pre.out")),
    ]);
    checkpoint.action_graph.terminals = vec!["verify-swap".to_owned()];
    let mission = sample_mission();
    let mut runtime = ActiveRun::new(mission, checkpoint);
    runtime.envelopes.insert(
        "env.swap".to_owned(),
        RuntimeEnvelope {
            envelope_id: "env.swap".to_owned(),
            kind: RuntimeEnvelopeKind::EvmEnvelope,
            chain: "eip155:1".to_owned(),
            payload: serde_json::json!({"raw_tx":"0x0102"}),
            provenance: Some("test".to_owned()),
        },
    );
    runtime.checkpoint.effect_contracts.insert(
        "effect.swap".to_owned(),
        swap_effect_contract("post.decoded_u256 == \"120\""),
    );

    let asserter = Asserter::new();
    let provider = ProviderBuilder::new().connect_mocked_client(asserter.clone());
    asserter.push_success(&encode_u256_return(90));
    asserter.push_success(&encode_u256_return(1));
    asserter.push_success(&tx_hash);
    asserter.push_success(&sample_receipt(tx_hash, 100));
    asserter.push_success(&104u64);
    asserter.push_success(&encode_u256_return(120));

    let observe = apply_live_evm_observe_with_provider(&mut runtime, &provider)
        .await
        .expect("observe transition");
    assert_eq!(observe.kind, StepTransitionKind::Observe);

    let simulate = apply_live_evm_simulate_with_provider(&mut runtime, &provider)
        .await
        .expect("simulate transition");
    assert_eq!(simulate.kind, StepTransitionKind::Simulate);

    let govern = StepOnce::apply(&mut runtime).await;
    assert_eq!(
        govern.applied_transition.as_ref().map(|t| t.kind),
        Some(StepTransitionKind::Govern)
    );
    assert_eq!(
        runtime.checkpoint.lifecycle.status,
        ais_agent_core::runtime::RunStatus::AwaitingSigner
    );

    runtime
        .pending_signer_state
        .as_mut()
        .expect("pending signer")
        .apply_decision(ais_agent_core::runtime::SignerDecision {
            request_id: ais_agent_control::ids::SignerRequestId(
                runtime
                    .checkpoint
                    .pending_requests
                    .pending_signer_request_id
                    .clone()
                    .expect("request id"),
            ),
            kind: ais_agent_core::runtime::SignerDecisionKind::Approved,
            decision_at_ms: None,
            tx_hash: None,
        });

    let signer = StepOnce::apply(&mut runtime).await;
    assert_eq!(
        signer.applied_transition.as_ref().map(|t| t.kind),
        Some(StepTransitionKind::Signer)
    );
    assert_eq!(
        runtime
            .checkpoint
            .action_graph
            .nodes
            .get("broadcast-swap")
            .map(|node| node.status.clone()),
        Some(ActionNodeStatus::Ready)
    );

    let broadcast = apply_live_evm_broadcast_with_provider(&mut runtime, &provider)
        .await
        .expect("broadcast transition");
    assert_eq!(broadcast.kind, StepTransitionKind::Broadcast);

    let verify = apply_live_evm_verify_with_provider(&mut runtime, &provider)
        .await
        .expect("verify transition");
    assert_eq!(verify.kind, StepTransitionKind::Verify);
    assert_eq!(
        runtime
            .checkpoint
            .action_graph
            .nodes
            .get("verify-swap")
            .map(|node| node.status.clone()),
        Some(ActionNodeStatus::Succeeded)
    );

    let complete = StepOnce::apply(&mut runtime).await;
    assert_eq!(
        complete.applied_transition.as_ref().map(|t| t.kind),
        Some(StepTransitionKind::Complete)
    );
    assert_eq!(
        runtime.checkpoint.lifecycle.status,
        ais_agent_core::runtime::RunStatus::Completed
    );
    assert!(runtime
        .checkpoint
        .evidence_graph
        .records
        .contains_key("state.pre.out"));
    assert!(runtime
        .checkpoint
        .evidence_graph
        .records
        .contains_key("receipt.verify-swap"));
    assert!(runtime
        .checkpoint
        .evidence_graph
        .records
        .contains_key("post.verify-swap"));
    assert!(runtime
        .checkpoint
        .evidence_graph
        .records
        .contains_key("effect.verify-swap"));
}

fn sample_mission() -> Mission {
    Mission {
        mission_id: "mission-1".to_owned(),
        goal: "observe and simulate".to_owned(),
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

fn checkpoint_with_nodes(nodes: Vec<ActionNode>) -> CheckpointSnapshot {
    let mut lifecycle = RunLifecycleState::new(RunId("run-1".to_owned()), "mission-1");
    lifecycle.mark_running(RunPhase::Planning);

    CheckpointSnapshot {
        run_id: "run-1".to_owned(),
        mission_id: "mission-1".to_owned(),
        checkpoint_seq: 0,
        plan_epoch: 0,
        lifecycle,
        action_graph: ActionGraph {
            graph_id: Some("graph-1".to_owned()),
            roots: Vec::new(),
            terminals: Vec::new(),
            nodes: nodes
                .into_iter()
                .map(|node| (node.node_id.clone(), node))
                .collect(),
        },
        evidence_graph: EvidenceGraph::default(),
        effect_contracts: Default::default(),
        pending_requests: PendingRequestsSnapshot::default(),
        last_completed_node_id: None,
        actuation_records: Vec::new(),
    }
}

fn observe_block_number_node(node_id: &str) -> ActionNode {
    ActionNode {
        node_id: node_id.to_owned(),
        kind: ActionNodeKind::Observe,
        origin: ActionOrigin::DriverFragment,
        status: ActionNodeStatus::Pending,
        depends_on: Vec::new(),
        inputs: Vec::new(),
        evidence_refs: Vec::new(),
        payload: ActionPayload::Observe(ObserveAction {
            source_kind: ObserveSourceKind::ChainRead,
            source_hint: "evm block number".to_owned(),
            output_key: Some("observed.block_number".to_owned()),
            live: Some(ObserveLiveBinding::Evm(EvmObserveLiveBinding {
                connection: None,
                binding: EvmObserveBinding::BlockNumber,
                request: EvmObserveRequest::BlockNumber,
            })),
        }),
        implementation_hint: Some("evm.read.block_number".to_owned()),
        expected_effect_ref: None,
    }
}

fn observe_pre_balance_node(node_id: &str) -> ActionNode {
    ActionNode {
        node_id: node_id.to_owned(),
        kind: ActionNodeKind::Observe,
        origin: ActionOrigin::DriverFragment,
        status: ActionNodeStatus::Pending,
        depends_on: Vec::new(),
        inputs: Vec::new(),
        evidence_refs: Vec::new(),
        payload: ActionPayload::Observe(ObserveAction {
            source_kind: ObserveSourceKind::ChainRead,
            source_hint: "evm pre balance".to_owned(),
            output_key: Some("state.pre.out".to_owned()),
            live: Some(ObserveLiveBinding::Evm(EvmObserveLiveBinding {
                connection: None,
                binding: EvmObserveBinding::Erc20BalanceOf,
                request: EvmObserveRequest::Erc20BalanceOf {
                    token: address!("3333333333333333333333333333333333333333"),
                    owner: address!("4444444444444444444444444444444444444444"),
                },
            })),
        }),
        implementation_hint: Some("evm.read.erc20_balance_of".to_owned()),
        expected_effect_ref: None,
    }
}

fn simulate_eth_call_node(node_id: &str) -> ActionNode {
    ActionNode {
        node_id: node_id.to_owned(),
        kind: ActionNodeKind::Simulate,
        origin: ActionOrigin::DriverFragment,
        status: ActionNodeStatus::Pending,
        depends_on: Vec::new(),
        inputs: Vec::new(),
        evidence_refs: Vec::new(),
        payload: ActionPayload::Simulate(SimulateAction {
            simulate_kind: SimulateKind::Call,
            simulator_hint: "evm eth_call".to_owned(),
            live: Some(SimulateLiveBinding::Evm(EvmSimulateLiveBinding {
                connection: None,
                binding: EvmSimulateBinding::EthCall,
                request: EvmCallRequest {
                    from: None,
                    to: address!("1111111111111111111111111111111111111111"),
                    data: bytes!("06fdde03"),
                    value: None,
                },
            })),
        }),
        implementation_hint: Some("evm.simulate.eth_call".to_owned()),
        expected_effect_ref: None,
    }
}

fn simulate_swap_call_node(node_id: &str, depends_on: Vec<&str>) -> ActionNode {
    ActionNode {
        node_id: node_id.to_owned(),
        kind: ActionNodeKind::Simulate,
        origin: ActionOrigin::DriverFragment,
        status: ActionNodeStatus::Pending,
        depends_on: depends_on.into_iter().map(str::to_owned).collect(),
        inputs: Vec::new(),
        evidence_refs: Vec::new(),
        payload: ActionPayload::Simulate(SimulateAction {
            simulate_kind: SimulateKind::Call,
            simulator_hint: "evm swap eth_call".to_owned(),
            live: Some(SimulateLiveBinding::Evm(EvmSimulateLiveBinding {
                connection: None,
                binding: EvmSimulateBinding::EthCall,
                request: EvmCallRequest {
                    from: None,
                    to: address!("2222222222222222222222222222222222222222"),
                    data: bytes!("01020304"),
                    value: None,
                },
            })),
        }),
        implementation_hint: Some("evm.simulate.eth_call".to_owned()),
        expected_effect_ref: None,
    }
}

fn broadcast_swap_node(node_id: &str) -> ActionNode {
    ActionNode {
        node_id: node_id.to_owned(),
        kind: ActionNodeKind::Actuate,
        origin: ActionOrigin::RawEnvelopePath,
        status: ActionNodeStatus::Ready,
        depends_on: Vec::new(),
        inputs: Vec::new(),
        evidence_refs: Vec::new(),
        payload: ActionPayload::Actuate(ActuateAction {
            mode: ActuateMode::RawEnvelope,
            actuator_hint: "evm live broadcast".to_owned(),
            chain: Some("eip155:1".to_owned()),
            envelope_ref: Some("env.swap".to_owned()),
            requires_effect_contract: true,
            live: Some(ActuateLiveBinding::Evm(EvmActuateLiveBinding {
                connection: Some(EvmConnectionSpec {
                    rpc_url: "http://localhost:8545".to_owned(),
                }),
                binding: EvmActuateBinding::BroadcastRawTransaction,
            })),
        }),
        implementation_hint: Some("evm.broadcast.raw_transaction".to_owned()),
        expected_effect_ref: Some("effect.swap".to_owned()),
    }
}

fn broadcast_swap_with_simulation_dep_node(node_id: &str, depends_on: Vec<&str>) -> ActionNode {
    let mut node = broadcast_swap_node(node_id);
    node.status = ActionNodeStatus::Pending;
    node.depends_on = depends_on.into_iter().map(str::to_owned).collect();
    node
}

fn succeeded_broadcast_swap_node(node_id: &str) -> ActionNode {
    let mut node = broadcast_swap_node(node_id);
    node.status = ActionNodeStatus::Succeeded;
    node
}

fn verify_receipt_node(node_id: &str, depends_on: Vec<&str>) -> ActionNode {
    ActionNode {
        node_id: node_id.to_owned(),
        kind: ActionNodeKind::Verify,
        origin: ActionOrigin::DriverFragment,
        status: ActionNodeStatus::Pending,
        depends_on: depends_on.into_iter().map(str::to_owned).collect(),
        inputs: Vec::new(),
        evidence_refs: Vec::new(),
        payload: ActionPayload::Verify(VerifyAction {
            verify_kind: VerifyKind::ReceiptObserved,
            verifier_hint: "evm receipt status".to_owned(),
            pre_observation_ref: None,
            post_observation_ref: None,
            live: Some(VerifyLiveBinding::Evm(EvmVerifyLiveBinding {
                connection: Some(EvmConnectionSpec {
                    rpc_url: "http://localhost:8545".to_owned(),
                }),
                binding: EvmVerifyBinding::ReceiptStatus,
                post_request: None,
            })),
        }),
        implementation_hint: Some("evm.verify.receipt_status".to_owned()),
        expected_effect_ref: Some("effect.swap".to_owned()),
    }
}

fn verify_effect_node(
    node_id: &str,
    depends_on: Vec<&str>,
    pre_observation_ref: Option<&str>,
) -> ActionNode {
    ActionNode {
        node_id: node_id.to_owned(),
        kind: ActionNodeKind::Verify,
        origin: ActionOrigin::DriverFragment,
        status: ActionNodeStatus::Pending,
        depends_on: depends_on.into_iter().map(str::to_owned).collect(),
        inputs: Vec::new(),
        evidence_refs: Vec::new(),
        payload: ActionPayload::Verify(VerifyAction {
            verify_kind: VerifyKind::EffectContract,
            verifier_hint: "evm effect verify".to_owned(),
            pre_observation_ref: pre_observation_ref.map(str::to_owned),
            post_observation_ref: Some(format!("post.{node_id}")),
            live: Some(VerifyLiveBinding::Evm(EvmVerifyLiveBinding {
                connection: Some(EvmConnectionSpec {
                    rpc_url: "http://localhost:8545".to_owned(),
                }),
                binding: EvmVerifyBinding::EffectContractFromReceiptAndPostState,
                post_request: Some(EvmObserveRequest::Erc20BalanceOf {
                    token: address!("3333333333333333333333333333333333333333"),
                    owner: address!("4444444444444444444444444444444444444444"),
                }),
            })),
        }),
        implementation_hint: Some("evm.verify.effect_contract".to_owned()),
        expected_effect_ref: Some("effect.swap".to_owned()),
    }
}

fn swap_effect_contract(expression: &str) -> EffectContract {
    EffectContract {
        effect_id: "effect.swap".to_owned(),
        kind: EffectContractKind::AssetDelta,
        assertions: vec![EffectAssertion {
            expression: expression.to_owned(),
            description: "post state must satisfy swap effect".to_owned(),
        }],
        tolerance_hint: Some("swap".to_owned()),
    }
}

fn pre_balance_evidence(evidence_id: &str, decoded_u256: &str) -> EvidenceRecord {
    EvidenceRecord {
        evidence_id: evidence_id.to_owned(),
        kind: EvidenceKind::ExternalObservation,
        provenance: EvidenceProvenance {
            source: "test.pre_state".to_owned(),
            chain_scope: Some("eip155:1".to_owned()),
            trace_hint: Some("verify-swap".to_owned()),
        },
        freshness: EvidenceFreshness {
            observed_at_ms: Some(1),
            expires_at_ms: None,
            max_age_ms: None,
        },
        confidence_ppm: Some(1_000_000),
        payload: json!({
            "decoded_u256": decoded_u256,
            "source_hint": "test.pre_state",
        }),
    }
}

fn encode_u256_return(value: u64) -> alloy::primitives::Bytes {
    let mut word = [0u8; 32];
    word[24..].copy_from_slice(&value.to_be_bytes());
    alloy::primitives::Bytes::from(word.to_vec())
}

fn sample_receipt(
    tx_hash: alloy::primitives::TxHash,
    block_number: u64,
) -> Option<TransactionReceipt> {
    Some(TransactionReceipt {
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
    })
}
