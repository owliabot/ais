use std::collections::BTreeMap;

use ais_agent_control::{
    commands::{
        BeginRunCommand, CancelRunCommand, EnvelopeKind, EnvelopeSubmission, EvidenceKind,
        EvidenceSubmission, ExpectedRuntimeVersion, MissionBudgetSubmission, MissionSubmission,
        RequestCancelRunCommand, RunCommand, SignerDecisionKind, SignerDecisionSubmission,
        StepBudget, StepRunCommand, StepUntil, SubmitEnvelopeCommand, SubmitEvidenceCommand,
        SubmitSignerDecisionCommand,
    },
    events::RunEvent,
    ids::{CommandId, IdempotencyKey, RunId, SignerRequestId},
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
    checkpoint::{CheckpointSnapshot, PendingRequestsSnapshot},
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
use alloy::{providers::ProviderBuilder, transports::mock::Asserter};
use serde_json::json;

use crate::{
    persistence::{
        CheckpointArchiveEntry, CheckpointArchiveKind, CheckpointRepository, EventArchive,
        EventArchiveError, EventArchiveQuery, InMemoryCheckpointRepository, InMemoryEventArchive,
        InMemoryMissionRepository, InMemoryRunCatalogRepository, InMemorySignerStateArchive,
        MissionRepository, MissionRepositoryError, RunCatalogEntry, RunCatalogRepository,
        RunCatalogRepositoryError, SignerStateArchive, SignerStateArchiveError,
    },
    runtime::{ActiveRun, InMemoryRunRepository, RunRepository},
    service::RuntimeHostService,
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
            command: RunCommand::SubmitSignerDecision(SubmitSignerDecisionCommand {
                command_id: CommandId("cmd-signer-for-cancel".to_owned()),
                run_id: run_id.clone(),
                decision: SignerDecisionSubmission {
                    request_id: SignerRequestId("signer-1".to_owned()),
                    decision: SignerDecisionKind::Submitted,
                    tx_hash: Some("0xabc".to_owned()),
                    details: BTreeMap::new(),
                },
                expected_version: None,
            }),
        })
        .await;

    let _step = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: None,
            command: RunCommand::StepRun(StepRunCommand {
                command_id: CommandId("cmd-step-for-cancel".to_owned()),
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
            command: RunCommand::SubmitSignerDecision(SubmitSignerDecisionCommand {
                command_id: CommandId("cmd-signer".to_owned()),
                run_id: run_id.clone(),
                decision: SignerDecisionSubmission {
                    request_id: SignerRequestId("signer-1".to_owned()),
                    decision: SignerDecisionKind::Submitted,
                    tx_hash: Some("0xabc".to_owned()),
                    details: BTreeMap::new(),
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
        HostCommandResponse::Pause(pause) => {
            assert_eq!(
                pause.kind,
                ais_agent_host::inspect::PauseKind::NeedConfirmation
            );
            assert_eq!(pause.pending_confirmations.len(), 1);
        }
        other => panic!("unexpected response: {other:?}"),
    }
}

#[tokio::test]
async fn runtime_host_service_accepts_replacement_envelope_and_resumes_recovering() {
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
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(snapshot.run_id, run_id);
            assert_eq!(snapshot.status, RunStatus::Running);
            assert_eq!(
                snapshot.phase,
                ais_agent_host::inspect::RunPhase::Recovering
            );
            assert!(snapshot.failure_context.is_none());
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
            command: RunCommand::SubmitSignerDecision(SubmitSignerDecisionCommand {
                command_id: CommandId("cmd-side-effect-signer".to_owned()),
                run_id: run_id.clone(),
                decision: SignerDecisionSubmission {
                    request_id: SignerRequestId("signer-1".to_owned()),
                    decision: SignerDecisionKind::Submitted,
                    tx_hash: Some("0xabc".to_owned()),
                    details: BTreeMap::new(),
                },
                expected_version: None,
            }),
        })
        .await;
    assert!(matches!(submit.response, HostCommandResponse::Inspect(_)));

    let stepped = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: Some("request-side-effect-step".into()),
            command: RunCommand::StepRun(StepRunCommand {
                command_id: CommandId("cmd-side-effect-step".to_owned()),
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
    assert!(matches!(stepped.response, HostCommandResponse::Pause(_)));

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
            assert_eq!(snapshot.status, RunStatus::AwaitingEvidence);
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
            command: RunCommand::SubmitSignerDecision(SubmitSignerDecisionCommand {
                command_id: CommandId("cmd-sol-signer".to_owned()),
                run_id: run_id.clone(),
                decision: SignerDecisionSubmission {
                    request_id: SignerRequestId("solana-signer-1".to_owned()),
                    decision: SignerDecisionKind::Submitted,
                    tx_hash: Some("solana-signature-1".to_owned()),
                    details: BTreeMap::new(),
                },
                expected_version: None,
            }),
        })
        .await;
    match submit.response {
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(snapshot.status, RunStatus::AwaitingSigner);
        }
        other => panic!("unexpected signer response: {other:?}"),
    }

    let stepped = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: Some("request-sol-step".into()),
            command: RunCommand::StepRun(StepRunCommand {
                command_id: CommandId("cmd-sol-step".to_owned()),
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
            assert_eq!(
                pause.kind,
                ais_agent_host::inspect::PauseKind::NeedConfirmation
            );
            assert_eq!(pause.pending_confirmations.len(), 1);
        }
        other => panic!("unexpected step response: {other:?}"),
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
    assert!(matches!(submit.response, HostCommandResponse::Inspect(_)));

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
    assert!(matches!(submit.response, HostCommandResponse::Inspect(_)));

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
    let mut service = RuntimeHostService::new_with_signer_archive(
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
        FailingSignerStateArchive::fail_on_nth_write(1),
    );

    let failed = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-fail-signer-archive".into()),
            command: RunCommand::SubmitSignerDecision(SubmitSignerDecisionCommand {
                command_id: CommandId("cmd-fail-signer-archive".to_owned()),
                run_id: run_id.clone(),
                decision: SignerDecisionSubmission {
                    request_id: SignerRequestId("signer-1".to_owned()),
                    decision: SignerDecisionKind::Submitted,
                    tx_hash: Some("0xabc".to_owned()),
                    details: BTreeMap::new(),
                },
                expected_version: None,
            }),
        })
        .await;

    match failed.response {
        HostCommandResponse::Error(error) => assert_eq!(error.code, "signer_archive_error"),
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
    ) = service.into_parts_with_signer_archive();
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
        Some("signer-1")
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
    assert!(matches!(submit.response, HostCommandResponse::Inspect(_)));

    let failed = service
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

    match failed.response {
        HostCommandResponse::Error(error) => assert_eq!(error.code, "event_archive_error"),
        other => panic!("unexpected failure response: {other:?}"),
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
struct FailingSignerStateArchive {
    inner: InMemorySignerStateArchive,
    writes: usize,
    fail_on_nth_write: usize,
}

impl FailingSignerStateArchive {
    fn fail_on_nth_write(fail_on_nth_write: usize) -> Self {
        Self {
            inner: InMemorySignerStateArchive::default(),
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

impl SignerStateArchive for FailingSignerStateArchive {
    fn upsert(&mut self, signer_state: SignerRequestState) -> Result<(), SignerStateArchiveError> {
        self.writes = self.writes.saturating_add(1);
        if self.writes == self.fail_on_nth_write {
            return Err(SignerStateArchiveError::Storage {
                message: "injected signer archive failure".to_owned(),
            });
        }
        self.inner.upsert(signer_state)
    }

    fn load(&self, run_id: &RunId) -> Result<SignerRequestState, SignerStateArchiveError> {
        self.inner.load(run_id)
    }

    fn clear(&mut self, run_id: &RunId) -> Result<(), SignerStateArchiveError> {
        self.writes = self.writes.saturating_add(1);
        if self.writes == self.fail_on_nth_write {
            return Err(SignerStateArchiveError::Storage {
                message: "injected signer archive failure".to_owned(),
            });
        }
        self.inner.clear(run_id)
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
    }
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
