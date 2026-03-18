use std::collections::BTreeMap;

use ais_agent_control::{
    commands::{
        BeginRunCommand, CancelRunCommand, ClaimRunCommand, EnvelopeKind, EnvelopeSubmission,
        EvidenceKind, EvidenceSubmission, ExpectedRuntimeVersion, MissionBudgetSubmission,
        MissionSubmission, ReleaseRunClaimCommand, RenewRunClaimCommand, RequestCancelRunCommand,
        RunCommand, SignerResolutionKind, SignerResolutionSubmission, StepBudget, StepRunCommand,
        StepUntil, SubmitEnvelopeCommand, SubmitEvidenceCommand,
        SubmitExecutionArtifactContinuationCommand, SubmitSignerResolutionCommand,
    },
    events::{RunAwaitingSigner, RunEvent, RunEventEnvelope, RunEventTraceContext},
    execution_artifact::{
        BranchStage, BranchTarget, ComparisonOperator, ContinuationStage, EffectSpec,
        EvmTransactionCandidate, ExecutionArtifactActor, ExecutionArtifactLaunchSpec,
        ExecutionChainFamily, ExecutionStage, ExecutionTransactionCandidate, ObservationSpec,
        ObserveStage, OutputExportSpec, PredicateSpec, TransactionStage, ValueRef,
    },
    ids::{CommandId, EventId, IdempotencyKey, RunId, SignerRequestId},
    launch_spec::{LaunchSpecSubmission, PrebuiltFragmentLaunchSpec, ReflectionRequestLaunchSpec},
    ownership::{RunClaimMode, RunClaimOwnerKind},
    recovery::{
        CancelState, InterruptionClass, RunFailureCode, RunFailureContext, RunFailureStage,
    },
};
use ais_agent_core::{
    action::{
        kinds::{
            actuate::{ActuateAction, ActuateLiveBinding, ActuateMode, SolanaActuateLiveBinding},
            derive::{DeriveAction, DeriveKind},
            verify::{SolanaVerifyLiveBinding, VerifyAction, VerifyKind, VerifyLiveBinding},
        },
        ActionGraph, ActionNode, ActionNodeKind, ActionNodeStatus, ActionOrigin, ActionPayload,
    },
    binding::solana::{SolanaActuateBinding, SolanaConnectionSpec, SolanaVerifyBinding},
    checkpoint::{
        ArtifactContinuationSnapshot, CheckpointSnapshot, ExecutionArtifactRuntimeSnapshot,
        PendingRequestsSnapshot,
    },
    evidence::{EvidenceGraph, EvidenceRequirement},
    mission::{Mission, MissionBudget, MissionPolicy},
    runtime::{RunLifecycleState, RunPhase, SignerRequestState},
};
use ais_agent_evm::read::live::EvmAlloyReadPort;
use ais_agent_host::{
    control::{HostCommandResponse, HostCommandService},
    events::{HostRunEventQuery, HostRunEventService},
    inspect::RunStatus,
    session::{
        HostCommandEnvelope, HostRunLink, HostSessionId, HostSessionStore, InMemoryHostSessionStore,
    },
};
use alloy::{primitives::U256, providers::ProviderBuilder, transports::mock::Asserter};
use serde_json::json;

use crate::{
    persistence::{
        CheckpointArchiveEntry, CheckpointArchiveKind, CheckpointRepository, EventArchive,
        EventArchiveError, EventArchiveQuery, InMemoryCheckpointRepository, InMemoryEventArchive,
        InMemoryMissionRepository, InMemoryRunCatalogRepository, InMemoryRunClaimRepository,
        InMemorySignerStateStore, MissionRepository, MissionRepositoryError, RunCatalogEntry,
        RunCatalogRepository, RunCatalogRepositoryError, RunClaimRepository, RunWaitStateRecord,
        RunWaitStateStore, SignerStateStoreError,
    },
    runtime::{ActiveRun, InMemoryRunRepository, RunRepository},
    service::{RuntimeExecutionWiring, RuntimeHostService},
    tests::tracing_capture::capture_tracing_output,
};

#[tokio::test]
async fn runtime_host_service_handles_begin_inspect_and_cancel() {
    let mut service = RuntimeHostService::new(
        InMemoryRunRepository::default(),
        InMemoryCheckpointRepository::default(),
        InMemoryMissionRepository::default(),
        InMemoryRunCatalogRepository::default(),
        InMemoryEventArchive::default(),
        InMemoryHostSessionStore::default(),
    );
    let host_session_id: HostSessionId = "session-1".into();

    let begin = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: None,
            command: RunCommand::BeginRun(BeginRunCommand {
                command_id: CommandId("cmd-begin".to_owned()),
                idempotency_key: IdempotencyKey("idem-1".to_owned()),
                mission: MissionSubmission {
                    goal: "swap".to_owned(),
                    allowed_chains: vec!["eip155:1".to_owned()],
                    constraints: BTreeMap::new(),
                    budget: Some(MissionBudgetSubmission {
                        max_steps: Some(8),
                        max_signer_requests: Some(1),
                        max_wall_clock_ms: Some(30_000),
                    }),
                    metadata: BTreeMap::new(),
                },
                launch_spec: empty_prebuilt_launch_spec(),
            }),
        })
        .await;

    let run_id = match begin.response {
        HostCommandResponse::Accepted(response) => response.run_id.expect("run id"),
        other => panic!("unexpected response: {other:?}"),
    };
    assert_eq!(begin.events.len(), 1);
    assert_eq!(begin.events[0].event_seq, 1);
    assert_eq!(begin.events[0].checkpoint_seq, 0);
    assert_eq!(begin.events[0].plan_epoch, 0);
    assert!(matches!(begin.events[0].event, RunEvent::Started(_)));

    let inspect = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: None,
            command: RunCommand::InspectRun(ais_agent_control::commands::InspectRunCommand {
                command_id: CommandId("cmd-inspect".to_owned()),
                run_id: run_id.clone(),
            }),
        })
        .await;
    match inspect.response {
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(snapshot.run_id, run_id);
            assert_eq!(snapshot.status, RunStatus::Running);
            assert_eq!(snapshot.recent_events.len(), 1);
            assert_eq!(
                snapshot.recent_events[0].event_type,
                "run.lifecycle.started"
            );
        }
        other => panic!("unexpected response: {other:?}"),
    }

    let cancel = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: None,
            command: RunCommand::CancelRun(CancelRunCommand {
                command_id: CommandId("cmd-cancel".to_owned()),
                run_id,
                reason: Some("user cancelled".to_owned()),
                expected_version: None,
            }),
        })
        .await;
    match cancel.response {
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(snapshot.status, RunStatus::Cancelled);
            assert_eq!(snapshot.cancel_state, Some(CancelState::Cancelled));
        }
        other => panic!("unexpected response: {other:?}"),
    }
}

#[tokio::test]
async fn runtime_host_service_inspect_surfaces_recent_archived_events() {
    let run_id = RunId("run-inspect-events".to_owned());
    let host_session_id: HostSessionId = "session-inspect-events".into();
    let mission = sample_mission();
    let mut checkpoint = checkpoint_with_nodes(Vec::new(), Vec::new());
    checkpoint.run_id = run_id.0.clone();
    checkpoint.mission_id = mission.mission_id.clone();
    checkpoint.lifecycle.run_id = run_id.clone();
    let mission_repo = preloaded_mission_repo(run_id.clone(), mission.clone());
    let mut checkpoint_repo = InMemoryCheckpointRepository::default();
    checkpoint_repo
        .append(CheckpointArchiveEntry {
            snapshot: checkpoint,
            kind: CheckpointArchiveKind::Boundary,
        })
        .expect("append checkpoint");
    let mut event_archive = InMemoryEventArchive::default();
    event_archive
        .append(RunEventEnvelope {
            run_id: run_id.clone(),
            event_seq: 1,
            checkpoint_seq: 4,
            plan_epoch: 2,
            trace_context: Some(RunEventTraceContext {
                trace_id: "trace-run-inspect-events:signer".to_owned(),
                span_id: "awaiting_signer:1".to_owned(),
            }),
            event: RunEvent::AwaitingSigner(RunAwaitingSigner {
                event_id: EventId("event-awaiting-signer".to_owned()),
                run_id: run_id.clone(),
                request_id: SignerRequestId("signer-1".to_owned()),
                reason: "waiting for signer approval".to_owned(),
            }),
        })
        .expect("append event");
    let mut session_store = InMemoryHostSessionStore::default();
    session_store.link_run(HostRunLink::new(
        host_session_id.clone(),
        run_id.clone(),
        mission.goal.clone(),
        mission.allowed_chains.clone(),
    ));

    let mut service = RuntimeHostService::new(
        InMemoryRunRepository::default(),
        checkpoint_repo,
        mission_repo,
        InMemoryRunCatalogRepository::default(),
        event_archive,
        session_store,
    );

    let response = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: None,
            command: RunCommand::InspectRun(ais_agent_control::commands::InspectRunCommand {
                command_id: CommandId("cmd-inspect-events".to_owned()),
                run_id,
            }),
        })
        .await;

    match response.response {
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(snapshot.recent_events.len(), 1);
            assert_eq!(
                snapshot.recent_events[0].event_type,
                "run.signer.request_created"
            );
            assert_eq!(
                snapshot.recent_events[0].summary,
                "waiting for signer approval"
            );
            assert_eq!(
                snapshot.recent_events[0]
                    .trace_context
                    .as_ref()
                    .map(|trace| trace.trace_id.as_str()),
                Some("trace-run-inspect-events:signer")
            );
        }
        other => panic!("unexpected response: {other:?}"),
    }
}

#[tokio::test]
async fn runtime_host_service_begin_run_requires_launch_spec() {
    let mut service = RuntimeHostService::new(
        InMemoryRunRepository::default(),
        InMemoryCheckpointRepository::default(),
        InMemoryMissionRepository::default(),
        InMemoryRunCatalogRepository::default(),
        InMemoryEventArchive::default(),
        InMemoryHostSessionStore::default(),
    );

    let begin = service
        .handle(HostCommandEnvelope {
            host_session_id: HostSessionId("session-missing-launch-spec".into()),
            host_request_id: None,
            command: RunCommand::BeginRun(BeginRunCommand {
                command_id: CommandId("cmd-missing-launch-spec".to_owned()),
                idempotency_key: IdempotencyKey("idem-missing-launch-spec".to_owned()),
                mission: MissionSubmission {
                    goal: "swap".to_owned(),
                    allowed_chains: vec!["eip155:1".to_owned()],
                    constraints: BTreeMap::new(),
                    budget: None,
                    metadata: BTreeMap::new(),
                },
                launch_spec: None,
            }),
        })
        .await;

    match begin.response {
        HostCommandResponse::Error(error) => {
            assert_eq!(error.code, "invalid_command");
            assert!(error.message.contains("launch_spec"));
        }
        other => panic!("unexpected response: {other:?}"),
    }
}

#[tokio::test]
async fn runtime_host_service_begin_run_rejects_reflection_request_launch_spec() {
    let mut service = RuntimeHostService::new(
        InMemoryRunRepository::default(),
        InMemoryCheckpointRepository::default(),
        InMemoryMissionRepository::default(),
        InMemoryRunCatalogRepository::default(),
        InMemoryEventArchive::default(),
        InMemoryHostSessionStore::default(),
    );

    let begin = service
        .handle(HostCommandEnvelope {
            host_session_id: HostSessionId("session-reflection-request".into()),
            host_request_id: None,
            command: RunCommand::BeginRun(BeginRunCommand {
                command_id: CommandId("cmd-reflection-request".to_owned()),
                idempotency_key: IdempotencyKey("idem-reflection-request".to_owned()),
                mission: MissionSubmission {
                    goal: "reflect".to_owned(),
                    allowed_chains: vec!["eip155:1".to_owned()],
                    constraints: BTreeMap::new(),
                    budget: None,
                    metadata: BTreeMap::new(),
                },
                launch_spec: Some(LaunchSpecSubmission::ReflectionRequest(
                    ReflectionRequestLaunchSpec {
                        request: json!({ "protocol": "uniswap_v3", "intent": "swap_exact_in" }),
                    },
                )),
            }),
        })
        .await;

    match begin.response {
        HostCommandResponse::Error(error) => {
            assert_eq!(error.code, "invalid_command");
            assert!(error.message.contains("reflection_request"));
        }
        other => panic!("unexpected response: {other:?}"),
    }
}

#[tokio::test]
async fn runtime_host_service_begin_run_seeds_execution_artifact_runtime_state_for_exports_and_continuation(
) {
    let host_session_id: HostSessionId = "session-execution-artifact".into();
    let mut service = RuntimeHostService::new(
        InMemoryRunRepository::default(),
        InMemoryCheckpointRepository::default(),
        InMemoryMissionRepository::default(),
        InMemoryRunCatalogRepository::default(),
        InMemoryEventArchive::default(),
        InMemoryHostSessionStore::default(),
    )
    .with_execution_wiring(RuntimeExecutionWiring {
        evm_rpc_url: Some("http://127.0.0.1:8545".to_owned()),
        solana_rpc_url: None,
    });

    let begin = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: None,
            command: RunCommand::BeginRun(BeginRunCommand {
                command_id: CommandId("cmd-execution-artifact".to_owned()),
                idempotency_key: IdempotencyKey("idem-execution-artifact".to_owned()),
                mission: MissionSubmission {
                    goal: "artifact".to_owned(),
                    allowed_chains: vec!["8453".to_owned()],
                    constraints: BTreeMap::new(),
                    budget: None,
                    metadata: BTreeMap::new(),
                },
                launch_spec: Some(LaunchSpecSubmission::ExecutionArtifact(
                    ExecutionArtifactLaunchSpec {
                        protocol_package_id: "owliabot.uniswap_v3".to_owned(),
                        action_key: "swap".to_owned(),
                        chain_family: ExecutionChainFamily::Evm,
                        allowed_chains: vec!["8453".to_owned()],
                        entry_stage_id: "stage.swap".into(),
                        actor: None,
                        transactions: vec![ExecutionTransactionCandidate::EvmTransaction(
                            EvmTransactionCandidate {
                                candidate_id: "swap.direct".into(),
                                to: "0x1111111111111111111111111111111111111111".to_owned(),
                                value: Some("0".to_owned()),
                                calldata: Some("0xdeadbeef".to_owned()),
                            },
                        )],
                        stages: vec![
                            ExecutionStage::Transaction(TransactionStage {
                                stage_id: "stage.swap".into(),
                                candidate_ref: "swap.direct".into(),
                                exports: vec![OutputExportSpec {
                                    output_key: "swap.tx_hash".into(),
                                    source: ValueRef::Ref {
                                        reference: "refs.receipts.stage.swap.tx_hash".to_owned(),
                                    },
                                }],
                                next_stage_id: Some("stage.continue".into()),
                            }),
                            ExecutionStage::Continuation(ContinuationStage {
                                stage_id: "stage.continue".into(),
                                required_outputs: vec!["swap.tx_hash".into()],
                                package_entry: "build_aave_supply_from_swap_output".into(),
                                next_stage_id: None,
                            }),
                        ],
                        observations: Vec::new(),
                        preconditions: Vec::new(),
                        postconditions: Vec::new(),
                        expected_effects: Vec::new(),
                        execution_policy: None,
                        risk_class: None,
                        risk_tags: Vec::new(),
                        decoded_intent: None,
                        candidate_envelopes: Vec::new(),
                        decode_spec: None,
                        validation_plan: None,
                        evidence: json!({}),
                        metadata: BTreeMap::new(),
                    },
                )),
            }),
        })
        .await;

    let run_id = match begin.response {
        HostCommandResponse::Accepted(response) => response.run_id.expect("run id"),
        other => panic!("unexpected response: {other:?}"),
    };

    let (run_repo, checkpoint_repo, _, _, _, _, _) = service.into_parts();
    let runtime = run_repo.load(&run_id).expect("runtime");
    let artifact = runtime
        .checkpoint
        .execution_artifact
        .as_ref()
        .expect("execution artifact state");
    assert_eq!(
        artifact
            .active_stage_id
            .as_ref()
            .map(|value| value.as_str()),
        Some("stage.swap")
    );
    assert!(artifact
        .planned_stage_graphs
        .contains_key(&"stage.swap".into()));
    assert!(runtime
        .checkpoint
        .action_graph
        .nodes
        .contains_key("artifact.stage.swap.verify"));

    let latest = checkpoint_repo
        .latest(run_id.0.as_str())
        .expect("latest checkpoint");
    assert!(latest.execution_artifact.is_some());
}

#[tokio::test]
async fn runtime_host_service_begin_run_seeds_simple_execution_artifact_checkpoint() {
    let host_session_id: HostSessionId = "session-execution-artifact-simple".into();
    let mut service = RuntimeHostService::new(
        InMemoryRunRepository::default(),
        InMemoryCheckpointRepository::default(),
        InMemoryMissionRepository::default(),
        InMemoryRunCatalogRepository::default(),
        InMemoryEventArchive::default(),
        InMemoryHostSessionStore::default(),
    )
    .with_execution_wiring(RuntimeExecutionWiring {
        evm_rpc_url: Some("http://127.0.0.1:8545".to_owned()),
        solana_rpc_url: None,
    });

    let begin = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: Some("request-execution-artifact-simple".into()),
            command: RunCommand::BeginRun(BeginRunCommand {
                command_id: CommandId("cmd-execution-artifact-simple".to_owned()),
                idempotency_key: IdempotencyKey("idem-execution-artifact-simple".to_owned()),
                mission: MissionSubmission {
                    goal: "artifact_simple".to_owned(),
                    allowed_chains: vec!["eip155:8453".to_owned()],
                    constraints: BTreeMap::new(),
                    budget: None,
                    metadata: BTreeMap::new(),
                },
                launch_spec: Some(LaunchSpecSubmission::ExecutionArtifact(
                    ExecutionArtifactLaunchSpec {
                        protocol_package_id: "owliabot.transfer".to_owned(),
                        action_key: "erc20_transfer".to_owned(),
                        chain_family: ExecutionChainFamily::Evm,
                        allowed_chains: vec!["eip155:8453".to_owned()],
                        entry_stage_id: "stage.transfer".into(),
                        actor: None,
                        transactions: vec![ExecutionTransactionCandidate::EvmTransaction(
                            EvmTransactionCandidate {
                                candidate_id: "transfer.call".into(),
                                to: "0x1111111111111111111111111111111111111111".to_owned(),
                                value: Some("0".to_owned()),
                                calldata: Some("0xa9059cbb".to_owned()),
                            },
                        )],
                        stages: vec![ExecutionStage::Transaction(TransactionStage {
                            stage_id: "stage.transfer".into(),
                            candidate_ref: "transfer.call".into(),
                            exports: Vec::new(),
                            next_stage_id: None,
                        })],
                        observations: Vec::new(),
                        preconditions: Vec::new(),
                        postconditions: Vec::new(),
                        expected_effects: Vec::new(),
                        execution_policy: None,
                        risk_class: None,
                        risk_tags: Vec::new(),
                        decoded_intent: None,
                        candidate_envelopes: Vec::new(),
                        decode_spec: None,
                        validation_plan: None,
                        evidence: json!({}),
                        metadata: BTreeMap::new(),
                    },
                )),
            }),
        })
        .await;

    let run_id = match begin.response {
        HostCommandResponse::Accepted(response) => response.run_id.expect("run id"),
        other => panic!("unexpected response: {other:?}"),
    };

    let (run_repo, checkpoint_repo, _, _, _, _, _) = service.into_parts();
    let runtime = run_repo.load(&run_id).expect("runtime");
    assert!(runtime
        .checkpoint
        .action_graph
        .nodes
        .contains_key("artifact.stage.transfer.simulate"));
    assert!(runtime
        .checkpoint
        .action_graph
        .nodes
        .contains_key("artifact.stage.transfer.actuate"));
    assert!(runtime
        .checkpoint
        .action_graph
        .nodes
        .contains_key("artifact.stage.transfer.verify"));

    match &runtime
        .checkpoint
        .action_graph
        .nodes
        .get("artifact.stage.transfer.simulate")
        .expect("simulate node")
        .payload
    {
        ActionPayload::Simulate(action) => {
            let live = action.live.as_ref().expect("live simulate binding");
            let ais_agent_core::action::kinds::simulate::SimulateLiveBinding::Evm(live) = live
            else {
                panic!("expected evm simulate binding");
            };
            assert_eq!(
                live.request.data,
                alloy::primitives::Bytes::copy_from_slice(&[0xa9, 0x05, 0x9c, 0xbb])
            );
        }
        other => panic!("unexpected simulate payload: {other:?}"),
    }

    let latest = checkpoint_repo
        .latest(run_id.0.as_str())
        .expect("latest checkpoint");
    assert!(latest
        .action_graph
        .nodes
        .contains_key("artifact.stage.transfer.verify"));
}

#[tokio::test]
async fn runtime_host_service_accepts_generic_execution_artifact_for_new_protocol_package() {
    let host_session_id: HostSessionId = "session-execution-artifact-generic-package".into();
    let mut service = RuntimeHostService::new(
        InMemoryRunRepository::default(),
        InMemoryCheckpointRepository::default(),
        InMemoryMissionRepository::default(),
        InMemoryRunCatalogRepository::default(),
        InMemoryEventArchive::default(),
        InMemoryHostSessionStore::default(),
    )
    .with_execution_wiring(RuntimeExecutionWiring {
        evm_rpc_url: Some("http://127.0.0.1:8545".to_owned()),
        solana_rpc_url: None,
    });

    let begin = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: Some("request-execution-artifact-generic-package".into()),
            command: RunCommand::BeginRun(BeginRunCommand {
                command_id: CommandId("cmd-execution-artifact-generic-package".to_owned()),
                idempotency_key: IdempotencyKey(
                    "idem-execution-artifact-generic-package".to_owned(),
                ),
                mission: MissionSubmission {
                    goal: "owliabot:demo.protocol:custom_call".to_owned(),
                    allowed_chains: vec!["8453".to_owned()],
                    constraints: BTreeMap::from([
                        ("owliabot_action_key".to_owned(), json!("custom_call")),
                        (
                            "owliabot_protocol_package_id".to_owned(),
                            json!("demo.protocol"),
                        ),
                        ("owliabot_execution_mode".to_owned(), json!("harness")),
                    ]),
                    budget: None,
                    metadata: BTreeMap::from([("proof".to_owned(), json!("m38.generic_package"))]),
                },
                launch_spec: Some(LaunchSpecSubmission::ExecutionArtifact(
                    ExecutionArtifactLaunchSpec {
                        protocol_package_id: "demo.protocol".to_owned(),
                        action_key: "custom_call".to_owned(),
                        chain_family: ExecutionChainFamily::Evm,
                        allowed_chains: vec!["8453".to_owned()],
                        entry_stage_id: "stage.call".into(),
                        actor: None,
                        transactions: vec![ExecutionTransactionCandidate::EvmTransaction(
                            EvmTransactionCandidate {
                                candidate_id: "call.direct".into(),
                                to: "0x9999999999999999999999999999999999999999".to_owned(),
                                value: Some("0".to_owned()),
                                calldata: Some("0xdeadbeef".to_owned()),
                            },
                        )],
                        stages: vec![ExecutionStage::Transaction(TransactionStage {
                            stage_id: "stage.call".into(),
                            candidate_ref: "call.direct".into(),
                            exports: Vec::new(),
                            next_stage_id: None,
                        })],
                        observations: Vec::new(),
                        preconditions: Vec::new(),
                        postconditions: Vec::new(),
                        expected_effects: Vec::new(),
                        execution_policy: None,
                        risk_class: None,
                        risk_tags: Vec::new(),
                        decoded_intent: None,
                        candidate_envelopes: Vec::new(),
                        decode_spec: None,
                        validation_plan: None,
                        evidence: json!({ "shape": "generic" }),
                        metadata: BTreeMap::from([("builder".to_owned(), json!("test.generic"))]),
                    },
                )),
            }),
        })
        .await;

    let run_id = match begin.response {
        HostCommandResponse::Accepted(response) => response.run_id.expect("run id"),
        other => panic!("unexpected response: {other:?}"),
    };

    let (run_repo, checkpoint_repo, mission_repo, _, _, _, _) = service.into_parts();
    let mission = mission_repo.load(&run_id).expect("persisted mission");
    assert_eq!(mission.goal, "owliabot:demo.protocol:custom_call");
    assert_eq!(
        mission.constraints.get("owliabot_protocol_package_id"),
        Some(&json!("demo.protocol"))
    );

    let runtime = run_repo.load(&run_id).expect("runtime");
    let artifact = runtime
        .checkpoint
        .execution_artifact
        .as_ref()
        .expect("execution artifact runtime state");
    assert_eq!(artifact.launch_spec.protocol_package_id, "demo.protocol");
    assert_eq!(artifact.launch_spec.action_key, "custom_call");
    assert!(runtime
        .checkpoint
        .action_graph
        .nodes
        .contains_key("artifact.stage.call.simulate"));
    assert!(runtime
        .checkpoint
        .action_graph
        .nodes
        .contains_key("artifact.stage.call.actuate"));
    assert!(runtime
        .checkpoint
        .action_graph
        .nodes
        .contains_key("artifact.stage.call.verify"));

    let latest = checkpoint_repo
        .latest(run_id.0.as_str())
        .expect("latest checkpoint");
    assert!(latest.execution_artifact.is_some());
}

#[tokio::test]
async fn runtime_host_service_begin_run_accepts_observe_only_execution_artifact() {
    let host_session_id: HostSessionId = "session-execution-artifact-observe".into();
    let mut service = RuntimeHostService::new(
        InMemoryRunRepository::default(),
        InMemoryCheckpointRepository::default(),
        InMemoryMissionRepository::default(),
        InMemoryRunCatalogRepository::default(),
        InMemoryEventArchive::default(),
        InMemoryHostSessionStore::default(),
    )
    .with_execution_wiring(RuntimeExecutionWiring {
        evm_rpc_url: Some("http://127.0.0.1:8545".to_owned()),
        solana_rpc_url: None,
    });

    let begin = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: Some("request-execution-artifact-observe".into()),
            command: RunCommand::BeginRun(BeginRunCommand {
                command_id: CommandId("cmd-execution-artifact-observe".to_owned()),
                idempotency_key: IdempotencyKey("idem-execution-artifact-observe".to_owned()),
                mission: MissionSubmission {
                    goal: "artifact_observe".to_owned(),
                    allowed_chains: vec!["eip155:1".to_owned()],
                    constraints: BTreeMap::new(),
                    budget: None,
                    metadata: BTreeMap::new(),
                },
                launch_spec: Some(LaunchSpecSubmission::ExecutionArtifact(
                    ExecutionArtifactLaunchSpec {
                        protocol_package_id: "owliabot.uniswap_v3".to_owned(),
                        action_key: "quote_exact_in_single".to_owned(),
                        chain_family: ExecutionChainFamily::Evm,
                        allowed_chains: vec!["eip155:1".to_owned()],
                        entry_stage_id: "stage.quote".into(),
                        actor: None,
                        transactions: Vec::new(),
                        stages: vec![ExecutionStage::Observe(ObserveStage {
                            stage_id: "stage.quote".into(),
                            observation_ref: "query.quote".to_owned(),
                            exports: vec![OutputExportSpec {
                                output_key: "quote.amount_out_atomic".into(),
                                source: ValueRef::Ref {
                                    reference: "refs.evidence.query.quote.amount_out_atomic"
                                        .to_owned(),
                                },
                            }],
                            next_stage_id: None,
                        })],
                        observations: vec![ObservationSpec {
                            observation_id: "query.quote".to_owned(),
                            kind: "evm.contract_state_read".to_owned(),
                            params: BTreeMap::from([
                                (
                                    "to".to_owned(),
                                    json!("0xb27308f9F90D607463bb33eA1BeBb41C27CE5AB6"),
                                ),
                                (
                                    "data".to_owned(),
                                    json!("0xf7729d43000000000000000000000000"),
                                ),
                            ]),
                        }],
                        preconditions: Vec::new(),
                        postconditions: Vec::new(),
                        expected_effects: Vec::new(),
                        execution_policy: None,
                        risk_class: None,
                        risk_tags: Vec::new(),
                        decoded_intent: None,
                        candidate_envelopes: Vec::new(),
                        decode_spec: None,
                        validation_plan: None,
                        evidence: json!({}),
                        metadata: BTreeMap::new(),
                    },
                )),
            }),
        })
        .await;

    let run_id = match begin.response {
        HostCommandResponse::Accepted(response) => response.run_id.expect("run id"),
        other => panic!("unexpected response: {other:?}"),
    };

    let (run_repo, checkpoint_repo, _, _, _, _, _) = service.into_parts();
    let runtime = run_repo.load(&run_id).expect("runtime");
    let artifact = runtime
        .checkpoint
        .execution_artifact
        .as_ref()
        .expect("execution artifact state");
    assert_eq!(
        artifact
            .active_stage_id
            .as_ref()
            .map(|value| value.as_str()),
        Some("stage.quote")
    );
    assert!(artifact
        .planned_stage_graphs
        .contains_key(&"stage.quote".into()));
    assert!(runtime
        .checkpoint
        .action_graph
        .nodes
        .contains_key("artifact.stage.quote.observe"));

    let latest = checkpoint_repo
        .latest(run_id.0.as_str())
        .expect("latest checkpoint");
    assert!(latest.execution_artifact.is_some());
}

#[tokio::test]
async fn runtime_host_service_submits_execution_artifact_continuation_and_reseeds_runtime() {
    let (
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
        run_id,
        host_session_id,
    ) = preloaded_continuation_wait_runtime();
    let mut service = RuntimeHostService::new(
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
    )
    .with_execution_wiring(RuntimeExecutionWiring {
        evm_rpc_url: Some("http://127.0.0.1:8545".to_owned()),
        solana_rpc_url: None,
    });

    let inspect = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: None,
            command: RunCommand::InspectRun(ais_agent_control::commands::InspectRunCommand {
                command_id: CommandId("cmd-execution-artifact-continuation-inspect".to_owned()),
                run_id: run_id.clone(),
            }),
        })
        .await;

    match inspect.response {
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(snapshot.status, RunStatus::AwaitingContinuation);
            assert_eq!(snapshot.pending_continuations.len(), 1);
        }
        other => panic!("unexpected inspect response: {other:?}"),
    }

    let paused = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-execution-artifact-continuation-pause".into()),
            command: RunCommand::StepRun(StepRunCommand {
                command_id: CommandId("cmd-execution-artifact-continuation-pause".to_owned()),
                run_id: run_id.clone(),
                until: StepUntil::CompleteOrBoundary,
                budget: Some(StepBudget {
                    max_nodes: Some(1),
                    max_wall_clock_ms: None,
                }),
                expected_version: None,
            }),
        })
        .await;

    match paused.response {
        HostCommandResponse::Pause(pause) => {
            assert_eq!(
                pause.kind,
                ais_agent_host::inspect::PauseKind::NeedContinuation
            );
            assert_eq!(pause.pending_continuations.len(), 1);
            assert_eq!(
                pause.pending_continuations[0].package_entry.as_str(),
                "build_aave_supply_from_swap_output"
            );
        }
        other => panic!("unexpected pause response: {other:?}"),
    }

    let continuation_batch = service
        .list_events(HostRunEventQuery {
            run_id: run_id.clone(),
            after_event_seq: Some(0),
            limit: Some(20),
        })
        .await
        .expect("continuation event batch");
    assert!(continuation_batch.events.iter().any(|event| matches!(
        event.event,
        RunEvent::AwaitingContinuation(ref awaiting)
            if awaiting.stage_id.as_ref().map(|stage_id| stage_id.as_str()) == Some("stage.continue")
                && awaiting
                    .package_entry
                    .as_ref()
                    .map(|package_entry| package_entry.as_str())
                    == Some("build_aave_supply_from_swap_output")
                && awaiting.required_outputs.len() == 1
                && awaiting.required_outputs[0].as_str() == "swap.tx_hash"
                && awaiting
                    .resolved_outputs
                    .get(&"swap.tx_hash".into())
                    .and_then(|value| value.as_str())
                    == Some("0xabc")
    )));

    let submit = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-execution-artifact-continuation-submit".into()),
            command: RunCommand::SubmitExecutionArtifactContinuation(
                SubmitExecutionArtifactContinuationCommand {
                    command_id: CommandId("cmd-execution-artifact-continuation-submit".to_owned()),
                    run_id: run_id.clone(),
                    package_entry: "build_aave_supply_from_swap_output".into(),
                    artifact: ExecutionArtifactLaunchSpec {
                        protocol_package_id: "owliabot.aave_v3".to_owned(),
                        action_key: "supply".to_owned(),
                        chain_family: ExecutionChainFamily::Evm,
                        allowed_chains: vec!["8453".to_owned()],
                        entry_stage_id: "stage.supply".into(),
                        actor: None,
                        transactions: vec![ExecutionTransactionCandidate::EvmTransaction(
                            EvmTransactionCandidate {
                                candidate_id: "supply.direct".into(),
                                to: "0x2222222222222222222222222222222222222222".to_owned(),
                                value: Some("0".to_owned()),
                                calldata: Some("0xbeadfeed".to_owned()),
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
                        risk_class: None,
                        risk_tags: Vec::new(),
                        decoded_intent: None,
                        candidate_envelopes: Vec::new(),
                        decode_spec: None,
                        validation_plan: None,
                        evidence: json!({}),
                        metadata: BTreeMap::new(),
                    },
                    expected_version: None,
                },
            ),
        })
        .await;

    match submit.response {
        HostCommandResponse::Pause(pause) => {
            assert_eq!(
                pause.kind,
                ais_agent_host::inspect::PauseKind::NeedUserInput
            );
            assert_eq!(
                pause.recovery_disposition,
                ais_agent_control::recovery::RecoveryDisposition::AwaitPatch
            );
            assert!(pause.pending_continuations.is_empty());
        }
        other => panic!("unexpected submit response: {other:?}"),
    }

    let (run_repo, checkpoint_repo, _, _, _, _, _) = service.into_parts();
    let runtime = run_repo.load(&run_id).expect("runtime");
    let artifact = runtime
        .checkpoint
        .execution_artifact
        .as_ref()
        .expect("execution artifact state");
    assert_eq!(artifact.launch_spec.action_key, "supply");
    assert_eq!(
        artifact
            .active_stage_id
            .as_ref()
            .map(|stage_id| stage_id.as_str()),
        Some("stage.supply")
    );
    assert!(artifact.awaiting_continuation.is_none());
    assert_eq!(
        artifact
            .exported_outputs
            .get(&"swap.tx_hash".into())
            .and_then(|value| value.as_str()),
        Some("0xabc")
    );

    let latest = checkpoint_repo
        .latest(run_id.0.as_str())
        .expect("latest checkpoint");
    assert_eq!(
        latest.lifecycle.status,
        ais_agent_core::runtime::RunStatus::Paused
    );
    assert!(latest
        .execution_artifact
        .as_ref()
        .and_then(|artifact| artifact.awaiting_continuation.as_ref())
        .is_none());
}

#[tokio::test]
async fn runtime_host_service_begin_run_accepts_branching_execution_artifact_entry_stage() {
    let mut service = RuntimeHostService::new(
        InMemoryRunRepository::default(),
        InMemoryCheckpointRepository::default(),
        InMemoryMissionRepository::default(),
        InMemoryRunCatalogRepository::default(),
        InMemoryEventArchive::default(),
        InMemoryHostSessionStore::default(),
    )
    .with_execution_wiring(RuntimeExecutionWiring {
        evm_rpc_url: Some("http://127.0.0.1:8545".to_owned()),
        solana_rpc_url: None,
    });

    let begin = service
        .handle(HostCommandEnvelope {
            host_session_id: HostSessionId("session-execution-artifact-branch".into()),
            host_request_id: None,
            command: RunCommand::BeginRun(BeginRunCommand {
                command_id: CommandId("cmd-execution-artifact-branch".to_owned()),
                idempotency_key: IdempotencyKey("idem-execution-artifact-branch".to_owned()),
                mission: MissionSubmission {
                    goal: "artifact_branch".to_owned(),
                    allowed_chains: vec!["8453".to_owned()],
                    constraints: BTreeMap::new(),
                    budget: None,
                    metadata: BTreeMap::new(),
                },
                launch_spec: Some(LaunchSpecSubmission::ExecutionArtifact(
                    ExecutionArtifactLaunchSpec {
                        protocol_package_id: "owliabot.uniswap_v3".to_owned(),
                        action_key: "swap".to_owned(),
                        chain_family: ExecutionChainFamily::Evm,
                        allowed_chains: vec!["8453".to_owned()],
                        entry_stage_id: "stage.allowance".into(),
                        actor: None,
                        transactions: vec![ExecutionTransactionCandidate::EvmTransaction(
                            EvmTransactionCandidate {
                                candidate_id: "swap.call".into(),
                                to: "0x1111111111111111111111111111111111111111".to_owned(),
                                value: Some("0".to_owned()),
                                calldata: Some("0xdeadbeef".to_owned()),
                            },
                        )],
                        stages: vec![
                            ExecutionStage::Branch(BranchStage {
                                stage_id: "stage.allowance".into(),
                                predicate: PredicateSpec::Comparison {
                                    left: ValueRef::Ref {
                                        reference: "refs.allowance.current_atomic".to_owned(),
                                    },
                                    op: ComparisonOperator::Lt,
                                    right: ValueRef::Literal {
                                        value: json!("100"),
                                    },
                                },
                                if_true: BranchTarget::GotoStage {
                                    stage_id: "stage.swap".into(),
                                },
                                if_false: BranchTarget::Assert {
                                    failure_code: "unexpected".to_owned(),
                                    message: "unexpected".to_owned(),
                                },
                            }),
                            ExecutionStage::Transaction(TransactionStage {
                                stage_id: "stage.swap".into(),
                                candidate_ref: "swap.call".into(),
                                exports: Vec::new(),
                                next_stage_id: None,
                            }),
                        ],
                        observations: Vec::new(),
                        preconditions: Vec::new(),
                        postconditions: Vec::new(),
                        expected_effects: Vec::new(),
                        execution_policy: None,
                        risk_class: None,
                        risk_tags: Vec::new(),
                        decoded_intent: None,
                        candidate_envelopes: Vec::new(),
                        decode_spec: None,
                        validation_plan: None,
                        evidence: json!({}),
                        metadata: BTreeMap::new(),
                    },
                )),
            }),
        })
        .await;

    let run_id = match begin.response {
        HostCommandResponse::Accepted(response) => response.run_id.expect("run id"),
        other => panic!("unexpected response: {other:?}"),
    };

    let (run_repo, checkpoint_repo, _, _, _, _, _) = service.into_parts();
    let runtime = run_repo.load(&run_id).expect("runtime");
    assert!(runtime.checkpoint.action_graph.nodes.is_empty());
    let artifact = runtime
        .checkpoint
        .execution_artifact
        .as_ref()
        .expect("execution artifact state");
    assert_eq!(
        artifact
            .active_stage_id
            .as_ref()
            .map(|value| value.as_str()),
        Some("stage.allowance")
    );

    let latest = checkpoint_repo
        .latest(run_id.0.as_str())
        .expect("latest checkpoint");
    assert!(latest.execution_artifact.is_some());
    assert!(latest.action_graph.nodes.is_empty());
}

#[tokio::test]
async fn runtime_host_service_begin_run_rejects_invalid_prebuilt_action_graph() {
    let mut service = RuntimeHostService::new(
        InMemoryRunRepository::default(),
        InMemoryCheckpointRepository::default(),
        InMemoryMissionRepository::default(),
        InMemoryRunCatalogRepository::default(),
        InMemoryEventArchive::default(),
        InMemoryHostSessionStore::default(),
    );

    let begin = service
        .handle(HostCommandEnvelope {
            host_session_id: HostSessionId("session-invalid-prebuilt-action-graph".into()),
            host_request_id: None,
            command: RunCommand::BeginRun(BeginRunCommand {
                command_id: CommandId("cmd-invalid-prebuilt-action-graph".to_owned()),
                idempotency_key: IdempotencyKey("idem-invalid-prebuilt-action-graph".to_owned()),
                mission: MissionSubmission {
                    goal: "invalid_prebuilt".to_owned(),
                    allowed_chains: vec!["eip155:1".to_owned()],
                    constraints: BTreeMap::new(),
                    budget: None,
                    metadata: BTreeMap::new(),
                },
                launch_spec: Some(LaunchSpecSubmission::PrebuiltFragment(
                    PrebuiltFragmentLaunchSpec {
                        action_graph: Some(json!("not-an-action-graph")),
                        evidence_graph: None,
                        effect_contracts: None,
                    },
                )),
            }),
        })
        .await;

    match begin.response {
        HostCommandResponse::Error(error) => {
            assert_eq!(error.code, "invalid_command");
            assert!(error.message.contains("prebuilt_fragment.action_graph"));
        }
        other => panic!("unexpected response: {other:?}"),
    }
}

#[tokio::test]
async fn runtime_host_service_begin_run_rejects_prebuilt_effect_contract_key_mismatch() {
    let mut service = RuntimeHostService::new(
        InMemoryRunRepository::default(),
        InMemoryCheckpointRepository::default(),
        InMemoryMissionRepository::default(),
        InMemoryRunCatalogRepository::default(),
        InMemoryEventArchive::default(),
        InMemoryHostSessionStore::default(),
    );

    let begin = service
        .handle(HostCommandEnvelope {
            host_session_id: HostSessionId("session-invalid-prebuilt-effect-contracts".into()),
            host_request_id: None,
            command: RunCommand::BeginRun(BeginRunCommand {
                command_id: CommandId("cmd-invalid-prebuilt-effect-contracts".to_owned()),
                idempotency_key: IdempotencyKey(
                    "idem-invalid-prebuilt-effect-contracts".to_owned(),
                ),
                mission: MissionSubmission {
                    goal: "invalid_prebuilt".to_owned(),
                    allowed_chains: vec!["eip155:1".to_owned()],
                    constraints: BTreeMap::new(),
                    budget: None,
                    metadata: BTreeMap::new(),
                },
                launch_spec: Some(LaunchSpecSubmission::PrebuiltFragment(
                    PrebuiltFragmentLaunchSpec {
                        action_graph: None,
                        evidence_graph: None,
                        effect_contracts: Some(json!({
                            "effect.expected": {
                                "effect_id": "effect.other",
                                "kind": "asset_delta",
                                "assertions": [],
                                "tolerance_hint": null
                            }
                        })),
                    },
                )),
            }),
        })
        .await;

    match begin.response {
        HostCommandResponse::Error(error) => {
            assert_eq!(error.code, "invalid_command");
            assert!(error.message.contains("effect_contracts key"));
        }
        other => panic!("unexpected response: {other:?}"),
    }
}

#[tokio::test]
async fn runtime_host_service_begin_run_accepts_native_transfer_execution_artifact() {
    let host_session_id: HostSessionId = "session-native-transfer-begin".into();
    let artifact = sample_native_transfer_execution_artifact();
    let mut service = RuntimeHostService::new(
        InMemoryRunRepository::default(),
        InMemoryCheckpointRepository::default(),
        InMemoryMissionRepository::default(),
        InMemoryRunCatalogRepository::default(),
        InMemoryEventArchive::default(),
        InMemoryHostSessionStore::default(),
    )
    .with_execution_wiring(RuntimeExecutionWiring {
        evm_rpc_url: Some("http://127.0.0.1:8545".to_owned()),
        solana_rpc_url: None,
    });

    let begin = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: Some("request-native-transfer-begin".into()),
            command: RunCommand::BeginRun(BeginRunCommand {
                command_id: CommandId("cmd-native-transfer-begin".to_owned()),
                idempotency_key: IdempotencyKey("idem-native-transfer-begin".to_owned()),
                mission: MissionSubmission {
                    goal: "owliabot:owliabot.transfer:native_transfer".to_owned(),
                    allowed_chains: vec!["11155111".to_owned()],
                    constraints: BTreeMap::from([
                        ("owliabot_action_key".to_owned(), json!("native_transfer")),
                        (
                            "owliabot_protocol_package_id".to_owned(),
                            json!("owliabot.transfer"),
                        ),
                        ("owliabot_execution_mode".to_owned(), json!("harness")),
                        (
                            "owliabot_execution_artifact".to_owned(),
                            serde_json::to_value(&artifact).expect("execution artifact json"),
                        ),
                    ]),
                    budget: Some(MissionBudgetSubmission {
                        max_steps: Some(8),
                        max_signer_requests: Some(1),
                        max_wall_clock_ms: Some(30_000),
                    }),
                    metadata: BTreeMap::from([
                        ("owliabot_agent_id".to_owned(), json!("test-agent")),
                        ("tool_name".to_owned(), json!("wallet_transfer")),
                    ]),
                },
                launch_spec: Some(LaunchSpecSubmission::ExecutionArtifact(artifact.clone())),
            }),
        })
        .await;

    let run_id = match begin.response {
        HostCommandResponse::Accepted(response) => response.run_id.expect("run id"),
        other => panic!("unexpected response: {other:?}"),
    };

    let (
        run_repo,
        checkpoint_repo,
        _mission_repo,
        _run_catalog_repo,
        _event_archive,
        _session_store,
        _signer_state_archive,
    ) = service.into_parts();
    let runtime = run_repo.load(&run_id).expect("runtime");
    let execution_artifact = runtime
        .checkpoint
        .execution_artifact
        .as_ref()
        .expect("execution artifact state");
    assert_eq!(execution_artifact.launch_spec.action_key, "native_transfer");
    assert_eq!(
        execution_artifact.launch_spec.expected_effects.len(),
        1,
        "native transfer artifact should seed one expected effect"
    );
    assert_eq!(
        execution_artifact
            .active_stage_id
            .as_ref()
            .map(|stage_id| stage_id.as_str()),
        Some("stage.transfer")
    );
    assert!(runtime
        .checkpoint
        .effect_contracts
        .contains_key("effect.transfer"));
    let verify_node = runtime
        .checkpoint
        .action_graph
        .nodes
        .get("artifact.stage.transfer.verify")
        .expect("verify node");
    let ActionPayload::Verify(verify_payload) = &verify_node.payload else {
        panic!("expected verify payload");
    };
    assert_eq!(verify_payload.verify_kind, VerifyKind::EffectContract);
    assert_eq!(
        verify_node.expected_effect_ref.as_deref(),
        Some("effect.transfer")
    );
    assert_eq!(
        execution_artifact
            .launch_spec
            .metadata
            .get("builder")
            .and_then(|value| value.as_str()),
        Some("buildNativeTransferExecutionArtifact")
    );
    assert!(runtime
        .checkpoint
        .action_graph
        .nodes
        .contains_key("artifact.stage.transfer.pre_observe.state.pre.recipient_balance"));
    assert!(runtime
        .checkpoint
        .action_graph
        .nodes
        .contains_key("artifact.stage.transfer.verify"));
    assert_eq!(runtime.checkpoint.effect_contracts.len(), 1);
    assert!(runtime.checkpoint.evidence_graph.records.is_empty());

    match &runtime
        .checkpoint
        .action_graph
        .nodes
        .get("artifact.stage.transfer.simulate")
        .expect("simulate node")
        .payload
    {
        ActionPayload::Simulate(action) => {
            let live = action.live.as_ref().expect("live simulate binding");
            let ais_agent_core::action::kinds::simulate::SimulateLiveBinding::Evm(live) = live
            else {
                panic!("expected evm simulate binding");
            };
            assert_eq!(live.request.value, Some(U256::from(30u64)));
        }
        other => panic!("unexpected simulate payload: {other:?}"),
    }

    let latest = checkpoint_repo
        .latest(run_id.0.as_str())
        .expect("latest checkpoint");
    assert!(latest.execution_artifact.as_ref().is_some_and(|artifact| {
        artifact
            .active_stage_id
            .as_ref()
            .map(|stage_id| stage_id.as_str())
            == Some("stage.transfer")
    }));
    assert!(latest
        .action_graph
        .nodes
        .contains_key("artifact.stage.transfer.actuate"));
}

#[tokio::test]
async fn runtime_host_service_begin_run_accepts_erc20_transfer_execution_artifact() {
    let host_session_id: HostSessionId = "session-erc20-transfer-begin".into();
    let artifact = sample_erc20_transfer_execution_artifact();
    let mut service = RuntimeHostService::new(
        InMemoryRunRepository::default(),
        InMemoryCheckpointRepository::default(),
        InMemoryMissionRepository::default(),
        InMemoryRunCatalogRepository::default(),
        InMemoryEventArchive::default(),
        InMemoryHostSessionStore::default(),
    )
    .with_execution_wiring(RuntimeExecutionWiring {
        evm_rpc_url: Some("http://127.0.0.1:8545".to_owned()),
        solana_rpc_url: None,
    });

    let begin = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: Some("request-erc20-transfer-begin".into()),
            command: RunCommand::BeginRun(BeginRunCommand {
                command_id: CommandId("cmd-erc20-transfer-begin".to_owned()),
                idempotency_key: IdempotencyKey("idem-erc20-transfer-begin".to_owned()),
                mission: MissionSubmission {
                    goal: "owliabot:owliabot.transfer:erc20_transfer".to_owned(),
                    allowed_chains: vec!["11155111".to_owned()],
                    constraints: BTreeMap::from([
                        ("owliabot_action_key".to_owned(), json!("erc20_transfer")),
                        (
                            "owliabot_protocol_package_id".to_owned(),
                            json!("owliabot.transfer"),
                        ),
                        ("owliabot_execution_mode".to_owned(), json!("harness")),
                        (
                            "owliabot_execution_artifact".to_owned(),
                            serde_json::to_value(&artifact).expect("execution artifact json"),
                        ),
                    ]),
                    budget: Some(MissionBudgetSubmission {
                        max_steps: Some(8),
                        max_signer_requests: Some(1),
                        max_wall_clock_ms: Some(30_000),
                    }),
                    metadata: BTreeMap::from([
                        ("owliabot_agent_id".to_owned(), json!("test-agent")),
                        ("tool_name".to_owned(), json!("wallet_transfer")),
                    ]),
                },
                launch_spec: Some(LaunchSpecSubmission::ExecutionArtifact(artifact.clone())),
            }),
        })
        .await;

    let run_id = match begin.response {
        HostCommandResponse::Accepted(response) => response.run_id.expect("run id"),
        other => panic!("unexpected response: {other:?}"),
    };

    let (
        run_repo,
        checkpoint_repo,
        _mission_repo,
        _run_catalog_repo,
        _event_archive,
        _session_store,
        _signer_state_archive,
    ) = service.into_parts();
    let runtime = run_repo.load(&run_id).expect("runtime");
    let execution_artifact = runtime
        .checkpoint
        .execution_artifact
        .as_ref()
        .expect("execution artifact state");
    assert_eq!(execution_artifact.launch_spec.action_key, "erc20_transfer");
    assert_eq!(
        execution_artifact.launch_spec.expected_effects.len(),
        1,
        "erc20 transfer artifact should seed one expected effect"
    );
    assert_eq!(
        execution_artifact
            .active_stage_id
            .as_ref()
            .map(|stage_id| stage_id.as_str()),
        Some("stage.transfer")
    );
    assert!(runtime
        .checkpoint
        .effect_contracts
        .contains_key("effect.transfer"));
    let verify_node = runtime
        .checkpoint
        .action_graph
        .nodes
        .get("artifact.stage.transfer.verify")
        .expect("verify node");
    let ActionPayload::Verify(verify_payload) = &verify_node.payload else {
        panic!("expected verify payload");
    };
    assert_eq!(verify_payload.verify_kind, VerifyKind::EffectContract);
    assert_eq!(
        verify_node.expected_effect_ref.as_deref(),
        Some("effect.transfer")
    );
    assert_eq!(
        execution_artifact
            .launch_spec
            .metadata
            .get("builder")
            .and_then(|value| value.as_str()),
        Some("buildErc20TransferExecutionArtifact")
    );
    assert!(runtime
        .checkpoint
        .action_graph
        .nodes
        .contains_key("artifact.stage.transfer.pre_observe.state.pre.recipient_token_balance"));
    assert!(runtime
        .checkpoint
        .action_graph
        .nodes
        .contains_key("artifact.stage.transfer.verify"));
    assert_eq!(runtime.checkpoint.effect_contracts.len(), 1);
    assert!(runtime.checkpoint.evidence_graph.records.is_empty());

    match &runtime
        .checkpoint
        .action_graph
        .nodes
        .get("artifact.stage.transfer.simulate")
        .expect("simulate node")
        .payload
    {
        ActionPayload::Simulate(action) => {
            let live = action.live.as_ref().expect("live simulate binding");
            let ais_agent_core::action::kinds::simulate::SimulateLiveBinding::Evm(live) = live
            else {
                panic!("expected evm simulate binding");
            };
            assert_eq!(
                live.request.to,
                alloy::primitives::address!("3333333333333333333333333333333333333333")
            );
            assert_eq!(live.request.data[0..4], [0xa9, 0x05, 0x9c, 0xbb]);
        }
        other => panic!("unexpected simulate payload: {other:?}"),
    }

    let latest = checkpoint_repo
        .latest(run_id.0.as_str())
        .expect("latest checkpoint");
    assert!(latest.execution_artifact.as_ref().is_some_and(|artifact| {
        artifact
            .active_stage_id
            .as_ref()
            .map(|stage_id| stage_id.as_str())
            == Some("stage.transfer")
    }));
    assert!(latest
        .action_graph
        .nodes
        .contains_key("artifact.stage.transfer.actuate"));
}

#[tokio::test]
async fn runtime_host_service_begin_run_accepts_owliabot_uniswap_v3_swap_execution_artifact() {
    let host_session_id: HostSessionId = "session-uniswap-v3-swap-begin".into();
    let artifact = sample_uniswap_v3_swap_execution_artifact();
    let mut service = RuntimeHostService::new(
        InMemoryRunRepository::default(),
        InMemoryCheckpointRepository::default(),
        InMemoryMissionRepository::default(),
        InMemoryRunCatalogRepository::default(),
        InMemoryEventArchive::default(),
        InMemoryHostSessionStore::default(),
    )
    .with_execution_wiring(RuntimeExecutionWiring {
        evm_rpc_url: Some("http://127.0.0.1:8545".to_owned()),
        solana_rpc_url: None,
    });

    let begin = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: Some("request-uniswap-v3-swap-begin".into()),
            command: RunCommand::BeginRun(BeginRunCommand {
                command_id: CommandId("cmd-uniswap-v3-swap-begin".to_owned()),
                idempotency_key: IdempotencyKey("idem-uniswap-v3-swap-begin".to_owned()),
                mission: MissionSubmission {
                    goal: "owliabot:owliabot.uniswap_v3:uniswap_v3_swap".to_owned(),
                    allowed_chains: vec!["8453".to_owned()],
                    constraints: BTreeMap::from([
                        ("owliabot_action_key".to_owned(), json!("uniswap_v3_swap")),
                        (
                            "owliabot_protocol_package_id".to_owned(),
                            json!("owliabot.uniswap_v3"),
                        ),
                        ("owliabot_execution_mode".to_owned(), json!("harness")),
                        (
                            "owliabot_execution_artifact".to_owned(),
                            serde_json::to_value(&artifact).expect("execution artifact json"),
                        ),
                    ]),
                    budget: Some(MissionBudgetSubmission {
                        max_steps: Some(8),
                        max_signer_requests: Some(1),
                        max_wall_clock_ms: Some(30_000),
                    }),
                    metadata: BTreeMap::from([
                        ("owliabot_agent_id".to_owned(), json!("test-agent")),
                        ("source".to_owned(), json!("skill:uniswap-v3-swap")),
                        ("tool_name".to_owned(), json!("ais_run_harness")),
                    ]),
                },
                launch_spec: Some(LaunchSpecSubmission::ExecutionArtifact(artifact.clone())),
            }),
        })
        .await;

    let run_id = match begin.response {
        HostCommandResponse::Accepted(response) => response.run_id.expect("run id"),
        other => panic!("unexpected response: {other:?}"),
    };

    let (
        run_repo,
        checkpoint_repo,
        mission_repo,
        _run_catalog_repo,
        _event_archive,
        _session_store,
        _signer_state_archive,
    ) = service.into_parts();
    let mission = mission_repo.load(&run_id).expect("persisted mission");
    assert_eq!(mission.goal, "owliabot:owliabot.uniswap_v3:uniswap_v3_swap");
    assert_eq!(mission.allowed_chains, vec!["8453".to_owned()]);
    assert_eq!(
        mission.constraints.get("owliabot_protocol_package_id"),
        Some(&json!("owliabot.uniswap_v3"))
    );
    assert_eq!(
        mission
            .constraints
            .get("owliabot_execution_artifact")
            .and_then(|value| value.get("action_key")),
        Some(&json!("uniswap_v3_swap"))
    );
    assert_eq!(
        mission.metadata.get("source"),
        Some(&json!("skill:uniswap-v3-swap"))
    );
    assert_eq!(
        mission.metadata.get("tool_name"),
        Some(&json!("ais_run_harness"))
    );

    let runtime = run_repo.load(&run_id).expect("runtime");
    let execution_artifact = runtime
        .checkpoint
        .execution_artifact
        .as_ref()
        .expect("execution artifact state");
    assert_eq!(execution_artifact.launch_spec.action_key, "uniswap_v3_swap");
    assert_eq!(
        execution_artifact.launch_spec.risk_class.as_deref(),
        Some("bounded_swap")
    );
    assert_eq!(
        execution_artifact.launch_spec.risk_tags,
        vec!["router_call"]
    );
    assert_eq!(execution_artifact.launch_spec.candidate_envelopes.len(), 1);
    assert_eq!(
        execution_artifact
            .active_stage_id
            .as_ref()
            .map(|stage_id| stage_id.as_str()),
        Some("stage.swap")
    );
    assert!(execution_artifact
        .planned_stage_graphs
        .contains_key(&"stage.swap".into()));
    assert!(runtime
        .checkpoint
        .action_graph
        .nodes
        .contains_key("artifact.stage.swap.simulate"));
    assert!(runtime
        .checkpoint
        .action_graph
        .nodes
        .contains_key("artifact.stage.swap.actuate"));
    assert!(runtime
        .checkpoint
        .action_graph
        .nodes
        .contains_key("artifact.stage.swap.verify"));

    match &runtime
        .checkpoint
        .action_graph
        .nodes
        .get("artifact.stage.swap.simulate")
        .expect("swap simulate node")
        .payload
    {
        ActionPayload::Simulate(action) => {
            let ais_agent_core::action::kinds::simulate::SimulateLiveBinding::Evm(live) =
                action.live.as_ref().expect("live simulate binding")
            else {
                panic!("expected evm simulate binding");
            };
            assert_eq!(
                format!("{:#x}", live.request.to),
                "0x5555555555555555555555555555555555555555"
            );
            assert_eq!(live.request.data[0..4], [0x41, 0x4b, 0xf3, 0x89]);
        }
        other => panic!("unexpected simulate payload: {other:?}"),
    }

    let latest = checkpoint_repo
        .latest(run_id.0.as_str())
        .expect("latest checkpoint");
    assert!(latest.execution_artifact.as_ref().is_some_and(|artifact| {
        artifact
            .active_stage_id
            .as_ref()
            .map(|stage_id| stage_id.as_str())
            == Some("stage.swap")
    }));
    assert_eq!(
        latest
            .execution_artifact
            .as_ref()
            .and_then(|artifact| artifact.launch_spec.risk_class.as_deref()),
        Some("bounded_swap")
    );
    assert!(latest
        .action_graph
        .nodes
        .contains_key("artifact.stage.swap.verify"));
}

#[tokio::test]
async fn runtime_host_service_begin_run_accepts_owliabot_uniswap_v3_lp_execution_artifact() {
    let host_session_id: HostSessionId = "session-uniswap-v3-lp-artifact-begin".into();
    let artifact = sample_uniswap_v3_lp_execution_artifact();
    let mut service = RuntimeHostService::new(
        InMemoryRunRepository::default(),
        InMemoryCheckpointRepository::default(),
        InMemoryMissionRepository::default(),
        InMemoryRunCatalogRepository::default(),
        InMemoryEventArchive::default(),
        InMemoryHostSessionStore::default(),
    )
    .with_execution_wiring(RuntimeExecutionWiring {
        evm_rpc_url: Some("http://127.0.0.1:8545".to_owned()),
        solana_rpc_url: None,
    });

    let begin = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: Some("request-uniswap-v3-lp-artifact-begin".into()),
            command: RunCommand::BeginRun(BeginRunCommand {
                command_id: CommandId("cmd-uniswap-v3-lp-artifact-begin".to_owned()),
                idempotency_key: IdempotencyKey("idem-uniswap-v3-lp-artifact-begin".to_owned()),
                mission: MissionSubmission {
                    goal: "owliabot:owliabot.uniswap_v3:uniswap_v3_lp".to_owned(),
                    allowed_chains: vec!["8453".to_owned()],
                    constraints: BTreeMap::from([
                        ("owliabot_action_key".to_owned(), json!("uniswap_v3_lp")),
                        (
                            "owliabot_protocol_package_id".to_owned(),
                            json!("owliabot.uniswap_v3"),
                        ),
                        ("owliabot_execution_mode".to_owned(), json!("harness")),
                        (
                            "owliabot_execution_artifact".to_owned(),
                            serde_json::to_value(&artifact).expect("execution artifact json"),
                        ),
                    ]),
                    budget: Some(MissionBudgetSubmission {
                        max_steps: Some(8),
                        max_signer_requests: Some(1),
                        max_wall_clock_ms: Some(30_000),
                    }),
                    metadata: BTreeMap::from([
                        ("owliabot_agent_id".to_owned(), json!("test-agent")),
                        ("source".to_owned(), json!("skill:uniswap-v3-lp")),
                        ("tool_name".to_owned(), json!("ais_run_harness")),
                    ]),
                },
                launch_spec: Some(LaunchSpecSubmission::ExecutionArtifact(artifact.clone())),
            }),
        })
        .await;

    let run_id = match begin.response {
        HostCommandResponse::Accepted(response) => response.run_id.expect("run id"),
        other => panic!("unexpected response: {other:?}"),
    };

    let (
        run_repo,
        checkpoint_repo,
        mission_repo,
        _run_catalog_repo,
        _event_archive,
        _session_store,
        _signer_state_archive,
    ) = service.into_parts();
    let mission = mission_repo.load(&run_id).expect("persisted mission");
    assert_eq!(mission.goal, "owliabot:owliabot.uniswap_v3:uniswap_v3_lp");
    assert_eq!(mission.allowed_chains, vec!["8453".to_owned()]);
    assert_eq!(
        mission.constraints.get("owliabot_protocol_package_id"),
        Some(&json!("owliabot.uniswap_v3"))
    );
    assert_eq!(
        mission
            .constraints
            .get("owliabot_execution_artifact")
            .and_then(|value| value.get("action_key")),
        Some(&json!("uniswap_v3_lp"))
    );
    assert_eq!(
        mission.metadata.get("source"),
        Some(&json!("skill:uniswap-v3-lp"))
    );
    assert_eq!(
        mission.metadata.get("tool_name"),
        Some(&json!("ais_run_harness"))
    );

    let runtime = run_repo.load(&run_id).expect("runtime");
    let execution_artifact = runtime
        .checkpoint
        .execution_artifact
        .as_ref()
        .expect("execution artifact state");
    assert_eq!(execution_artifact.launch_spec.action_key, "uniswap_v3_lp");
    assert_eq!(
        execution_artifact
            .active_stage_id
            .as_ref()
            .map(|stage_id| stage_id.as_str()),
        Some("stage.mint")
    );
    assert!(execution_artifact
        .planned_stage_graphs
        .contains_key(&"stage.mint".into()));
    assert!(runtime
        .checkpoint
        .action_graph
        .nodes
        .contains_key("artifact.stage.mint.pre_observe.state.pre.uniswap_v3_lp.position_count"));
    assert!(runtime
        .checkpoint
        .action_graph
        .nodes
        .contains_key("artifact.stage.mint.simulate"));
    assert!(runtime
        .checkpoint
        .action_graph
        .nodes
        .contains_key("artifact.stage.mint.verify"));
    assert!(runtime
        .checkpoint
        .action_graph
        .nodes
        .contains_key("artifact.stage.mint.post_observe.state.post.uniswap_v3_lp.position_count"));

    match &runtime
        .checkpoint
        .action_graph
        .nodes
        .get("artifact.stage.mint.simulate")
        .expect("lp mint simulate node")
        .payload
    {
        ActionPayload::Simulate(action) => {
            let ais_agent_core::action::kinds::simulate::SimulateLiveBinding::Evm(live) =
                action.live.as_ref().expect("live simulate binding")
            else {
                panic!("expected evm simulate binding");
            };
            assert_eq!(
                format!("{:#x}", live.request.to),
                "0x1234567890abcdef1234567890abcdef12345678"
            );
            assert_eq!(live.request.data[0..4], [0x88, 0x31, 0x64, 0x56]);
        }
        other => panic!("unexpected simulate payload: {other:?}"),
    }

    let latest = checkpoint_repo
        .latest(run_id.0.as_str())
        .expect("latest checkpoint");
    assert!(latest.execution_artifact.as_ref().is_some_and(|artifact| {
        artifact
            .active_stage_id
            .as_ref()
            .map(|stage_id| stage_id.as_str())
            == Some("stage.mint")
    }));
    assert!(latest
        .action_graph
        .nodes
        .contains_key("artifact.stage.mint.verify"));
}

#[tokio::test]
async fn runtime_host_service_request_cancel_marks_confirmation_wait_as_pending() {
    let (
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
        run_id,
        host_session_id,
    ) = preloaded_signer_wait_runtime();
    let mut service = RuntimeHostService::new(
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
    );

    let _submitted = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: None,
            command: RunCommand::SubmitSignerResolution(SubmitSignerResolutionCommand {
                command_id: CommandId("cmd-signer-for-cancel".to_owned()),
                run_id: run_id.clone(),
                resolution: SignerResolutionSubmission {
                    request_id: SignerRequestId("signer-1".to_owned()),
                    kind: SignerResolutionKind::Submitted,
                    tx_hash: Some("0xabc".to_owned()),
                    signed_payload: None,
                    details: BTreeMap::new(),
                },
                expected_version: None,
            }),
        })
        .await;

    let cancel = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: None,
            command: RunCommand::RequestCancelRun(RequestCancelRunCommand {
                command_id: CommandId("cmd-request-cancel".to_owned()),
                run_id,
                reason: Some("stop after submission".to_owned()),
                expected_version: None,
            }),
        })
        .await;

    match cancel.response {
        HostCommandResponse::Pause(pause) => {
            assert_eq!(
                pause.kind,
                ais_agent_host::inspect::PauseKind::NeedConfirmation
            );
            assert_eq!(pause.cancel_state, Some(CancelState::Pending));
            assert_eq!(
                pause.interruption_class,
                Some(InterruptionClass::HostCancelRequested)
            );
            assert!(!pause
                .allowed_recovery_actions
                .contains(&ais_agent_control::recovery::RecoveryActionKind::CancelRun));
            assert_eq!(pause.pending_confirmations.len(), 1);
        }
        other => panic!("unexpected response: {other:?}"),
    }
}

#[tokio::test]
async fn runtime_host_service_rejects_cancel_request_for_terminal_run() {
    let mut service = RuntimeHostService::new(
        InMemoryRunRepository::default(),
        InMemoryCheckpointRepository::default(),
        InMemoryMissionRepository::default(),
        InMemoryRunCatalogRepository::default(),
        InMemoryEventArchive::default(),
        InMemoryHostSessionStore::default(),
    );
    let host_session_id: HostSessionId = "session-cancel-reject".into();

    let begin = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: None,
            command: RunCommand::BeginRun(BeginRunCommand {
                command_id: CommandId("cmd-begin-cancel-reject".to_owned()),
                idempotency_key: IdempotencyKey("idem-cancel-reject".to_owned()),
                mission: MissionSubmission {
                    goal: "swap".to_owned(),
                    allowed_chains: vec!["eip155:1".to_owned()],
                    constraints: BTreeMap::new(),
                    budget: None,
                    metadata: BTreeMap::new(),
                },
                launch_spec: empty_prebuilt_launch_spec(),
            }),
        })
        .await;
    let run_id = match begin.response {
        HostCommandResponse::Accepted(response) => response.run_id.expect("run id"),
        other => panic!("unexpected response: {other:?}"),
    };

    let _cancelled = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: None,
            command: RunCommand::RequestCancelRun(RequestCancelRunCommand {
                command_id: CommandId("cmd-request-cancel-terminal".to_owned()),
                run_id: run_id.clone(),
                reason: Some("first cancel".to_owned()),
                expected_version: None,
            }),
        })
        .await;

    let rejected = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: None,
            command: RunCommand::RequestCancelRun(RequestCancelRunCommand {
                command_id: CommandId("cmd-request-cancel-terminal-again".to_owned()),
                run_id,
                reason: Some("second cancel".to_owned()),
                expected_version: None,
            }),
        })
        .await;

    match rejected.response {
        HostCommandResponse::Error(error) => assert_eq!(error.code, "cancel_rejected"),
        other => panic!("unexpected response: {other:?}"),
    }
}

#[tokio::test]
async fn runtime_host_service_claim_run_acquires_unclaimed_preloaded_run() {
    let (
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        _session_store,
        run_id,
        _original_session_id,
    ) = preloaded_evidence_wait_runtime();
    let host_session_id: HostSessionId = "session-claim-acquire".into();
    let mut service = RuntimeHostService::new(
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        InMemoryHostSessionStore::default(),
    );

    let claimed = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-claim-acquire".into()),
            command: RunCommand::ClaimRun(ClaimRunCommand {
                command_id: CommandId("cmd-claim-acquire".to_owned()),
                run_id: run_id.clone(),
                owner_kind: RunClaimOwnerKind::InteractiveHost,
                owner_instance_id: host_session_id.0.clone(),
                mode: RunClaimMode::ExclusiveMutation,
                requested_lease_ms: Some(30_000),
                allow_supersede: false,
                expected_current_claim_id: None,
                expected_current_claim_epoch: None,
            }),
        })
        .await;

    let ownership = response_ownership(&claimed.response);
    let claim = ownership
        .current_claim
        .as_ref()
        .expect("claim should be present after acquire");
    assert_eq!(claim.host_session_id, host_session_id.0);
    assert_eq!(claim.mode, RunClaimMode::ExclusiveMutation);
}

#[tokio::test]
async fn runtime_host_service_renews_and_releases_pre_side_effect_claim() {
    let (
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        _session_store,
        run_id,
        _original_session_id,
    ) = preloaded_evidence_wait_runtime();
    let host_session_id: HostSessionId = "session-claim-renew-release".into();
    let mut service = RuntimeHostService::new(
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        InMemoryHostSessionStore::default(),
    );

    let claimed = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-claim-seed".into()),
            command: RunCommand::ClaimRun(ClaimRunCommand {
                command_id: CommandId("cmd-claim-seed".to_owned()),
                run_id: run_id.clone(),
                owner_kind: RunClaimOwnerKind::InteractiveHost,
                owner_instance_id: host_session_id.0.clone(),
                mode: RunClaimMode::ExclusiveMutation,
                requested_lease_ms: Some(30_000),
                allow_supersede: false,
                expected_current_claim_id: None,
                expected_current_claim_epoch: None,
            }),
        })
        .await;
    let claimed_ownership = response_ownership(&claimed.response);
    let claimed_claim = claimed_ownership
        .current_claim
        .as_ref()
        .expect("claim should be present");
    let claim_id = claimed_claim.claim_id.clone();
    let claim_epoch = claimed_claim.claim_epoch;

    let renewed = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-claim-renew".into()),
            command: RunCommand::RenewRunClaim(RenewRunClaimCommand {
                command_id: CommandId("cmd-claim-renew".to_owned()),
                run_id: run_id.clone(),
                claim_id: claim_id.clone(),
                claim_epoch,
                requested_lease_ms: Some(60_000),
            }),
        })
        .await;
    let renewed_ownership = response_ownership(&renewed.response);
    let renewed_claim = renewed_ownership
        .current_claim
        .as_ref()
        .expect("claim should remain present");
    assert_eq!(renewed_claim.claim_id, claim_id);
    assert!(renewed_claim.claim_epoch > claim_epoch);
    let renewed_epoch = renewed_claim.claim_epoch;

    let released = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-claim-release".into()),
            command: RunCommand::ReleaseRunClaim(ReleaseRunClaimCommand {
                command_id: CommandId("cmd-claim-release".to_owned()),
                run_id,
                claim_id,
                claim_epoch: renewed_epoch,
                reason: Some("handoff".to_owned()),
            }),
        })
        .await;
    assert!(response_ownership(&released.response)
        .current_claim
        .is_none());
}

#[tokio::test]
async fn runtime_host_service_claim_run_can_reacquire_after_expiry() {
    let (
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
        run_id,
        original_session_id,
    ) = preloaded_evidence_wait_runtime();
    let mut claim_repo = InMemoryRunClaimRepository::default();
    claim_repo
        .acquire(sample_runtime_claim(
            &run_id,
            &original_session_id,
            "expired-claim",
            1,
            Some(1),
        ))
        .expect("seed expiring claim");
    let new_session_id: HostSessionId = "session-claim-reacquire".into();
    let mut service = RuntimeHostService::new_with_archives_and_claim_repo(
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
        InMemorySignerStateStore::default(),
        crate::persistence::InMemoryRuntimeAuditArchive::default(),
        claim_repo,
    );

    let claimed = service
        .handle(HostCommandEnvelope {
            host_session_id: new_session_id.clone(),
            host_request_id: Some("request-claim-reacquire".into()),
            command: RunCommand::ClaimRun(ClaimRunCommand {
                command_id: CommandId("cmd-claim-reacquire".to_owned()),
                run_id: run_id.clone(),
                owner_kind: RunClaimOwnerKind::InteractiveHost,
                owner_instance_id: new_session_id.0.clone(),
                mode: RunClaimMode::ExclusiveMutation,
                requested_lease_ms: Some(30_000),
                allow_supersede: false,
                expected_current_claim_id: None,
                expected_current_claim_epoch: None,
            }),
        })
        .await;

    let claim = response_ownership(&claimed.response)
        .current_claim
        .as_ref()
        .expect("claim should be present");
    assert_eq!(claim.host_session_id, new_session_id.0);
}

#[tokio::test]
async fn runtime_host_service_claim_run_allows_pre_side_effect_supersede_but_not_confirmation_takeover(
) {
    let (
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
        run_id,
        original_session_id,
    ) = preloaded_evidence_wait_runtime();
    let mut claim_repo = InMemoryRunClaimRepository::default();
    let original_claim = claim_repo
        .acquire(sample_runtime_claim(
            &run_id,
            &original_session_id,
            "claim-pre-side-effect",
            1,
            Some(u64::MAX / 4),
        ))
        .expect("seed active claim");
    let takeover_session: HostSessionId = "session-claim-takeover".into();
    let mut service = RuntimeHostService::new_with_archives_and_claim_repo(
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
        InMemorySignerStateStore::default(),
        crate::persistence::InMemoryRuntimeAuditArchive::default(),
        claim_repo,
    );

    let superseded = service
        .handle(HostCommandEnvelope {
            host_session_id: takeover_session.clone(),
            host_request_id: Some("request-claim-supersede".into()),
            command: RunCommand::ClaimRun(ClaimRunCommand {
                command_id: CommandId("cmd-claim-supersede".to_owned()),
                run_id: run_id.clone(),
                owner_kind: RunClaimOwnerKind::InteractiveHost,
                owner_instance_id: takeover_session.0.clone(),
                mode: RunClaimMode::ExclusiveMutation,
                requested_lease_ms: Some(30_000),
                allow_supersede: true,
                expected_current_claim_id: Some(original_claim.claim_id.clone()),
                expected_current_claim_epoch: Some(original_claim.claim_epoch),
            }),
        })
        .await;
    assert_eq!(
        response_ownership(&superseded.response)
            .current_claim
            .as_ref()
            .map(|claim| claim.host_session_id.as_str()),
        Some(takeover_session.0.as_str())
    );

    let mut confirmation_claim_repo = InMemoryRunClaimRepository::default();
    let confirmation_run_id = RunId("run-1".to_owned());
    let confirmation_session_id: HostSessionId = "session-confirm-owner".into();
    let confirmation_mission = sample_mission();
    let confirmation_checkpoint = confirmation_wait_checkpoint();
    let mut confirmation_run_repo = InMemoryRunRepository::default();
    confirmation_run_repo
        .insert(ActiveRun::new(
            confirmation_mission.clone(),
            confirmation_checkpoint.clone(),
        ))
        .expect("insert runtime");
    let mut confirmation_checkpoint_repo = InMemoryCheckpointRepository::default();
    confirmation_checkpoint_repo
        .append(CheckpointArchiveEntry {
            snapshot: confirmation_checkpoint,
            kind: CheckpointArchiveKind::SideEffect,
        })
        .expect("append checkpoint");
    let mut confirmation_session_store = InMemoryHostSessionStore::default();
    confirmation_session_store.link_run(HostRunLink::new(
        confirmation_session_id.clone(),
        confirmation_run_id.clone(),
        confirmation_mission.goal.clone(),
        confirmation_mission.allowed_chains.clone(),
    ));
    let confirmation_mission_repo =
        preloaded_mission_repo(confirmation_run_id.clone(), confirmation_mission);
    let confirmation_claim = confirmation_claim_repo
        .acquire(sample_runtime_claim(
            &confirmation_run_id,
            &confirmation_session_id,
            "claim-confirmation",
            1,
            Some(u64::MAX / 4),
        ))
        .expect("seed confirmation claim");
    let takeover_confirmation_session: HostSessionId = "session-confirm-takeover".into();
    let mut confirmation_service = RuntimeHostService::new_with_archives_and_claim_repo(
        confirmation_run_repo,
        confirmation_checkpoint_repo,
        confirmation_mission_repo,
        InMemoryRunCatalogRepository::default(),
        InMemoryEventArchive::default(),
        confirmation_session_store,
        InMemorySignerStateStore::default(),
        crate::persistence::InMemoryRuntimeAuditArchive::default(),
        confirmation_claim_repo,
    );

    let rejected = confirmation_service
        .handle(HostCommandEnvelope {
            host_session_id: takeover_confirmation_session,
            host_request_id: Some("request-claim-confirm-takeover".into()),
            command: RunCommand::ClaimRun(ClaimRunCommand {
                command_id: CommandId("cmd-claim-confirm-takeover".to_owned()),
                run_id: confirmation_run_id,
                owner_kind: RunClaimOwnerKind::InteractiveHost,
                owner_instance_id: "session-confirm-takeover".to_owned(),
                mode: RunClaimMode::ExclusiveMutation,
                requested_lease_ms: Some(30_000),
                allow_supersede: true,
                expected_current_claim_id: Some(confirmation_claim.claim_id),
                expected_current_claim_epoch: Some(confirmation_claim.claim_epoch),
            }),
        })
        .await;
    match rejected.response {
        HostCommandResponse::Error(error) => {
            assert_eq!(error.code, "claim_transfer_required");
        }
        other => panic!("unexpected response: {other:?}"),
    }
}

#[tokio::test]
async fn runtime_host_service_begin_run_skips_existing_durable_run_ids_after_restart() {
    let mut mission_repo = InMemoryMissionRepository::default();
    mission_repo
        .insert(RunId("run-1".to_owned()), sample_mission())
        .expect("insert first mission");
    let mut second_mission = sample_mission();
    second_mission.mission_id = "mission-2".to_owned();
    mission_repo
        .insert(RunId("run-2".to_owned()), second_mission)
        .expect("insert second mission");

    let mut service = RuntimeHostService::new(
        InMemoryRunRepository::default(),
        InMemoryCheckpointRepository::default(),
        mission_repo,
        InMemoryRunCatalogRepository::default(),
        InMemoryEventArchive::default(),
        InMemoryHostSessionStore::default(),
    );
    let host_session_id: HostSessionId = "session-restart-begin".into();

    let begin = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: None,
            command: RunCommand::BeginRun(BeginRunCommand {
                command_id: CommandId("cmd-begin-after-restart".to_owned()),
                idempotency_key: IdempotencyKey("idem-begin-after-restart".to_owned()),
                mission: MissionSubmission {
                    goal: "swap after restart".to_owned(),
                    allowed_chains: vec!["eip155:1".to_owned()],
                    constraints: BTreeMap::new(),
                    budget: Some(MissionBudgetSubmission {
                        max_steps: Some(8),
                        max_signer_requests: Some(1),
                        max_wall_clock_ms: Some(30_000),
                    }),
                    metadata: BTreeMap::new(),
                },
                launch_spec: empty_prebuilt_launch_spec(),
            }),
        })
        .await;

    let run_id = match begin.response {
        HostCommandResponse::Accepted(response) => response.run_id.expect("run id"),
        other => panic!("unexpected response: {other:?}"),
    };
    assert_eq!(run_id.0, "run-3");

    let (
        _run_repo,
        _checkpoint_repo,
        mission_repo,
        _run_catalog_repo,
        _event_archive,
        _session_store,
        _signer_state_archive,
    ) = service.into_parts();
    let mission = mission_repo.load(&run_id).expect("load persisted mission");
    assert_eq!(mission.mission_id, "mission-3");
}

#[tokio::test]
async fn runtime_host_service_fails_closed_when_grouped_begin_run_mission_write_fails() {
    let host_session_id: HostSessionId = "session-begin-mission-fail".into();
    let run_id = RunId("run-1".to_owned());
    let mut service = RuntimeHostService::new(
        InMemoryRunRepository::default(),
        InMemoryCheckpointRepository::default(),
        FailingMissionRepository::fail_on_nth_insert(1),
        InMemoryRunCatalogRepository::default(),
        InMemoryEventArchive::default(),
        InMemoryHostSessionStore::default(),
    );

    let begin = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: None,
            command: RunCommand::BeginRun(BeginRunCommand {
                command_id: CommandId("cmd-begin-mission-fail".to_owned()),
                idempotency_key: IdempotencyKey("idem-begin-mission-fail".to_owned()),
                mission: MissionSubmission {
                    goal: "swap".to_owned(),
                    allowed_chains: vec!["eip155:1".to_owned()],
                    constraints: BTreeMap::new(),
                    budget: Some(MissionBudgetSubmission {
                        max_steps: Some(8),
                        max_signer_requests: Some(1),
                        max_wall_clock_ms: Some(30_000),
                    }),
                    metadata: BTreeMap::new(),
                },
                launch_spec: empty_prebuilt_launch_spec(),
            }),
        })
        .await;

    match begin.response {
        HostCommandResponse::Error(error) => assert_eq!(error.code, "mission_error"),
        other => panic!("unexpected response: {other:?}"),
    }

    let (
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        _session_store,
        _signer_state_archive,
    ) = service.into_parts();
    assert!(run_repo.load(&run_id).is_err());
    assert!(checkpoint_repo.latest(&run_id.0).is_err());
    assert!(mission_repo.load(&run_id).is_err());
    assert!(run_catalog_repo.load(&run_id).is_err());
    assert!(event_archive
        .read(EventArchiveQuery {
            run_id,
            after_event_seq: None,
            limit: Some(8),
        })
        .is_err());
}

#[tokio::test]
async fn runtime_host_service_inspect_rejects_invalid_recovery_contract() {
    let run_id = RunId("run-invalid-recovery".to_owned());
    let host_session_id: HostSessionId = "session-invalid-recovery".into();
    let mission = sample_mission();
    let mut checkpoint = checkpoint_with_nodes(
        vec![derive_terminal_node("derive-quote")],
        vec!["derive-quote".to_owned()],
    );
    checkpoint.run_id = run_id.0.clone();
    checkpoint.lifecycle.run_id = run_id.clone();
    checkpoint.pending_requests.pending_evidence_refs = vec!["evidence.quote".to_owned()];
    checkpoint.lifecycle.status = ais_agent_core::runtime::RunStatus::AwaitingEvidence;
    checkpoint.lifecycle.failure = Some(RunFailureContext::new(
        RunFailureCode::MissingEvidence,
        RunFailureStage::Observe,
        checkpoint.checkpoint_seq,
        checkpoint.plan_epoch,
        Some(ais_agent_control::recovery::StableBoundaryKind::Evidence),
        "quote missing",
    ));

    let mut mission_repo = InMemoryMissionRepository::default();
    mission_repo
        .insert(run_id.clone(), mission.clone())
        .expect("insert mission");
    let mut checkpoint_repo = InMemoryCheckpointRepository::default();
    checkpoint_repo
        .append(CheckpointArchiveEntry {
            snapshot: checkpoint,
            kind: CheckpointArchiveKind::Boundary,
        })
        .expect("append checkpoint");
    let mut session_store = InMemoryHostSessionStore::default();
    session_store.link_run(HostRunLink::new(
        host_session_id.clone(),
        run_id.clone(),
        mission.goal.clone(),
        mission.allowed_chains.clone(),
    ));

    let mut service = RuntimeHostService::new(
        InMemoryRunRepository::default(),
        checkpoint_repo,
        mission_repo,
        InMemoryRunCatalogRepository::default(),
        InMemoryEventArchive::default(),
        session_store,
    );

    let response = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: None,
            command: RunCommand::InspectRun(ais_agent_control::commands::InspectRunCommand {
                command_id: CommandId("cmd-inspect-invalid-recovery".to_owned()),
                run_id,
            }),
        })
        .await;

    match response.response {
        HostCommandResponse::Error(error) => {
            assert_eq!(error.code, "recovery_contract_invalid");
        }
        other => panic!("unexpected response: {other:?}"),
    }
}

#[tokio::test]
async fn runtime_host_service_accepts_evidence_then_steps_to_completion() {
    let (
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
        run_id,
        host_session_id,
    ) = preloaded_evidence_wait_runtime();
    let mut service = RuntimeHostService::new(
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
    );

    let submit = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: None,
            command: RunCommand::SubmitEvidence(SubmitEvidenceCommand {
                command_id: CommandId("cmd-evidence".to_owned()),
                run_id: run_id.clone(),
                evidence: EvidenceSubmission {
                    evidence_id: "quote".to_owned(),
                    kind: EvidenceKind::RouteOrQuote,
                    source: "quote-api".to_owned(),
                    observed_at_ms: Some(10),
                    chain_scope: Some("eip155:1".to_owned()),
                    payload: json!({"amount_out":"1000"}),
                    confidence: Some(0.95),
                },
                expected_version: None,
            }),
        })
        .await;
    match submit.response {
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(snapshot.run_id, run_id);
        }
        other => panic!("unexpected response: {other:?}"),
    }

    let stepped = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: None,
            command: RunCommand::StepRun(StepRunCommand {
                command_id: CommandId("cmd-step".to_owned()),
                run_id,
                until: StepUntil::CompleteOrBoundary,
                budget: Some(StepBudget {
                    max_nodes: Some(8),
                    max_wall_clock_ms: None,
                }),
                expected_version: None,
            }),
        })
        .await;
    match stepped.response {
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(snapshot.status, RunStatus::Completed);
        }
        other => panic!("unexpected response: {other:?}"),
    }
}

#[tokio::test]
async fn runtime_host_service_fails_closed_when_grouped_checkpoint_write_fails() {
    let (
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
        run_id,
        host_session_id,
    ) = preloaded_evidence_wait_runtime();

    let mut service = RuntimeHostService::new(
        run_repo,
        FailingCheckpointRepository::fail_on_nth_append(checkpoint_repo, 1),
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
    );

    let failed = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: Some("request-fail-checkpoint-archive".into()),
            command: RunCommand::SubmitEvidence(SubmitEvidenceCommand {
                command_id: CommandId("cmd-fail-checkpoint-archive".to_owned()),
                run_id: run_id.clone(),
                evidence: EvidenceSubmission {
                    evidence_id: "quote".to_owned(),
                    kind: EvidenceKind::RouteOrQuote,
                    source: "quote-api".to_owned(),
                    observed_at_ms: Some(42),
                    chain_scope: Some("eip155:1".to_owned()),
                    payload: json!({"amount_out":"1000"}),
                    confidence: Some(0.95),
                },
                expected_version: None,
            }),
        })
        .await;

    match failed.response {
        HostCommandResponse::Error(error) => assert_eq!(error.code, "checkpoint_error"),
        other => panic!("unexpected failure response: {other:?}"),
    }

    let (
        run_repo,
        checkpoint_repo,
        _mission_repo,
        run_catalog_repo,
        event_archive,
        _session_store,
        _signer_state_archive,
    ) = service.into_parts();
    let hot_runtime = run_repo
        .load(&run_id)
        .expect("hot runtime remains pre-commit");
    assert_eq!(
        hot_runtime.checkpoint.lifecycle.status,
        ais_agent_core::runtime::RunStatus::AwaitingEvidence
    );
    let checkpoint = checkpoint_repo
        .latest(run_id.0.as_str())
        .expect("latest durable checkpoint");
    assert_eq!(
        checkpoint.lifecycle.status,
        ais_agent_core::runtime::RunStatus::AwaitingEvidence
    );
    assert!(run_catalog_repo.load(&run_id).is_err());
    assert!(event_archive
        .read(EventArchiveQuery {
            run_id,
            after_event_seq: None,
            limit: Some(8),
        })
        .is_err());
}

#[tokio::test]
async fn runtime_host_service_can_complete_from_alloy_backed_live_observation() {
    let (
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
        run_id,
        host_session_id,
    ) = preloaded_evidence_wait_runtime();
    let mut service = RuntimeHostService::new(
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
    );

    let asserter = Asserter::new();
    let provider = ProviderBuilder::new().connect_mocked_client(asserter.clone());
    asserter.push_success(&777u64);
    let live_observation = EvmAlloyReadPort::get_block_number_with_provider(&provider)
        .await
        .expect("live observation");

    let submit = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-live-evidence".into()),
            command: RunCommand::SubmitEvidence(SubmitEvidenceCommand {
                command_id: CommandId("cmd-live-evidence".to_owned()),
                run_id: run_id.clone(),
                evidence: EvidenceSubmission {
                    evidence_id: "quote".to_owned(),
                    kind: EvidenceKind::RouteOrQuote,
                    source: "evm.alloy.live_read".to_owned(),
                    observed_at_ms: Some(10),
                    chain_scope: Some("eip155:1".to_owned()),
                    payload: EvmAlloyReadPort::block_observation_payload(&live_observation),
                    confidence: Some(0.95),
                },
                expected_version: None,
            }),
        })
        .await;

    match submit.response {
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(snapshot.run_id, run_id);
        }
        other => panic!("unexpected response: {other:?}"),
    }

    let stepped = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: Some("request-live-step".into()),
            command: RunCommand::StepRun(StepRunCommand {
                command_id: CommandId("cmd-live-step".to_owned()),
                run_id,
                until: StepUntil::CompleteOrBoundary,
                budget: Some(StepBudget {
                    max_nodes: Some(8),
                    max_wall_clock_ms: None,
                }),
                expected_version: None,
            }),
        })
        .await;

    match stepped.response {
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(snapshot.status, RunStatus::Completed);
            assert_eq!(
                snapshot.effect_status,
                Some(ais_agent_host::inspect::EffectStatusView::Satisfied)
            );
        }
        other => panic!("unexpected response: {other:?}"),
    }
}

#[tokio::test]
async fn runtime_host_service_accepts_signer_decision_then_steps_to_completion() {
    let (
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
        run_id,
        host_session_id,
    ) = preloaded_signer_wait_runtime();
    let mut service = RuntimeHostService::new(
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
    );

    let submit = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: None,
            command: RunCommand::SubmitSignerResolution(SubmitSignerResolutionCommand {
                command_id: CommandId("cmd-signer".to_owned()),
                run_id: run_id.clone(),
                resolution: SignerResolutionSubmission {
                    request_id: SignerRequestId("signer-1".to_owned()),
                    kind: SignerResolutionKind::Submitted,
                    tx_hash: Some("0xabc".to_owned()),
                    signed_payload: None,
                    details: BTreeMap::new(),
                },
                expected_version: None,
            }),
        })
        .await;
    match submit.response {
        HostCommandResponse::Pause(snapshot) => {
            assert_eq!(snapshot.run_id, run_id);
            assert_eq!(
                snapshot.kind,
                ais_agent_host::inspect::PauseKind::NeedConfirmation
            );
        }
        other => panic!("unexpected response: {other:?}"),
    }
}

#[tokio::test]
async fn runtime_host_service_step_run_stops_at_awaiting_signer_without_advancing_consent() {
    let (
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
        run_id,
        host_session_id,
    ) = preloaded_signer_wait_runtime();
    let mut service = RuntimeHostService::new(
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
    );

    let stepped = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: Some("request-step-awaiting-signer".into()),
            command: RunCommand::StepRun(StepRunCommand {
                command_id: CommandId("cmd-step-awaiting-signer".to_owned()),
                run_id: run_id.clone(),
                until: StepUntil::CompleteOrBoundary,
                budget: Some(StepBudget {
                    max_nodes: Some(8),
                    max_wall_clock_ms: None,
                }),
                expected_version: None,
            }),
        })
        .await;

    match stepped.response {
        HostCommandResponse::Pause(pause) => {
            assert_eq!(pause.run_id, run_id);
            assert_eq!(pause.kind, ais_agent_host::inspect::PauseKind::NeedSigner);
            assert_eq!(pause.pending_signer_requests.len(), 1);
            assert_eq!(
                pause.pending_signer_requests[0].request_id.0.as_str(),
                "signer-1"
            );
        }
        other => panic!("unexpected response: {other:?}"),
    }
    assert!(stepped.events.is_empty());

    let (
        run_repo,
        checkpoint_repo,
        _mission_repo,
        _run_catalog_repo,
        _event_archive,
        _session_store,
        _signer_state_archive,
    ) = service.into_parts();
    let runtime = run_repo.load(&run_id).expect("runtime");
    assert_eq!(
        runtime.checkpoint.lifecycle.status,
        ais_agent_core::runtime::RunStatus::AwaitingSigner
    );
    assert_eq!(
        runtime
            .pending_signer_state
            .as_ref()
            .map(|state| state.status.clone()),
        Some(ais_agent_core::runtime::SignerRequestStatus::Pending)
    );
    let history = checkpoint_repo
        .history(run_id.0.as_str())
        .expect("checkpoint history");
    assert!(!history.is_empty());
    let latest = checkpoint_repo
        .latest(run_id.0.as_str())
        .expect("latest checkpoint");
    assert_eq!(
        latest.lifecycle.status,
        ais_agent_core::runtime::RunStatus::AwaitingSigner
    );
    assert_eq!(
        latest.pending_requests.pending_signer_request_id.as_deref(),
        Some("signer-1")
    );
    assert!(latest.pending_requests.pending_confirmation_id.is_none());
}

#[tokio::test]
async fn runtime_host_service_rejects_signed_signer_decision_without_payload() {
    let (
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
        run_id,
        host_session_id,
    ) = preloaded_signer_wait_runtime();
    let mut service = RuntimeHostService::new(
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
    );

    let submit = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: None,
            command: RunCommand::SubmitSignerResolution(SubmitSignerResolutionCommand {
                command_id: CommandId("cmd-signer-signed-missing-payload".to_owned()),
                run_id,
                resolution: SignerResolutionSubmission {
                    request_id: SignerRequestId("signer-1".to_owned()),
                    kind: SignerResolutionKind::Signed,
                    tx_hash: None,
                    signed_payload: None,
                    details: BTreeMap::new(),
                },
                expected_version: None,
            }),
        })
        .await;

    match submit.response {
        HostCommandResponse::Error(error) => {
            assert_eq!(error.code, "invalid_command");
            assert!(error.message.contains("signed_payload"));
        }
        other => panic!("unexpected response: {other:?}"),
    }
}

#[tokio::test]
async fn runtime_host_service_rejects_submitted_signer_decision_without_tx_hash() {
    let (
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
        run_id,
        host_session_id,
    ) = preloaded_signer_wait_runtime();
    let mut service = RuntimeHostService::new(
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
    );

    let submit = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: None,
            command: RunCommand::SubmitSignerResolution(SubmitSignerResolutionCommand {
                command_id: CommandId("cmd-signer-submitted-missing-tx-hash".to_owned()),
                run_id,
                resolution: SignerResolutionSubmission {
                    request_id: SignerRequestId("signer-1".to_owned()),
                    kind: SignerResolutionKind::Submitted,
                    tx_hash: None,
                    signed_payload: None,
                    details: BTreeMap::new(),
                },
                expected_version: None,
            }),
        })
        .await;

    match submit.response {
        HostCommandResponse::Error(error) => {
            assert_eq!(error.code, "invalid_command");
            assert!(error.message.contains("tx_hash"));
        }
        other => panic!("unexpected response: {other:?}"),
    }
}

#[tokio::test]
async fn runtime_host_service_accepts_replacement_envelope_and_stabilizes_retry_boundary() {
    let (
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
        run_id,
        host_session_id,
    ) = preloaded_envelope_wait_runtime();
    let mut service = RuntimeHostService::new(
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
    );

    let submit = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-envelope".into()),
            command: RunCommand::SubmitEnvelope(SubmitEnvelopeCommand {
                command_id: CommandId("cmd-envelope".to_owned()),
                run_id: run_id.clone(),
                envelope: sample_envelope_submission("env.swap"),
                expected_version: None,
            }),
        })
        .await;

    match submit.response {
        HostCommandResponse::Pause(bundle) => {
            assert_eq!(bundle.run_id, run_id);
            assert_eq!(
                bundle.kind,
                ais_agent_host::inspect::PauseKind::NeedUserInput
            );
            assert_eq!(
                bundle.interruption_class,
                Some(ais_agent_control::recovery::InterruptionClass::RecoveryRetryReady)
            );
            assert_eq!(
                bundle.recovery_disposition,
                ais_agent_control::recovery::RecoveryDisposition::RetryReady
            );
            assert!(bundle.failure_context.is_none());
            assert!(bundle
                .required_actions
                .iter()
                .any(|action| action.action == "step_run"));
        }
        other => panic!("unexpected response: {other:?}"),
    }

    let (
        run_repo,
        checkpoint_repo,
        _mission_repo,
        _run_catalog_repo,
        _event_archive,
        _session_store,
        _signer_state_archive,
    ) = service.into_parts();
    let runtime = run_repo.load(&run_id).expect("runtime");
    assert!(runtime
        .checkpoint
        .pending_requests
        .pending_envelope_refs
        .is_empty());
    assert_eq!(
        runtime
            .checkpoint
            .action_graph
            .nodes
            .get("swap")
            .map(|node| node.status.clone()),
        Some(ActionNodeStatus::Ready)
    );
    let latest = checkpoint_repo
        .latest(run_id.0.as_str())
        .expect("latest checkpoint");
    assert!(latest.pending_requests.pending_envelope_refs.is_empty());
    assert_eq!(
        latest.lifecycle.status,
        ais_agent_core::runtime::RunStatus::Paused
    );
    assert_eq!(
        latest
            .lifecycle
            .interruption
            .as_ref()
            .map(|interruption| interruption.class.clone()),
        Some(ais_agent_control::recovery::InterruptionClass::RecoveryRetryReady)
    );
}

#[tokio::test]
async fn runtime_host_service_rejects_second_signer_resolution_for_same_request() {
    let (
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
        run_id,
        host_session_id,
    ) = preloaded_signer_wait_runtime();
    let mut service = RuntimeHostService::new(
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
    );

    let first_submit = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: None,
            command: RunCommand::SubmitSignerResolution(SubmitSignerResolutionCommand {
                command_id: CommandId("cmd-signer-first".to_owned()),
                run_id: run_id.clone(),
                resolution: SignerResolutionSubmission {
                    request_id: SignerRequestId("signer-1".to_owned()),
                    kind: SignerResolutionKind::Submitted,
                    tx_hash: Some("0xabc".to_owned()),
                    signed_payload: None,
                    details: BTreeMap::new(),
                },
                expected_version: None,
            }),
        })
        .await;

    match first_submit.response {
        HostCommandResponse::Pause(snapshot) => {
            assert_eq!(
                snapshot.kind,
                ais_agent_host::inspect::PauseKind::NeedConfirmation
            );
        }
        other => panic!("unexpected first response: {other:?}"),
    }

    let second_submit = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: None,
            command: RunCommand::SubmitSignerResolution(SubmitSignerResolutionCommand {
                command_id: CommandId("cmd-signer-second".to_owned()),
                run_id,
                resolution: SignerResolutionSubmission {
                    request_id: SignerRequestId("signer-1".to_owned()),
                    kind: SignerResolutionKind::Submitted,
                    tx_hash: Some("0xdef".to_owned()),
                    signed_payload: None,
                    details: BTreeMap::new(),
                },
                expected_version: None,
            }),
        })
        .await;

    match second_submit.response {
        HostCommandResponse::Error(error) => {
            assert_eq!(error.code, "signer_resolution_mismatch");
        }
        other => panic!("unexpected second response: {other:?}"),
    }
}

#[tokio::test]
async fn runtime_host_service_rejects_unexpected_replacement_envelope_ref() {
    let (
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
        run_id,
        host_session_id,
    ) = preloaded_envelope_wait_runtime();
    let mut service = RuntimeHostService::new(
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
    );

    let submit = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: Some("request-envelope-wrong".into()),
            command: RunCommand::SubmitEnvelope(SubmitEnvelopeCommand {
                command_id: CommandId("cmd-envelope-wrong".to_owned()),
                run_id,
                envelope: sample_envelope_submission("env.other"),
                expected_version: None,
            }),
        })
        .await;

    match submit.response {
        HostCommandResponse::Error(error) => {
            assert_eq!(error.code, "envelope_invalid");
        }
        other => panic!("unexpected response: {other:?}"),
    }
}

#[tokio::test]
async fn runtime_host_service_persists_side_effect_cut_for_signer_submitted_confirmation_wait() {
    let (
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
        run_id,
        host_session_id,
    ) = preloaded_signer_wait_runtime();
    let mut service = RuntimeHostService::new(
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
    );

    let submit = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-side-effect-signer".into()),
            command: RunCommand::SubmitSignerResolution(SubmitSignerResolutionCommand {
                command_id: CommandId("cmd-side-effect-signer".to_owned()),
                run_id: run_id.clone(),
                resolution: SignerResolutionSubmission {
                    request_id: SignerRequestId("signer-1".to_owned()),
                    kind: SignerResolutionKind::Submitted,
                    tx_hash: Some("0xabc".to_owned()),
                    signed_payload: None,
                    details: BTreeMap::new(),
                },
                expected_version: None,
            }),
        })
        .await;
    assert!(matches!(submit.response, HostCommandResponse::Pause(_)));

    let (
        _run_repo,
        checkpoint_repo,
        _mission_repo,
        _run_catalog_repo,
        _event_archive,
        _session_store,
        _signer_state_archive,
    ) = service.into_parts();
    let history = checkpoint_repo
        .history(run_id.0.as_str())
        .expect("checkpoint history");
    let latest = history.last().expect("latest checkpoint");
    assert_eq!(latest.kind, CheckpointArchiveKind::SideEffect);
    assert_eq!(
        latest
            .snapshot
            .pending_requests
            .pending_confirmation_id
            .as_deref(),
        Some("0xabc")
    );
}

#[tokio::test]
async fn runtime_host_service_rejects_stale_mutating_command_versions() {
    let (
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
        run_id,
        host_session_id,
    ) = preloaded_evidence_wait_runtime();
    let mut service = RuntimeHostService::new(
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
    );

    let stale = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: None,
            command: RunCommand::SubmitEvidence(SubmitEvidenceCommand {
                command_id: CommandId("cmd-stale".to_owned()),
                run_id,
                evidence: EvidenceSubmission {
                    evidence_id: "quote".to_owned(),
                    kind: EvidenceKind::RouteOrQuote,
                    source: "quote-api".to_owned(),
                    observed_at_ms: Some(10),
                    chain_scope: Some("eip155:1".to_owned()),
                    payload: json!({"amount_out":"1000"}),
                    confidence: Some(0.95),
                },
                expected_version: Some(ExpectedRuntimeVersion {
                    checkpoint_seq: Some(99),
                    plan_epoch: Some(99),
                }),
            }),
        })
        .await;

    match stale.response {
        HostCommandResponse::Error(error) => {
            assert_eq!(error.code, "stale_command_conflict");
            assert!(error.message.contains("submit_evidence"));
        }
        other => panic!("unexpected response: {other:?}"),
    }
}

#[tokio::test]
async fn runtime_host_service_rejects_repeated_mutation_with_stale_expected_version() {
    let (
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
        run_id,
        host_session_id,
    ) = preloaded_evidence_wait_runtime();
    let mut service = RuntimeHostService::new(
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
    );

    let expected_version = Some(ExpectedRuntimeVersion {
        checkpoint_seq: Some(0),
        plan_epoch: Some(0),
    });

    let first = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-repeat-evidence-1".into()),
            command: RunCommand::SubmitEvidence(SubmitEvidenceCommand {
                command_id: CommandId("cmd-repeat-evidence-1".to_owned()),
                run_id: run_id.clone(),
                evidence: EvidenceSubmission {
                    evidence_id: "quote".to_owned(),
                    kind: EvidenceKind::RouteOrQuote,
                    source: "quote-api".to_owned(),
                    observed_at_ms: Some(10),
                    chain_scope: Some("eip155:1".to_owned()),
                    payload: json!({"amount_out":"1000"}),
                    confidence: Some(0.95),
                },
                expected_version: expected_version.clone(),
            }),
        })
        .await;
    assert!(matches!(first.response, HostCommandResponse::Inspect(_)));

    let repeated = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: Some("request-repeat-evidence-2".into()),
            command: RunCommand::SubmitEvidence(SubmitEvidenceCommand {
                command_id: CommandId("cmd-repeat-evidence-2".to_owned()),
                run_id: run_id.clone(),
                evidence: EvidenceSubmission {
                    evidence_id: "quote-2".to_owned(),
                    kind: EvidenceKind::RouteOrQuote,
                    source: "quote-api".to_owned(),
                    observed_at_ms: Some(11),
                    chain_scope: Some("eip155:1".to_owned()),
                    payload: json!({"amount_out":"1001"}),
                    confidence: Some(0.95),
                },
                expected_version,
            }),
        })
        .await;

    match repeated.response {
        HostCommandResponse::Error(error) => {
            assert_eq!(error.code, "stale_command_conflict");
            assert!(error.message.contains("submit_evidence"));
        }
        other => panic!("unexpected repeated mutation response: {other:?}"),
    }
}

#[tokio::test]
async fn runtime_host_service_replays_mutating_command_by_host_request_id() {
    let (
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
        run_id,
        host_session_id,
    ) = preloaded_evidence_wait_runtime();
    let mut service = RuntimeHostService::new(
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
    );

    let command = HostCommandEnvelope {
        host_session_id: host_session_id.clone(),
        host_request_id: Some("request-step-replay".into()),
        command: RunCommand::StepRun(StepRunCommand {
            command_id: CommandId("cmd-step-replay".to_owned()),
            run_id: run_id.clone(),
            until: StepUntil::CompleteOrBoundary,
            budget: Some(StepBudget {
                max_nodes: Some(8),
                max_wall_clock_ms: None,
            }),
            expected_version: None,
        }),
    };

    let first = service.handle(command.clone()).await;
    let replay = service.handle(command).await;

    match (&first.response, &replay.response) {
        (HostCommandResponse::Pause(first_pause), HostCommandResponse::Pause(replay_pause)) => {
            assert_eq!(first_pause.run_id, replay_pause.run_id);
            assert_eq!(first_pause.summary, replay_pause.summary);
        }
        other => panic!("unexpected replay responses: {other:?}"),
    }
    assert_eq!(first.events.len(), replay.events.len());
    assert_eq!(
        first
            .events
            .iter()
            .filter(|event| matches!(
                event.event,
                ais_agent_control::events::RunEvent::Completed(_)
            ))
            .count(),
        replay
            .events
            .iter()
            .filter(|event| matches!(
                event.event,
                ais_agent_control::events::RunEvent::Completed(_)
            ))
            .count()
    );
}

#[tokio::test]
async fn runtime_host_service_does_not_replay_mutation_after_claim_handoff() {
    let (
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
        run_id,
        host_session_id,
    ) = preloaded_evidence_wait_runtime();
    let mut service = RuntimeHostService::new(
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
    );

    let first_step_command = HostCommandEnvelope {
        host_session_id: host_session_id.clone(),
        host_request_id: Some("request-step-claim-scope".into()),
        command: RunCommand::StepRun(StepRunCommand {
            command_id: CommandId("cmd-step-claim-scope".to_owned()),
            run_id: run_id.clone(),
            until: StepUntil::BudgetExhausted,
            budget: Some(StepBudget {
                max_nodes: Some(0),
                max_wall_clock_ms: Some(0),
            }),
            expected_version: None,
        }),
    };

    let first = service.handle(first_step_command.clone()).await;
    let first_claim = response_ownership(&first.response)
        .current_claim
        .clone()
        .expect("bootstrapped claim");

    let released = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-release-claim-scope".into()),
            command: RunCommand::ReleaseRunClaim(ReleaseRunClaimCommand {
                command_id: CommandId("cmd-release-claim-scope".to_owned()),
                run_id: run_id.clone(),
                claim_id: first_claim.claim_id.clone(),
                claim_epoch: first_claim.claim_epoch,
                reason: Some("handoff".to_owned()),
            }),
        })
        .await;
    assert!(response_ownership(&released.response)
        .current_claim
        .is_none());

    let claimed = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-claim-claim-scope-2".into()),
            command: RunCommand::ClaimRun(ClaimRunCommand {
                command_id: CommandId("cmd-claim-claim-scope-2".to_owned()),
                run_id: run_id.clone(),
                owner_kind: RunClaimOwnerKind::InteractiveHost,
                owner_instance_id: host_session_id.0.clone(),
                mode: RunClaimMode::ExclusiveMutation,
                requested_lease_ms: None,
                allow_supersede: false,
                expected_current_claim_id: None,
                expected_current_claim_epoch: None,
            }),
        })
        .await;
    let reacquired_claim = response_ownership(&claimed.response)
        .current_claim
        .clone()
        .expect("reacquired claim");
    assert_ne!(reacquired_claim.claim_id, first_claim.claim_id);

    let repeated = service.handle(first_step_command).await;
    match repeated.response {
        HostCommandResponse::Error(error) => {
            assert_eq!(error.code, "idempotency_conflict");
        }
        other => panic!("unexpected response: {other:?}"),
    }
}

#[tokio::test]
async fn runtime_host_service_lists_streamable_event_batches() {
    let mut service = RuntimeHostService::new(
        InMemoryRunRepository::default(),
        InMemoryCheckpointRepository::default(),
        InMemoryMissionRepository::default(),
        InMemoryRunCatalogRepository::default(),
        InMemoryEventArchive::default(),
        InMemoryHostSessionStore::default(),
    );
    let host_session_id: HostSessionId = "session-events".into();

    let begin = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: Some("request-begin-events".into()),
            command: RunCommand::BeginRun(BeginRunCommand {
                command_id: CommandId("cmd-begin-events".to_owned()),
                idempotency_key: IdempotencyKey("idem-events".to_owned()),
                mission: MissionSubmission {
                    goal: "swap".to_owned(),
                    allowed_chains: vec!["eip155:1".to_owned()],
                    constraints: BTreeMap::new(),
                    budget: None,
                    metadata: BTreeMap::new(),
                },
                launch_spec: empty_prebuilt_launch_spec(),
            }),
        })
        .await;

    let run_id = match begin.response {
        HostCommandResponse::Accepted(response) => response.run_id.expect("run id"),
        other => panic!("unexpected response: {other:?}"),
    };

    let batch = service
        .list_events(HostRunEventQuery {
            run_id,
            after_event_seq: Some(0),
            limit: Some(10),
        })
        .await
        .expect("event batch");

    assert_eq!(batch.latest_event_seq, Some(1));
    assert_eq!(batch.next_after_event_seq, Some(1));
    assert_eq!(batch.events.len(), 1);
}

#[tokio::test]
async fn runtime_host_service_supports_host_collaboration_loop_for_evidence_wait() {
    let (
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
        run_id,
        host_session_id,
    ) = preloaded_evidence_wait_runtime();
    let mut service = RuntimeHostService::new(
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
    );

    let inspect_before = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-collab-inspect-1".into()),
            command: RunCommand::InspectRun(ais_agent_control::commands::InspectRunCommand {
                command_id: CommandId("cmd-collab-inspect-1".to_owned()),
                run_id: run_id.clone(),
            }),
        })
        .await;
    match inspect_before.response {
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(snapshot.status, RunStatus::AwaitingEvidence);
            assert!(snapshot.ownership.current_claim.is_none());
        }
        other => panic!("unexpected inspect response: {other:?}"),
    }

    let initial_batch = service
        .list_events(HostRunEventQuery {
            run_id: run_id.clone(),
            after_event_seq: Some(0),
            limit: Some(10),
        })
        .await
        .expect("initial event batch");
    assert!(initial_batch.events.is_empty());

    let submit = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-collab-evidence".into()),
            command: RunCommand::SubmitEvidence(SubmitEvidenceCommand {
                command_id: CommandId("cmd-collab-evidence".to_owned()),
                run_id: run_id.clone(),
                evidence: EvidenceSubmission {
                    evidence_id: "quote".to_owned(),
                    kind: EvidenceKind::RouteOrQuote,
                    source: "quote-api".to_owned(),
                    observed_at_ms: Some(10),
                    chain_scope: Some("eip155:1".to_owned()),
                    payload: json!({"amount_out":"1000"}),
                    confidence: Some(0.95),
                },
                expected_version: None,
            }),
        })
        .await;
    match submit.response {
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(snapshot.status, RunStatus::Completed);
            assert_eq!(
                snapshot
                    .ownership
                    .current_claim
                    .as_ref()
                    .map(|claim| claim.host_session_id.as_str()),
                Some(host_session_id.0.as_str())
            );
        }
        other => panic!("unexpected evidence response: {other:?}"),
    }

    let stepped = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: Some("request-collab-step".into()),
            command: RunCommand::StepRun(StepRunCommand {
                command_id: CommandId("cmd-collab-step".to_owned()),
                run_id: run_id.clone(),
                until: StepUntil::CompleteOrBoundary,
                budget: Some(StepBudget {
                    max_nodes: Some(8),
                    max_wall_clock_ms: None,
                }),
                expected_version: None,
            }),
        })
        .await;
    match stepped.response {
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(snapshot.status, RunStatus::Completed);
        }
        other => panic!("unexpected step response: {other:?}"),
    }

    let completed_batch = service
        .list_events(HostRunEventQuery {
            run_id,
            after_event_seq: Some(0),
            limit: Some(10),
        })
        .await
        .expect("completed event batch");
    assert!(completed_batch
        .events
        .iter()
        .any(|event| matches!(event.event, RunEvent::Completed(_))));
}

#[tokio::test]
async fn runtime_host_service_supports_host_collaboration_loop_for_solana_signer_wait() {
    let (
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
        run_id,
        host_session_id,
    ) = preloaded_solana_signer_wait_runtime();
    let mut service = RuntimeHostService::new(
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
    );

    let inspect_before = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-sol-inspect-1".into()),
            command: RunCommand::InspectRun(ais_agent_control::commands::InspectRunCommand {
                command_id: CommandId("cmd-sol-inspect-1".to_owned()),
                run_id: run_id.clone(),
            }),
        })
        .await;
    match inspect_before.response {
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(snapshot.status, RunStatus::AwaitingSigner);
        }
        other => panic!("unexpected inspect response: {other:?}"),
    }

    let initial_batch = service
        .list_events(HostRunEventQuery {
            run_id: run_id.clone(),
            after_event_seq: Some(0),
            limit: Some(10),
        })
        .await
        .expect("initial solana event batch");
    assert!(initial_batch.events.is_empty());

    let submit = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-sol-signer".into()),
            command: RunCommand::SubmitSignerResolution(SubmitSignerResolutionCommand {
                command_id: CommandId("cmd-sol-signer".to_owned()),
                run_id: run_id.clone(),
                resolution: SignerResolutionSubmission {
                    request_id: SignerRequestId("solana-signer-1".to_owned()),
                    kind: SignerResolutionKind::Submitted,
                    tx_hash: Some("solana-signature-1".to_owned()),
                    signed_payload: None,
                    details: BTreeMap::new(),
                },
                expected_version: None,
            }),
        })
        .await;
    match submit.response {
        HostCommandResponse::Pause(pause) => {
            assert_eq!(
                pause.kind,
                ais_agent_host::inspect::PauseKind::NeedConfirmation
            );
            assert_eq!(pause.pending_confirmations.len(), 1);
        }
        other => panic!("unexpected signer response: {other:?}"),
    }

    let confirmation_batch = service
        .list_events(HostRunEventQuery {
            run_id,
            after_event_seq: Some(0),
            limit: Some(20),
        })
        .await
        .expect("confirmation event batch");
    assert!(confirmation_batch
        .events
        .iter()
        .any(|event| matches!(event.event, RunEvent::AwaitingConfirm(_))));
    assert!(confirmation_batch.events.iter().any(|event| matches!(
        event.event,
        RunEvent::RecoveryAudit(ref audit)
            if audit.recovery_disposition
                == Some(ais_agent_control::recovery::RecoveryDisposition::ContinueWait)
    )));
}

#[tokio::test]
async fn runtime_host_service_persists_run_catalog_latest_pointers() {
    let mut service = RuntimeHostService::new(
        InMemoryRunRepository::default(),
        InMemoryCheckpointRepository::default(),
        InMemoryMissionRepository::default(),
        InMemoryRunCatalogRepository::default(),
        InMemoryEventArchive::default(),
        InMemoryHostSessionStore::default(),
    );
    let host_session_id: HostSessionId = "session-catalog".into();

    let begin = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-begin-catalog".into()),
            command: RunCommand::BeginRun(BeginRunCommand {
                command_id: CommandId("cmd-begin-catalog".to_owned()),
                idempotency_key: IdempotencyKey("idem-catalog".to_owned()),
                mission: MissionSubmission {
                    goal: "swap".to_owned(),
                    allowed_chains: vec!["eip155:1".to_owned()],
                    constraints: BTreeMap::new(),
                    budget: None,
                    metadata: BTreeMap::new(),
                },
                launch_spec: empty_prebuilt_launch_spec(),
            }),
        })
        .await;
    let run_id = match begin.response {
        HostCommandResponse::Accepted(response) => response.run_id.expect("run id"),
        other => panic!("unexpected response: {other:?}"),
    };

    let cancel = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: Some("request-cancel-catalog".into()),
            command: RunCommand::CancelRun(CancelRunCommand {
                command_id: CommandId("cmd-cancel-catalog".to_owned()),
                run_id: run_id.clone(),
                reason: Some("cancel for catalog coverage".to_owned()),
                expected_version: None,
            }),
        })
        .await;
    assert!(matches!(cancel.response, HostCommandResponse::Inspect(_)));

    let (
        _run_repo,
        _checkpoint_repo,
        _mission_repo,
        run_catalog_repo,
        event_archive,
        _session_store,
        _signer_state_archive,
    ) = service.into_parts();
    let catalog = run_catalog_repo.load(&run_id).expect("load catalog");
    assert_eq!(
        catalog.status,
        ais_agent_core::runtime::RunStatus::Cancelled
    );
    assert_eq!(catalog.latest_checkpoint_seq, 1);
    assert_eq!(catalog.latest_event_seq, Some(1));

    let events = event_archive
        .read(crate::persistence::EventArchiveQuery {
            run_id,
            after_event_seq: Some(0),
            limit: Some(10),
        })
        .expect("read event archive");
    assert_eq!(events.latest_event_seq, Some(1));
}

#[tokio::test]
async fn runtime_host_service_keeps_catalog_pointers_consistent_with_durable_history() {
    let (
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
        run_id,
        host_session_id,
    ) = preloaded_evidence_wait_runtime();
    let mut service = RuntimeHostService::new(
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
    );

    let submit = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-catalog-evidence".into()),
            command: RunCommand::SubmitEvidence(SubmitEvidenceCommand {
                command_id: CommandId("cmd-catalog-evidence".to_owned()),
                run_id: run_id.clone(),
                evidence: EvidenceSubmission {
                    evidence_id: "quote".to_owned(),
                    kind: EvidenceKind::RouteOrQuote,
                    source: "quote-api".to_owned(),
                    observed_at_ms: Some(10),
                    chain_scope: Some("eip155:1".to_owned()),
                    payload: json!({"amount_out":"1000"}),
                    confidence: Some(0.95),
                },
                expected_version: None,
            }),
        })
        .await;
    match submit.response {
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(snapshot.status, RunStatus::Completed);
        }
        other => panic!("unexpected submit response: {other:?}"),
    }

    let stepped = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: Some("request-catalog-step".into()),
            command: RunCommand::StepRun(StepRunCommand {
                command_id: CommandId("cmd-catalog-step".to_owned()),
                run_id: run_id.clone(),
                until: StepUntil::CompleteOrBoundary,
                budget: Some(StepBudget {
                    max_nodes: Some(8),
                    max_wall_clock_ms: None,
                }),
                expected_version: None,
            }),
        })
        .await;
    assert!(matches!(stepped.response, HostCommandResponse::Inspect(_)));

    let (
        _run_repo,
        checkpoint_repo,
        _mission_repo,
        run_catalog_repo,
        event_archive,
        _session_store,
        _signer_state_archive,
    ) = service.into_parts();
    let checkpoint = checkpoint_repo
        .latest(run_id.0.as_str())
        .expect("latest checkpoint");
    let checkpoint_history = checkpoint_repo
        .history(run_id.0.as_str())
        .expect("checkpoint history");
    let catalog = run_catalog_repo.load(&run_id).expect("run catalog");
    let events = event_archive
        .read(crate::persistence::EventArchiveQuery {
            run_id: run_id.clone(),
            after_event_seq: Some(0),
            limit: Some(32),
        })
        .expect("event archive");

    assert!(checkpoint_history.len() >= 3);
    assert_eq!(catalog.latest_checkpoint_seq, checkpoint.checkpoint_seq);
    assert_eq!(catalog.latest_revision, checkpoint.checkpoint_seq);
    assert_eq!(catalog.latest_event_seq, events.latest_event_seq);
}

#[tokio::test]
async fn runtime_host_service_prefers_durable_checkpoint_after_catalog_write_failure() {
    let (
        run_repo,
        checkpoint_repo,
        mission_repo,
        _run_catalog_repo,
        event_archive,
        session_store,
        run_id,
        host_session_id,
    ) = preloaded_evidence_wait_runtime();
    let mut service = RuntimeHostService::new(
        run_repo,
        checkpoint_repo,
        mission_repo,
        FailingRunCatalogRepository::fail_on_nth_upsert(2),
        event_archive,
        session_store,
    );

    let submit = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-fail-evidence".into()),
            command: RunCommand::SubmitEvidence(SubmitEvidenceCommand {
                command_id: CommandId("cmd-fail-evidence".to_owned()),
                run_id: run_id.clone(),
                evidence: EvidenceSubmission {
                    evidence_id: "quote".to_owned(),
                    kind: EvidenceKind::RouteOrQuote,
                    source: "quote-api".to_owned(),
                    observed_at_ms: Some(10),
                    chain_scope: Some("eip155:1".to_owned()),
                    payload: json!({"amount_out":"1000"}),
                    confidence: Some(0.95),
                },
                expected_version: None,
            }),
        })
        .await;
    match submit.response {
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(snapshot.status, RunStatus::Completed);
        }
        other => panic!("unexpected submit response: {other:?}"),
    }

    let failed = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-fail-step".into()),
            command: RunCommand::StepRun(StepRunCommand {
                command_id: CommandId("cmd-fail-step".to_owned()),
                run_id: run_id.clone(),
                until: StepUntil::CompleteOrBoundary,
                budget: Some(StepBudget {
                    max_nodes: Some(8),
                    max_wall_clock_ms: None,
                }),
                expected_version: None,
            }),
        })
        .await;

    match failed.response {
        HostCommandResponse::Error(error) => {
            assert_eq!(error.code, "run_catalog_error");
        }
        other => panic!("unexpected failure response: {other:?}"),
    }

    let replay = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-fail-step-replay".into()),
            command: RunCommand::StepRun(StepRunCommand {
                command_id: CommandId("cmd-fail-step-replay".to_owned()),
                run_id: run_id.clone(),
                until: StepUntil::CompleteOrBoundary,
                budget: Some(StepBudget {
                    max_nodes: Some(8),
                    max_wall_clock_ms: None,
                }),
                expected_version: None,
            }),
        })
        .await;
    match replay.response {
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(
                snapshot.status,
                ais_agent_host::inspect::RunStatus::Completed
            );
        }
        other => panic!("unexpected replay response: {other:?}"),
    }

    let (
        run_repo,
        checkpoint_repo,
        _mission_repo,
        run_catalog_repo,
        event_archive,
        _session_store,
        _signer_state_archive,
    ) = service.into_parts();
    let hot_runtime = run_repo.load(&run_id).expect("rehydrated hot runtime");
    assert_eq!(
        hot_runtime.checkpoint.lifecycle.status,
        ais_agent_core::runtime::RunStatus::Completed
    );
    let checkpoint = checkpoint_repo
        .latest(run_id.0.as_str())
        .expect("latest checkpoint");
    assert_eq!(
        checkpoint.lifecycle.status,
        ais_agent_core::runtime::RunStatus::Completed
    );
    let catalog = run_catalog_repo.load(&run_id).expect("existing catalog");
    assert_eq!(
        catalog.status,
        ais_agent_core::runtime::RunStatus::Completed
    );
    let events = event_archive
        .read(crate::persistence::EventArchiveQuery {
            run_id,
            after_event_seq: Some(0),
            limit: Some(32),
        })
        .expect("events");
    assert!(events.latest_event_seq.is_some());
}

#[test]
fn runtime_host_service_emits_tracing_for_restore_and_commit_paths() {
    let (output, (response, catalog)) = capture_tracing_output(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
            .block_on(async {
                let (
                    _run_repo,
                    checkpoint_repo,
                    mission_repo,
                    run_catalog_repo,
                    event_archive,
                    session_store,
                    run_id,
                    host_session_id,
                ) = preloaded_evidence_wait_runtime();
                let traced_run_id = run_id.clone();
                let mut service = RuntimeHostService::new(
                    InMemoryRunRepository::default(),
                    checkpoint_repo,
                    mission_repo,
                    run_catalog_repo,
                    event_archive,
                    session_store,
                );

                let response = service
                    .handle(HostCommandEnvelope {
                        host_session_id,
                        host_request_id: Some("request-tracing-step".into()),
                        command: RunCommand::StepRun(StepRunCommand {
                            command_id: CommandId("cmd-tracing-step".to_owned()),
                            run_id,
                            until: StepUntil::NextBoundary,
                            budget: Some(StepBudget {
                                max_nodes: Some(4),
                                max_wall_clock_ms: None,
                            }),
                            expected_version: None,
                        }),
                    })
                    .await;

                let (
                    _run_repo,
                    _checkpoint_repo,
                    _mission_repo,
                    run_catalog_repo,
                    _event_archive,
                    _session_store,
                    _signer_state_archive,
                ) = service.into_parts();
                let catalog = run_catalog_repo
                    .load(&traced_run_id)
                    .expect("load catalog after traced step");

                (response, catalog)
            })
    });

    assert!(matches!(response.response, HostCommandResponse::Pause(_)));
    assert!(!output.trim().is_empty());
    assert_eq!(catalog.run_id.0, "run-1");
    assert!(catalog.latest_checkpoint_seq > 0);
    assert!(catalog.latest_revision > 0);
}

#[tokio::test]
async fn runtime_host_service_fails_closed_when_durable_signer_state_lags_hot_runtime() {
    let (
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
        run_id,
        host_session_id,
    ) = preloaded_signer_wait_runtime();
    let mut service = RuntimeHostService::new_with_signer_state_store(
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
        FailingSignerStateStore::fail_on_nth_write(1),
    );

    let failed = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-fail-signer-archive".into()),
            command: RunCommand::SubmitSignerResolution(SubmitSignerResolutionCommand {
                command_id: CommandId("cmd-fail-signer-archive".to_owned()),
                run_id: run_id.clone(),
                resolution: SignerResolutionSubmission {
                    request_id: SignerRequestId("signer-1".to_owned()),
                    kind: SignerResolutionKind::Submitted,
                    tx_hash: Some("0xabc".to_owned()),
                    signed_payload: None,
                    details: BTreeMap::new(),
                },
                expected_version: None,
            }),
        })
        .await;

    match failed.response {
        HostCommandResponse::Error(error) => assert_eq!(error.code, "wait_state_store_error"),
        other => panic!("unexpected failure response: {other:?}"),
    }

    let replay = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: Some("request-fail-signer-archive-replay".into()),
            command: RunCommand::StepRun(StepRunCommand {
                command_id: CommandId("cmd-fail-signer-archive-replay".to_owned()),
                run_id: run_id.clone(),
                until: StepUntil::CompleteOrBoundary,
                budget: Some(StepBudget {
                    max_nodes: Some(4),
                    max_wall_clock_ms: None,
                }),
                expected_version: None,
            }),
        })
        .await;
    match replay.response {
        HostCommandResponse::Error(error) => assert_eq!(error.code, "restore_error"),
        other => panic!("unexpected replay response: {other:?}"),
    }

    let (
        run_repo,
        checkpoint_repo,
        _mission_repo,
        run_catalog_repo,
        _event_archive,
        _session_store,
        _signer_state_archive,
    ) = service.into_parts_with_signer_state_store();
    let missing = run_repo
        .load(&run_id)
        .expect_err("hot runtime should be invalidated");
    assert_eq!(
        missing,
        crate::runtime::RunRepositoryError::NotFound {
            run_id: run_id.0.clone(),
        }
    );
    let checkpoint = checkpoint_repo
        .latest(run_id.0.as_str())
        .expect("latest checkpoint");
    assert_eq!(
        checkpoint
            .pending_requests
            .pending_signer_request_id
            .as_deref(),
        None
    );
    assert_eq!(
        checkpoint
            .pending_requests
            .pending_confirmation_id
            .as_deref(),
        Some("0xabc")
    );
    let catalog = run_catalog_repo
        .load(&run_id)
        .expect("catalog entry may persist before signer member failure");
    assert_eq!(catalog.latest_checkpoint_seq, checkpoint.checkpoint_seq);
    assert_eq!(
        catalog.active_boundary_kind,
        checkpoint
            .lifecycle
            .active_boundary
            .as_ref()
            .map(|boundary| boundary.kind.clone())
    );
}

#[tokio::test]
async fn runtime_host_service_prefers_durable_checkpoint_after_event_archive_write_failure() {
    let (
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        _event_archive,
        session_store,
        run_id,
        host_session_id,
    ) = preloaded_evidence_wait_runtime();
    let mut service = RuntimeHostService::new(
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        FailingEventArchive::fail_on_nth_append(1),
        session_store,
    );

    let submit = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-event-archive-evidence".into()),
            command: RunCommand::SubmitEvidence(SubmitEvidenceCommand {
                command_id: CommandId("cmd-event-archive-evidence".to_owned()),
                run_id: run_id.clone(),
                evidence: EvidenceSubmission {
                    evidence_id: "quote".to_owned(),
                    kind: EvidenceKind::RouteOrQuote,
                    source: "quote-api".to_owned(),
                    observed_at_ms: Some(10),
                    chain_scope: Some("eip155:1".to_owned()),
                    payload: json!({"amount_out":"1000"}),
                    confidence: Some(0.95),
                },
                expected_version: None,
            }),
        })
        .await;
    match submit.response {
        HostCommandResponse::Error(error) => assert_eq!(error.code, "event_archive_error"),
        other => panic!("unexpected submit response: {other:?}"),
    }

    let recovered = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-event-archive-step".into()),
            command: RunCommand::StepRun(StepRunCommand {
                command_id: CommandId("cmd-event-archive-step".to_owned()),
                run_id: run_id.clone(),
                until: StepUntil::CompleteOrBoundary,
                budget: Some(StepBudget {
                    max_nodes: Some(8),
                    max_wall_clock_ms: None,
                }),
                expected_version: None,
            }),
        })
        .await;

    match recovered.response {
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(
                snapshot.status,
                ais_agent_host::inspect::RunStatus::Completed
            );
        }
        other => panic!("unexpected recovery response: {other:?}"),
    }

    let replay = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: Some("request-event-archive-step-replay".into()),
            command: RunCommand::StepRun(StepRunCommand {
                command_id: CommandId("cmd-event-archive-step-replay".to_owned()),
                run_id: run_id.clone(),
                until: StepUntil::CompleteOrBoundary,
                budget: Some(StepBudget {
                    max_nodes: Some(8),
                    max_wall_clock_ms: None,
                }),
                expected_version: None,
            }),
        })
        .await;
    match replay.response {
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(
                snapshot.status,
                ais_agent_host::inspect::RunStatus::Completed
            );
        }
        other => panic!("unexpected replay response: {other:?}"),
    }

    let (
        run_repo,
        checkpoint_repo,
        _mission_repo,
        run_catalog_repo,
        _event_archive,
        _session_store,
        _signer_state_archive,
    ) = service.into_parts();
    let hot_runtime = run_repo.load(&run_id).expect("rehydrated hot runtime");
    assert_eq!(
        hot_runtime.checkpoint.lifecycle.status,
        ais_agent_core::runtime::RunStatus::Completed
    );
    let checkpoint = checkpoint_repo
        .latest(run_id.0.as_str())
        .expect("latest checkpoint");
    assert_eq!(
        checkpoint.lifecycle.status,
        ais_agent_core::runtime::RunStatus::Completed
    );
    let catalog = run_catalog_repo.load(&run_id).expect("run catalog");
    assert_eq!(
        catalog.status,
        ais_agent_core::runtime::RunStatus::Completed
    );
}

fn preloaded_evidence_wait_runtime() -> (
    InMemoryRunRepository,
    InMemoryCheckpointRepository,
    InMemoryMissionRepository,
    InMemoryRunCatalogRepository,
    InMemoryEventArchive,
    InMemoryHostSessionStore,
    RunId,
    HostSessionId,
) {
    let run_id = RunId("run-1".to_owned());
    let host_session_id: HostSessionId = "session-evidence".into();
    let mission = sample_mission();
    let mut checkpoint = checkpoint_with_nodes(
        vec![derive_terminal_node("derive-quote")],
        vec!["derive-quote".to_owned()],
    );
    checkpoint
        .evidence_graph
        .requirements
        .push(EvidenceRequirement {
            requirement_id: "req-1".to_owned(),
            reference: "evidence.quote".to_owned(),
            reason: "quote required".to_owned(),
            required_by_node_id: Some("derive-quote".to_owned()),
            satisfied_by_evidence_id: None,
        });
    checkpoint.pending_requests.pending_evidence_refs = vec!["evidence.quote".to_owned()];
    checkpoint
        .lifecycle
        .await_evidence("need quote", vec!["evidence.quote".to_owned()]);

    let runtime = ActiveRun::new(mission.clone(), checkpoint.clone());
    let mut run_repo = InMemoryRunRepository::default();
    run_repo.insert(runtime).expect("insert runtime");
    let mut checkpoint_repo = InMemoryCheckpointRepository::default();
    checkpoint_repo
        .append(CheckpointArchiveEntry {
            snapshot: checkpoint,
            kind: CheckpointArchiveKind::Boundary,
        })
        .expect("save checkpoint");
    let mut session_store = InMemoryHostSessionStore::default();
    session_store.link_run(HostRunLink::new(
        host_session_id.clone(),
        run_id.clone(),
        mission.goal.clone(),
        mission.allowed_chains.clone(),
    ));
    let mission_repo = preloaded_mission_repo(run_id.clone(), mission);
    let run_catalog_repo = InMemoryRunCatalogRepository::default();
    let event_archive = InMemoryEventArchive::default();

    (
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
        run_id,
        host_session_id,
    )
}

fn preloaded_continuation_wait_runtime() -> (
    InMemoryRunRepository,
    InMemoryCheckpointRepository,
    InMemoryMissionRepository,
    InMemoryRunCatalogRepository,
    InMemoryEventArchive,
    InMemoryHostSessionStore,
    RunId,
    HostSessionId,
) {
    let run_id = RunId("run-1".to_owned());
    let host_session_id: HostSessionId = "session-continuation".into();
    let mission = sample_mission();
    let mut checkpoint = checkpoint_with_nodes(Vec::new(), Vec::new());
    checkpoint.lifecycle.await_artifact_continuation(
        "waiting for continuation artifact",
        vec!["swap.tx_hash".to_owned()],
    );
    checkpoint.execution_artifact = Some(ExecutionArtifactRuntimeSnapshot {
        launch_spec: ExecutionArtifactLaunchSpec {
            protocol_package_id: "owliabot.uniswap_v3".to_owned(),
            action_key: "swap".to_owned(),
            chain_family: ExecutionChainFamily::Evm,
            allowed_chains: vec!["8453".to_owned()],
            entry_stage_id: "stage.swap".into(),
            actor: None,
            transactions: vec![ExecutionTransactionCandidate::EvmTransaction(
                EvmTransactionCandidate {
                    candidate_id: "swap.direct".into(),
                    to: "0x1111111111111111111111111111111111111111".to_owned(),
                    value: Some("0".to_owned()),
                    calldata: Some("0xdeadbeef".to_owned()),
                },
            )],
            stages: vec![
                ExecutionStage::Transaction(TransactionStage {
                    stage_id: "stage.swap".into(),
                    candidate_ref: "swap.direct".into(),
                    exports: vec![OutputExportSpec {
                        output_key: "swap.tx_hash".into(),
                        source: ValueRef::Ref {
                            reference: "refs.receipts.stage.swap.tx_hash".to_owned(),
                        },
                    }],
                    next_stage_id: Some("stage.continue".into()),
                }),
                ExecutionStage::Continuation(ContinuationStage {
                    stage_id: "stage.continue".into(),
                    required_outputs: vec!["swap.tx_hash".into()],
                    package_entry: "build_aave_supply_from_swap_output".into(),
                    next_stage_id: None,
                }),
            ],
            observations: Vec::new(),
            preconditions: Vec::new(),
            postconditions: Vec::new(),
            expected_effects: Vec::new(),
            execution_policy: None,
            risk_class: None,
            risk_tags: Vec::new(),
            decoded_intent: None,
            candidate_envelopes: Vec::new(),
            decode_spec: None,
            validation_plan: None,
            evidence: json!({}),
            metadata: BTreeMap::new(),
        },
        active_stage_id: Some("stage.continue".into()),
        planned_stage_graphs: BTreeMap::new(),
        exported_outputs: BTreeMap::from([("swap.tx_hash".into(), json!("0xabc"))]),
        branch_trace: Vec::new(),
        awaiting_continuation: Some(ArtifactContinuationSnapshot {
            stage_id: "stage.continue".into(),
            required_outputs: vec!["swap.tx_hash".into()],
            package_entry: "build_aave_supply_from_swap_output".into(),
            next_stage_id: None,
        }),
    });

    let runtime = ActiveRun::new(mission.clone(), checkpoint.clone());
    let mut run_repo = InMemoryRunRepository::default();
    run_repo.insert(runtime).expect("insert runtime");
    let mut checkpoint_repo = InMemoryCheckpointRepository::default();
    checkpoint_repo
        .append(CheckpointArchiveEntry {
            snapshot: checkpoint,
            kind: CheckpointArchiveKind::Boundary,
        })
        .expect("save checkpoint");
    let mut session_store = InMemoryHostSessionStore::default();
    session_store.link_run(HostRunLink::new(
        host_session_id.clone(),
        run_id.clone(),
        mission.goal.clone(),
        mission.allowed_chains.clone(),
    ));
    let mission_repo = preloaded_mission_repo(run_id.clone(), mission);
    let run_catalog_repo = InMemoryRunCatalogRepository::default();
    let event_archive = InMemoryEventArchive::default();

    (
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
        run_id,
        host_session_id,
    )
}

fn preloaded_signer_wait_runtime() -> (
    InMemoryRunRepository,
    InMemoryCheckpointRepository,
    InMemoryMissionRepository,
    InMemoryRunCatalogRepository,
    InMemoryEventArchive,
    InMemoryHostSessionStore,
    RunId,
    HostSessionId,
) {
    let run_id = RunId("run-1".to_owned());
    let host_session_id: HostSessionId = "session-signer".into();
    let mission = sample_mission();
    let mut checkpoint = checkpoint_with_nodes(
        vec![
            actuate_blocked_node("swap", vec![]),
            verify_terminal_node("verify-swap", vec!["swap"]),
        ],
        vec!["verify-swap".to_owned()],
    );
    checkpoint.pending_requests.pending_signer_request_id = Some("signer-1".to_owned());
    checkpoint
        .lifecycle
        .await_signer("await signer", SignerRequestId("signer-1".to_owned()));

    let mut runtime = ActiveRun::new(mission.clone(), checkpoint.clone());
    runtime.pending_signer_state = Some(
        SignerRequestState::new_pending(
            SignerRequestId("signer-1".to_owned()),
            run_id.clone(),
            "eip155:1",
            "sign swap",
        )
        .with_node_id("swap"),
    );

    let mut run_repo = InMemoryRunRepository::default();
    run_repo.insert(runtime).expect("insert runtime");
    let mut checkpoint_repo = InMemoryCheckpointRepository::default();
    checkpoint_repo
        .append(CheckpointArchiveEntry {
            snapshot: checkpoint,
            kind: CheckpointArchiveKind::Boundary,
        })
        .expect("save checkpoint");
    let mut session_store = InMemoryHostSessionStore::default();
    session_store.link_run(HostRunLink::new(
        host_session_id.clone(),
        run_id.clone(),
        mission.goal.clone(),
        mission.allowed_chains.clone(),
    ));
    let mission_repo = preloaded_mission_repo(run_id.clone(), mission);
    let run_catalog_repo = InMemoryRunCatalogRepository::default();
    let event_archive = InMemoryEventArchive::default();

    (
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
        run_id,
        host_session_id,
    )
}

fn preloaded_solana_signer_wait_runtime() -> (
    InMemoryRunRepository,
    InMemoryCheckpointRepository,
    InMemoryMissionRepository,
    InMemoryRunCatalogRepository,
    InMemoryEventArchive,
    InMemoryHostSessionStore,
    RunId,
    HostSessionId,
) {
    let run_id = RunId("run-1".to_owned());
    let host_session_id: HostSessionId = "session-signer-solana".into();
    let mission = sample_solana_mission();
    let mut checkpoint = checkpoint_with_nodes(
        vec![
            actuate_solana_blocked_node("swap-sol", vec![]),
            verify_solana_terminal_node("verify-swap-sol", vec!["swap-sol"]),
        ],
        vec!["verify-swap-sol".to_owned()],
    );
    checkpoint.pending_requests.pending_signer_request_id = Some("solana-signer-1".to_owned());
    checkpoint.lifecycle.await_signer(
        "await signer",
        SignerRequestId("solana-signer-1".to_owned()),
    );

    let mut runtime = ActiveRun::new(mission.clone(), checkpoint.clone());
    runtime.pending_signer_state = Some(
        SignerRequestState::new_pending(
            SignerRequestId("solana-signer-1".to_owned()),
            run_id.clone(),
            "solana:mainnet",
            "sign solana swap",
        )
        .with_node_id("swap-sol"),
    );

    let mut run_repo = InMemoryRunRepository::default();
    run_repo.insert(runtime).expect("insert runtime");
    let mut checkpoint_repo = InMemoryCheckpointRepository::default();
    checkpoint_repo
        .append(CheckpointArchiveEntry {
            snapshot: checkpoint,
            kind: CheckpointArchiveKind::Boundary,
        })
        .expect("save checkpoint");
    let mut session_store = InMemoryHostSessionStore::default();
    session_store.link_run(HostRunLink::new(
        host_session_id.clone(),
        run_id.clone(),
        mission.goal.clone(),
        mission.allowed_chains.clone(),
    ));
    let mission_repo = preloaded_mission_repo(run_id.clone(), mission);
    let run_catalog_repo = InMemoryRunCatalogRepository::default();
    let event_archive = InMemoryEventArchive::default();

    (
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
        run_id,
        host_session_id,
    )
}

fn preloaded_mission_repo(run_id: RunId, mission: Mission) -> InMemoryMissionRepository {
    let mut mission_repo = InMemoryMissionRepository::default();
    mission_repo
        .insert(run_id, mission)
        .expect("insert mission");
    mission_repo
}

#[derive(Debug)]
struct FailingRunCatalogRepository {
    inner: InMemoryRunCatalogRepository,
    upsert_calls: usize,
    fail_on_nth_upsert: usize,
}

#[derive(Debug)]
struct FailingMissionRepository {
    inner: InMemoryMissionRepository,
    writes: usize,
    fail_on_nth_insert: Option<usize>,
    fail_on_nth_upsert: Option<usize>,
}

#[derive(Debug)]
struct FailingCheckpointRepository {
    inner: InMemoryCheckpointRepository,
    appends: usize,
    fail_on_nth_append: usize,
}

#[derive(Debug, Default)]
struct FailingSignerStateStore {
    inner: InMemorySignerStateStore,
    writes: usize,
    fail_on_nth_write: usize,
}

impl FailingSignerStateStore {
    fn fail_on_nth_write(fail_on_nth_write: usize) -> Self {
        Self {
            inner: InMemorySignerStateStore::default(),
            writes: 0,
            fail_on_nth_write,
        }
    }
}

impl FailingMissionRepository {
    fn fail_on_nth_insert(fail_on_nth_insert: usize) -> Self {
        Self {
            inner: InMemoryMissionRepository::default(),
            writes: 0,
            fail_on_nth_insert: Some(fail_on_nth_insert),
            fail_on_nth_upsert: None,
        }
    }
}

impl MissionRepository for FailingMissionRepository {
    fn insert(&mut self, run_id: RunId, mission: Mission) -> Result<(), MissionRepositoryError> {
        self.writes = self.writes.saturating_add(1);
        if self.fail_on_nth_insert == Some(self.writes) {
            return Err(MissionRepositoryError::Storage {
                message: "injected mission repository failure".to_owned(),
            });
        }
        self.inner.insert(run_id, mission)
    }

    fn upsert(&mut self, run_id: RunId, mission: Mission) -> Result<(), MissionRepositoryError> {
        self.writes = self.writes.saturating_add(1);
        if self.fail_on_nth_upsert == Some(self.writes) {
            return Err(MissionRepositoryError::Storage {
                message: "injected mission repository failure".to_owned(),
            });
        }
        self.inner.upsert(run_id, mission)
    }

    fn load(&self, run_id: &RunId) -> Result<Mission, MissionRepositoryError> {
        self.inner.load(run_id)
    }
}

impl FailingCheckpointRepository {
    fn fail_on_nth_append(inner: InMemoryCheckpointRepository, fail_on_nth_append: usize) -> Self {
        Self {
            inner,
            appends: 0,
            fail_on_nth_append,
        }
    }
}

impl CheckpointRepository for FailingCheckpointRepository {
    fn latest(
        &self,
        run_id: &str,
    ) -> Result<CheckpointSnapshot, crate::persistence::CheckpointRepositoryError> {
        self.inner.latest(run_id)
    }

    fn append(
        &mut self,
        entry: CheckpointArchiveEntry,
    ) -> Result<(), crate::persistence::CheckpointRepositoryError> {
        self.appends = self.appends.saturating_add(1);
        if self.appends == self.fail_on_nth_append {
            return Err(crate::persistence::CheckpointRepositoryError::Storage {
                message: "injected checkpoint repository failure".to_owned(),
            });
        }
        self.inner.append(entry)
    }

    fn history(
        &self,
        run_id: &str,
    ) -> Result<Vec<CheckpointArchiveEntry>, crate::persistence::CheckpointRepositoryError> {
        self.inner.history(run_id)
    }
}

impl RunWaitStateStore for FailingSignerStateStore {
    fn upsert_wait_state(
        &mut self,
        wait_state: RunWaitStateRecord,
    ) -> Result<(), SignerStateStoreError> {
        self.writes = self.writes.saturating_add(1);
        if self.writes == self.fail_on_nth_write {
            return Err(SignerStateStoreError::Storage {
                message: "injected signer state store failure".to_owned(),
            });
        }
        self.inner.upsert_wait_state(wait_state)
    }

    fn load_wait_state(&self, run_id: &RunId) -> Result<RunWaitStateRecord, SignerStateStoreError> {
        self.inner.load_wait_state(run_id)
    }

    fn clear_wait_state(&mut self, run_id: &RunId) -> Result<(), SignerStateStoreError> {
        self.writes = self.writes.saturating_add(1);
        if self.writes == self.fail_on_nth_write {
            return Err(SignerStateStoreError::Storage {
                message: "injected signer state store failure".to_owned(),
            });
        }
        self.inner.clear_wait_state(run_id)
    }
}

#[derive(Debug, Default)]
struct FailingEventArchive {
    inner: InMemoryEventArchive,
    appends: usize,
    fail_on_nth_append: usize,
}

impl FailingEventArchive {
    fn fail_on_nth_append(fail_on_nth_append: usize) -> Self {
        Self {
            inner: InMemoryEventArchive::default(),
            appends: 0,
            fail_on_nth_append,
        }
    }
}

impl EventArchive for FailingEventArchive {
    fn append(
        &mut self,
        event: ais_agent_control::events::RunEventEnvelope,
    ) -> Result<(), EventArchiveError> {
        self.appends = self.appends.saturating_add(1);
        if self.appends == self.fail_on_nth_append {
            return Err(EventArchiveError::Storage {
                message: "injected event archive failure".to_owned(),
            });
        }
        self.inner.append(event)
    }

    fn read(
        &self,
        query: EventArchiveQuery,
    ) -> Result<crate::persistence::EventArchiveSlice, EventArchiveError> {
        self.inner.read(query)
    }
}

impl FailingRunCatalogRepository {
    fn fail_on_nth_upsert(fail_on_nth_upsert: usize) -> Self {
        Self {
            inner: InMemoryRunCatalogRepository::default(),
            upsert_calls: 0,
            fail_on_nth_upsert,
        }
    }
}

impl RunCatalogRepository for FailingRunCatalogRepository {
    fn upsert(&mut self, entry: RunCatalogEntry) -> Result<(), RunCatalogRepositoryError> {
        self.upsert_calls = self.upsert_calls.saturating_add(1);
        if self.upsert_calls == self.fail_on_nth_upsert {
            return Err(RunCatalogRepositoryError::Storage {
                message: "injected run catalog failure".to_owned(),
            });
        }
        self.inner.upsert(entry)
    }

    fn load(&self, run_id: &RunId) -> Result<RunCatalogEntry, RunCatalogRepositoryError> {
        self.inner.load(run_id)
    }
}

fn preloaded_envelope_wait_runtime() -> (
    InMemoryRunRepository,
    InMemoryCheckpointRepository,
    InMemoryMissionRepository,
    InMemoryRunCatalogRepository,
    InMemoryEventArchive,
    InMemoryHostSessionStore,
    RunId,
    HostSessionId,
) {
    let run_id = RunId("run-1".to_owned());
    let host_session_id: HostSessionId = "session-envelope".into();
    let mission = sample_mission();
    let mut checkpoint = checkpoint_with_nodes(vec![actuate_blocked_node("swap", vec![])], vec![]);
    checkpoint.pending_requests.pending_envelope_refs = vec!["env.swap".to_owned()];
    checkpoint.lifecycle.pause_with_failure(
        RunFailureStage::Broadcast,
        RunFailureCode::EnvelopeInvalid,
        "broadcast requires replacement envelope",
    );
    if let Some(boundary) = checkpoint.lifecycle.active_boundary.as_mut() {
        boundary.blocking_refs = checkpoint.pending_requests.pending_envelope_refs.clone();
    }
    if let Some(failure) = checkpoint.lifecycle.failure.as_mut() {
        failure.node_refs.push("swap".to_owned());
        failure.effect_refs.push("effect.swap".to_owned());
    }

    let runtime = ActiveRun::new(mission.clone(), checkpoint.clone());
    let mut run_repo = InMemoryRunRepository::default();
    run_repo.insert(runtime).expect("insert runtime");
    let mut checkpoint_repo = InMemoryCheckpointRepository::default();
    checkpoint_repo
        .append(CheckpointArchiveEntry {
            snapshot: checkpoint,
            kind: CheckpointArchiveKind::Boundary,
        })
        .expect("save checkpoint");
    let mut session_store = InMemoryHostSessionStore::default();
    session_store.link_run(HostRunLink::new(
        host_session_id.clone(),
        run_id.clone(),
        mission.goal.clone(),
        mission.allowed_chains.clone(),
    ));
    let mission_repo = preloaded_mission_repo(run_id.clone(), mission);
    let run_catalog_repo = InMemoryRunCatalogRepository::default();
    let event_archive = InMemoryEventArchive::default();

    (
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
        run_id,
        host_session_id,
    )
}

fn sample_envelope_submission(envelope_id: &str) -> EnvelopeSubmission {
    EnvelopeSubmission {
        envelope_id: envelope_id.to_owned(),
        kind: EnvelopeKind::EvmEnvelope,
        chain: "eip155:1".to_owned(),
        payload: json!({"raw_tx":"0x0102"}),
        expected_effect: Some(json!({
            "effect_id":"effect.swap",
            "kind":"state_transition",
            "assertions":[],
            "tolerance_hint":null
        })),
        provenance: Some("host.recovery".to_owned()),
    }
}

fn response_ownership(
    response: &HostCommandResponse,
) -> &ais_agent_control::ownership::RunOwnershipSnapshot {
    match response {
        HostCommandResponse::Inspect(snapshot) => &snapshot.ownership,
        HostCommandResponse::Pause(bundle) => &bundle.ownership,
        other => panic!("unexpected response shape: {other:?}"),
    }
}

fn sample_runtime_claim(
    run_id: &RunId,
    host_session_id: &HostSessionId,
    claim_id: &str,
    claim_epoch: u64,
    lease_expires_at_ms: Option<u64>,
) -> ais_agent_control::ownership::RunClaim {
    ais_agent_control::ownership::RunClaim {
        claim_id: ais_agent_control::ids::ClaimId(claim_id.to_owned()),
        run_id: run_id.clone(),
        host_session_id: host_session_id.0.clone(),
        owner_kind: RunClaimOwnerKind::InteractiveHost,
        owner_instance_id: host_session_id.0.clone(),
        lease_started_at_ms: 1,
        lease_expires_at_ms,
        last_renewed_at_ms: Some(1),
        claim_epoch,
        mode: RunClaimMode::ExclusiveMutation,
        status: ais_agent_control::ownership::RunClaimStatus::Active,
    }
}

fn sample_mission() -> Mission {
    Mission {
        mission_id: "mission-1".to_owned(),
        goal: "swap usdc to eth".to_owned(),
        allowed_chains: vec!["eip155:1".to_owned()],
        budget: MissionBudget {
            max_steps: Some(16),
            max_signer_requests: Some(2),
            max_wall_clock_ms: Some(60_000),
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

fn sample_solana_mission() -> Mission {
    Mission {
        mission_id: "mission-sol-1".to_owned(),
        goal: "swap usdc to sol".to_owned(),
        allowed_chains: vec!["solana:mainnet".to_owned()],
        budget: MissionBudget {
            max_steps: Some(16),
            max_signer_requests: Some(2),
            max_wall_clock_ms: Some(60_000),
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

fn checkpoint_with_nodes(nodes: Vec<ActionNode>, terminals: Vec<String>) -> CheckpointSnapshot {
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
            terminals,
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

fn confirmation_wait_checkpoint() -> CheckpointSnapshot {
    let mut checkpoint = checkpoint_with_nodes(
        vec![
            actuate_blocked_node("swap", vec![]),
            verify_terminal_node("verify-swap", vec!["swap"]),
        ],
        vec!["verify-swap".to_owned()],
    );
    checkpoint
        .lifecycle
        .await_confirmation("waiting for receipt 0xabc");
    checkpoint.lifecycle.phase = RunPhase::AwaitingHost;
    checkpoint.pending_requests.pending_confirmation_id = Some("0xabc".to_owned());
    checkpoint.last_completed_node_id = Some("swap".to_owned());
    checkpoint
}

fn derive_terminal_node(node_id: &str) -> ActionNode {
    ActionNode {
        node_id: node_id.to_owned(),
        kind: ActionNodeKind::Derive,
        origin: ActionOrigin::DriverFragment,
        status: ActionNodeStatus::Pending,
        depends_on: Vec::new(),
        inputs: Vec::new(),
        evidence_refs: vec!["evidence.quote".to_owned()],
        payload: ActionPayload::Derive(DeriveAction {
            derive_kind: DeriveKind::Parameter,
            derivation_hint: "derive quote".to_owned(),
            output_key: Some("quote".to_owned()),
        }),
        implementation_hint: None,
        expected_effect_ref: None,
    }
}

fn actuate_blocked_node(node_id: &str, depends_on: Vec<&str>) -> ActionNode {
    ActionNode {
        node_id: node_id.to_owned(),
        kind: ActionNodeKind::Actuate,
        origin: ActionOrigin::DriverFragment,
        status: ActionNodeStatus::Blocked,
        depends_on: depends_on.into_iter().map(str::to_owned).collect(),
        inputs: Vec::new(),
        evidence_refs: Vec::new(),
        payload: ActionPayload::Actuate(ActuateAction {
            mode: ActuateMode::DriverCall,
            actuator_hint: "swap".to_owned(),
            chain: Some("eip155:1".to_owned()),
            envelope_ref: Some("env.swap".to_owned()),
            requires_effect_contract: true,
            live: None,
        }),
        implementation_hint: None,
        expected_effect_ref: Some("effect.swap".to_owned()),
    }
}

fn verify_terminal_node(node_id: &str, depends_on: Vec<&str>) -> ActionNode {
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
            verifier_hint: "effect verifier".to_owned(),
            pre_observation_ref: None,
            post_observation_ref: None,
            live: None,
        }),
        implementation_hint: None,
        expected_effect_ref: Some("effect.swap".to_owned()),
    }
}

fn actuate_solana_blocked_node(node_id: &str, depends_on: Vec<&str>) -> ActionNode {
    ActionNode {
        node_id: node_id.to_owned(),
        kind: ActionNodeKind::Actuate,
        origin: ActionOrigin::DriverFragment,
        status: ActionNodeStatus::Blocked,
        depends_on: depends_on.into_iter().map(str::to_owned).collect(),
        inputs: Vec::new(),
        evidence_refs: Vec::new(),
        payload: ActionPayload::Actuate(ActuateAction {
            mode: ActuateMode::DriverCall,
            actuator_hint: "solana swap".to_owned(),
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

fn verify_solana_terminal_node(node_id: &str, depends_on: Vec<&str>) -> ActionNode {
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
            verifier_hint: "solana effect verifier".to_owned(),
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

fn sample_uniswap_v3_swap_execution_artifact() -> ExecutionArtifactLaunchSpec {
    ExecutionArtifactLaunchSpec {
        protocol_package_id: "owliabot.uniswap_v3".to_owned(),
        action_key: "uniswap_v3_swap".to_owned(),
        chain_family: ExecutionChainFamily::Evm,
        allowed_chains: vec!["8453".to_owned()],
        entry_stage_id: "stage.swap".into(),
        actor: Some(ExecutionArtifactActor {
            sender_address_hint: Some("0x2222222222222222222222222222222222222222".to_owned()),
            recipient_address: Some("0x1111111111111111111111111111111111111111".to_owned()),
        }),
        transactions: vec![
            ExecutionTransactionCandidate::EvmTransaction(EvmTransactionCandidate {
                candidate_id: "approve.direct".into(),
                to: "0x3333333333333333333333333333333333333333".to_owned(),
                value: Some("0".to_owned()),
                calldata: Some("0x095ea7b300".to_owned()),
            }),
            ExecutionTransactionCandidate::EvmTransaction(EvmTransactionCandidate {
                candidate_id: "swap.direct".into(),
                to: "0x5555555555555555555555555555555555555555".to_owned(),
                value: Some("0".to_owned()),
                calldata: Some("0x414bf38900".to_owned()),
            }),
        ],
        stages: vec![
            ExecutionStage::Branch(BranchStage {
                stage_id: "stage.quote_freshness".into(),
                predicate: PredicateSpec::Comparison {
                    left: ValueRef::Ref {
                        reference: "refs.evidence.quote.expires_at_ms".to_owned(),
                    },
                    op: ComparisonOperator::Gte,
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
                    op: ComparisonOperator::Eq,
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
        risk_class: Some("bounded_swap".to_owned()),
        risk_tags: vec!["router_call".to_owned()],
        decoded_intent: Some(json!({
            "kind": "swap_exact_in",
            "candidate_ref": "swap.direct",
            "token_in": "0x4444444444444444444444444444444444444444",
            "token_out": "0x1111111111111111111111111111111111111111",
            "amount_in_atomic": "25000000",
            "min_amount_out_atomic": "9900000000000000",
        })),
        candidate_envelopes: vec![json!({
            "candidate_ref": "swap.direct",
            "kind": "evm_transaction",
            "source": {
                "kind": "package"
            }
        })],
        decode_spec: Some(json!({
            "kind": "abi",
            "allow": [{
                "candidate_ref": "swap.direct",
                "selector": "0x414bf389"
            }]
        })),
        validation_plan: Some(json!({
            "checks": [{
                "kind": "target_selector_match",
                "candidate_ref": "swap.direct"
            }]
        })),
        evidence: json!({
            "clock": {
                "now_ms": 1710000015000u64,
            },
            "quote": {
                "source": "uniswap.quote",
                "quoted_at_ms": 1710000000000u64,
                "expires_at_ms": 1710000030000u64,
                "route_summary": "USDC -> WETH",
                "amount_in_atomic": "25000000",
                "amount_out_atomic": "10000000000000000",
                "min_amount_out_atomic": "9900000000000000",
            },
            "router": {
                "router_address": "0x5555555555555555555555555555555555555555",
                "approval_target_address": "0x5555555555555555555555555555555555555555",
                "approval_required": false,
            },
            "deadline": {
                "deadline_unix_seconds": 1710000900u64,
            }
        }),
        metadata: BTreeMap::from([
            ("source".to_owned(), json!("skill:uniswap-v3-swap")),
            ("tool_name".to_owned(), json!("ais_run_harness")),
        ]),
    }
}

fn sample_uniswap_v3_lp_execution_artifact() -> ExecutionArtifactLaunchSpec {
    ExecutionArtifactLaunchSpec {
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
        risk_class: None,
        risk_tags: Vec::new(),
        decoded_intent: None,
        candidate_envelopes: Vec::new(),
        decode_spec: None,
        validation_plan: None,
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
            ("source".to_owned(), json!("skill:uniswap-v3-lp")),
            ("tool_name".to_owned(), json!("ais_run_harness")),
            ("builder".to_owned(), json!("buildUniswapV3LpExecutionArtifact")),
        ]),
    }
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

fn sample_native_transfer_execution_artifact() -> ExecutionArtifactLaunchSpec {
    ExecutionArtifactLaunchSpec {
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
        risk_class: None,
        risk_tags: Vec::new(),
        decoded_intent: None,
        candidate_envelopes: Vec::new(),
        decode_spec: None,
        validation_plan: None,
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
        metadata: BTreeMap::from([
            ("tool_name".to_owned(), json!("wallet_transfer")),
            (
                "builder".to_owned(),
                json!("buildNativeTransferExecutionArtifact"),
            ),
        ]),
    }
}

fn sample_erc20_transfer_execution_artifact() -> ExecutionArtifactLaunchSpec {
    ExecutionArtifactLaunchSpec {
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
        risk_class: None,
        risk_tags: Vec::new(),
        decoded_intent: None,
        candidate_envelopes: Vec::new(),
        decode_spec: None,
        validation_plan: None,
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
                "resolution_source": "wallet_transfer",
                "user_confirmed": true
            }
        }),
        metadata: BTreeMap::from([
            ("tool_name".to_owned(), json!("wallet_transfer")),
            (
                "builder".to_owned(),
                json!("buildErc20TransferExecutionArtifact"),
            ),
        ]),
    }
}

fn empty_prebuilt_launch_spec() -> Option<LaunchSpecSubmission> {
    Some(LaunchSpecSubmission::PrebuiltFragment(
        PrebuiltFragmentLaunchSpec::default(),
    ))
}
