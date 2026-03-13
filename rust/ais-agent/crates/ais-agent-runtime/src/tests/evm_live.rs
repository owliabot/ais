use std::collections::BTreeMap;

use ais_agent_control::{ids::RunId, recovery::RunFailureCode};
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
    primitives::{address, b256, bytes, Bytes, U256},
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
    service::{seed_action_family_checkpoint, RuntimeExecutionWiring},
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

#[tokio::test]
async fn native_transfer_action_family_can_complete_via_signer_submission_and_live_verify() {
    let tx_hash = b256!("1111111111111111111111111111111111111111111111111111111111111111");
    let mission = sample_native_transfer_action_family_mission();
    let mut checkpoint = checkpoint_with_nodes(Vec::new());
    seed_action_family_checkpoint(
        &mission,
        &mut checkpoint,
        &RuntimeExecutionWiring {
            evm_rpc_url: Some("http://127.0.0.1:8545".to_owned()),
            solana_rpc_url: None,
            native_transfer_enabled: true,
            erc20_transfer_enabled: false,
            uniswap_v3_swap_enabled: false,
            uniswap_v3_lp_enabled: false,
        },
    )
    .expect("native transfer should seed");
    let mut runtime = ActiveRun::new(mission, checkpoint);

    let asserter = Asserter::new();
    let provider = ProviderBuilder::new().connect_mocked_client(asserter.clone());
    asserter.push_success(&U256::from(90u64));
    asserter.push_success(&Bytes::default());
    asserter.push_success(&sample_receipt(tx_hash, 100));
    asserter.push_success(&104u64);
    asserter.push_success(&U256::from(120u64));

    let observe = apply_live_evm_observe_with_provider(&mut runtime, &provider)
        .await
        .expect("observe transition");
    assert_eq!(
        observe.node_id.as_deref(),
        Some("observe.native_transfer.recipient_balance")
    );

    let simulate = apply_live_evm_simulate_with_provider(&mut runtime, &provider)
        .await
        .expect("simulate transition");
    assert_eq!(
        simulate.node_id.as_deref(),
        Some("simulate.native_transfer.call")
    );

    let govern = StepOnce::apply(&mut runtime).await;
    assert_eq!(
        govern.applied_transition.as_ref().map(|t| t.kind),
        Some(StepTransitionKind::Govern)
    );
    assert_eq!(
        runtime.checkpoint.lifecycle.status,
        ais_agent_core::runtime::RunStatus::AwaitingSigner
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
        .apply_decision(ais_agent_core::runtime::SignerDecision {
            request_id: ais_agent_control::ids::SignerRequestId(request_id),
            kind: ais_agent_core::runtime::SignerDecisionKind::Submitted,
            decision_at_ms: None,
            tx_hash: Some(format!("{tx_hash:#x}")),
        });

    let signer = StepOnce::apply(&mut runtime).await;
    assert_eq!(
        signer.applied_transition.as_ref().map(|t| t.kind),
        Some(StepTransitionKind::Signer)
    );
    assert_eq!(
        runtime
            .checkpoint
            .pending_requests
            .pending_confirmation_id
            .as_deref(),
        Some("0x1111111111111111111111111111111111111111111111111111111111111111")
    );

    let verify = apply_live_evm_verify_with_provider(&mut runtime, &provider)
        .await
        .expect("verify transition");
    assert_eq!(
        verify.node_id.as_deref(),
        Some("verify.native_transfer.effect")
    );
    assert_eq!(
        runtime
            .checkpoint
            .action_graph
            .nodes
            .get("verify.native_transfer.effect")
            .map(|node| node.status.clone()),
        Some(ActionNodeStatus::Succeeded)
    );
    assert!(runtime
        .checkpoint
        .evidence_graph
        .records
        .contains_key("state.pre.recipient_balance"));
    assert!(runtime
        .checkpoint
        .evidence_graph
        .records
        .contains_key("state.post.recipient_balance"));
    assert!(runtime
        .checkpoint
        .evidence_graph
        .records
        .contains_key("effect.verify.native_transfer.effect"));

    let complete = StepOnce::apply(&mut runtime).await;
    assert_eq!(
        complete.applied_transition.as_ref().map(|t| t.kind),
        Some(StepTransitionKind::Complete)
    );
    assert_eq!(
        runtime.checkpoint.lifecycle.status,
        ais_agent_core::runtime::RunStatus::Completed
    );
}

#[tokio::test]
async fn native_transfer_action_family_can_restart_from_signer_submitted_side_effect_cut() {
    let tx_hash = b256!("2222222222222222222222222222222222222222222222222222222222222222");
    let mission = sample_native_transfer_action_family_mission();
    let mut checkpoint = checkpoint_with_nodes(Vec::new());
    seed_action_family_checkpoint(
        &mission,
        &mut checkpoint,
        &RuntimeExecutionWiring {
            evm_rpc_url: Some("http://127.0.0.1:8545".to_owned()),
            solana_rpc_url: None,
            native_transfer_enabled: true,
            erc20_transfer_enabled: false,
            uniswap_v3_swap_enabled: false,
            uniswap_v3_lp_enabled: false,
        },
    )
    .expect("native transfer should seed");
    let mut runtime = ActiveRun::new(mission.clone(), checkpoint);

    let asserter = Asserter::new();
    let provider = ProviderBuilder::new().connect_mocked_client(asserter.clone());
    asserter.push_success(&U256::from(90u64));
    asserter.push_success(&Bytes::default());

    apply_live_evm_observe_with_provider(&mut runtime, &provider)
        .await
        .expect("observe transition");
    apply_live_evm_simulate_with_provider(&mut runtime, &provider)
        .await
        .expect("simulate transition");
    StepOnce::apply(&mut runtime).await;

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
        .apply_decision(ais_agent_core::runtime::SignerDecision {
            request_id: ais_agent_control::ids::SignerRequestId(request_id),
            kind: ais_agent_core::runtime::SignerDecisionKind::Submitted,
            decision_at_ms: None,
            tx_hash: Some(format!("{tx_hash:#x}")),
        });
    StepOnce::apply(&mut runtime).await;

    let mut checkpoint_repo = InMemoryCheckpointRepository::default();
    persist_side_effect_checkpoint(&mut checkpoint_repo, &runtime)
        .expect("persist side-effect checkpoint");
    let mut mission_repo = InMemoryMissionRepository::default();
    mission_repo
        .insert(runtime.run_id.clone(), mission)
        .expect("insert mission");

    let mut restored = restore_active_run(
        &runtime.run_id,
        &mission_repo,
        &checkpoint_repo,
        &InMemorySignerStateArchive::default(),
    )
    .expect("restore runtime after signer submission");

    let verify_asserter = Asserter::new();
    let verify_provider = ProviderBuilder::new().connect_mocked_client(verify_asserter.clone());
    verify_asserter.push_success(&sample_receipt(tx_hash, 100));
    verify_asserter.push_success(&104u64);
    verify_asserter.push_success(&U256::from(120u64));

    let verify = apply_live_evm_verify_with_provider(&mut restored, &verify_provider)
        .await
        .expect("verify transition");
    assert_eq!(
        verify.node_id.as_deref(),
        Some("verify.native_transfer.effect")
    );

    let complete = StepOnce::apply(&mut restored).await;
    assert_eq!(
        complete.applied_transition.as_ref().map(|t| t.kind),
        Some(StepTransitionKind::Complete)
    );
    assert_eq!(
        restored.checkpoint.lifecycle.status,
        ais_agent_core::runtime::RunStatus::Completed
    );
}

#[tokio::test]
async fn erc20_transfer_action_family_surfaces_undecodable_token_observe_as_await_evidence() {
    let mission = sample_erc20_transfer_action_family_mission();
    let mut checkpoint = checkpoint_with_nodes(Vec::new());
    seed_action_family_checkpoint(
        &mission,
        &mut checkpoint,
        &RuntimeExecutionWiring {
            evm_rpc_url: Some("http://127.0.0.1:8545".to_owned()),
            solana_rpc_url: None,
            native_transfer_enabled: false,
            erc20_transfer_enabled: true,
            uniswap_v3_swap_enabled: false,
            uniswap_v3_lp_enabled: false,
        },
    )
    .expect("erc20 transfer should seed");
    let mut runtime = ActiveRun::new(mission, checkpoint);

    let asserter = Asserter::new();
    let provider = ProviderBuilder::new().connect_mocked_client(asserter.clone());
    asserter.push_success(&Bytes::default());

    let observe = apply_live_evm_observe_with_provider(&mut runtime, &provider)
        .await
        .expect("observe transition");
    assert_eq!(
        observe.node_id.as_deref(),
        Some("observe.erc20_transfer.recipient_token_balance")
    );
    assert_eq!(
        runtime.checkpoint.lifecycle.status,
        ais_agent_core::runtime::RunStatus::AwaitingEvidence
    );
    assert_eq!(
        runtime
            .checkpoint
            .lifecycle
            .failure
            .as_ref()
            .map(|failure| &failure.code),
        Some(&RunFailureCode::StaleEvidence)
    );
    assert!(runtime
        .checkpoint
        .lifecycle
        .failure
        .as_ref()
        .is_some_and(|failure| failure
            .evidence_refs
            .contains(&"evidence.transfer.token".to_owned())));
    assert_eq!(
        runtime
            .checkpoint
            .action_graph
            .nodes
            .get("observe.erc20_transfer.recipient_token_balance")
            .map(|node| node.status.clone()),
        Some(ActionNodeStatus::Blocked)
    );
}

#[tokio::test]
async fn erc20_transfer_action_family_can_complete_via_signer_submission_and_live_verify() {
    let tx_hash = b256!("3333333333333333333333333333333333333333333333333333333333333333");
    let mission = sample_erc20_transfer_action_family_mission();
    let mut checkpoint = checkpoint_with_nodes(Vec::new());
    seed_action_family_checkpoint(
        &mission,
        &mut checkpoint,
        &RuntimeExecutionWiring {
            evm_rpc_url: Some("http://127.0.0.1:8545".to_owned()),
            solana_rpc_url: None,
            native_transfer_enabled: false,
            erc20_transfer_enabled: true,
            uniswap_v3_swap_enabled: false,
            uniswap_v3_lp_enabled: false,
        },
    )
    .expect("erc20 transfer should seed");
    let mut runtime = ActiveRun::new(mission, checkpoint);

    let asserter = Asserter::new();
    let provider = ProviderBuilder::new().connect_mocked_client(asserter.clone());
    asserter.push_success(&encode_u256_return(90));
    asserter.push_success(&Bytes::default());
    asserter.push_success(&sample_receipt(tx_hash, 100));
    asserter.push_success(&104u64);
    asserter.push_success(&encode_u256_return(120));

    let observe = apply_live_evm_observe_with_provider(&mut runtime, &provider)
        .await
        .expect("observe transition");
    assert_eq!(
        observe.node_id.as_deref(),
        Some("observe.erc20_transfer.recipient_token_balance")
    );

    let simulate = apply_live_evm_simulate_with_provider(&mut runtime, &provider)
        .await
        .expect("simulate transition");
    assert_eq!(
        simulate.node_id.as_deref(),
        Some("simulate.erc20_transfer.call")
    );

    let govern = StepOnce::apply(&mut runtime).await;
    assert_eq!(
        govern.applied_transition.as_ref().map(|t| t.kind),
        Some(StepTransitionKind::Govern)
    );
    assert_eq!(
        runtime.checkpoint.lifecycle.status,
        ais_agent_core::runtime::RunStatus::AwaitingSigner
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
        .apply_decision(ais_agent_core::runtime::SignerDecision {
            request_id: ais_agent_control::ids::SignerRequestId(request_id),
            kind: ais_agent_core::runtime::SignerDecisionKind::Submitted,
            decision_at_ms: None,
            tx_hash: Some(format!("{tx_hash:#x}")),
        });

    let signer = StepOnce::apply(&mut runtime).await;
    assert_eq!(
        signer.applied_transition.as_ref().map(|t| t.kind),
        Some(StepTransitionKind::Signer)
    );
    assert_eq!(
        runtime
            .checkpoint
            .pending_requests
            .pending_confirmation_id
            .as_deref(),
        Some("0x3333333333333333333333333333333333333333333333333333333333333333")
    );

    let verify = apply_live_evm_verify_with_provider(&mut runtime, &provider)
        .await
        .expect("verify transition");
    assert_eq!(
        verify.node_id.as_deref(),
        Some("verify.erc20_transfer.effect")
    );
    assert_eq!(
        runtime
            .checkpoint
            .action_graph
            .nodes
            .get("verify.erc20_transfer.effect")
            .map(|node| node.status.clone()),
        Some(ActionNodeStatus::Succeeded)
    );
    assert!(runtime
        .checkpoint
        .evidence_graph
        .records
        .contains_key("state.pre.recipient_token_balance"));
    assert!(runtime
        .checkpoint
        .evidence_graph
        .records
        .contains_key("state.post.recipient_token_balance"));
    assert!(runtime
        .checkpoint
        .evidence_graph
        .records
        .contains_key("effect.verify.erc20_transfer.effect"));

    let complete = StepOnce::apply(&mut runtime).await;
    assert_eq!(
        complete.applied_transition.as_ref().map(|t| t.kind),
        Some(StepTransitionKind::Complete)
    );
    assert_eq!(
        runtime.checkpoint.lifecycle.status,
        ais_agent_core::runtime::RunStatus::Completed
    );
}

#[tokio::test]
async fn erc20_transfer_action_family_can_restart_from_signer_submitted_side_effect_cut() {
    let tx_hash = b256!("4444444444444444444444444444444444444444444444444444444444444444");
    let mission = sample_erc20_transfer_action_family_mission();
    let mut checkpoint = checkpoint_with_nodes(Vec::new());
    seed_action_family_checkpoint(
        &mission,
        &mut checkpoint,
        &RuntimeExecutionWiring {
            evm_rpc_url: Some("http://127.0.0.1:8545".to_owned()),
            solana_rpc_url: None,
            native_transfer_enabled: false,
            erc20_transfer_enabled: true,
            uniswap_v3_swap_enabled: false,
            uniswap_v3_lp_enabled: false,
        },
    )
    .expect("erc20 transfer should seed");
    let mut runtime = ActiveRun::new(mission.clone(), checkpoint);

    let asserter = Asserter::new();
    let provider = ProviderBuilder::new().connect_mocked_client(asserter.clone());
    asserter.push_success(&encode_u256_return(90));
    asserter.push_success(&Bytes::default());

    apply_live_evm_observe_with_provider(&mut runtime, &provider)
        .await
        .expect("observe transition");
    apply_live_evm_simulate_with_provider(&mut runtime, &provider)
        .await
        .expect("simulate transition");
    StepOnce::apply(&mut runtime).await;

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
        .apply_decision(ais_agent_core::runtime::SignerDecision {
            request_id: ais_agent_control::ids::SignerRequestId(request_id),
            kind: ais_agent_core::runtime::SignerDecisionKind::Submitted,
            decision_at_ms: None,
            tx_hash: Some(format!("{tx_hash:#x}")),
        });
    StepOnce::apply(&mut runtime).await;

    let mut checkpoint_repo = InMemoryCheckpointRepository::default();
    persist_side_effect_checkpoint(&mut checkpoint_repo, &runtime)
        .expect("persist side-effect checkpoint");
    let mut mission_repo = InMemoryMissionRepository::default();
    mission_repo
        .insert(runtime.run_id.clone(), mission)
        .expect("insert mission");

    let mut restored = restore_active_run(
        &runtime.run_id,
        &mission_repo,
        &checkpoint_repo,
        &InMemorySignerStateArchive::default(),
    )
    .expect("restore runtime after signer submission");

    let verify_asserter = Asserter::new();
    let verify_provider = ProviderBuilder::new().connect_mocked_client(verify_asserter.clone());
    verify_asserter.push_success(&sample_receipt(tx_hash, 100));
    verify_asserter.push_success(&104u64);
    verify_asserter.push_success(&encode_u256_return(120));

    let verify = apply_live_evm_verify_with_provider(&mut restored, &verify_provider)
        .await
        .expect("verify transition");
    assert_eq!(
        verify.node_id.as_deref(),
        Some("verify.erc20_transfer.effect")
    );

    let complete = StepOnce::apply(&mut restored).await;
    assert_eq!(
        complete.applied_transition.as_ref().map(|t| t.kind),
        Some(StepTransitionKind::Complete)
    );
    assert_eq!(
        restored.checkpoint.lifecycle.status,
        ais_agent_core::runtime::RunStatus::Completed
    );
}

#[tokio::test]
async fn uniswap_v3_swap_action_family_can_complete_via_signer_submission_and_live_verify() {
    let tx_hash = b256!("5555555555555555555555555555555555555555555555555555555555555555");
    let mission = sample_uniswap_v3_swap_action_family_mission(false, false);
    let mut checkpoint = checkpoint_with_nodes(Vec::new());
    seed_action_family_checkpoint(
        &mission,
        &mut checkpoint,
        &RuntimeExecutionWiring {
            evm_rpc_url: Some("http://127.0.0.1:8545".to_owned()),
            solana_rpc_url: None,
            native_transfer_enabled: false,
            erc20_transfer_enabled: false,
            uniswap_v3_swap_enabled: true,
            uniswap_v3_lp_enabled: false,
        },
    )
    .expect("uniswap swap should seed");
    let mut runtime = ActiveRun::new(mission, checkpoint);

    let asserter = Asserter::new();
    let provider = ProviderBuilder::new().connect_mocked_client(asserter.clone());
    asserter.push_success(&encode_u256_return(90));
    asserter.push_success(&Bytes::default());
    asserter.push_success(&sample_receipt(tx_hash, 100));
    asserter.push_success(&104u64);
    asserter.push_success(&encode_u256_return(120));

    let observe = apply_live_evm_observe_with_provider(&mut runtime, &provider)
        .await
        .expect("observe transition");
    assert_eq!(
        observe.node_id.as_deref(),
        Some("observe.uniswap_v3_swap.recipient_out_balance")
    );

    let simulate = apply_live_evm_simulate_with_provider(&mut runtime, &provider)
        .await
        .expect("simulate transition");
    assert_eq!(
        simulate.node_id.as_deref(),
        Some("simulate.uniswap_v3_swap.call")
    );

    let govern = StepOnce::apply(&mut runtime).await;
    assert_eq!(
        govern.applied_transition.as_ref().map(|t| t.kind),
        Some(StepTransitionKind::Govern)
    );
    assert_eq!(
        runtime.checkpoint.lifecycle.status,
        ais_agent_core::runtime::RunStatus::AwaitingSigner
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
        .apply_decision(ais_agent_core::runtime::SignerDecision {
            request_id: ais_agent_control::ids::SignerRequestId(request_id),
            kind: ais_agent_core::runtime::SignerDecisionKind::Submitted,
            decision_at_ms: None,
            tx_hash: Some(format!("{tx_hash:#x}")),
        });

    let signer = StepOnce::apply(&mut runtime).await;
    assert_eq!(
        signer.applied_transition.as_ref().map(|t| t.kind),
        Some(StepTransitionKind::Signer)
    );

    let verify = apply_live_evm_verify_with_provider(&mut runtime, &provider)
        .await
        .expect("verify transition");
    assert_eq!(
        verify.node_id.as_deref(),
        Some("verify.uniswap_v3_swap.effect")
    );
    assert!(runtime
        .checkpoint
        .evidence_graph
        .records
        .contains_key("state.pre.uniswap_v3_swap.recipient_out_balance"));
    assert!(runtime
        .checkpoint
        .evidence_graph
        .records
        .contains_key("state.post.uniswap_v3_swap.recipient_out_balance"));
    assert!(runtime
        .checkpoint
        .evidence_graph
        .records
        .contains_key("effect.verify.uniswap_v3_swap.effect"));

    let mut saw_complete = false;
    for _ in 0..3 {
        let next = StepOnce::apply(&mut runtime).await;
        match next.applied_transition.as_ref().map(|t| t.kind) {
            Some(StepTransitionKind::Verify) => {}
            Some(StepTransitionKind::Complete) => {
                saw_complete = true;
                break;
            }
            other => panic!("unexpected follow-up transition after live verify: {other:?}"),
        }
    }

    assert!(
        saw_complete,
        "expected bounded follow-up transitions to reach complete"
    );
    assert_eq!(
        runtime.checkpoint.lifecycle.status,
        ais_agent_core::runtime::RunStatus::Completed
    );
}

#[tokio::test]
async fn uniswap_v3_swap_action_family_with_stale_quote_awaits_evidence() {
    let mission = sample_uniswap_v3_swap_action_family_mission(true, false);
    let mut checkpoint = checkpoint_with_nodes(Vec::new());
    seed_action_family_checkpoint(
        &mission,
        &mut checkpoint,
        &RuntimeExecutionWiring {
            evm_rpc_url: Some("http://127.0.0.1:8545".to_owned()),
            solana_rpc_url: None,
            native_transfer_enabled: false,
            erc20_transfer_enabled: false,
            uniswap_v3_swap_enabled: true,
            uniswap_v3_lp_enabled: false,
        },
    )
    .expect("uniswap swap should seed");
    let mut runtime = ActiveRun::new(mission, checkpoint);

    let asserter = Asserter::new();
    let provider = ProviderBuilder::new().connect_mocked_client(asserter.clone());
    asserter.push_success(&encode_u256_return(90));
    asserter.push_success(&Bytes::default());

    apply_live_evm_observe_with_provider(&mut runtime, &provider)
        .await
        .expect("observe transition");
    apply_live_evm_simulate_with_provider(&mut runtime, &provider)
        .await
        .expect("simulate transition");

    let govern = StepOnce::apply(&mut runtime).await;
    assert_eq!(
        govern.applied_transition.as_ref().map(|t| t.kind),
        Some(StepTransitionKind::Govern)
    );
    assert_eq!(
        runtime.checkpoint.lifecycle.status,
        ais_agent_core::runtime::RunStatus::AwaitingEvidence
    );
    assert_eq!(
        runtime
            .checkpoint
            .lifecycle
            .failure
            .as_ref()
            .map(|failure| &failure.code),
        Some(&RunFailureCode::StaleEvidence)
    );
    assert!(runtime
        .checkpoint
        .lifecycle
        .failure
        .as_ref()
        .is_some_and(|failure| failure
            .evidence_refs
            .contains(&"evidence.uniswap.swap.quote".to_owned())));
}

#[tokio::test]
async fn uniswap_v3_swap_action_family_with_approval_branch_can_complete() {
    let approval_tx_hash =
        b256!("6666666666666666666666666666666666666666666666666666666666666666");
    let swap_tx_hash = b256!("7777777777777777777777777777777777777777777777777777777777777777");
    let mission = sample_uniswap_v3_swap_action_family_mission(false, true);
    let mut checkpoint = checkpoint_with_nodes(Vec::new());
    seed_action_family_checkpoint(
        &mission,
        &mut checkpoint,
        &RuntimeExecutionWiring {
            evm_rpc_url: Some("http://127.0.0.1:8545".to_owned()),
            solana_rpc_url: None,
            native_transfer_enabled: false,
            erc20_transfer_enabled: false,
            uniswap_v3_swap_enabled: true,
            uniswap_v3_lp_enabled: false,
        },
    )
    .expect("uniswap swap approval path should seed");
    let mut runtime = ActiveRun::new(mission, checkpoint);

    let asserter = Asserter::new();
    let provider = ProviderBuilder::new().connect_mocked_client(asserter.clone());
    asserter.push_success(&encode_u256_return(90));
    asserter.push_success(&encode_u256_return(0));
    asserter.push_success(&Bytes::default());
    asserter.push_success(&sample_receipt(approval_tx_hash, 100));
    asserter.push_success(&104u64);
    asserter.push_success(&encode_u256_return(10_000_000));
    asserter.push_success(&Bytes::default());
    asserter.push_success(&sample_receipt(swap_tx_hash, 101));
    asserter.push_success(&105u64);
    asserter.push_success(&encode_u256_return(120));

    let first_observe = apply_live_evm_observe_with_provider(&mut runtime, &provider)
        .await
        .expect("first observe");
    let second_observe = apply_live_evm_observe_with_provider(&mut runtime, &provider)
        .await
        .expect("second observe");
    let observed = [
        first_observe.node_id.as_deref(),
        second_observe.node_id.as_deref(),
    ];
    assert!(observed.contains(&Some("observe.uniswap_v3_swap.recipient_out_balance")));
    assert!(observed.contains(&Some("observe.uniswap_v3_swap.allowance")));

    let simulate_approval = apply_live_evm_simulate_with_provider(&mut runtime, &provider)
        .await
        .expect("simulate approval");
    assert_eq!(
        simulate_approval.node_id.as_deref(),
        Some("simulate.uniswap_v3_swap.approve_call")
    );

    let govern_approval = StepOnce::apply(&mut runtime).await;
    assert_eq!(
        govern_approval.applied_transition.as_ref().map(|t| t.kind),
        Some(StepTransitionKind::Govern)
    );

    let approval_request_id = runtime
        .checkpoint
        .pending_requests
        .pending_signer_request_id
        .clone()
        .expect("approval signer request");
    runtime
        .pending_signer_state
        .as_mut()
        .expect("approval pending signer")
        .apply_decision(ais_agent_core::runtime::SignerDecision {
            request_id: ais_agent_control::ids::SignerRequestId(approval_request_id),
            kind: ais_agent_core::runtime::SignerDecisionKind::Submitted,
            decision_at_ms: None,
            tx_hash: Some(format!("{approval_tx_hash:#x}")),
        });

    let approval_signer = StepOnce::apply(&mut runtime).await;
    assert_eq!(
        approval_signer.applied_transition.as_ref().map(|t| t.kind),
        Some(StepTransitionKind::Signer)
    );

    let approval_verify = apply_live_evm_verify_with_provider(&mut runtime, &provider)
        .await
        .expect("verify approval");
    assert_eq!(
        approval_verify.node_id.as_deref(),
        Some("verify.uniswap_v3_swap.approval")
    );

    let simulate_swap = apply_live_evm_simulate_with_provider(&mut runtime, &provider)
        .await
        .expect("simulate swap");
    assert_eq!(
        simulate_swap.node_id.as_deref(),
        Some("simulate.uniswap_v3_swap.call")
    );

    let govern_swap = StepOnce::apply(&mut runtime).await;
    assert_eq!(
        govern_swap.applied_transition.as_ref().map(|t| t.kind),
        Some(StepTransitionKind::Govern)
    );

    let swap_request_id = runtime
        .checkpoint
        .pending_requests
        .pending_signer_request_id
        .clone()
        .expect("swap signer request");
    runtime
        .pending_signer_state
        .as_mut()
        .expect("swap pending signer")
        .apply_decision(ais_agent_core::runtime::SignerDecision {
            request_id: ais_agent_control::ids::SignerRequestId(swap_request_id),
            kind: ais_agent_core::runtime::SignerDecisionKind::Submitted,
            decision_at_ms: None,
            tx_hash: Some(format!("{swap_tx_hash:#x}")),
        });

    let swap_signer = StepOnce::apply(&mut runtime).await;
    assert_eq!(
        swap_signer.applied_transition.as_ref().map(|t| t.kind),
        Some(StepTransitionKind::Signer)
    );

    let swap_verify = apply_live_evm_verify_with_provider(&mut runtime, &provider)
        .await
        .expect("verify swap");
    assert_eq!(
        swap_verify.node_id.as_deref(),
        Some("verify.uniswap_v3_swap.effect")
    );

    let mut saw_complete = false;
    for _ in 0..4 {
        let next = StepOnce::apply(&mut runtime).await;
        match next.applied_transition.as_ref().map(|t| t.kind) {
            Some(StepTransitionKind::Verify) => {}
            Some(StepTransitionKind::Complete) => {
                saw_complete = true;
                break;
            }
            other => {
                panic!("unexpected follow-up transition after approval+swap verify: {other:?}")
            }
        }
    }

    assert!(
        saw_complete,
        "expected approval+swap branch to reach complete"
    );
    assert_eq!(
        runtime.checkpoint.lifecycle.status,
        ais_agent_core::runtime::RunStatus::Completed
    );
    assert!(runtime
        .checkpoint
        .evidence_graph
        .records
        .contains_key("state.post.uniswap_v3_swap.allowance"));
    assert!(runtime
        .checkpoint
        .evidence_graph
        .records
        .contains_key("effect.verify.uniswap_v3_swap.approval"));
    assert!(runtime
        .checkpoint
        .evidence_graph
        .records
        .contains_key("effect.verify.uniswap_v3_swap.effect"));
}

#[tokio::test]
async fn uniswap_v3_lp_action_family_mint_can_complete_via_signer_submission_and_live_verify() {
    let tx_hash = b256!("8888888888888888888888888888888888888888888888888888888888888888");
    let mission = sample_uniswap_v3_lp_action_family_mission();
    let mut checkpoint = checkpoint_with_nodes(Vec::new());
    seed_action_family_checkpoint(
        &mission,
        &mut checkpoint,
        &RuntimeExecutionWiring {
            evm_rpc_url: Some("http://127.0.0.1:8545".to_owned()),
            solana_rpc_url: None,
            native_transfer_enabled: false,
            erc20_transfer_enabled: false,
            uniswap_v3_swap_enabled: false,
            uniswap_v3_lp_enabled: true,
        },
    )
    .expect("uniswap lp should seed");
    let mut runtime = ActiveRun::new(mission, checkpoint);

    let asserter = Asserter::new();
    let provider = ProviderBuilder::new().connect_mocked_client(asserter.clone());
    asserter.push_success(&encode_u256_return(0));
    asserter.push_success(&Bytes::default());
    asserter.push_success(&sample_receipt(tx_hash, 100));
    asserter.push_success(&104u64);
    asserter.push_success(&encode_u256_return(1));

    let observe = apply_live_evm_observe_with_provider(&mut runtime, &provider)
        .await
        .expect("observe transition");
    assert_eq!(
        observe.node_id.as_deref(),
        Some("observe.uniswap_v3_lp.position_count")
    );

    let simulate = apply_live_evm_simulate_with_provider(&mut runtime, &provider)
        .await
        .expect("simulate transition");
    assert_eq!(
        simulate.node_id.as_deref(),
        Some("simulate.uniswap_v3_lp.mint_call")
    );

    let govern = StepOnce::apply(&mut runtime).await;
    assert_eq!(
        govern.applied_transition.as_ref().map(|t| t.kind),
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
        .apply_decision(ais_agent_core::runtime::SignerDecision {
            request_id: ais_agent_control::ids::SignerRequestId(request_id),
            kind: ais_agent_core::runtime::SignerDecisionKind::Submitted,
            decision_at_ms: None,
            tx_hash: Some(format!("{tx_hash:#x}")),
        });

    let signer = StepOnce::apply(&mut runtime).await;
    assert_eq!(
        signer.applied_transition.as_ref().map(|t| t.kind),
        Some(StepTransitionKind::Signer)
    );

    let verify = apply_live_evm_verify_with_provider(&mut runtime, &provider)
        .await
        .expect("verify transition");
    assert_eq!(
        verify.node_id.as_deref(),
        Some("verify.uniswap_v3_lp.effect")
    );
    assert!(runtime
        .checkpoint
        .evidence_graph
        .records
        .contains_key("state.pre.uniswap_v3_lp.position_count"));
    assert!(runtime
        .checkpoint
        .evidence_graph
        .records
        .contains_key("state.post.uniswap_v3_lp.position_count"));
    assert!(runtime
        .checkpoint
        .evidence_graph
        .records
        .contains_key("effect.verify.uniswap_v3_lp.effect"));

    let complete = StepOnce::apply(&mut runtime).await;
    assert_eq!(
        complete.applied_transition.as_ref().map(|t| t.kind),
        Some(StepTransitionKind::Complete)
    );
    assert_eq!(
        runtime.checkpoint.lifecycle.status,
        ais_agent_core::runtime::RunStatus::Completed
    );
}

#[tokio::test]
async fn uniswap_v3_lp_action_family_mint_can_restart_from_signer_submitted_side_effect_cut() {
    let tx_hash = b256!("9999999999999999999999999999999999999999999999999999999999999999");
    let mission = sample_uniswap_v3_lp_action_family_mission();
    let mut checkpoint = checkpoint_with_nodes(Vec::new());
    seed_action_family_checkpoint(
        &mission,
        &mut checkpoint,
        &RuntimeExecutionWiring {
            evm_rpc_url: Some("http://127.0.0.1:8545".to_owned()),
            solana_rpc_url: None,
            native_transfer_enabled: false,
            erc20_transfer_enabled: false,
            uniswap_v3_swap_enabled: false,
            uniswap_v3_lp_enabled: true,
        },
    )
    .expect("uniswap lp should seed");
    let mut runtime = ActiveRun::new(mission.clone(), checkpoint);

    let asserter = Asserter::new();
    let provider = ProviderBuilder::new().connect_mocked_client(asserter.clone());
    asserter.push_success(&encode_u256_return(0));
    asserter.push_success(&Bytes::default());

    apply_live_evm_observe_with_provider(&mut runtime, &provider)
        .await
        .expect("observe transition");
    apply_live_evm_simulate_with_provider(&mut runtime, &provider)
        .await
        .expect("simulate transition");
    StepOnce::apply(&mut runtime).await;

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
        .apply_decision(ais_agent_core::runtime::SignerDecision {
            request_id: ais_agent_control::ids::SignerRequestId(request_id),
            kind: ais_agent_core::runtime::SignerDecisionKind::Submitted,
            decision_at_ms: None,
            tx_hash: Some(format!("{tx_hash:#x}")),
        });
    StepOnce::apply(&mut runtime).await;

    let mut checkpoint_repo = InMemoryCheckpointRepository::default();
    persist_side_effect_checkpoint(&mut checkpoint_repo, &runtime)
        .expect("persist side-effect checkpoint");
    let mut mission_repo = InMemoryMissionRepository::default();
    mission_repo
        .insert(runtime.run_id.clone(), mission)
        .expect("insert mission");

    let mut restored = restore_active_run(
        &runtime.run_id,
        &mission_repo,
        &checkpoint_repo,
        &InMemorySignerStateArchive::default(),
    )
    .expect("restore runtime after signer submission");

    let verify_asserter = Asserter::new();
    let verify_provider = ProviderBuilder::new().connect_mocked_client(verify_asserter.clone());
    verify_asserter.push_success(&sample_receipt(tx_hash, 100));
    verify_asserter.push_success(&104u64);
    verify_asserter.push_success(&encode_u256_return(1));

    let verify = apply_live_evm_verify_with_provider(&mut restored, &verify_provider)
        .await
        .expect("verify transition");
    assert_eq!(
        verify.node_id.as_deref(),
        Some("verify.uniswap_v3_lp.effect")
    );

    let complete = StepOnce::apply(&mut restored).await;
    assert_eq!(
        complete.applied_transition.as_ref().map(|t| t.kind),
        Some(StepTransitionKind::Complete)
    );
    assert_eq!(
        restored.checkpoint.lifecycle.status,
        ais_agent_core::runtime::RunStatus::Completed
    );
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

fn sample_native_transfer_action_family_mission() -> Mission {
    Mission {
        mission_id: "mission-native-transfer".to_owned(),
        goal: "owliabot:native_transfer".to_owned(),
        allowed_chains: vec!["eip155:11155111".to_owned()],
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
        constraints: BTreeMap::from([
            (
                "owliabot_action_family".to_owned(),
                json!("native_transfer"),
            ),
            (
                "owliabot_submission".to_owned(),
                json!({
                    "payload": {
                        "chain": "11155111",
                        "recipient": "0x1111111111111111111111111111111111111111",
                        "requested_amount": "0.00000000000000003",
                        "asset_symbol": "ETH",
                        "sender_address_hint": "0x2222222222222222222222222222222222222222"
                    },
                    "evidence": {
                        "recipient": {
                            "user_input": "0x1111111111111111111111111111111111111111",
                            "normalized_address": "0x1111111111111111111111111111111111111111",
                            "source": "wallet_transfer",
                            "user_confirmed": true
                        },
                        "amount": {
                            "user_input": "0.00000000000000003",
                            "normalized_amount": "0.00000000000000003",
                            "atomic_amount": "30",
                            "decimals": 18,
                            "source": "wallet_transfer",
                            "user_confirmed": true
                        }
                    }
                }),
            ),
        ]),
        metadata: BTreeMap::new(),
    }
}

fn sample_erc20_transfer_action_family_mission() -> Mission {
    Mission {
        mission_id: "mission-erc20-transfer".to_owned(),
        goal: "owliabot:erc20_transfer".to_owned(),
        allowed_chains: vec!["eip155:11155111".to_owned()],
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
        constraints: BTreeMap::from([
            ("owliabot_action_family".to_owned(), json!("erc20_transfer")),
            (
                "owliabot_submission".to_owned(),
                json!({
                    "payload": {
                        "chain": "11155111",
                        "token_address": "0x3333333333333333333333333333333333333333",
                        "token_symbol": "USDC",
                        "recipient": "0x1111111111111111111111111111111111111111",
                        "requested_amount": "10",
                        "sender_address_hint": "0x2222222222222222222222222222222222222222"
                    },
                    "evidence": {
                        "recipient": {
                            "user_input": "0x1111111111111111111111111111111111111111",
                            "normalized_address": "0x1111111111111111111111111111111111111111",
                            "source": "wallet_transfer",
                            "user_confirmed": true
                        },
                        "amount": {
                            "user_input": "10",
                            "normalized_amount": "10",
                            "atomic_amount": "10000000",
                            "decimals": 6,
                            "source": "wallet_transfer",
                            "user_confirmed": true
                        },
                        "token": {
                            "token_address": "0x3333333333333333333333333333333333333333",
                            "token_symbol": "USDC",
                            "decimals": 6,
                            "resolution_source": "token_registry",
                            "user_confirmed": true
                        }
                    }
                }),
            ),
        ]),
        metadata: BTreeMap::new(),
    }
}

fn sample_uniswap_v3_swap_action_family_mission(
    stale_quote: bool,
    approval_required: bool,
) -> Mission {
    let (quoted_at_ms, expires_at_ms) = if stale_quote {
        (1u64, Some(2u64))
    } else {
        (4_102_444_800_000u64, Some(4_102_444_900_000u64))
    };

    Mission {
        mission_id: "mission-uniswap-v3-swap".to_owned(),
        goal: "owliabot:uniswap_v3_swap".to_owned(),
        allowed_chains: vec!["eip155:11155111".to_owned()],
        budget: MissionBudget {
            max_steps: Some(8),
            max_signer_requests: Some(if approval_required { 2 } else { 1 }),
            max_wall_clock_ms: Some(30_000),
        },
        policy: MissionPolicy {
            policy_mode: Some("guarded".to_owned()),
            allow_raw_envelopes: true,
            require_effect_contract_for_writes: true,
        },
        constraints: BTreeMap::from([
            (
                "owliabot_action_family".to_owned(),
                json!("uniswap_v3_swap"),
            ),
            (
                "owliabot_submission".to_owned(),
                json!({
                    "payload": {
                        "chain": "11155111",
                        "token_in_address": "0x3333333333333333333333333333333333333333",
                        "token_in_symbol": "USDC",
                        "token_out_address": "0x4444444444444444444444444444444444444444",
                        "token_out_symbol": "WETH",
                        "fee_tier": 3000,
                        "requested_amount": "10",
                        "amount_mode": "exact_in",
                        "slippage_bps": 50,
                        "deadline_seconds": 4102444800u64,
                        "router_address": "0x5555555555555555555555555555555555555555",
                        "recipient_address": "0x1111111111111111111111111111111111111111",
                        "sender_address_hint": "0x2222222222222222222222222222222222222222",
                        "unwrap_native_out": false
                    },
                    "evidence": {
                        "token_in": {
                            "token_address": "0x3333333333333333333333333333333333333333",
                            "token_symbol": "USDC",
                            "decimals": 6,
                            "resolution_source": "token_registry",
                            "user_confirmed": true
                        },
                        "token_out": {
                            "token_address": "0x4444444444444444444444444444444444444444",
                            "token_symbol": "WETH",
                            "decimals": 18,
                            "resolution_source": "token_registry",
                            "user_confirmed": true
                        },
                        "quote": {
                            "source": "quoter",
                            "quoted_at_ms": quoted_at_ms,
                            "expires_at_ms": expires_at_ms,
                            "route_summary": "USDC/WETH 0.3%",
                            "amount_in_atomic": "10000000",
                            "amount_out_atomic": "3000000000000000",
                            "min_amount_out_atomic": "2900000000000000",
                            "user_confirmed": true
                        },
                        "router": {
                            "router_address": "0x5555555555555555555555555555555555555555",
                            "approval_target_address": "0x5555555555555555555555555555555555555555",
                            "approval_required": approval_required,
                            "quoter_address": "0x6666666666666666666666666666666666666666",
                            "resolution_source": "sepolia_registry",
                            "user_confirmed": true
                        },
                        "deadline": {
                            "deadline_unix_seconds": 4102444800u64,
                            "source": "policy",
                            "user_confirmed": true
                        }
                    }
                }),
            ),
        ]),
        metadata: BTreeMap::new(),
    }
}

fn sample_uniswap_v3_lp_action_family_mission() -> Mission {
    Mission {
        mission_id: "mission-uniswap-v3-lp".to_owned(),
        goal: "owliabot:uniswap_v3_lp".to_owned(),
        allowed_chains: vec!["eip155:11155111".to_owned()],
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
        constraints: BTreeMap::from([
            ("owliabot_action_family".to_owned(), json!("uniswap_v3_lp")),
            (
                "owliabot_submission".to_owned(),
                json!({
                    "payload": {
                        "chain": "11155111",
                        "operation": "mint",
                        "token0_address": "0x3333333333333333333333333333333333333333",
                        "token0_symbol": "USDC",
                        "token1_address": "0x4444444444444444444444444444444444444444",
                        "token1_symbol": "WETH",
                        "fee_tier": 3000,
                        "desired_amount0": "10",
                        "desired_amount1": "0.003",
                        "tick_lower": -600,
                        "tick_upper": 600,
                        "position_manager_address": "0x1238536071E1c677A632429e3655c799b22cDA52",
                        "deadline_seconds": 4102444800u64,
                        "sender_address_hint": "0x2222222222222222222222222222222222222222"
                    },
                    "evidence": {
                        "token0": {
                            "token_address": "0x3333333333333333333333333333333333333333",
                            "token_symbol": "USDC",
                            "decimals": 6,
                            "resolution_source": "token_registry",
                            "user_confirmed": true
                        },
                        "token1": {
                            "token_address": "0x4444444444444444444444444444444444444444",
                            "token_symbol": "WETH",
                            "decimals": 18,
                            "resolution_source": "token_registry",
                            "user_confirmed": true
                        },
                        "pool": {
                            "pool_address": "0x5555555555555555555555555555555555555555",
                            "token0_address": "0x3333333333333333333333333333333333333333",
                            "token1_address": "0x4444444444444444444444444444444444444444",
                            "fee_tier": 3000,
                            "tick_spacing": 60,
                            "slot0_tick": 0,
                            "observed_at_ms": 4102444800000u64,
                            "resolution_source": "sepolia_registry",
                            "user_confirmed": true
                        },
                        "deadline": {
                            "deadline_unix_seconds": 4102444800u64,
                            "source": "policy",
                            "user_confirmed": true
                        }
                    }
                }),
            ),
        ]),
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
