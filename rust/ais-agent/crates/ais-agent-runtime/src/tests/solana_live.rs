use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use std::collections::BTreeMap;

use ais_agent_control::ids::RunId;
use solana_sdk::{
    account::Account,
    hash::Hash,
    instruction::{AccountMeta, Instruction},
    message::VersionedMessage,
    pubkey::Pubkey,
    signature::Signature,
    transaction::VersionedTransaction,
};

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
        ActionGraph, ActionNode, ActionNodeKind, ActionNodeStatus, ActionOrigin, ActionPayload,
    },
    actuation::ActuationKind,
    binding::solana::{
        SolanaActuateBinding, SolanaConnectionSpec, SolanaObserveBinding, SolanaObserveRequest,
        SolanaSimulateBinding, SolanaTransactionRequest, SolanaVerifyBinding,
    },
    checkpoint::{CheckpointSnapshot, PendingRequestsSnapshot},
    effect::{EffectAssertion, EffectContract, EffectContractKind},
    envelope::{RuntimeEnvelope, RuntimeEnvelopeKind},
    evidence::EvidenceGraph,
    mission::{Mission, MissionBudget, MissionPolicy},
    runtime::{RunLifecycleState, RunPhase},
};
use ais_agent_solana::{
    broadcast::live::SolanaRpcBroadcastClient,
    read::live::{SolanaRpcReadClient, SolanaSignatureStatusSnapshot, SolanaTokenBalanceSnapshot},
    receipt::live::SolanaRpcReceiptClient,
    simulate::live::{
        compile_transaction_request, SolanaRpcSimulateClient, SolanaTransactionSimulationReport,
    },
};

use crate::{
    persistence::{
        persist_side_effect_checkpoint, restore_active_run, InMemoryCheckpointRepository,
        InMemoryMissionRepository, InMemorySignerStateStore, MissionRepository,
    },
    runtime::ActiveRun,
    stepper::{
        apply_live_solana_broadcast_with_client, apply_live_solana_observe_with_client,
        apply_live_solana_simulate_with_client, apply_live_solana_verify_with_client,
    },
};

#[derive(Debug, Default)]
struct FakeSolanaReadClient;

#[async_trait]
impl SolanaRpcReadClient for FakeSolanaReadClient {
    async fn get_slot(&self) -> Result<u64, String> {
        Ok(4242)
    }

    async fn get_balance(&self, _address: &Pubkey) -> Result<u64, String> {
        Ok(123_456)
    }

    async fn get_token_balance(
        &self,
        _token_account: &Pubkey,
    ) -> Result<SolanaTokenBalanceSnapshot, String> {
        Ok(SolanaTokenBalanceSnapshot {
            amount: "42".to_owned(),
            decimals: 6,
            ui_amount_string: "0.000042".to_owned(),
        })
    }

    async fn get_account(&self, address: &Pubkey) -> Result<Account, String> {
        Ok(Account {
            lamports: 999,
            data: vec![1, 2, 3, 4],
            owner: *address,
            executable: false,
            rent_epoch: 77,
        })
    }

    async fn get_signature_status(
        &self,
        _signature: &Signature,
    ) -> Result<SolanaSignatureStatusSnapshot, String> {
        Ok(SolanaSignatureStatusSnapshot {
            slot: Some(8),
            confirmations: Some(1),
            confirmation_status: Some("processed".to_owned()),
            error: None,
        })
    }
}

#[derive(Debug, Default)]
struct FakeSolanaSimulateClient;

#[async_trait]
impl SolanaRpcSimulateClient for FakeSolanaSimulateClient {
    async fn simulate_transaction(
        &self,
        _transaction: &solana_sdk::transaction::VersionedTransaction,
    ) -> Result<SolanaTransactionSimulationReport, String> {
        Ok(SolanaTransactionSimulationReport {
            accepted: true,
            logs: vec!["Program log: ok".to_owned()],
            units_consumed: Some(18_500),
            error: None,
            replacement_blockhash: Some(Hash::new_from_array([9u8; 32]).to_string()),
            source_hint: "fake_sol_rpc:simulate_transaction".to_owned(),
        })
    }
}

#[derive(Debug, Default)]
struct FakeSolanaBroadcastClient;

#[async_trait]
impl SolanaRpcBroadcastClient for FakeSolanaBroadcastClient {
    async fn send_transaction(
        &self,
        transaction: &VersionedTransaction,
    ) -> Result<Signature, String> {
        transaction
            .signatures
            .first()
            .copied()
            .ok_or_else(|| "missing signature".to_owned())
    }
}

#[derive(Debug, Default)]
struct FakeSolanaReceiptClient;

#[async_trait]
impl SolanaRpcReceiptClient for FakeSolanaReceiptClient {
    async fn get_slot(&self) -> Result<u64, String> {
        Ok(1_000)
    }

    async fn get_signature_status(
        &self,
        signature: &Signature,
    ) -> Result<SolanaSignatureStatusSnapshot, String> {
        Ok(SolanaSignatureStatusSnapshot {
            slot: Some(995),
            confirmations: Some(1),
            confirmation_status: Some("confirmed".to_owned()),
            error: if *signature == failed_signature() {
                Some("InstructionError(0, Custom(1))".to_owned())
            } else {
                None
            },
        })
    }
}

#[derive(Debug)]
struct FakeFailingSolanaBroadcastClient {
    message: &'static str,
}

#[async_trait]
impl SolanaRpcBroadcastClient for FakeFailingSolanaBroadcastClient {
    async fn send_transaction(
        &self,
        _transaction: &VersionedTransaction,
    ) -> Result<Signature, String> {
        Err(self.message.to_owned())
    }
}

#[derive(Debug)]
struct FakeFailingSolanaReceiptClient {
    message: &'static str,
}

#[async_trait]
impl SolanaRpcReceiptClient for FakeFailingSolanaReceiptClient {
    async fn get_slot(&self) -> Result<u64, String> {
        Ok(1_000)
    }

    async fn get_signature_status(
        &self,
        _signature: &Signature,
    ) -> Result<SolanaSignatureStatusSnapshot, String> {
        Err(self.message.to_owned())
    }
}

#[tokio::test]
async fn runtime_observe_node_can_emit_machine_readable_sol_observation() {
    let checkpoint = checkpoint_with_nodes(vec![observe_slot_node("observe-slot")]);
    let mission = sample_mission();
    let mut runtime = ActiveRun::new(mission, checkpoint);

    let transition = apply_live_solana_observe_with_client(&mut runtime, &FakeSolanaReadClient)
        .await
        .expect("observe transition");

    assert_eq!(transition.node_id.as_deref(), Some("observe-slot"));
    assert_eq!(
        runtime
            .checkpoint
            .action_graph
            .nodes
            .get("observe-slot")
            .map(|node| node.status.clone()),
        Some(ActionNodeStatus::Succeeded)
    );

    let payload = runtime
        .checkpoint
        .evidence_graph
        .records
        .get("observed.slot")
        .expect("slot evidence")
        .payload
        .clone();
    assert_eq!(payload["slot"], 4242);
    assert_eq!(payload["source_hint"], "solana_rpc:get_slot");
}

#[tokio::test]
async fn runtime_simulate_node_can_emit_machine_readable_sol_simulation_report() {
    let checkpoint = checkpoint_with_nodes(vec![simulate_legacy_transaction_node("simulate-sol")]);
    let mission = sample_mission();
    let mut runtime = ActiveRun::new(mission, checkpoint);

    let transition =
        apply_live_solana_simulate_with_client(&mut runtime, &FakeSolanaSimulateClient)
            .await
            .expect("simulate transition");

    assert_eq!(transition.node_id.as_deref(), Some("simulate-sol"));
    assert_eq!(
        runtime
            .checkpoint
            .action_graph
            .nodes
            .get("simulate-sol")
            .map(|node| node.status.clone()),
        Some(ActionNodeStatus::Succeeded)
    );

    let payload = runtime
        .checkpoint
        .evidence_graph
        .records
        .get("simulation.simulate-sol")
        .expect("simulation record")
        .payload
        .clone();
    assert_eq!(payload["accepted"], true);
    assert_eq!(payload["units_consumed"], 18_500);
    assert_eq!(payload["source_hint"], "fake_sol_rpc:simulate_transaction");
}

#[tokio::test]
async fn runtime_broadcast_node_can_submit_live_solana_signature_and_enter_confirmation_wait() {
    let checkpoint = checkpoint_with_nodes(vec![broadcast_solana_node("broadcast-sol")]);
    let mission = sample_mission();
    let mut runtime = ActiveRun::new(mission, checkpoint);
    runtime.envelopes.insert(
        "env.sol".to_owned(),
        RuntimeEnvelope {
            envelope_id: "env.sol".to_owned(),
            kind: RuntimeEnvelopeKind::SolanaEnvelope,
            chain: "solana:mainnet".to_owned(),
            payload: encoded_transaction_payload(&signed_legacy_transaction(signed_signature())),
            provenance: Some("test".to_owned()),
        },
    );

    let transition =
        apply_live_solana_broadcast_with_client(&mut runtime, &FakeSolanaBroadcastClient)
            .await
            .expect("broadcast transition");

    assert_eq!(transition.node_id.as_deref(), Some("broadcast-sol"));
    assert_eq!(
        runtime
            .checkpoint
            .pending_requests
            .pending_confirmation_id
            .as_deref(),
        Some(signed_signature_string().as_str())
    );
    assert!(runtime.checkpoint.actuation_records.iter().any(|record| {
        matches!(record.kind, ActuationKind::BroadcastSubmitted)
            && record.tx_hash.as_deref() == Some(&signed_signature().to_string())
    }));
}

#[tokio::test]
async fn runtime_broadcast_uncertainty_pauses_for_user_review() {
    let checkpoint = checkpoint_with_nodes(vec![broadcast_solana_node("broadcast-sol")]);
    let mission = sample_mission();
    let mut runtime = ActiveRun::new(mission, checkpoint);
    runtime.envelopes.insert(
        "env.sol".to_owned(),
        RuntimeEnvelope {
            envelope_id: "env.sol".to_owned(),
            kind: RuntimeEnvelopeKind::SolanaEnvelope,
            chain: "solana:mainnet".to_owned(),
            payload: encoded_transaction_payload(&signed_legacy_transaction(signed_signature())),
            provenance: Some("test".to_owned()),
        },
    );

    let transition = apply_live_solana_broadcast_with_client(
        &mut runtime,
        &FakeFailingSolanaBroadcastClient {
            message: "transaction already processed; status unknown",
        },
    )
    .await
    .expect("broadcast uncertainty transition");

    assert_eq!(transition.node_id.as_deref(), Some("broadcast-sol"));
    assert_eq!(
        runtime.checkpoint.lifecycle.status,
        ais_agent_core::runtime::RunStatus::Paused
    );
    assert_eq!(
        runtime
            .checkpoint
            .lifecycle
            .failure
            .as_ref()
            .map(|failure| &failure.code),
        Some(&ais_agent_control::recovery::RunFailureCode::BroadcastUncertain)
    );
    assert_eq!(
        runtime
            .checkpoint
            .action_graph
            .nodes
            .get("broadcast-sol")
            .map(|node| node.status.clone()),
        Some(ActionNodeStatus::Blocked)
    );
    let recovery = crate::runtime::classify_recovery_view(&runtime.checkpoint);
    assert_eq!(
        recovery.recovery_disposition,
        Some(ais_agent_control::recovery::RecoveryDisposition::AwaitUserInput)
    );
}

#[tokio::test]
async fn runtime_verify_node_can_observe_live_solana_signature_status_and_complete() {
    let mut checkpoint = checkpoint_with_nodes(vec![
        succeeded_broadcast_solana_node("broadcast-sol"),
        verify_solana_signature_node("verify-sol", vec!["broadcast-sol"]),
    ]);
    checkpoint.effect_contracts.insert(
        "effect.sol".to_owned(),
        EffectContract {
            effect_id: "effect.sol".to_owned(),
            kind: EffectContractKind::StateTransition,
            assertions: vec![EffectAssertion {
                expression: "receipt.raw.error == null".to_owned(),
                description: "signature status should not have an error".to_owned(),
            }],
            tolerance_hint: Some("signature_status_required".to_owned()),
        },
    );

    let mission = sample_mission();
    let mut runtime = ActiveRun::new(mission, checkpoint);
    runtime.checkpoint.pending_requests.pending_confirmation_id =
        Some(signed_signature().to_string());

    let transition = apply_live_solana_verify_with_client(&mut runtime, &FakeSolanaReceiptClient)
        .await
        .expect("verify transition");

    assert_eq!(transition.node_id.as_deref(), Some("verify-sol"));
    assert_eq!(
        runtime.checkpoint.pending_requests.pending_confirmation_id,
        None
    );
    assert_eq!(
        runtime
            .checkpoint
            .action_graph
            .nodes
            .get("verify-sol")
            .map(|node| node.status.clone()),
        Some(ActionNodeStatus::Succeeded)
    );
    assert!(runtime
        .checkpoint
        .evidence_graph
        .records
        .contains_key("receipt.verify-sol"));
    assert!(runtime
        .checkpoint
        .evidence_graph
        .records
        .contains_key("effect.verify-sol"));
}

#[tokio::test]
async fn runtime_verify_timeout_enters_retry_ready_with_confirmation_context() {
    let checkpoint = checkpoint_with_nodes(vec![
        succeeded_broadcast_solana_node("broadcast-sol"),
        verify_solana_signature_node("verify-sol", vec!["broadcast-sol"]),
    ]);
    let mission = sample_mission();
    let mut runtime = ActiveRun::new(mission, checkpoint);
    runtime.checkpoint.pending_requests.pending_confirmation_id =
        Some(signed_signature().to_string());

    let transition = apply_live_solana_verify_with_client(
        &mut runtime,
        &FakeFailingSolanaReceiptClient {
            message: "deadline exceeded while waiting for signature status",
        },
    )
    .await
    .expect("verify timeout transition");

    assert_eq!(transition.node_id.as_deref(), Some("verify-sol"));
    assert_eq!(
        runtime.checkpoint.lifecycle.status,
        ais_agent_core::runtime::RunStatus::AwaitingConfirmation
    );
    let failure = runtime
        .checkpoint
        .lifecycle
        .failure
        .as_ref()
        .expect("retryable confirmation failure");
    assert_eq!(
        failure.code,
        ais_agent_control::recovery::RunFailureCode::ConfirmationTimeout
    );
    assert_eq!(
        failure.confirmation_refs,
        vec![signed_signature().to_string()]
    );
    let recovery = crate::runtime::classify_recovery_view(&runtime.checkpoint);
    assert_eq!(
        recovery.recovery_disposition,
        Some(ais_agent_control::recovery::RecoveryDisposition::RetryReady)
    );
}

#[tokio::test]
async fn runtime_verify_provider_failure_enters_retry_ready_with_provider_error() {
    let checkpoint = checkpoint_with_nodes(vec![
        succeeded_broadcast_solana_node("broadcast-sol"),
        verify_solana_signature_node("verify-sol", vec!["broadcast-sol"]),
    ]);
    let mission = sample_mission();
    let mut runtime = ActiveRun::new(mission, checkpoint);
    runtime.checkpoint.pending_requests.pending_confirmation_id =
        Some(signed_signature().to_string());

    let transition = apply_live_solana_verify_with_client(
        &mut runtime,
        &FakeFailingSolanaReceiptClient {
            message: "429 rate limited by rpc provider",
        },
    )
    .await
    .expect("verify provider failure transition");

    assert_eq!(transition.node_id.as_deref(), Some("verify-sol"));
    let failure = runtime
        .checkpoint
        .lifecycle
        .failure
        .as_ref()
        .expect("provider failure");
    assert_eq!(
        failure.code,
        ais_agent_control::recovery::RunFailureCode::ProviderUnavailable
    );
    assert_eq!(
        failure
            .provider_error
            .as_ref()
            .map(|error| error.provider.as_str()),
        Some("solana.rpc")
    );
    let recovery = crate::runtime::classify_recovery_view(&runtime.checkpoint);
    assert_eq!(
        recovery.recovery_disposition,
        Some(ais_agent_control::recovery::RecoveryDisposition::RetryReady)
    );
}

#[tokio::test]
async fn runtime_can_restart_from_durable_side_effect_cut_after_solana_broadcast_success() {
    let mut checkpoint = checkpoint_with_nodes(vec![
        broadcast_solana_node("broadcast-sol"),
        verify_solana_signature_node("verify-sol", vec!["broadcast-sol"]),
    ]);
    checkpoint.effect_contracts.insert(
        "effect.sol".to_owned(),
        EffectContract {
            effect_id: "effect.sol".to_owned(),
            kind: EffectContractKind::StateTransition,
            assertions: vec![EffectAssertion {
                expression: "receipt.raw.error == null".to_owned(),
                description: "signature status should not have an error".to_owned(),
            }],
            tolerance_hint: Some("signature_status_required".to_owned()),
        },
    );

    let mission = sample_mission();
    let run_id = RunId("run.sol.live".to_owned());
    let mut runtime = ActiveRun::new(mission.clone(), checkpoint);
    runtime.envelopes.insert(
        "env.sol".to_owned(),
        RuntimeEnvelope {
            envelope_id: "env.sol".to_owned(),
            kind: RuntimeEnvelopeKind::SolanaEnvelope,
            chain: "solana:mainnet".to_owned(),
            payload: encoded_transaction_payload(&signed_legacy_transaction(signed_signature())),
            provenance: Some("test".to_owned()),
        },
    );

    apply_live_solana_broadcast_with_client(&mut runtime, &FakeSolanaBroadcastClient)
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
        &InMemorySignerStateStore::default(),
    )
    .expect("restore from durable side-effect cut");

    let transition = apply_live_solana_verify_with_client(&mut restored, &FakeSolanaReceiptClient)
        .await
        .expect("verify transition after restart");

    assert_eq!(transition.node_id.as_deref(), Some("verify-sol"));
    assert_eq!(
        restored.checkpoint.pending_requests.pending_confirmation_id,
        None
    );
    assert_eq!(
        restored
            .checkpoint
            .action_graph
            .nodes
            .get("verify-sol")
            .map(|node| node.status.clone()),
        Some(ActionNodeStatus::Succeeded)
    );
    assert!(restored
        .checkpoint
        .evidence_graph
        .records
        .contains_key("receipt.verify-sol"));
    assert!(restored
        .checkpoint
        .evidence_graph
        .records
        .contains_key("effect.verify-sol"));
}

#[tokio::test]
async fn runtime_broadcast_and_verify_support_v0_lut_solana_transaction() {
    let mut checkpoint = checkpoint_with_nodes(vec![
        broadcast_solana_node("broadcast-sol-v0"),
        verify_solana_signature_node("verify-sol-v0", vec!["broadcast-sol-v0"]),
    ]);
    checkpoint.effect_contracts.insert(
        "effect.sol".to_owned(),
        EffectContract {
            effect_id: "effect.sol".to_owned(),
            kind: EffectContractKind::StateTransition,
            assertions: vec![EffectAssertion {
                expression: "receipt.raw.confirmation_status == \"confirmed\"".to_owned(),
                description: "signature should reach confirmed status".to_owned(),
            }],
            tolerance_hint: Some("signature_status_required".to_owned()),
        },
    );

    let mission = sample_mission();
    let mut runtime = ActiveRun::new(mission, checkpoint);
    runtime.envelopes.insert(
        "env.sol".to_owned(),
        RuntimeEnvelope {
            envelope_id: "env.sol".to_owned(),
            kind: RuntimeEnvelopeKind::SolanaEnvelope,
            chain: "solana:mainnet".to_owned(),
            payload: encoded_transaction_payload(&signed_v0_transaction(signed_signature())),
            provenance: Some("test".to_owned()),
        },
    );

    apply_live_solana_broadcast_with_client(&mut runtime, &FakeSolanaBroadcastClient)
        .await
        .expect("broadcast transition");
    let transition = apply_live_solana_verify_with_client(&mut runtime, &FakeSolanaReceiptClient)
        .await
        .expect("verify transition");

    assert_eq!(transition.node_id.as_deref(), Some("verify-sol-v0"));
    assert_eq!(
        runtime
            .checkpoint
            .action_graph
            .nodes
            .get("verify-sol-v0")
            .map(|node| node.status.clone()),
        Some(ActionNodeStatus::Succeeded)
    );
}

fn sample_mission() -> Mission {
    Mission {
        mission_id: "mission.sol.live".to_owned(),
        goal: "test solana live observe/simulate".to_owned(),
        allowed_chains: vec!["solana:mainnet".to_owned()],
        budget: MissionBudget {
            max_steps: Some(8),
            max_signer_requests: Some(1),
            max_wall_clock_ms: Some(10_000),
        },
        policy: MissionPolicy {
            policy_mode: Some("guarded".to_owned()),
            allow_raw_envelopes: false,
            require_effect_contract_for_writes: true,
        },
        constraints: BTreeMap::new(),
        metadata: Default::default(),
    }
}

fn checkpoint_with_nodes(nodes: Vec<ActionNode>) -> CheckpointSnapshot {
    let roots = nodes
        .iter()
        .filter(|node| node.depends_on.is_empty())
        .map(|node| node.node_id.clone())
        .collect();
    let terminals = nodes.iter().map(|node| node.node_id.clone()).collect();

    let mut lifecycle =
        RunLifecycleState::new(RunId("run.sol.live".to_owned()), "mission.sol.live");
    lifecycle.mark_running(RunPhase::Planning);

    CheckpointSnapshot {
        run_id: "run.sol.live".to_owned(),
        mission_id: "mission.sol.live".to_owned(),
        checkpoint_seq: 0,
        plan_epoch: 0,
        lifecycle,
        action_graph: ActionGraph {
            graph_id: Some("graph.sol.live".to_owned()),
            nodes: nodes
                .into_iter()
                .map(|node| (node.node_id.clone(), node))
                .collect(),
            roots,
            terminals,
        },
        evidence_graph: EvidenceGraph::default(),
        pending_requests: PendingRequestsSnapshot::default(),
        last_completed_node_id: None,
        actuation_records: Vec::new(),
        execution_artifact: None,
        effect_contracts: Default::default(),
    }
}

fn observe_slot_node(node_id: &str) -> ActionNode {
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
            source_hint: "solana get slot".to_owned(),
            output_key: Some("observed.slot".to_owned()),
            live: Some(ObserveLiveBinding::Solana(SolanaObserveLiveBinding {
                connection: Some(SolanaConnectionSpec {
                    rpc_url: "http://localhost:8899".to_owned(),
                    ws_url: None,
                }),
                binding: SolanaObserveBinding::Slot,
                request: SolanaObserveRequest::Slot,
            })),
        }),
        implementation_hint: Some("solana.observe.slot".to_owned()),
        expected_effect_ref: None,
    }
}

fn simulate_legacy_transaction_node(node_id: &str) -> ActionNode {
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
            simulator_hint: "solana simulate tx".to_owned(),
            live: Some(SimulateLiveBinding::Solana(SolanaSimulateLiveBinding {
                connection: Some(SolanaConnectionSpec {
                    rpc_url: "http://localhost:8899".to_owned(),
                    ws_url: None,
                }),
                binding: SolanaSimulateBinding::SimulateTransaction,
                request: SolanaTransactionRequest::Legacy {
                    recent_blockhash: None,
                    payer: Some(Pubkey::new_from_array([3u8; 32])),
                    instructions: vec![Instruction {
                        program_id: Pubkey::new_from_array([4u8; 32]),
                        accounts: Vec::new(),
                        data: vec![1, 2, 3],
                    }],
                },
            })),
        }),
        implementation_hint: Some("solana.simulate.tx".to_owned()),
        expected_effect_ref: None,
    }
}

fn broadcast_solana_node(node_id: &str) -> ActionNode {
    ActionNode {
        node_id: node_id.to_owned(),
        kind: ActionNodeKind::Actuate,
        origin: ActionOrigin::DriverFragment,
        status: ActionNodeStatus::Ready,
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
                    rpc_url: "http://localhost:8899".to_owned(),
                    ws_url: None,
                }),
                binding: SolanaActuateBinding::BroadcastSignedTransaction,
            })),
        }),
        implementation_hint: Some("solana.broadcast".to_owned()),
        expected_effect_ref: Some("effect.sol".to_owned()),
    }
}

fn succeeded_broadcast_solana_node(node_id: &str) -> ActionNode {
    let mut node = broadcast_solana_node(node_id);
    node.status = ActionNodeStatus::Succeeded;
    node
}

fn verify_solana_signature_node(node_id: &str, depends_on: Vec<&str>) -> ActionNode {
    ActionNode {
        node_id: node_id.to_owned(),
        kind: ActionNodeKind::Verify,
        origin: ActionOrigin::DriverFragment,
        status: ActionNodeStatus::Pending,
        depends_on: depends_on
            .into_iter()
            .map(|value| value.to_owned())
            .collect(),
        inputs: Vec::new(),
        evidence_refs: Vec::new(),
        payload: ActionPayload::Verify(VerifyAction {
            verify_kind: VerifyKind::EffectContract,
            verifier_hint: "solana signature status".to_owned(),
            pre_observation_ref: None,
            post_observation_ref: None,
            live: Some(VerifyLiveBinding::Solana(SolanaVerifyLiveBinding {
                connection: Some(SolanaConnectionSpec {
                    rpc_url: "http://localhost:8899".to_owned(),
                    ws_url: None,
                }),
                binding: SolanaVerifyBinding::EffectContractFromSignatureStatus,
                post_request: None,
            })),
        }),
        implementation_hint: Some("solana.verify".to_owned()),
        expected_effect_ref: Some("effect.sol".to_owned()),
    }
}

fn signed_legacy_transaction(signature: Signature) -> VersionedTransaction {
    let request = SolanaTransactionRequest::Legacy {
        recent_blockhash: Some(Hash::new_from_array([7u8; 32])),
        payer: Some(Pubkey::new_from_array([8u8; 32])),
        instructions: vec![Instruction {
            program_id: Pubkey::new_from_array([9u8; 32]),
            accounts: Vec::new(),
            data: vec![1, 2, 3],
        }],
    };
    let mut transaction = compile_transaction_request(&request).expect("legacy tx");
    transaction.signatures = vec![signature];
    transaction
}

fn signed_v0_transaction(signature: Signature) -> VersionedTransaction {
    let lookup_address = Pubkey::new_from_array([14u8; 32]);
    let request = SolanaTransactionRequest::V0 {
        recent_blockhash: Some(Hash::new_from_array([10u8; 32])),
        payer: Some(Pubkey::new_from_array([11u8; 32])),
        instructions: vec![Instruction {
            program_id: Pubkey::new_from_array([12u8; 32]),
            accounts: vec![AccountMeta::new_readonly(lookup_address, false)],
            data: vec![4, 5, 6],
        }],
        address_lookup_tables: vec![solana_sdk::message::AddressLookupTableAccount {
            key: Pubkey::new_from_array([13u8; 32]),
            addresses: vec![lookup_address],
        }],
    };
    let mut transaction = compile_transaction_request(&request).expect("v0 tx");
    transaction.signatures = vec![signature];
    if let VersionedMessage::V0(message) = &transaction.message {
        assert!(!message.address_table_lookups.is_empty());
    } else {
        panic!("expected v0 message");
    }
    transaction
}

fn signed_signature() -> Signature {
    Signature::from([3u8; 64])
}

fn signed_signature_string() -> String {
    signed_signature().to_string()
}

fn failed_signature() -> Signature {
    Signature::from([4u8; 64])
}

fn encoded_transaction_payload(transaction: &VersionedTransaction) -> serde_json::Value {
    let encoded = bincode::serde::encode_to_vec(transaction, bincode::config::standard())
        .expect("serialize signed solana tx");
    serde_json::json!({
        "transaction_base64": BASE64.encode(encoded),
    })
}
