use std::{collections::BTreeMap, str::FromStr};

use ais_agent_control::{
    commands::SubmitExecutionArtifactContinuationCommand,
    execution_artifact::{
        BranchStage, BranchTarget, ContinuationStage, EffectSpec, EvmTransactionCandidate,
        ExecutionArtifactActor, ExecutionArtifactLaunchSpec, ExecutionChainFamily, ExecutionStage,
        ExecutionTransactionCandidate, ObservationSpec, OutputExportSpec, PredicateSpec,
        TransactionStage, ValueRef,
    },
    ids::CommandId,
    ids::RunId,
    launch_spec::LaunchSpecSubmission,
    recovery::RunFailureCode,
};
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
    runtime::{RunLifecycleState, RunPhase, RunStatus},
};
use ais_agent_host::{
    control::HostCommandResponse,
    session::{HostRunLink, HostSessionId, HostSessionStore, InMemoryHostSessionStore},
};
use alloy::{
    consensus::{Receipt, ReceiptEnvelope},
    primitives::{address, b256, bytes, Address, Bytes, U256},
    providers::ProviderBuilder,
    rpc::types::TransactionReceipt,
    transports::mock::Asserter,
};
use serde_json::json;

use crate::{
    persistence::{
        persist_side_effect_checkpoint, restore_active_run, restore_active_run_from_parts,
        CheckpointArchiveEntry, CheckpointArchiveKind, CheckpointRepository,
        InMemoryCheckpointRepository, InMemoryEventArchive, InMemoryMissionRepository,
        InMemoryRunCatalogRepository, InMemorySignerStateArchive, MissionRepository,
    },
    runtime::{ActiveRun, InMemoryRunRepository, RunRepository},
    service::{seed_launch_spec_checkpoint, RuntimeExecutionWiring, RuntimeHostService},
    stepper::{
        apply_execution_artifact_transition, apply_live_evm_broadcast_with_provider,
        apply_live_evm_observe_with_provider, apply_live_evm_simulate_with_provider,
        apply_live_evm_verify_with_provider, StepOnce, StepTransitionKind,
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

    if runtime.checkpoint.lifecycle.status != RunStatus::Completed {
        let complete = StepOnce::apply(&mut runtime).await;
        assert_eq!(
            complete.applied_transition.as_ref().map(|t| t.kind),
            Some(StepTransitionKind::Complete)
        );
    }
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
async fn native_transfer_launch_spec_can_complete_via_signer_submission_and_live_verify() {
    let tx_hash = b256!("1111111111111111111111111111111111111111111111111111111111111111");
    let mission = sample_native_transfer_mission();
    let mut checkpoint = checkpoint_with_nodes(Vec::new());
    seed_launch_spec_checkpoint(
        &mut checkpoint,
        &RuntimeExecutionWiring {
            evm_rpc_url: Some("http://127.0.0.1:8545".to_owned()),
            solana_rpc_url: None,
            allowed_protocol_packages: vec!["owliabot.transfer".to_owned()],
        },
        &sample_native_transfer_launch_spec(),
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
    asserter.push_success(&U256::from(120u64));

    let observe = apply_live_evm_observe_with_provider(&mut runtime, &provider)
        .await
        .expect("observe transition");
    assert_eq!(
        observe.node_id.as_deref(),
        Some("artifact.stage.transfer.pre_observe.state.pre.recipient_balance")
    );

    let simulate = apply_live_evm_simulate_with_provider(&mut runtime, &provider)
        .await
        .expect("simulate transition");
    assert_eq!(
        simulate.node_id.as_deref(),
        Some("artifact.stage.transfer.simulate")
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
        Some("artifact.stage.transfer.verify")
    );
    assert_eq!(
        runtime
            .checkpoint
            .action_graph
            .nodes
            .get("artifact.stage.transfer.verify")
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
        .contains_key("effect.artifact.stage.transfer.verify"));

    let post_observe = apply_live_evm_observe_with_provider(&mut runtime, &provider)
        .await
        .expect("post observe transition");
    assert_eq!(
        post_observe.node_id.as_deref(),
        Some("artifact.stage.transfer.post_observe.state.post.recipient_balance")
    );
    assert!(runtime
        .checkpoint
        .evidence_graph
        .records
        .contains_key("state.post.recipient_balance"));

    let export = StepOnce::apply(&mut runtime).await;
    assert_eq!(
        export.applied_transition.as_ref().map(|t| t.kind),
        Some(StepTransitionKind::Artifact)
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
}

#[tokio::test]
async fn query_first_native_transfer_artifact_can_branch_into_write_path() {
    let tx_hash = b256!("1212121212121212121212121212121212121212121212121212121212121212");
    let mission = sample_native_transfer_mission();
    let mut checkpoint = checkpoint_with_nodes(Vec::new());
    let launch_spec =
        LaunchSpecSubmission::ExecutionArtifact(sample_query_first_native_transfer_launch_spec());
    seed_launch_spec_checkpoint(
        &mut checkpoint,
        &RuntimeExecutionWiring {
            evm_rpc_url: Some("http://127.0.0.1:8545".to_owned()),
            solana_rpc_url: None,
            allowed_protocol_packages: vec!["owliabot.transfer".to_owned()],
        },
        &launch_spec,
    )
    .expect("query-first native transfer should seed");
    let mut runtime = ActiveRun::new(mission, checkpoint);

    let asserter = Asserter::new();
    let provider = ProviderBuilder::new().connect_mocked_client(asserter.clone());
    asserter.push_success(&777u64);
    asserter.push_success(&U256::from(90u64));
    asserter.push_success(&Bytes::default());
    asserter.push_success(&sample_receipt(tx_hash, 100));
    asserter.push_success(&104u64);
    asserter.push_success(&U256::from(120u64));
    asserter.push_success(&U256::from(120u64));

    let query_observe = apply_live_evm_observe_with_provider(&mut runtime, &provider)
        .await
        .expect("query observe transition");
    assert_eq!(
        query_observe.node_id.as_deref(),
        Some("artifact.stage.query_block.observe")
    );

    let export_query = StepOnce::apply(&mut runtime).await;
    assert_eq!(
        export_query.applied_transition.as_ref().map(|t| t.kind),
        Some(StepTransitionKind::Artifact)
    );
    assert_eq!(
        runtime
            .checkpoint
            .execution_artifact
            .as_ref()
            .and_then(|snapshot| snapshot.exported_outputs.get(&"query.block_number".into()))
            .and_then(|value| value.as_u64()),
        Some(777)
    );
    assert_eq!(
        runtime
            .checkpoint
            .execution_artifact
            .as_ref()
            .and_then(|snapshot| snapshot.active_stage_id.as_ref())
            .map(|stage| stage.as_str()),
        Some("stage.transfer_gate")
    );

    let branch = StepOnce::apply(&mut runtime).await;
    assert_eq!(
        branch.applied_transition.as_ref().map(|t| t.kind),
        Some(StepTransitionKind::Artifact)
    );
    assert_eq!(
        runtime
            .checkpoint
            .execution_artifact
            .as_ref()
            .and_then(|snapshot| snapshot.active_stage_id.as_ref())
            .map(|stage| stage.as_str()),
        Some("stage.transfer")
    );

    let observe = apply_live_evm_observe_with_provider(&mut runtime, &provider)
        .await
        .expect("pre observe transition");
    assert_eq!(
        observe.node_id.as_deref(),
        Some("artifact.stage.transfer.pre_observe.state.pre.recipient_balance")
    );

    let simulate = apply_live_evm_simulate_with_provider(&mut runtime, &provider)
        .await
        .expect("simulate transition");
    assert_eq!(
        simulate.node_id.as_deref(),
        Some("artifact.stage.transfer.simulate")
    );

    let govern = StepOnce::apply(&mut runtime).await;
    assert_eq!(
        govern.applied_transition.as_ref().map(|t| t.kind),
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
        Some("artifact.stage.transfer.verify")
    );

    let post_observe = apply_live_evm_observe_with_provider(&mut runtime, &provider)
        .await
        .expect("post observe transition");
    assert_eq!(
        post_observe.node_id.as_deref(),
        Some("artifact.stage.transfer.post_observe.state.post.recipient_balance")
    );

    let export = StepOnce::apply(&mut runtime).await;
    assert_eq!(
        export.applied_transition.as_ref().map(|t| t.kind),
        Some(StepTransitionKind::Artifact)
    );

    let complete = StepOnce::apply(&mut runtime).await;
    assert_eq!(
        complete.applied_transition.as_ref().map(|t| t.kind),
        Some(StepTransitionKind::Complete)
    );
    assert_eq!(runtime.checkpoint.lifecycle.status, RunStatus::Completed);
}

#[tokio::test]
async fn native_transfer_launch_spec_can_restart_from_signer_submitted_side_effect_cut() {
    let tx_hash = b256!("2222222222222222222222222222222222222222222222222222222222222222");
    let mission = sample_native_transfer_mission();
    let mut checkpoint = checkpoint_with_nodes(Vec::new());
    seed_launch_spec_checkpoint(
        &mut checkpoint,
        &RuntimeExecutionWiring {
            evm_rpc_url: Some("http://127.0.0.1:8545".to_owned()),
            solana_rpc_url: None,
            allowed_protocol_packages: vec!["owliabot.transfer".to_owned()],
        },
        &sample_native_transfer_launch_spec(),
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
    verify_asserter.push_success(&U256::from(120u64));

    let verify = apply_live_evm_verify_with_provider(&mut restored, &verify_provider)
        .await
        .expect("verify transition");
    assert_eq!(
        verify.node_id.as_deref(),
        Some("artifact.stage.transfer.verify")
    );

    let post_observe = apply_live_evm_observe_with_provider(&mut restored, &verify_provider)
        .await
        .expect("post observe transition");
    assert_eq!(
        post_observe.node_id.as_deref(),
        Some("artifact.stage.transfer.post_observe.state.post.recipient_balance")
    );

    let export = StepOnce::apply(&mut restored).await;
    assert_eq!(
        export.applied_transition.as_ref().map(|t| t.kind),
        Some(StepTransitionKind::Artifact)
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
async fn erc20_transfer_launch_spec_fail_closes_on_undecodable_token_observe() {
    let mission = sample_erc20_transfer_mission();
    let mut checkpoint = checkpoint_with_nodes(Vec::new());
    seed_launch_spec_checkpoint(
        &mut checkpoint,
        &RuntimeExecutionWiring {
            evm_rpc_url: Some("http://127.0.0.1:8545".to_owned()),
            solana_rpc_url: None,
            allowed_protocol_packages: vec!["owliabot.transfer".to_owned()],
        },
        &sample_erc20_transfer_launch_spec(),
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
        Some("artifact.stage.transfer.pre_observe.state.pre.recipient_token_balance")
    );
    assert_eq!(
        runtime.checkpoint.lifecycle.status,
        ais_agent_core::runtime::RunStatus::Paused
    );
    assert!(runtime.checkpoint.lifecycle.failure.is_some());
    assert_eq!(
        runtime
            .checkpoint
            .action_graph
            .nodes
            .get("artifact.stage.transfer.pre_observe.state.pre.recipient_token_balance")
            .map(|node| node.status.clone()),
        Some(ActionNodeStatus::Blocked)
    );
}

#[tokio::test]
async fn erc20_transfer_launch_spec_can_complete_via_signer_submission_and_live_verify() {
    let tx_hash = b256!("3333333333333333333333333333333333333333333333333333333333333333");
    let mission = sample_erc20_transfer_mission();
    let mut checkpoint = checkpoint_with_nodes(Vec::new());
    seed_launch_spec_checkpoint(
        &mut checkpoint,
        &RuntimeExecutionWiring {
            evm_rpc_url: Some("http://127.0.0.1:8545".to_owned()),
            solana_rpc_url: None,
            allowed_protocol_packages: vec!["owliabot.transfer".to_owned()],
        },
        &sample_erc20_transfer_launch_spec(),
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
    asserter.push_success(&encode_u256_return(120));

    let observe = apply_live_evm_observe_with_provider(&mut runtime, &provider)
        .await
        .expect("observe transition");
    assert_eq!(
        observe.node_id.as_deref(),
        Some("artifact.stage.transfer.pre_observe.state.pre.recipient_token_balance")
    );

    let simulate = apply_live_evm_simulate_with_provider(&mut runtime, &provider)
        .await
        .expect("simulate transition");
    assert_eq!(
        simulate.node_id.as_deref(),
        Some("artifact.stage.transfer.simulate")
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
        Some("artifact.stage.transfer.verify")
    );
    assert_eq!(
        runtime
            .checkpoint
            .action_graph
            .nodes
            .get("artifact.stage.transfer.verify")
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
        .contains_key("effect.artifact.stage.transfer.verify"));

    let post_observe = apply_live_evm_observe_with_provider(&mut runtime, &provider)
        .await
        .expect("post observe transition");
    assert_eq!(
        post_observe.node_id.as_deref(),
        Some("artifact.stage.transfer.post_observe.state.post.recipient_token_balance")
    );
    assert!(runtime
        .checkpoint
        .evidence_graph
        .records
        .contains_key("state.post.recipient_token_balance"));

    let export = StepOnce::apply(&mut runtime).await;
    assert_eq!(
        export.applied_transition.as_ref().map(|t| t.kind),
        Some(StepTransitionKind::Artifact)
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
}

#[tokio::test]
async fn erc20_transfer_launch_spec_can_restart_from_signer_submitted_side_effect_cut() {
    let tx_hash = b256!("4444444444444444444444444444444444444444444444444444444444444444");
    let mission = sample_erc20_transfer_mission();
    let mut checkpoint = checkpoint_with_nodes(Vec::new());
    seed_launch_spec_checkpoint(
        &mut checkpoint,
        &RuntimeExecutionWiring {
            evm_rpc_url: Some("http://127.0.0.1:8545".to_owned()),
            solana_rpc_url: None,
            allowed_protocol_packages: vec!["owliabot.transfer".to_owned()],
        },
        &sample_erc20_transfer_launch_spec(),
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
    verify_asserter.push_success(&encode_u256_return(120));

    let verify = apply_live_evm_verify_with_provider(&mut restored, &verify_provider)
        .await
        .expect("verify transition");
    assert_eq!(
        verify.node_id.as_deref(),
        Some("artifact.stage.transfer.verify")
    );

    let post_observe = apply_live_evm_observe_with_provider(&mut restored, &verify_provider)
        .await
        .expect("post observe transition");
    assert_eq!(
        post_observe.node_id.as_deref(),
        Some("artifact.stage.transfer.post_observe.state.post.recipient_token_balance")
    );

    let export = StepOnce::apply(&mut restored).await;
    assert_eq!(
        export.applied_transition.as_ref().map(|t| t.kind),
        Some(StepTransitionKind::Artifact)
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
async fn execution_artifact_uniswap_exact_in_can_complete_via_generic_branching_and_live_verify() {
    let tx_hash = b256!("9191919191919191919191919191919191919191919191919191919191919191");
    let wiring = RuntimeExecutionWiring {
        evm_rpc_url: Some("http://127.0.0.1:8545".to_owned()),
        solana_rpc_url: None,
        allowed_protocol_packages: vec!["owliabot.uniswap_v3".to_owned()],
    };
    let mut checkpoint = checkpoint_with_nodes(Vec::new());
    seed_launch_spec_checkpoint(
        &mut checkpoint,
        &wiring,
        &LaunchSpecSubmission::ExecutionArtifact(sample_uniswap_exact_in_execution_artifact(
            false, false,
        )),
    )
    .expect("generic uniswap artifact should seed");
    let mut runtime = ActiveRun::new(sample_uniswap_artifact_mission(1), checkpoint);

    let quote_branch = StepOnce::apply(&mut runtime).await;
    assert_eq!(
        quote_branch.applied_transition.as_ref().map(|t| t.kind),
        Some(StepTransitionKind::Artifact)
    );
    let approval_branch = StepOnce::apply(&mut runtime).await;
    assert_eq!(
        approval_branch.applied_transition.as_ref().map(|t| t.kind),
        Some(StepTransitionKind::Artifact)
    );

    let asserter = Asserter::new();
    let provider = ProviderBuilder::new().connect_mocked_client(asserter.clone());
    asserter.push_success(&Bytes::default());
    asserter.push_success(&sample_receipt(tx_hash, 100));
    asserter.push_success(&104u64);

    let simulate = apply_live_evm_simulate_with_provider(&mut runtime, &provider)
        .await
        .expect("simulate swap");
    assert_eq!(
        simulate.node_id.as_deref(),
        Some("artifact.stage.swap.simulate")
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
        .expect("swap signer request");
    runtime
        .pending_signer_state
        .as_mut()
        .expect("swap pending signer")
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
        .expect("verify swap");
    assert_eq!(
        verify.node_id.as_deref(),
        Some("artifact.stage.swap.verify")
    );

    let follow_up = step_until_status(&mut runtime, RunStatus::Completed, 8).await;
    assert_eq!(follow_up.last(), Some(&StepTransitionKind::Complete));
    assert!(follow_up.contains(&StepTransitionKind::Artifact));
    assert_eq!(runtime.checkpoint.lifecycle.status, RunStatus::Completed);
}

#[tokio::test]
async fn execution_artifact_uniswap_exact_in_with_approval_branch_can_complete_via_generic_runtime()
{
    let approval_tx_hash =
        b256!("9292929292929292929292929292929292929292929292929292929292929292");
    let swap_tx_hash = b256!("9393939393939393939393939393939393939393939393939393939393939393");
    let wiring = RuntimeExecutionWiring {
        evm_rpc_url: Some("http://127.0.0.1:8545".to_owned()),
        solana_rpc_url: None,
        allowed_protocol_packages: vec!["owliabot.uniswap_v3".to_owned()],
    };
    let mut checkpoint = checkpoint_with_nodes(Vec::new());
    seed_launch_spec_checkpoint(
        &mut checkpoint,
        &wiring,
        &LaunchSpecSubmission::ExecutionArtifact(sample_uniswap_exact_in_execution_artifact(
            false, true,
        )),
    )
    .expect("generic uniswap approval artifact should seed");
    let mut runtime = ActiveRun::new(sample_uniswap_artifact_mission(2), checkpoint);

    StepOnce::apply(&mut runtime).await;
    StepOnce::apply(&mut runtime).await;

    let asserter = Asserter::new();
    let provider = ProviderBuilder::new().connect_mocked_client(asserter.clone());
    asserter.push_success(&Bytes::default());
    asserter.push_success(&sample_receipt(approval_tx_hash, 100));
    asserter.push_success(&104u64);
    asserter.push_success(&Bytes::default());
    asserter.push_success(&sample_receipt(swap_tx_hash, 101));
    asserter.push_success(&105u64);

    let simulate_approval = apply_live_evm_simulate_with_provider(&mut runtime, &provider)
        .await
        .expect("simulate approval");
    assert_eq!(
        simulate_approval.node_id.as_deref(),
        Some("artifact.stage.approve.simulate")
    );

    let approval_request_id = step_until_pending_signer_request(&mut runtime, 4).await;
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
    StepOnce::apply(&mut runtime).await;

    let approval_verify = apply_live_evm_verify_with_provider(&mut runtime, &provider)
        .await
        .expect("verify approval");
    assert_eq!(
        approval_verify.node_id.as_deref(),
        Some("artifact.stage.approve.verify")
    );

    let approval_advance = StepOnce::apply(&mut runtime).await;
    assert_eq!(
        approval_advance.applied_transition.as_ref().map(|t| t.kind),
        Some(StepTransitionKind::Artifact)
    );

    let simulate_swap = apply_live_evm_simulate_with_provider(&mut runtime, &provider)
        .await
        .expect("simulate swap");
    assert_eq!(
        simulate_swap.node_id.as_deref(),
        Some("artifact.stage.swap.simulate")
    );

    let swap_request_id = step_until_pending_signer_request(&mut runtime, 4).await;
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
    StepOnce::apply(&mut runtime).await;

    let swap_verify = apply_live_evm_verify_with_provider(&mut runtime, &provider)
        .await
        .expect("verify swap");
    assert_eq!(
        swap_verify.node_id.as_deref(),
        Some("artifact.stage.swap.verify")
    );

    let follow_up = step_until_status(&mut runtime, RunStatus::Completed, 8).await;
    assert_eq!(follow_up.last(), Some(&StepTransitionKind::Complete));
    assert_eq!(runtime.checkpoint.lifecycle.status, RunStatus::Completed);
}

#[tokio::test]
async fn execution_artifact_uniswap_exact_in_stale_quote_fails_closed_on_branch_assert() {
    let wiring = RuntimeExecutionWiring {
        evm_rpc_url: Some("http://127.0.0.1:8545".to_owned()),
        solana_rpc_url: None,
        allowed_protocol_packages: vec!["owliabot.uniswap_v3".to_owned()],
    };
    let mut checkpoint = checkpoint_with_nodes(Vec::new());
    seed_launch_spec_checkpoint(
        &mut checkpoint,
        &wiring,
        &LaunchSpecSubmission::ExecutionArtifact(sample_uniswap_exact_in_execution_artifact(
            true, false,
        )),
    )
    .expect("stale quote artifact should seed");
    let mut runtime = ActiveRun::new(sample_uniswap_artifact_mission(1), checkpoint);

    let branch = StepOnce::apply(&mut runtime).await;
    assert_eq!(
        branch.applied_transition.as_ref().map(|t| t.kind),
        Some(StepTransitionKind::Artifact)
    );
    assert_eq!(runtime.checkpoint.lifecycle.status, RunStatus::Paused);
    assert_eq!(
        runtime
            .checkpoint
            .lifecycle
            .failure
            .as_ref()
            .map(|failure| &failure.code),
        Some(&RunFailureCode::VerifyMismatch)
    );
    assert!(runtime
        .checkpoint
        .lifecycle
        .failure
        .as_ref()
        .is_some_and(|failure| failure.summary.contains("stale_quote")));
}

#[tokio::test]
async fn execution_artifact_uniswap_trading_api_swap_can_complete_without_protocol_binder() {
    let tx_hash = b256!("9494949494949494949494949494949494949494949494949494949494949494");
    let wiring = RuntimeExecutionWiring {
        evm_rpc_url: Some("http://127.0.0.1:8545".to_owned()),
        solana_rpc_url: None,
        allowed_protocol_packages: vec!["owliabot.uniswap_v3".to_owned()],
    };
    let artifact = sample_uniswap_trading_api_execution_artifact();
    let expected_swap_calldata = artifact
        .transactions
        .iter()
        .find_map(|candidate| match candidate {
            ExecutionTransactionCandidate::EvmTransaction(candidate)
                if candidate.candidate_id.as_str() == "swap.direct" =>
            {
                candidate.calldata.clone()
            }
            _ => None,
        })
        .expect("swap calldata");

    let mut checkpoint = checkpoint_with_nodes(Vec::new());
    seed_launch_spec_checkpoint(
        &mut checkpoint,
        &wiring,
        &LaunchSpecSubmission::ExecutionArtifact(artifact),
    )
    .expect("trading api artifact should seed");
    let mut runtime = ActiveRun::new(sample_uniswap_artifact_mission(1), checkpoint);

    let quote_branch = StepOnce::apply(&mut runtime).await;
    assert_eq!(
        quote_branch.applied_transition.as_ref().map(|t| t.kind),
        Some(StepTransitionKind::Artifact)
    );
    let approval_branch = StepOnce::apply(&mut runtime).await;
    assert_eq!(
        approval_branch.applied_transition.as_ref().map(|t| t.kind),
        Some(StepTransitionKind::Artifact)
    );

    match &runtime
        .checkpoint
        .action_graph
        .nodes
        .get("artifact.stage.swap.simulate")
        .expect("swap simulate node")
        .payload
    {
        ActionPayload::Simulate(action) => {
            let SimulateLiveBinding::Evm(live) = action.live.as_ref().expect("evm simulate live")
            else {
                panic!("expected evm simulate binding");
            };
            assert_eq!(
                format!("{:#x}", live.request.to),
                "0xe592427a0aece92de3edee1f18e0157c05861564"
            );
            assert_eq!(
                format!("0x{}", hex_encode_bytes(live.request.data.as_ref())),
                expected_swap_calldata
            );
        }
        other => panic!("unexpected swap simulate payload: {other:?}"),
    }

    let asserter = Asserter::new();
    let provider = ProviderBuilder::new().connect_mocked_client(asserter.clone());
    asserter.push_success(&Bytes::default());
    asserter.push_success(&sample_receipt(tx_hash, 100));
    asserter.push_success(&104u64);

    apply_live_evm_simulate_with_provider(&mut runtime, &provider)
        .await
        .expect("simulate swap");
    let request_id = step_until_pending_signer_request(&mut runtime, 4).await;
    runtime
        .pending_signer_state
        .as_mut()
        .expect("swap pending signer")
        .apply_decision(ais_agent_core::runtime::SignerDecision {
            request_id: ais_agent_control::ids::SignerRequestId(request_id),
            kind: ais_agent_core::runtime::SignerDecisionKind::Submitted,
            decision_at_ms: None,
            tx_hash: Some(format!("{tx_hash:#x}")),
        });
    StepOnce::apply(&mut runtime).await;
    apply_live_evm_verify_with_provider(&mut runtime, &provider)
        .await
        .expect("verify swap");
    let follow_up = step_until_status(&mut runtime, RunStatus::Completed, 8).await;
    assert_eq!(follow_up.last(), Some(&StepTransitionKind::Complete));
    assert_eq!(runtime.checkpoint.lifecycle.status, RunStatus::Completed);
}

#[tokio::test]
async fn execution_artifact_uniswap_swap_can_continue_into_aave_supply_from_exported_output() {
    let swap_tx_hash = b256!("1212121212121212121212121212121212121212121212121212121212121212");
    let supply_tx_hash = b256!("3434343434343434343434343434343434343434343434343434343434343434");
    let wiring = RuntimeExecutionWiring {
        evm_rpc_url: Some("http://127.0.0.1:8545".to_owned()),
        solana_rpc_url: None,
        allowed_protocol_packages: vec![
            "owliabot.uniswap_v3".to_owned(),
            "owliabot.aave_v3".to_owned(),
        ],
    };

    let mut checkpoint = checkpoint_with_nodes(Vec::new());
    seed_launch_spec_checkpoint(
        &mut checkpoint,
        &wiring,
        &LaunchSpecSubmission::ExecutionArtifact(sample_uniswap_swap_to_aave_execution_artifact()),
    )
    .expect("uniswap continuation artifact should seed");
    let mut runtime = ActiveRun::new(sample_swap_to_aave_artifact_mission(), checkpoint);

    let swap_asserter = Asserter::new();
    let swap_provider = ProviderBuilder::new().connect_mocked_client(swap_asserter.clone());
    swap_asserter.push_success(&encode_u256_return(100));
    swap_asserter.push_success(&Bytes::default());
    swap_asserter.push_success(&sample_receipt(swap_tx_hash, 100));
    swap_asserter.push_success(&104u64);
    swap_asserter.push_success(&encode_u256_return(125));

    let pre_observe = apply_live_evm_observe_with_provider(&mut runtime, &swap_provider)
        .await
        .expect("pre observe transition");
    assert_eq!(
        pre_observe.node_id.as_deref(),
        Some("artifact.stage.swap.pre_observe.state.pre.swap.recipient_out_balance")
    );

    let simulate = apply_live_evm_simulate_with_provider(&mut runtime, &swap_provider)
        .await
        .expect("swap simulate transition");
    assert_eq!(
        simulate.node_id.as_deref(),
        Some("artifact.stage.swap.simulate")
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

    let signer_request_id = runtime
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
            request_id: ais_agent_control::ids::SignerRequestId(signer_request_id),
            kind: ais_agent_core::runtime::SignerDecisionKind::Submitted,
            decision_at_ms: None,
            tx_hash: Some(format!("{swap_tx_hash:#x}")),
        });

    let signer = StepOnce::apply(&mut runtime).await;
    assert_eq!(
        signer.applied_transition.as_ref().map(|t| t.kind),
        Some(StepTransitionKind::Signer)
    );
    assert_eq!(
        runtime.checkpoint.lifecycle.status,
        ais_agent_core::runtime::RunStatus::AwaitingConfirmation
    );

    let verify = apply_live_evm_verify_with_provider(&mut runtime, &swap_provider)
        .await
        .expect("swap verify transition");
    assert_eq!(
        verify.node_id.as_deref(),
        Some("artifact.stage.swap.verify")
    );

    let post_observe = apply_live_evm_observe_with_provider(&mut runtime, &swap_provider)
        .await
        .expect("post observe transition");
    assert_eq!(
        post_observe.node_id.as_deref(),
        Some("artifact.stage.swap.post_observe.state.post.swap.recipient_out_balance")
    );
    assert_eq!(
        runtime
            .checkpoint
            .action_graph
            .nodes
            .get("artifact.stage.swap.post_observe.state.post.swap.recipient_out_balance")
            .map(|node| node.status.clone()),
        Some(ActionNodeStatus::Succeeded),
        "unexpected post-observe summary: {} status: {:?} failure: {:?}",
        post_observe.summary,
        runtime
            .checkpoint
            .action_graph
            .nodes
            .get("artifact.stage.swap.post_observe.state.post.swap.recipient_out_balance")
            .map(|node| node.status.clone()),
        runtime.checkpoint.lifecycle.failure
    );

    let export = StepOnce::apply(&mut runtime).await;
    assert_eq!(
        export
            .applied_transition
            .as_ref()
            .map(|transition| transition.kind),
        Some(StepTransitionKind::Artifact)
    );
    let wait = StepOnce::apply(&mut runtime).await;
    assert_eq!(
        wait.applied_transition
            .as_ref()
            .map(|transition| transition.kind),
        Some(StepTransitionKind::Artifact)
    );
    assert_eq!(
        runtime.checkpoint.lifecycle.status,
        ais_agent_core::runtime::RunStatus::AwaitingArtifactContinuation
    );

    let received_atomic = runtime
        .checkpoint
        .execution_artifact
        .as_ref()
        .and_then(|snapshot| {
            snapshot
                .exported_outputs
                .get(&"swap.received_atomic".into())
        })
        .and_then(|value| value.as_str())
        .expect("received output")
        .to_owned();
    assert_eq!(received_atomic, "25");
    assert_eq!(
        runtime
            .checkpoint
            .execution_artifact
            .as_ref()
            .and_then(|snapshot| snapshot.exported_outputs.get(&"swap.tx_hash".into()))
            .and_then(|value| value.as_str()),
        Some("0x1212121212121212121212121212121212121212121212121212121212121212")
    );

    let token_out_address = runtime
        .checkpoint
        .execution_artifact
        .as_ref()
        .and_then(|snapshot| {
            snapshot
                .exported_outputs
                .get(&"swap.token_out_address".into())
        })
        .and_then(|value| value.as_str())
        .expect("token out output")
        .to_owned();
    let recipient_address = runtime
        .checkpoint
        .execution_artifact
        .as_ref()
        .and_then(|snapshot| {
            snapshot
                .exported_outputs
                .get(&"swap.recipient_address".into())
        })
        .and_then(|value| value.as_str())
        .expect("recipient output")
        .to_owned();

    let continuation_artifact = build_aave_supply_continuation_artifact(
        &received_atomic,
        &token_out_address,
        &recipient_address,
    );
    let expected_supply_calldata =
        aave_supply_calldata(&token_out_address, &received_atomic, &recipient_address);
    let mut resumed = submit_continuation_artifact(runtime, continuation_artifact, &wiring).await;

    match &resumed
        .checkpoint
        .action_graph
        .nodes
        .get("artifact.stage.supply.simulate")
        .expect("supply simulate node")
        .payload
    {
        ActionPayload::Simulate(action) => {
            let SimulateLiveBinding::Evm(live) = action.live.as_ref().expect("evm simulate live")
            else {
                panic!("expected evm simulate binding");
            };
            assert_eq!(live.request.data, expected_supply_calldata);
        }
        other => panic!("unexpected supply simulate payload: {other:?}"),
    }

    let supply_asserter = Asserter::new();
    let supply_provider = ProviderBuilder::new().connect_mocked_client(supply_asserter.clone());
    supply_asserter.push_success(&Bytes::default());
    supply_asserter.push_success(&sample_receipt(supply_tx_hash, 101));
    supply_asserter.push_success(&105u64);

    let supply_simulate = apply_live_evm_simulate_with_provider(&mut resumed, &supply_provider)
        .await
        .expect("supply simulate transition");
    assert_eq!(
        supply_simulate.node_id.as_deref(),
        Some("artifact.stage.supply.simulate")
    );

    let supply_govern = StepOnce::apply(&mut resumed).await;
    assert_eq!(
        supply_govern.applied_transition.as_ref().map(|t| t.kind),
        Some(StepTransitionKind::Govern)
    );

    let supply_signer_request_id = resumed
        .checkpoint
        .pending_requests
        .pending_signer_request_id
        .clone()
        .expect("supply signer request");
    resumed
        .pending_signer_state
        .as_mut()
        .expect("supply pending signer")
        .apply_decision(ais_agent_core::runtime::SignerDecision {
            request_id: ais_agent_control::ids::SignerRequestId(supply_signer_request_id),
            kind: ais_agent_core::runtime::SignerDecisionKind::Submitted,
            decision_at_ms: None,
            tx_hash: Some(format!("{supply_tx_hash:#x}")),
        });

    let supply_signer = StepOnce::apply(&mut resumed).await;
    assert_eq!(
        supply_signer.applied_transition.as_ref().map(|t| t.kind),
        Some(StepTransitionKind::Signer)
    );

    let supply_verify = apply_live_evm_verify_with_provider(&mut resumed, &supply_provider)
        .await
        .expect("supply verify transition");
    assert_eq!(
        supply_verify.node_id.as_deref(),
        Some("artifact.stage.supply.verify")
    );

    let artifact_complete =
        apply_execution_artifact_transition(&mut resumed).expect("supply artifact transition");
    let complete = StepOnce::apply(&mut resumed).await;
    assert_eq!(artifact_complete.kind, StepTransitionKind::Artifact);
    assert_eq!(
        complete
            .applied_transition
            .as_ref()
            .map(|transition| transition.kind),
        Some(StepTransitionKind::Complete)
    );
    assert_eq!(
        resumed.checkpoint.lifecycle.status,
        ais_agent_core::runtime::RunStatus::Completed
    );
    assert_eq!(
        resumed
            .checkpoint
            .execution_artifact
            .as_ref()
            .and_then(|snapshot| snapshot
                .exported_outputs
                .get(&"swap.received_atomic".into()))
            .and_then(|value| value.as_str()),
        Some("25")
    );
}

#[tokio::test]
async fn execution_artifact_uniswap_v3_lp_mint_can_complete_via_generic_runtime() {
    let tx_hash = b256!("8a8a8a8a8a8a8a8a8a8a8a8a8a8a8a8a8a8a8a8a8a8a8a8a8a8a8a8a8a8a8a8a");
    let wiring = RuntimeExecutionWiring {
        evm_rpc_url: Some("http://127.0.0.1:8545".to_owned()),
        solana_rpc_url: None,
        allowed_protocol_packages: vec!["owliabot.uniswap_v3".to_owned()],
    };
    let mut checkpoint = checkpoint_with_nodes(Vec::new());
    seed_launch_spec_checkpoint(
        &mut checkpoint,
        &wiring,
        &LaunchSpecSubmission::ExecutionArtifact(sample_uniswap_v3_lp_execution_artifact()),
    )
    .expect("generic uniswap lp artifact should seed");
    let mut runtime = ActiveRun::new(sample_uniswap_lp_artifact_mission(1), checkpoint);

    let asserter = Asserter::new();
    let provider = ProviderBuilder::new().connect_mocked_client(asserter.clone());
    asserter.push_success(&encode_u256_return(0));
    asserter.push_success(&Bytes::default());
    asserter.push_success(&sample_receipt(tx_hash, 100));
    asserter.push_success(&104u64);
    asserter.push_success(&encode_u256_return(1));

    let observe = apply_live_evm_observe_with_provider(&mut runtime, &provider)
        .await
        .expect("observe lp position count");
    assert_eq!(
        observe.node_id.as_deref(),
        Some("artifact.stage.mint.pre_observe.state.pre.uniswap_v3_lp.position_count")
    );

    let simulate = apply_live_evm_simulate_with_provider(&mut runtime, &provider)
        .await
        .expect("simulate lp mint");
    assert_eq!(
        simulate.node_id.as_deref(),
        Some("artifact.stage.mint.simulate")
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
        .expect("lp signer request");
    runtime
        .pending_signer_state
        .as_mut()
        .expect("lp pending signer")
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
        .expect("verify lp mint");
    assert_eq!(
        verify.node_id.as_deref(),
        Some("artifact.stage.mint.verify")
    );

    let post_observe = apply_live_evm_observe_with_provider(&mut runtime, &provider)
        .await
        .expect("observe post lp position count");
    assert_eq!(
        post_observe.node_id.as_deref(),
        Some("artifact.stage.mint.post_observe.state.post.uniswap_v3_lp.position_count")
    );
    assert_eq!(
        runtime
            .checkpoint
            .action_graph
            .nodes
            .get("artifact.stage.mint.post_observe.state.post.uniswap_v3_lp.position_count")
            .map(|node| node.status.clone()),
        Some(ActionNodeStatus::Succeeded),
        "unexpected post-observe summary: {} status: {:?} failure: {:?}",
        post_observe.summary,
        runtime
            .checkpoint
            .action_graph
            .nodes
            .get("artifact.stage.mint.post_observe.state.post.uniswap_v3_lp.position_count")
            .map(|node| node.status.clone()),
        runtime.checkpoint.lifecycle.failure
    );

    let export = StepOnce::apply(&mut runtime).await;
    assert_eq!(
        export
            .applied_transition
            .as_ref()
            .map(|transition| transition.kind),
        Some(StepTransitionKind::Artifact)
    );

    let complete = StepOnce::apply(&mut runtime).await;
    assert_eq!(
        complete
            .applied_transition
            .as_ref()
            .map(|transition| transition.kind),
        Some(StepTransitionKind::Complete)
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
    assert_eq!(runtime.checkpoint.lifecycle.status, RunStatus::Completed);
}

#[tokio::test]
async fn uniswap_v3_lp_launch_spec_mint_can_complete_via_owliabot_boundary_signer_submission_and_live_verify(
) {
    let tx_hash = b256!("8888888888888888888888888888888888888888888888888888888888888888");
    let mission = sample_uniswap_lp_artifact_mission(1);
    let mut checkpoint = checkpoint_with_nodes(Vec::new());
    seed_launch_spec_checkpoint(
        &mut checkpoint,
        &RuntimeExecutionWiring {
            evm_rpc_url: Some("http://127.0.0.1:8545".to_owned()),
            solana_rpc_url: None,
            allowed_protocol_packages: vec!["owliabot.uniswap_v3".to_owned()],
        },
        &sample_uniswap_v3_lp_launch_spec(),
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
        Some("artifact.stage.mint.pre_observe.state.pre.uniswap_v3_lp.position_count")
    );

    let simulate = apply_live_evm_simulate_with_provider(&mut runtime, &provider)
        .await
        .expect("simulate transition");
    assert_eq!(
        simulate.node_id.as_deref(),
        Some("artifact.stage.mint.simulate")
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
    let tx_hash_string = format!("{tx_hash:#x}");
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
        Some(tx_hash_string.as_str())
    );

    let verify = apply_live_evm_verify_with_provider(&mut runtime, &provider)
        .await
        .expect("verify transition");
    assert_eq!(
        verify.node_id.as_deref(),
        Some("artifact.stage.mint.verify")
    );
    assert!(runtime
        .checkpoint
        .evidence_graph
        .records
        .contains_key("state.pre.uniswap_v3_lp.position_count"));
    let post_observe = apply_live_evm_observe_with_provider(&mut runtime, &provider)
        .await
        .expect("post observe transition");
    assert_eq!(
        post_observe.node_id.as_deref(),
        Some("artifact.stage.mint.post_observe.state.post.uniswap_v3_lp.position_count")
    );
    assert!(runtime
        .checkpoint
        .evidence_graph
        .records
        .contains_key("state.post.uniswap_v3_lp.position_count"));

    let export = StepOnce::apply(&mut runtime).await;
    assert_eq!(
        export.applied_transition.as_ref().map(|t| t.kind),
        Some(StepTransitionKind::Artifact)
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
}

#[tokio::test]
async fn uniswap_v3_lp_launch_spec_mint_can_restart_from_owliabot_boundary_signer_submitted_side_effect_cut(
) {
    let tx_hash = b256!("9999999999999999999999999999999999999999999999999999999999999999");
    let mission = sample_uniswap_lp_artifact_mission(1);
    let mut checkpoint = checkpoint_with_nodes(Vec::new());
    seed_launch_spec_checkpoint(
        &mut checkpoint,
        &RuntimeExecutionWiring {
            evm_rpc_url: Some("http://127.0.0.1:8545".to_owned()),
            solana_rpc_url: None,
            allowed_protocol_packages: vec!["owliabot.uniswap_v3".to_owned()],
        },
        &sample_uniswap_v3_lp_launch_spec(),
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
    let tx_hash_string = format!("{tx_hash:#x}");
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
        Some(tx_hash_string.as_str())
    );

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
        Some("artifact.stage.mint.verify")
    );

    let post_observe = apply_live_evm_observe_with_provider(&mut restored, &verify_provider)
        .await
        .expect("post observe transition");
    assert_eq!(
        post_observe.node_id.as_deref(),
        Some("artifact.stage.mint.post_observe.state.post.uniswap_v3_lp.position_count")
    );

    let export = StepOnce::apply(&mut restored).await;
    assert_eq!(
        export.applied_transition.as_ref().map(|t| t.kind),
        Some(StepTransitionKind::Artifact)
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
            require_effect_contract_for_writes: false,
        },
        constraints: BTreeMap::new(),
        metadata: BTreeMap::new(),
    }
}

fn sample_native_transfer_mission() -> Mission {
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
            require_effect_contract_for_writes: false,
        },
        constraints: BTreeMap::new(),
        metadata: BTreeMap::new(),
    }
}

fn sample_erc20_transfer_mission() -> Mission {
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
            require_effect_contract_for_writes: false,
        },
        constraints: BTreeMap::new(),
        metadata: BTreeMap::new(),
    }
}

fn sample_swap_to_aave_artifact_mission() -> Mission {
    Mission {
        mission_id: "mission-swap-to-aave".to_owned(),
        goal: "owliabot:swap_to_aave_supply".to_owned(),
        allowed_chains: vec!["8453".to_owned()],
        budget: MissionBudget {
            max_steps: Some(16),
            max_signer_requests: Some(2),
            max_wall_clock_ms: Some(60_000),
        },
        policy: MissionPolicy {
            policy_mode: Some("guarded".to_owned()),
            allow_raw_envelopes: true,
            require_effect_contract_for_writes: false,
        },
        constraints: BTreeMap::new(),
        metadata: BTreeMap::from([("proof".to_owned(), json!("m37.swap_to_aave"))]),
    }
}

fn sample_native_transfer_launch_spec() -> LaunchSpecSubmission {
    LaunchSpecSubmission::ExecutionArtifact(ExecutionArtifactLaunchSpec {
        protocol_package_id: "owliabot.transfer".to_owned(),
        action_key: "native_transfer".to_owned(),
        chain_family: ExecutionChainFamily::Evm,
        allowed_chains: vec!["11155111".to_owned()],
        entry_stage_id: "stage.transfer".into(),
        actor: Some(ExecutionArtifactActor {
            sender_address_hint: Some("0x2222222222222222222222222222222222222222".to_owned()),
            recipient_address: Some("0x1111111111111111111111111111111111111111".to_owned()),
        }),
        transactions: vec![ExecutionTransactionCandidate::EvmTransaction(
            EvmTransactionCandidate {
                candidate_id: "transfer.call".into(),
                to: "0x1111111111111111111111111111111111111111".to_owned(),
                value: Some("30".to_owned()),
                calldata: None,
            },
        )],
        stages: vec![ExecutionStage::Transaction(TransactionStage {
            stage_id: "stage.transfer".into(),
            candidate_ref: "transfer.call".into(),
            exports: Vec::new(),
            next_stage_id: None,
        })],
        observations: Vec::new(),
        preconditions: vec![ObservationSpec {
            observation_id: "state.pre.recipient_balance".to_owned(),
            kind: "evm.native_balance".to_owned(),
            params: BTreeMap::from([(
                "address".to_owned(),
                json!("0x1111111111111111111111111111111111111111"),
            )]),
        }],
        postconditions: vec![ObservationSpec {
            observation_id: "state.post.recipient_balance".to_owned(),
            kind: "evm.native_balance".to_owned(),
            params: BTreeMap::from([(
                "address".to_owned(),
                json!("0x1111111111111111111111111111111111111111"),
            )]),
        }],
        expected_effects: vec![sample_native_transfer_expected_effect()],
        execution_policy: None,
        evidence: json!({
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
        }),
        metadata: BTreeMap::from([(
            "builder".to_owned(),
            json!("buildNativeTransferExecutionArtifact"),
        )]),
    })
}

fn sample_query_first_native_transfer_launch_spec() -> ExecutionArtifactLaunchSpec {
    let LaunchSpecSubmission::ExecutionArtifact(mut spec) = sample_native_transfer_launch_spec()
    else {
        unreachable!("native transfer launch spec should be an execution artifact");
    };

    spec.entry_stage_id = "stage.query_block".into();
    spec.observations = vec![ObservationSpec {
        observation_id: "query.block_number".to_owned(),
        kind: "evm.block_number".to_owned(),
        params: BTreeMap::new(),
    }];
    spec.stages = vec![
        ExecutionStage::Observe(ais_agent_control::execution_artifact::ObserveStage {
            stage_id: "stage.query_block".into(),
            observation_ref: "query.block_number".to_owned(),
            exports: vec![OutputExportSpec {
                output_key: "query.block_number".into(),
                source: ValueRef::Ref {
                    reference: "refs.evidence.query.block_number.block_number".to_owned(),
                },
            }],
            next_stage_id: Some("stage.transfer_gate".into()),
        }),
        ExecutionStage::Branch(BranchStage {
            stage_id: "stage.transfer_gate".into(),
            predicate: PredicateSpec::Comparison {
                left: ValueRef::Ref {
                    reference: "refs.outputs.query.block_number".to_owned(),
                },
                op: ais_agent_control::execution_artifact::ComparisonOperator::Gt,
                right: ValueRef::Literal { value: json!(0) },
            },
            if_true: BranchTarget::GotoStage {
                stage_id: "stage.transfer".into(),
            },
            if_false: BranchTarget::Assert {
                failure_code: "missing_chain_head".to_owned(),
                message: "query-first transfer requires a positive block number".to_owned(),
            },
        }),
        ExecutionStage::Transaction(TransactionStage {
            stage_id: "stage.transfer".into(),
            candidate_ref: "transfer.call".into(),
            exports: Vec::new(),
            next_stage_id: None,
        }),
    ];

    spec
}

fn sample_erc20_transfer_launch_spec() -> LaunchSpecSubmission {
    LaunchSpecSubmission::ExecutionArtifact(ExecutionArtifactLaunchSpec {
        protocol_package_id: "owliabot.transfer".to_owned(),
        action_key: "erc20_transfer".to_owned(),
        chain_family: ExecutionChainFamily::Evm,
        allowed_chains: vec!["11155111".to_owned()],
        entry_stage_id: "stage.transfer".into(),
        actor: Some(ExecutionArtifactActor {
            sender_address_hint: Some("0x2222222222222222222222222222222222222222".to_owned()),
            recipient_address: Some("0x1111111111111111111111111111111111111111".to_owned()),
        }),
        transactions: vec![ExecutionTransactionCandidate::EvmTransaction(
            EvmTransactionCandidate {
                candidate_id: "transfer.call".into(),
                to: "0x3333333333333333333333333333333333333333".to_owned(),
                value: Some("0".to_owned()),
                calldata: Some(
                    "0xa9059cbb00000000000000000000000011111111111111111111111111111111111111110000000000000000000000000000000000000000000000000000000000989680".to_owned(),
                ),
            },
        )],
        stages: vec![ExecutionStage::Transaction(TransactionStage {
            stage_id: "stage.transfer".into(),
            candidate_ref: "transfer.call".into(),
            exports: Vec::new(),
            next_stage_id: None,
        })],
        observations: Vec::new(),
        preconditions: vec![ObservationSpec {
            observation_id: "state.pre.recipient_token_balance".to_owned(),
            kind: "evm.erc20_balance_of".to_owned(),
            params: BTreeMap::from([
                (
                    "token".to_owned(),
                    json!("0x3333333333333333333333333333333333333333"),
                ),
                (
                    "owner".to_owned(),
                    json!("0x1111111111111111111111111111111111111111"),
                ),
            ]),
        }],
        postconditions: vec![ObservationSpec {
            observation_id: "state.post.recipient_token_balance".to_owned(),
            kind: "evm.erc20_balance_of".to_owned(),
            params: BTreeMap::from([
                (
                    "token".to_owned(),
                    json!("0x3333333333333333333333333333333333333333"),
                ),
                (
                    "owner".to_owned(),
                    json!("0x1111111111111111111111111111111111111111"),
                ),
            ]),
        }],
        expected_effects: vec![sample_erc20_transfer_expected_effect()],
        execution_policy: None,
        evidence: json!({
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
        }),
        metadata: BTreeMap::from([(
            "builder".to_owned(),
            json!("buildErc20TransferExecutionArtifact"),
        )]),
    })
}

fn sample_native_transfer_expected_effect() -> EffectSpec {
    EffectSpec {
        effect_id: "effect.transfer".to_owned(),
        stage_id: "stage.transfer".into(),
        kind: "asset_delta".to_owned(),
        params: BTreeMap::from([
            (
                "assertions".to_owned(),
                json!([{
                    "expression": "receipt.status == true && post.decoded_u256 != pre.decoded_u256",
                    "description": "native transfer must change recipient balance"
                }]),
            ),
            (
                "pre_observation_id".to_owned(),
                json!("state.pre.recipient_balance"),
            ),
            (
                "post_observation_id".to_owned(),
                json!("state.post.recipient_balance"),
            ),
            (
                "tolerance_hint".to_owned(),
                json!("recipient balance delta"),
            ),
        ]),
    }
}

fn sample_erc20_transfer_expected_effect() -> EffectSpec {
    EffectSpec {
        effect_id: "effect.transfer".to_owned(),
        stage_id: "stage.transfer".into(),
        kind: "asset_delta".to_owned(),
        params: BTreeMap::from([
            (
                "assertions".to_owned(),
                json!([{
                    "expression": "receipt.status == true && post.decoded_u256 != pre.decoded_u256",
                    "description": "erc20 transfer must change recipient token balance"
                }]),
            ),
            (
                "pre_observation_id".to_owned(),
                json!("state.pre.recipient_token_balance"),
            ),
            (
                "post_observation_id".to_owned(),
                json!("state.post.recipient_token_balance"),
            ),
            (
                "tolerance_hint".to_owned(),
                json!("recipient token balance delta"),
            ),
        ]),
    }
}

fn sample_uniswap_v3_lp_launch_spec() -> LaunchSpecSubmission {
    LaunchSpecSubmission::ExecutionArtifact(ExecutionArtifactLaunchSpec {
        protocol_package_id: "owliabot.uniswap_v3".to_owned(),
        action_key: "uniswap_v3_lp".to_owned(),
        chain_family: ExecutionChainFamily::Evm,
        allowed_chains: vec!["8453".to_owned()],
        entry_stage_id: "stage.mint".into(),
        actor: Some(ExecutionArtifactActor {
            sender_address_hint: Some("0x742d35Cc6634C0532925a3b844Bc9e7595f8fE00".to_owned()),
            recipient_address: Some("0x742d35Cc6634C0532925a3b844Bc9e7595f8fE00".to_owned()),
        }),
        transactions: vec![ExecutionTransactionCandidate::EvmTransaction(
            EvmTransactionCandidate {
                candidate_id: "lp.mint".into(),
                to: "0x1234567890abcdef1234567890ABCDEF12345678".to_owned(),
                value: Some("0".to_owned()),
                calldata: Some("0x8831645600".to_owned()),
            },
        )],
        stages: vec![ExecutionStage::Transaction(TransactionStage {
            stage_id: "stage.mint".into(),
            candidate_ref: "lp.mint".into(),
            exports: Vec::new(),
            next_stage_id: None,
        })],
        observations: Vec::new(),
        preconditions: vec![ObservationSpec {
            observation_id: "state.pre.uniswap_v3_lp.position_count".to_owned(),
            kind: "evm.contract_state_read".to_owned(),
            params: BTreeMap::from([
                (
                    "to".to_owned(),
                    json!("0x1234567890abcdef1234567890ABCDEF12345678"),
                ),
                (
                    "data".to_owned(),
                    json!("0x70a08231000000000000000000000000742d35cc6634c0532925a3b844bc9e7595f8fe00"),
                ),
            ]),
        }],
        postconditions: vec![ObservationSpec {
            observation_id: "state.post.uniswap_v3_lp.position_count".to_owned(),
            kind: "evm.contract_state_read".to_owned(),
            params: BTreeMap::from([
                (
                    "to".to_owned(),
                    json!("0x1234567890abcdef1234567890ABCDEF12345678"),
                ),
                (
                    "data".to_owned(),
                    json!("0x70a08231000000000000000000000000742d35cc6634c0532925a3b844bc9e7595f8fe00"),
                ),
            ]),
        }],
        expected_effects: Vec::new(),
        execution_policy: None,
        evidence: json!({
            "token0": {
                "token_address": "0x3333333333333333333333333333333333333333",
                "token_symbol": "USDC",
                "decimals": 6,
                "resolution_source": "wallet_transfer",
                "user_confirmed": true
            },
            "token1": {
                "token_address": "0x4444444444444444444444444444444444444444",
                "token_symbol": "WETH",
                "decimals": 18,
                "resolution_source": "wallet_transfer",
                "user_confirmed": true
            },
            "pool": {
                "pool_address": "0x6666666666666666666666666666666666666666",
                "token0_address": "0x3333333333333333333333333333333333333333",
                "token1_address": "0x4444444444444444444444444444444444444444",
                "fee_tier": 3000,
                "tick_spacing": 60,
                "slot0_sqrt_price_x96": "79228162514264337593543950336",
                "slot0_tick": 0,
                "observed_at_ms": 1710000000000u64,
                "resolution_source": "pool_state",
                "user_confirmed": true
            },
            "deadline": {
                "deadline_unix_seconds": 4102444800u64,
                "source": "wallet_transfer",
                "user_confirmed": true
            }
        }),
        metadata: sample_owliabot_uniswap_v3_lp_metadata(),
    })
}

fn sample_uniswap_artifact_mission(max_signer_requests: u32) -> Mission {
    Mission {
        mission_id: "mission-uniswap-artifact".to_owned(),
        goal: "owliabot:owliabot.uniswap_v3:uniswap_v3_swap".to_owned(),
        allowed_chains: vec!["8453".to_owned()],
        budget: MissionBudget {
            max_steps: Some(16),
            max_signer_requests: Some(max_signer_requests),
            max_wall_clock_ms: Some(60_000),
        },
        policy: MissionPolicy {
            policy_mode: Some("guarded".to_owned()),
            allow_raw_envelopes: true,
            require_effect_contract_for_writes: false,
        },
        constraints: BTreeMap::from([
            ("owliabot_action_key".to_owned(), json!("uniswap_v3_swap")),
            (
                "owliabot_protocol_package_id".to_owned(),
                json!("owliabot.uniswap_v3"),
            ),
            ("owliabot_execution_mode".to_owned(), json!("harness")),
        ]),
        metadata: BTreeMap::from([
            ("owliabot_agent_id".to_owned(), json!("test-agent")),
            ("source".to_owned(), json!("m37.generic_uniswap_proof")),
        ]),
    }
}

fn sample_uniswap_lp_artifact_mission(max_signer_requests: u32) -> Mission {
    Mission {
        mission_id: "mission-uniswap-lp-artifact".to_owned(),
        goal: "owliabot:owliabot.uniswap_v3:uniswap_v3_lp".to_owned(),
        allowed_chains: vec!["8453".to_owned()],
        budget: MissionBudget {
            max_steps: Some(16),
            max_signer_requests: Some(max_signer_requests),
            max_wall_clock_ms: Some(60_000),
        },
        policy: MissionPolicy {
            policy_mode: Some("guarded".to_owned()),
            allow_raw_envelopes: true,
            require_effect_contract_for_writes: false,
        },
        constraints: BTreeMap::from([
            ("owliabot_action_key".to_owned(), json!("uniswap_v3_lp")),
            (
                "owliabot_protocol_package_id".to_owned(),
                json!("owliabot.uniswap_v3"),
            ),
            ("owliabot_execution_mode".to_owned(), json!("harness")),
        ]),
        metadata: BTreeMap::from([
            ("owliabot_agent_id".to_owned(), json!("test-agent")),
            ("source".to_owned(), json!("m37.generic_uniswap_lp_proof")),
        ]),
    }
}

async fn step_until_pending_signer_request(runtime: &mut ActiveRun, max_steps: usize) -> String {
    let mut transitions = Vec::new();
    for _ in 0..max_steps {
        if let Some(request_id) = runtime
            .checkpoint
            .pending_requests
            .pending_signer_request_id
            .clone()
        {
            return request_id;
        }

        let next = StepOnce::apply(runtime).await;
        transitions.push(
            next.applied_transition
                .as_ref()
                .map(|transition| transition.kind),
        );
        assert!(
            next.applied_transition.is_some(),
            "expected pending signer request within {max_steps} step(s); transitions={transitions:?} status={:?} active_stage={:?}",
            runtime.checkpoint.lifecycle.status,
            runtime
                .checkpoint
                .execution_artifact
                .as_ref()
                .and_then(|snapshot| snapshot.active_stage_id.clone())
        );
    }

    runtime
        .checkpoint
        .pending_requests
        .pending_signer_request_id
        .clone()
        .unwrap_or_else(|| {
            panic!(
                "expected pending signer request within {max_steps} step(s); transitions={transitions:?} status={:?} active_stage={:?}",
                runtime.checkpoint.lifecycle.status,
                runtime
                    .checkpoint
                    .execution_artifact
                    .as_ref()
                    .and_then(|snapshot| snapshot.active_stage_id.clone())
            )
        })
}

async fn step_until_status(
    runtime: &mut ActiveRun,
    status: RunStatus,
    max_steps: usize,
) -> Vec<StepTransitionKind> {
    let mut transitions = Vec::new();
    for _ in 0..max_steps {
        if runtime.checkpoint.lifecycle.status == status {
            return transitions;
        }

        let next = StepOnce::apply(runtime).await;
        let kind = next
            .applied_transition
            .as_ref()
            .map(|transition| transition.kind)
            .unwrap_or_else(|| panic!("expected transition before reaching status `{status:?}`"));
        transitions.push(kind);
    }

    assert_eq!(
        runtime.checkpoint.lifecycle.status,
        status,
        "transitions={transitions:?} active_stage={:?} pending_confirmation={:?}",
        runtime
            .checkpoint
            .execution_artifact
            .as_ref()
            .and_then(|snapshot| snapshot.active_stage_id.clone()),
        runtime.checkpoint.pending_requests.pending_confirmation_id
    );
    transitions
}

fn sample_uniswap_exact_in_execution_artifact(
    stale_quote: bool,
    approval_required: bool,
) -> ExecutionArtifactLaunchSpec {
    let now_ms = 4_102_444_800_000u64;
    let expires_at_ms = if stale_quote {
        now_ms.saturating_sub(1)
    } else {
        now_ms + 60_000
    };

    ExecutionArtifactLaunchSpec {
        protocol_package_id: "owliabot.uniswap_v3".to_owned(),
        action_key: "uniswap_v3_swap".to_owned(),
        chain_family: ExecutionChainFamily::Evm,
        allowed_chains: vec!["8453".to_owned()],
        entry_stage_id: "stage.quote_freshness".into(),
        actor: Some(
            ais_agent_control::execution_artifact::ExecutionArtifactActor {
                sender_address_hint: Some("0x742d35Cc6634C0532925a3b844Bc9e7595f8fE00".to_owned()),
                recipient_address: Some("0x742d35Cc6634C0532925a3b844Bc9e7595f8fE00".to_owned()),
            },
        ),
        transactions: vec![
            ExecutionTransactionCandidate::EvmTransaction(EvmTransactionCandidate {
                candidate_id: "approve.direct".into(),
                to: "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48".to_owned(),
                value: Some("0".to_owned()),
                calldata: Some(format!(
                    "0x{}",
                    hex_encode_bytes(
                        erc20_approve_calldata(
                            "0xE592427A0AEce92De3Edee1F18E0157C05861564",
                            "25000000",
                        )
                        .as_ref()
                    )
                )),
            }),
            ExecutionTransactionCandidate::EvmTransaction(EvmTransactionCandidate {
                candidate_id: "swap.direct".into(),
                to: "0xE592427A0AEce92De3Edee1F18E0157C05861564".to_owned(),
                value: Some("0".to_owned()),
                calldata: Some(format!(
                    "0x{}",
                    hex_encode_bytes(
                        uniswap_exact_input_single_calldata("25000000", "9900000000000000")
                            .as_ref()
                    )
                )),
            }),
        ],
        stages: vec![
            ExecutionStage::Branch(BranchStage {
                stage_id: "stage.quote_freshness".into(),
                predicate: PredicateSpec::Comparison {
                    left: ValueRef::Ref {
                        reference: "refs.evidence.quote.expires_at_ms".to_owned(),
                    },
                    op: ais_agent_control::execution_artifact::ComparisonOperator::Gte,
                    right: ValueRef::Ref {
                        reference: "refs.evidence.clock.now_ms".to_owned(),
                    },
                },
                if_true: BranchTarget::GotoStage {
                    stage_id: "stage.approval_required".into(),
                },
                if_false: BranchTarget::Assert {
                    failure_code: "stale_quote".to_owned(),
                    message: "quote evidence is stale".to_owned(),
                },
            }),
            ExecutionStage::Branch(BranchStage {
                stage_id: "stage.approval_required".into(),
                predicate: PredicateSpec::Comparison {
                    left: ValueRef::Ref {
                        reference: "refs.evidence.router.approval_required".to_owned(),
                    },
                    op: ais_agent_control::execution_artifact::ComparisonOperator::Eq,
                    right: ValueRef::Literal { value: json!(true) },
                },
                if_true: BranchTarget::GotoStage {
                    stage_id: "stage.approve".into(),
                },
                if_false: BranchTarget::GotoStage {
                    stage_id: "stage.swap".into(),
                },
            }),
            ExecutionStage::Transaction(TransactionStage {
                stage_id: "stage.approve".into(),
                candidate_ref: "approve.direct".into(),
                exports: Vec::new(),
                next_stage_id: Some("stage.swap".into()),
            }),
            ExecutionStage::Transaction(TransactionStage {
                stage_id: "stage.swap".into(),
                candidate_ref: "swap.direct".into(),
                exports: Vec::new(),
                next_stage_id: None,
            }),
        ],
        observations: Vec::new(),
        preconditions: Vec::new(),
        postconditions: Vec::new(),
        expected_effects: Vec::new(),
        execution_policy: None,
        evidence: json!({
            "clock": {
                "now_ms": now_ms,
            },
            "quote": {
                "source": "uniswap.quote",
                "quoted_at_ms": now_ms.saturating_sub(15_000),
                "expires_at_ms": expires_at_ms,
                "amount_in_atomic": "25000000",
                "amount_out_atomic": "10000000000000000",
                "min_amount_out_atomic": "9900000000000000",
            },
            "router": {
                "approval_required": approval_required,
            }
        }),
        metadata: BTreeMap::from([("builder".to_owned(), json!("m37.uniswap_exact_in"))]),
    }
}

fn sample_uniswap_trading_api_execution_artifact() -> ExecutionArtifactLaunchSpec {
    let now_ms = 4_102_444_800_000u64;
    let swap_calldata = format!(
        "0x{}",
        hex_encode_bytes(
            uniswap_exact_input_single_calldata("25000000", "9900000000000000").as_ref()
        )
    );
    ExecutionArtifactLaunchSpec {
        protocol_package_id: "owliabot.uniswap_v3".to_owned(),
        action_key: "uniswap_v3_swap".to_owned(),
        chain_family: ExecutionChainFamily::Evm,
        allowed_chains: vec!["8453".to_owned()],
        entry_stage_id: "stage.quote_freshness".into(),
        actor: Some(
            ais_agent_control::execution_artifact::ExecutionArtifactActor {
                sender_address_hint: Some("0x742d35Cc6634C0532925a3b844Bc9e7595f8fE00".to_owned()),
                recipient_address: Some("0x742d35Cc6634C0532925a3b844Bc9e7595f8fE00".to_owned()),
            },
        ),
        transactions: vec![ExecutionTransactionCandidate::EvmTransaction(
            EvmTransactionCandidate {
                candidate_id: "swap.direct".into(),
                to: "0xE592427A0AEce92De3Edee1F18E0157C05861564".to_owned(),
                value: Some("0".to_owned()),
                calldata: Some(swap_calldata),
            },
        )],
        stages: vec![
            ExecutionStage::Branch(BranchStage {
                stage_id: "stage.quote_freshness".into(),
                predicate: PredicateSpec::Comparison {
                    left: ValueRef::Ref {
                        reference: "refs.evidence.trading_api.quote.expires_at_ms".to_owned(),
                    },
                    op: ais_agent_control::execution_artifact::ComparisonOperator::Gte,
                    right: ValueRef::Ref {
                        reference: "refs.evidence.clock.now_ms".to_owned(),
                    },
                },
                if_true: BranchTarget::GotoStage {
                    stage_id: "stage.approval_required".into(),
                },
                if_false: BranchTarget::Assert {
                    failure_code: "stale_quote".to_owned(),
                    message: "trading api quote is stale".to_owned(),
                },
            }),
            ExecutionStage::Branch(BranchStage {
                stage_id: "stage.approval_required".into(),
                predicate: PredicateSpec::Comparison {
                    left: ValueRef::Ref {
                        reference: "refs.evidence.trading_api.check_approval.approval_required"
                            .to_owned(),
                    },
                    op: ais_agent_control::execution_artifact::ComparisonOperator::Eq,
                    right: ValueRef::Literal { value: json!(true) },
                },
                if_true: BranchTarget::Assert {
                    failure_code: "unexpected_approval_requirement".to_owned(),
                    message: "trading api artifact expected direct swap path".to_owned(),
                },
                if_false: BranchTarget::GotoStage {
                    stage_id: "stage.swap".into(),
                },
            }),
            ExecutionStage::Transaction(TransactionStage {
                stage_id: "stage.swap".into(),
                candidate_ref: "swap.direct".into(),
                exports: Vec::new(),
                next_stage_id: None,
            }),
        ],
        observations: Vec::new(),
        preconditions: Vec::new(),
        postconditions: Vec::new(),
        expected_effects: Vec::new(),
        execution_policy: None,
        evidence: json!({
            "clock": {
                "now_ms": now_ms,
            },
            "trading_api": {
                "quote": {
                    "quoted_at_ms": now_ms.saturating_sub(15_000),
                    "expires_at_ms": now_ms + 60_000,
                    "amount_in_atomic": "25000000",
                    "min_amount_out_atomic": "9900000000000000",
                },
                "check_approval": {
                    "approval_required": false,
                },
                "swap": {
                    "router_address": "0xE592427A0AEce92De3Edee1F18E0157C05861564",
                    "calldata": "trading_api.swap.direct",
                }
            }
        }),
        metadata: BTreeMap::from([
            ("builder".to_owned(), json!("m37.uniswap_trading_api")),
            (
                "trading_api_key_source".to_owned(),
                json!("env:UNISWAP_TRADING_API_KEY"),
            ),
        ]),
    }
}

fn sample_uniswap_swap_to_aave_execution_artifact() -> ExecutionArtifactLaunchSpec {
    ExecutionArtifactLaunchSpec {
        protocol_package_id: "owliabot.uniswap_v3".to_owned(),
        action_key: "uniswap_v3_swap".to_owned(),
        chain_family: ExecutionChainFamily::Evm,
        allowed_chains: vec!["8453".to_owned()],
        entry_stage_id: "stage.swap".into(),
        actor: Some(ais_agent_control::execution_artifact::ExecutionArtifactActor {
            sender_address_hint: Some("0x742d35Cc6634C0532925a3b844Bc9e7595f8fE00".to_owned()),
            recipient_address: Some("0x742d35Cc6634C0532925a3b844Bc9e7595f8fE00".to_owned()),
        }),
        transactions: vec![ExecutionTransactionCandidate::EvmTransaction(
            EvmTransactionCandidate {
                candidate_id: "swap.direct".into(),
                to: "0xE592427A0AEce92De3Edee1F18E0157C05861564".to_owned(),
                value: Some("0".to_owned()),
                calldata: Some("0x414bf389".to_owned()),
            },
        )],
        stages: vec![
            ExecutionStage::Transaction(TransactionStage {
                stage_id: "stage.swap".into(),
                candidate_ref: "swap.direct".into(),
                exports: vec![
                    OutputExportSpec {
                        output_key: "swap.received_atomic".into(),
                        source: ValueRef::Cel {
                            expression: "string(refs.evidence.state.post.swap.recipient_out_balance.decoded_u256 - refs.evidence.state.pre.swap.recipient_out_balance.decoded_u256)".to_owned(),
                        },
                    },
                    OutputExportSpec {
                        output_key: "swap.tx_hash".into(),
                        source: ValueRef::Ref {
                            reference: "refs.receipts.stage.swap.tx_hash".to_owned(),
                        },
                    },
                    OutputExportSpec {
                        output_key: "swap.token_out_address".into(),
                        source: ValueRef::Literal {
                            value: json!("0x4200000000000000000000000000000000000006"),
                        },
                    },
                    OutputExportSpec {
                        output_key: "swap.recipient_address".into(),
                        source: ValueRef::Literal {
                            value: json!("0x742d35Cc6634C0532925a3b844Bc9e7595f8fE00"),
                        },
                    },
                ],
                next_stage_id: Some("stage.continue_aave".into()),
            }),
            ExecutionStage::Continuation(ContinuationStage {
                stage_id: "stage.continue_aave".into(),
                required_outputs: vec![
                    "swap.received_atomic".into(),
                    "swap.tx_hash".into(),
                    "swap.token_out_address".into(),
                    "swap.recipient_address".into(),
                ],
                package_entry: "build_aave_supply_from_swap_output".into(),
                next_stage_id: None,
            }),
        ],
        observations: Vec::new(),
        preconditions: vec![ObservationSpec {
            observation_id: "state.pre.swap.recipient_out_balance".to_owned(),
            kind: "evm.erc20_balance_of".to_owned(),
            params: BTreeMap::from([
                (
                    "token".to_owned(),
                    json!("0x4200000000000000000000000000000000000006"),
                ),
                (
                    "owner".to_owned(),
                    json!("0x742d35Cc6634C0532925a3b844Bc9e7595f8fE00"),
                ),
            ]),
        }],
        postconditions: vec![ObservationSpec {
            observation_id: "state.post.swap.recipient_out_balance".to_owned(),
            kind: "evm.erc20_balance_of".to_owned(),
            params: BTreeMap::from([
                (
                    "token".to_owned(),
                    json!("0x4200000000000000000000000000000000000006"),
                ),
                (
                    "owner".to_owned(),
                    json!("0x742d35Cc6634C0532925a3b844Bc9e7595f8fE00"),
                ),
            ]),
        }],
        expected_effects: Vec::new(),
        execution_policy: None,
        evidence: json!({
            "quote": {
                "source": "uniswap.quote",
                "quotedAtMs": 4102444800000u64,
                "amountOutAtomic": "25"
            }
        }),
        metadata: BTreeMap::from([("source".to_owned(), json!("m37-proof"))]),
    }
}

fn sample_uniswap_v3_lp_execution_artifact() -> ExecutionArtifactLaunchSpec {
    ExecutionArtifactLaunchSpec {
        protocol_package_id: "owliabot.uniswap_v3".to_owned(),
        action_key: "uniswap_v3_lp".to_owned(),
        chain_family: ExecutionChainFamily::Evm,
        allowed_chains: vec!["8453".to_owned()],
        entry_stage_id: "stage.mint".into(),
        actor: Some(
            ais_agent_control::execution_artifact::ExecutionArtifactActor {
                sender_address_hint: Some("0x742d35Cc6634C0532925a3b844Bc9e7595f8fE00".to_owned()),
                recipient_address: Some("0x742d35Cc6634C0532925a3b844Bc9e7595f8fE00".to_owned()),
            },
        ),
        transactions: vec![ExecutionTransactionCandidate::EvmTransaction(
            EvmTransactionCandidate {
                candidate_id: "lp.mint".into(),
                to: "0x1234567890abcdef1234567890ABCDEF12345678".to_owned(),
                value: Some("0".to_owned()),
                calldata: Some(format!(
                    "0x{}",
                    hex_encode_bytes(
                        uniswap_v3_mint_calldata("25000000", "10000000000000000").as_ref()
                    )
                )),
            },
        )],
        stages: vec![ExecutionStage::Transaction(TransactionStage {
            stage_id: "stage.mint".into(),
            candidate_ref: "lp.mint".into(),
            exports: Vec::new(),
            next_stage_id: None,
        })],
        observations: Vec::new(),
        preconditions: vec![ObservationSpec {
            observation_id: "state.pre.uniswap_v3_lp.position_count".to_owned(),
            kind: "evm.contract_state_read".to_owned(),
            params: BTreeMap::from([
                (
                    "to".to_owned(),
                    json!("0x1234567890abcdef1234567890ABCDEF12345678"),
                ),
                (
                    "data".to_owned(),
                    json!(format!(
                        "0x{}",
                        hex_encode_bytes(
                            erc721_balance_of_calldata(
                                "0x742d35Cc6634C0532925a3b844Bc9e7595f8fE00",
                            )
                            .as_ref()
                        )
                    )),
                ),
            ]),
        }],
        postconditions: vec![ObservationSpec {
            observation_id: "state.post.uniswap_v3_lp.position_count".to_owned(),
            kind: "evm.contract_state_read".to_owned(),
            params: BTreeMap::from([
                (
                    "to".to_owned(),
                    json!("0x1234567890abcdef1234567890ABCDEF12345678"),
                ),
                (
                    "data".to_owned(),
                    json!(format!(
                        "0x{}",
                        hex_encode_bytes(
                            erc721_balance_of_calldata(
                                "0x742d35Cc6634C0532925a3b844Bc9e7595f8fE00",
                            )
                            .as_ref()
                        )
                    )),
                ),
            ]),
        }],
        expected_effects: Vec::new(),
        execution_policy: None,
        evidence: json!({
            "token0": {
                "token_address": "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48",
            },
            "token1": {
                "token_address": "0x4200000000000000000000000000000000000006",
            },
            "pool": {
                "pool_address": "0x1111111111111111111111111111111111111111",
                "token0_address": "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48",
                "token1_address": "0x4200000000000000000000000000000000000006",
                "fee_tier": 3000,
                "tick_spacing": 60,
                "slot0_sqrt_price_x96": "79228162514264337593543950336",
                "slot0_tick": 0,
                "observed_at_ms": 4102444800000u64,
            },
            "deadline": {
                "deadline_unix_seconds": 4102444800u64,
            }
        }),
        metadata: BTreeMap::from([
            ("builder".to_owned(), json!("m37.uniswap_v3_lp")),
            ("source".to_owned(), json!("skill:uniswap-v3-lp")),
        ]),
    }
}

fn uniswap_exact_input_single_calldata(
    amount_in_atomic: &str,
    min_amount_out_atomic: &str,
) -> Bytes {
    let amount_in =
        U256::from_str_radix(amount_in_atomic, 10).expect("amount_in_atomic must be decimal u256");
    let min_amount_out = U256::from_str_radix(min_amount_out_atomic, 10)
        .expect("min_amount_out_atomic must be decimal u256");
    let mut encoded = Vec::with_capacity(4 + 32 * 8);
    encoded.extend_from_slice(&[0x41, 0x4b, 0xf3, 0x89]);
    encoded.extend_from_slice(&evm_abi_address_word(
        "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48",
    ));
    encoded.extend_from_slice(&evm_abi_address_word(
        "0x4200000000000000000000000000000000000006",
    ));
    encoded.extend_from_slice(&U256::from(3000u64).to_be_bytes::<32>());
    encoded.extend_from_slice(&evm_abi_address_word(
        "0x742d35Cc6634C0532925a3b844Bc9e7595f8fE00",
    ));
    encoded.extend_from_slice(&U256::from(4_102_444_800u64).to_be_bytes::<32>());
    encoded.extend_from_slice(&amount_in.to_be_bytes::<32>());
    encoded.extend_from_slice(&U256::ZERO.to_be_bytes::<32>());
    encoded.extend_from_slice(&min_amount_out.to_be_bytes::<32>());
    Bytes::from(encoded)
}

fn erc20_approve_calldata(spender: &str, amount_atomic: &str) -> Bytes {
    let amount =
        U256::from_str_radix(amount_atomic, 10).expect("amount_atomic must be decimal u256");
    let mut encoded = Vec::with_capacity(4 + 32 * 2);
    encoded.extend_from_slice(&[0x09, 0x5e, 0xa7, 0xb3]);
    encoded.extend_from_slice(&evm_abi_address_word(spender));
    encoded.extend_from_slice(&amount.to_be_bytes::<32>());
    Bytes::from(encoded)
}

fn erc721_balance_of_calldata(owner: &str) -> Bytes {
    let mut encoded = Vec::with_capacity(4 + 32);
    encoded.extend_from_slice(&[0x70, 0xa0, 0x82, 0x31]);
    encoded.extend_from_slice(&evm_abi_address_word(owner));
    Bytes::from(encoded)
}

fn uniswap_v3_mint_calldata(amount0_desired_atomic: &str, amount1_desired_atomic: &str) -> Bytes {
    let amount0_desired = U256::from_str_radix(amount0_desired_atomic, 10)
        .expect("amount0_desired_atomic must be decimal u256");
    let amount1_desired = U256::from_str_radix(amount1_desired_atomic, 10)
        .expect("amount1_desired_atomic must be decimal u256");
    let mut encoded = Vec::with_capacity(4 + 32 * 11);
    encoded.extend_from_slice(&[0x88, 0x31, 0x64, 0x56]);
    encoded.extend_from_slice(&evm_abi_address_word(
        "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48",
    ));
    encoded.extend_from_slice(&evm_abi_address_word(
        "0x4200000000000000000000000000000000000006",
    ));
    encoded.extend_from_slice(&U256::from(3000u64).to_be_bytes::<32>());
    encoded.extend_from_slice(&evm_abi_int24_word(-887220));
    encoded.extend_from_slice(&evm_abi_int24_word(887220));
    encoded.extend_from_slice(&amount0_desired.to_be_bytes::<32>());
    encoded.extend_from_slice(&amount1_desired.to_be_bytes::<32>());
    encoded.extend_from_slice(&U256::ZERO.to_be_bytes::<32>());
    encoded.extend_from_slice(&U256::ZERO.to_be_bytes::<32>());
    encoded.extend_from_slice(&evm_abi_address_word(
        "0x742d35Cc6634C0532925a3b844Bc9e7595f8fE00",
    ));
    encoded.extend_from_slice(&U256::from(4_102_444_800u64).to_be_bytes::<32>());
    Bytes::from(encoded)
}

fn build_aave_supply_continuation_artifact(
    received_atomic: &str,
    asset_address: &str,
    on_behalf_of: &str,
) -> ExecutionArtifactLaunchSpec {
    ExecutionArtifactLaunchSpec {
        protocol_package_id: "owliabot.aave_v3".to_owned(),
        action_key: "aave_v3_supply".to_owned(),
        chain_family: ExecutionChainFamily::Evm,
        allowed_chains: vec!["8453".to_owned()],
        entry_stage_id: "stage.supply".into(),
        actor: Some(
            ais_agent_control::execution_artifact::ExecutionArtifactActor {
                sender_address_hint: Some(on_behalf_of.to_owned()),
                recipient_address: Some(on_behalf_of.to_owned()),
            },
        ),
        transactions: vec![ExecutionTransactionCandidate::EvmTransaction(
            EvmTransactionCandidate {
                candidate_id: "supply.direct".into(),
                to: "0xA238Dd80C259a72e81d7e4664a9801593F98d1c5".to_owned(),
                value: Some("0".to_owned()),
                calldata: Some(format!(
                    "0x{}",
                    hex_encode_bytes(
                        aave_supply_calldata(asset_address, received_atomic, on_behalf_of).as_ref()
                    )
                )),
            },
        )],
        stages: vec![ExecutionStage::Transaction(TransactionStage {
            stage_id: "stage.supply".into(),
            candidate_ref: "supply.direct".into(),
            exports: Vec::new(),
            next_stage_id: None,
        })],
        observations: Vec::new(),
        preconditions: Vec::new(),
        postconditions: Vec::new(),
        expected_effects: Vec::new(),
        execution_policy: None,
        evidence: json!({
            "upstream": {
                "swap_received_atomic": received_atomic
            }
        }),
        metadata: BTreeMap::from([("continuation".to_owned(), json!("swap_to_aave_supply"))]),
    }
}

fn aave_supply_calldata(asset_address: &str, received_atomic: &str, on_behalf_of: &str) -> Bytes {
    let amount =
        U256::from_str_radix(received_atomic, 10).expect("received_atomic must be decimal u256");
    let mut encoded = Vec::with_capacity(4 + 32 * 4);
    encoded.extend_from_slice(&[0x61, 0x7b, 0x5b, 0xc3]);
    encoded.extend_from_slice(&evm_abi_address_word(asset_address));
    encoded.extend_from_slice(&amount.to_be_bytes::<32>());
    encoded.extend_from_slice(&evm_abi_address_word(on_behalf_of));
    encoded.extend_from_slice(&U256::ZERO.to_be_bytes::<32>());
    Bytes::from(encoded)
}

fn hex_encode_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn evm_abi_address_word(address: &str) -> [u8; 32] {
    let normalized = address.strip_prefix("0x").unwrap_or(address);
    let parsed = Address::from_str(normalized).expect("address should be valid hex");
    let mut word = [0u8; 32];
    word[12..].copy_from_slice(parsed.as_slice());
    word
}

fn evm_abi_int24_word(value: i32) -> [u8; 32] {
    let sign_fill = if value.is_negative() { 0xff } else { 0x00 };
    let mut word = [sign_fill; 32];
    word[29..].copy_from_slice(&value.to_be_bytes()[1..4]);
    word
}

async fn submit_continuation_artifact(
    runtime: ActiveRun,
    artifact: ExecutionArtifactLaunchSpec,
    wiring: &RuntimeExecutionWiring,
) -> ActiveRun {
    let run_id = runtime.run_id.clone();
    let host_session_id: HostSessionId = "session-m37-continuation".into();
    let mission = runtime.mission.clone();
    let checkpoint = runtime.checkpoint.clone();

    let mut run_repo = InMemoryRunRepository::default();
    run_repo.insert(runtime).expect("insert runtime");

    let mut checkpoint_repo = InMemoryCheckpointRepository::default();
    checkpoint_repo
        .append(CheckpointArchiveEntry {
            snapshot: checkpoint,
            kind: CheckpointArchiveKind::Boundary,
        })
        .expect("append checkpoint");

    let mut mission_repo = InMemoryMissionRepository::default();
    mission_repo
        .insert(run_id.clone(), mission.clone())
        .expect("insert mission");

    let run_catalog_repo = InMemoryRunCatalogRepository::default();
    let event_archive = InMemoryEventArchive::default();
    let mut session_store = InMemoryHostSessionStore::default();
    session_store.link_run(HostRunLink::new(
        host_session_id.clone(),
        run_id.clone(),
        mission.goal.clone(),
        mission.allowed_chains.clone(),
    ));

    let mut service = RuntimeHostService::new(
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
    )
    .with_execution_wiring(wiring.clone());

    let outcome = service
        .submit_execution_artifact_continuation(
            host_session_id,
            SubmitExecutionArtifactContinuationCommand {
                command_id: CommandId("cmd-m37-continuation".to_owned()),
                run_id: run_id.clone(),
                package_entry: "build_aave_supply_from_swap_output".into(),
                artifact,
                expected_version: None,
            },
        )
        .await
        .expect("submit continuation");

    match outcome.response {
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(snapshot.run_id, run_id);
            assert_eq!(snapshot.status, ais_agent_host::inspect::RunStatus::Running);
        }
        other => panic!("unexpected continuation response: {other:?}"),
    }

    let (run_repo, _, _, _, _, _, _) = service.into_parts();
    run_repo.load(&run_id).expect("continued runtime")
}

fn sample_owliabot_uniswap_v3_lp_metadata() -> BTreeMap<String, serde_json::Value> {
    BTreeMap::from([
        ("source".to_owned(), json!("skill:uniswap-v3-lp")),
        ("tool_name".to_owned(), json!("ais_run_harness")),
    ])
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
        execution_artifact: None,
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
