use std::collections::BTreeMap;

use ais_agent_control::{
    commands::{
        ClaimRunCommand, EvidenceKind, EvidenceSubmission, InspectRunCommand,
        RequestCancelRunCommand, RunCommand, SignerDecisionKind, SignerDecisionSubmission,
        StepBudget, StepRunCommand, StepUntil, SubmitEvidenceCommand, SubmitPlanPatchCommand,
        SubmitSignerDecisionCommand,
    },
    events::{RunEvent, RunStarted},
    ids::{ClaimId, CommandId, EventId, RunId, SignerRequestId},
    ownership::{RunClaim, RunClaimMode, RunClaimOwnerKind, RunClaimStatus},
    patch::{PlanPatchOperation, PlanPatchSubmission, PlanPatchTarget},
    recovery::{CancelState, InterruptionClass, RecoveryDisposition, RunFailureCode},
};
use ais_agent_core::{
    action::{
        kinds::{
            actuate::{ActuateAction, ActuateMode},
            verify::{VerifyAction, VerifyKind},
        },
        ActionGraph, ActionNode, ActionNodeKind, ActionNodeStatus, ActionOrigin, ActionPayload,
    },
    actuation::{ActuationKind, ActuationRecord, ActuationStatus},
    checkpoint::{CheckpointSnapshot, PendingRequestsSnapshot},
    effect::{EffectAssertion, EffectContract, EffectContractKind},
    evidence::{EvidenceGraph, EvidenceRequirement},
    mission::{Mission, MissionBudget, MissionPolicy},
    runtime::{RunLifecycleState, RunPhase, SignerRequestState},
};
use ais_agent_host::{
    control::{HostCommandResponse, HostCommandService},
    events::{HostRunEventQuery, HostRunEventService},
    session::{
        HostCommandEnvelope, HostRunLink, HostSessionId, HostSessionStore, InMemoryHostSessionStore,
    },
};
use serde_json::json;

use crate::{
    persistence::{
        restore_active_run, CheckpointArchiveEntry, CheckpointArchiveKind, CheckpointRepository,
        EventArchive, InMemoryCheckpointRepository, InMemoryEventArchive,
        InMemoryMissionRepository, InMemoryRunCatalogRepository, InMemoryRunClaimRepository,
        InMemoryRuntimeAuditArchive, InMemorySignerStateArchive, MissionRepository,
        RunClaimRepository, SignerStateArchive,
    },
    runtime::{ActiveRun, InMemoryRunRepository, RunRepository, RunRepositoryError},
    service::RuntimeHostService,
};

#[tokio::test]
async fn inspect_after_restart_reads_from_durable_archives_without_rehydrating_hot_cache() {
    let host_session_id: HostSessionId = "session-restart-inspect".into();
    let run_id = RunId("run-1".to_owned());
    let mission = sample_mission();
    let checkpoint = completed_checkpoint();

    let run_repo = InMemoryRunRepository::default();
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
    let mut mission_repo = InMemoryMissionRepository::default();
    mission_repo
        .insert(run_id.clone(), mission)
        .expect("insert mission");

    let mut service = RuntimeHostService::new(
        run_repo,
        checkpoint_repo,
        mission_repo,
        InMemoryRunCatalogRepository::default(),
        InMemoryEventArchive::default(),
        session_store,
    );

    let inspect = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: Some("request-restart-inspect".into()),
            command: RunCommand::InspectRun(InspectRunCommand {
                command_id: CommandId("cmd-restart-inspect".to_owned()),
                run_id: run_id.clone(),
            }),
        })
        .await;

    match inspect.response {
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(
                snapshot.status,
                ais_agent_host::inspect::RunStatus::Completed
            );
        }
        other => panic!("unexpected response: {other:?}"),
    }

    let (
        run_repo,
        _checkpoint_repo,
        _mission_repo,
        _run_catalog_repo,
        _event_archive,
        _session_store,
        _signer_state_archive,
    ) = service.into_parts();
    let not_found = run_repo
        .load(&run_id)
        .expect_err("inspect should not repopulate hot cache");
    assert_eq!(
        not_found,
        RunRepositoryError::NotFound {
            run_id: run_id.0.clone(),
        }
    );
}

#[tokio::test]
async fn restart_requires_inspect_relink_before_mutation_and_relinks_operability() {
    let host_session_id: HostSessionId = "session-restart-relink".into();
    let run_id = RunId("run-1".to_owned());
    let mission = sample_mission();
    let checkpoint = evidence_wait_checkpoint();

    let mut checkpoint_repo = InMemoryCheckpointRepository::default();
    checkpoint_repo
        .append(CheckpointArchiveEntry {
            snapshot: checkpoint,
            kind: CheckpointArchiveKind::Boundary,
        })
        .expect("append checkpoint");
    let mut mission_repo = InMemoryMissionRepository::default();
    mission_repo
        .insert(run_id.clone(), mission)
        .expect("insert mission");

    let mut service = RuntimeHostService::new(
        InMemoryRunRepository::default(),
        checkpoint_repo,
        mission_repo,
        InMemoryRunCatalogRepository::default(),
        InMemoryEventArchive::default(),
        InMemoryHostSessionStore::default(),
    );

    let submit_before_relink = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-restart-relink-evidence-before".into()),
            command: RunCommand::SubmitEvidence(SubmitEvidenceCommand {
                command_id: CommandId("cmd-restart-relink-evidence-before".to_owned()),
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
    match submit_before_relink.response {
        HostCommandResponse::Error(error) => {
            assert_eq!(error.code, "session_relink_required");
        }
        other => panic!("unexpected response before relink: {other:?}"),
    }

    let inspect = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-restart-relink-inspect".into()),
            command: RunCommand::InspectRun(InspectRunCommand {
                command_id: CommandId("cmd-restart-relink-inspect".to_owned()),
                run_id: run_id.clone(),
            }),
        })
        .await;
    match inspect.response {
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(
                snapshot.status,
                ais_agent_host::inspect::RunStatus::AwaitingEvidence
            );
        }
        other => panic!("unexpected inspect response: {other:?}"),
    }

    let submit_after_relink = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-restart-relink-evidence-after".into()),
            command: RunCommand::SubmitEvidence(SubmitEvidenceCommand {
                command_id: CommandId("cmd-restart-relink-evidence-after".to_owned()),
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
    assert!(matches!(
        submit_after_relink.response,
        HostCommandResponse::Inspect(_)
    ));

    let stepped = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: Some("request-restart-relink-step".into()),
            command: RunCommand::StepRun(StepRunCommand {
                command_id: CommandId("cmd-restart-relink-step".to_owned()),
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
            assert_eq!(
                snapshot.status,
                ais_agent_host::inspect::RunStatus::Completed
            );
        }
        other => panic!("unexpected step response after relink: {other:?}"),
    }
}

#[tokio::test]
async fn restart_preserves_active_claim_truth_and_allows_same_owner_mutation_after_inspect() {
    let host_session_id: HostSessionId = "session-restart-claim-active".into();
    let run_id = RunId("run-1".to_owned());
    let mission = sample_mission();
    let checkpoint = evidence_wait_checkpoint();

    let mut checkpoint_repo = InMemoryCheckpointRepository::default();
    checkpoint_repo
        .append(CheckpointArchiveEntry {
            snapshot: checkpoint,
            kind: CheckpointArchiveKind::Boundary,
        })
        .expect("append checkpoint");
    let mut mission_repo = InMemoryMissionRepository::default();
    mission_repo
        .insert(run_id.clone(), mission)
        .expect("insert mission");
    let mut claim_repo = InMemoryRunClaimRepository::default();
    claim_repo
        .acquire(sample_runtime_claim(
            &run_id,
            &host_session_id,
            "claim-active-1",
            RunClaimStatus::Active,
            Some(u64::MAX / 2),
            1,
        ))
        .expect("acquire claim");

    let mut service = RuntimeHostService::new_with_archives_and_claim_repo(
        InMemoryRunRepository::default(),
        checkpoint_repo,
        mission_repo,
        InMemoryRunCatalogRepository::default(),
        InMemoryEventArchive::default(),
        InMemoryHostSessionStore::default(),
        InMemorySignerStateArchive::default(),
        InMemoryRuntimeAuditArchive::default(),
        claim_repo,
    );

    let inspect = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-restart-claim-active-inspect".into()),
            command: RunCommand::InspectRun(InspectRunCommand {
                command_id: CommandId("cmd-restart-claim-active-inspect".to_owned()),
                run_id: run_id.clone(),
            }),
        })
        .await;
    match inspect.response {
        HostCommandResponse::Inspect(snapshot) => {
            let current_claim = snapshot
                .ownership
                .current_claim
                .expect("active claim in inspect");
            assert_eq!(current_claim.claim_id.0, "claim-active-1");
            assert_eq!(current_claim.status, RunClaimStatus::Active);
        }
        other => panic!("unexpected inspect response: {other:?}"),
    }

    let submit = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: Some("request-restart-claim-active-evidence".into()),
            command: RunCommand::SubmitEvidence(SubmitEvidenceCommand {
                command_id: CommandId("cmd-restart-claim-active-evidence".to_owned()),
                run_id,
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
    assert!(matches!(submit.response, HostCommandResponse::Inspect(_)));
}

#[tokio::test]
async fn restart_requires_reacquire_for_expired_claim_and_blocks_released_claim_mutation() {
    let host_session_id: HostSessionId = "session-restart-claim-history".into();
    let run_id = RunId("run-1".to_owned());
    let mission = sample_mission();
    let checkpoint = evidence_wait_checkpoint();

    let mut checkpoint_repo = InMemoryCheckpointRepository::default();
    checkpoint_repo
        .append(CheckpointArchiveEntry {
            snapshot: checkpoint,
            kind: CheckpointArchiveKind::Boundary,
        })
        .expect("append checkpoint");
    let mut mission_repo = InMemoryMissionRepository::default();
    mission_repo
        .insert(run_id.clone(), mission)
        .expect("insert mission");
    let mut claim_repo = InMemoryRunClaimRepository::default();
    claim_repo
        .acquire(sample_runtime_claim(
            &run_id,
            &host_session_id,
            "claim-history-1",
            RunClaimStatus::Active,
            Some(1),
            1,
        ))
        .expect("acquire expiring claim");

    let mut expired_service = RuntimeHostService::new_with_archives_and_claim_repo(
        InMemoryRunRepository::default(),
        checkpoint_repo,
        mission_repo,
        InMemoryRunCatalogRepository::default(),
        InMemoryEventArchive::default(),
        InMemoryHostSessionStore::default(),
        InMemorySignerStateArchive::default(),
        InMemoryRuntimeAuditArchive::default(),
        claim_repo,
    );

    let inspect = expired_service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-restart-claim-expired-inspect".into()),
            command: RunCommand::InspectRun(InspectRunCommand {
                command_id: CommandId("cmd-restart-claim-expired-inspect".to_owned()),
                run_id: run_id.clone(),
            }),
        })
        .await;
    match inspect.response {
        HostCommandResponse::Inspect(snapshot) => {
            let current_claim = snapshot
                .ownership
                .current_claim
                .expect("expired claim in inspect");
            assert_eq!(current_claim.claim_id.0, "claim-history-1");
            assert_eq!(current_claim.status, RunClaimStatus::Expired);
            assert_eq!(
                snapshot.ownership.last_terminal_claim_id,
                Some(ClaimId("claim-history-1".to_owned()))
            );
        }
        other => panic!("unexpected inspect response: {other:?}"),
    }

    let submit_expired = expired_service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-restart-claim-expired-evidence".into()),
            command: RunCommand::SubmitEvidence(SubmitEvidenceCommand {
                command_id: CommandId("cmd-restart-claim-expired-evidence".to_owned()),
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
    match submit_expired.response {
        HostCommandResponse::Error(error) => assert_eq!(error.code, "claim_expired"),
        other => panic!("unexpected expired-claim response: {other:?}"),
    }

    let reacquired = expired_service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-restart-claim-expired-reacquire".into()),
            command: RunCommand::ClaimRun(ClaimRunCommand {
                command_id: CommandId("cmd-restart-claim-expired-reacquire".to_owned()),
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
    match reacquired.response {
        HostCommandResponse::Inspect(snapshot) => {
            let current_claim = snapshot.ownership.current_claim.expect("reacquired claim");
            assert_ne!(current_claim.claim_id.0, "claim-history-1");
            assert_eq!(current_claim.status, RunClaimStatus::Active);
        }
        other => panic!("unexpected reacquire response: {other:?}"),
    }

    let (
        _run_repo,
        checkpoint_repo,
        mission_repo,
        _run_catalog_repo,
        _event_archive,
        _session_store,
        signer_state_archive,
        audit_archive,
        mut claim_repo,
    ) = expired_service.into_parts_with_claim_repo();
    let current_claim = claim_repo
        .load_active(&run_id)
        .expect("active claim after reacquire")
        .expect("some active claim");
    claim_repo
        .release(crate::persistence::ClaimReleaseRequest {
            run_id: run_id.clone(),
            claim_id: current_claim.claim_id.clone(),
            claim_epoch: current_claim.claim_epoch,
        })
        .expect("release claim");

    let mut released_service = RuntimeHostService::new_with_archives_and_claim_repo(
        InMemoryRunRepository::default(),
        checkpoint_repo,
        mission_repo,
        InMemoryRunCatalogRepository::default(),
        InMemoryEventArchive::default(),
        InMemoryHostSessionStore::default(),
        signer_state_archive,
        audit_archive,
        claim_repo,
    );

    let inspect_released = released_service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-restart-claim-released-inspect".into()),
            command: RunCommand::InspectRun(InspectRunCommand {
                command_id: CommandId("cmd-restart-claim-released-inspect".to_owned()),
                run_id: run_id.clone(),
            }),
        })
        .await;
    match inspect_released.response {
        HostCommandResponse::Inspect(snapshot) => {
            assert!(snapshot.ownership.current_claim.is_none());
            assert_eq!(
                snapshot.ownership.last_claim_transition,
                Some(ais_agent_control::ownership::ClaimTransitionKind::ClaimReleased)
            );
            assert!(snapshot.ownership.last_terminal_claim_id.is_some());
        }
        other => panic!("unexpected released inspect response: {other:?}"),
    }

    let submit_released = released_service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: Some("request-restart-claim-released-evidence".into()),
            command: RunCommand::SubmitEvidence(SubmitEvidenceCommand {
                command_id: CommandId("cmd-restart-claim-released-evidence".to_owned()),
                run_id,
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
    match submit_released.response {
        HostCommandResponse::Error(error) => assert_eq!(error.code, "claim_required"),
        other => panic!("unexpected released-claim response: {other:?}"),
    }
}

#[tokio::test]
async fn event_query_after_restart_reads_from_durable_archive_and_preserves_event_sequence() {
    let host_session_id: HostSessionId = "session-restart-events".into();
    let run_id = RunId("run-1".to_owned());
    let mission = sample_mission();
    let checkpoint = verifying_after_broadcast_checkpoint();

    let run_repo = InMemoryRunRepository::default();
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
    let mut mission_repo = InMemoryMissionRepository::default();
    mission_repo
        .insert(run_id.clone(), mission)
        .expect("insert mission");
    let mut event_archive = InMemoryEventArchive::default();
    event_archive
        .append(archived_started_event(run_id.clone()))
        .expect("append archived started event");

    let mut service = RuntimeHostService::new(
        run_repo,
        checkpoint_repo,
        mission_repo,
        InMemoryRunCatalogRepository::default(),
        event_archive,
        session_store,
    );

    let initial_batch = service
        .list_events(HostRunEventQuery {
            run_id: run_id.clone(),
            after_event_seq: Some(0),
            limit: Some(10),
        })
        .await
        .expect("initial durable event batch");
    assert_eq!(initial_batch.latest_event_seq, Some(1));
    assert_eq!(initial_batch.events.len(), 1);
    assert_eq!(initial_batch.events[0].event_seq, 1);

    let stepped = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: Some("request-restart-events-step".into()),
            command: RunCommand::StepRun(StepRunCommand {
                command_id: CommandId("cmd-restart-events-step".to_owned()),
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

    let durable_batch = service
        .list_events(HostRunEventQuery {
            run_id,
            after_event_seq: Some(0),
            limit: Some(10),
        })
        .await
        .expect("durable event batch after restart step");
    let event_seqs = durable_batch
        .events
        .iter()
        .map(|event| event.event_seq)
        .collect::<Vec<_>>();
    assert_eq!(event_seqs.first().copied(), Some(1));
    assert!(event_seqs.len() > 1);
    assert_eq!(durable_batch.latest_event_seq, event_seqs.last().copied());
    assert_eq!(
        event_seqs,
        (1..=event_seqs.len() as u64).collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn restart_restores_awaiting_evidence_and_completes_through_real_host_service() {
    let host_session_id: HostSessionId = "session-restart-evidence".into();
    let run_id = RunId("run-1".to_owned());
    let mission = sample_mission();
    let checkpoint = evidence_wait_checkpoint();

    let mut run_repo = InMemoryRunRepository::default();
    run_repo
        .insert(ActiveRun::new(mission.clone(), checkpoint.clone()))
        .expect("insert runtime");
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
    let mut mission_repo = InMemoryMissionRepository::default();
    mission_repo
        .insert(run_id.clone(), mission.clone())
        .expect("insert mission");
    let service = RuntimeHostService::new(
        run_repo,
        checkpoint_repo,
        mission_repo,
        InMemoryRunCatalogRepository::default(),
        InMemoryEventArchive::default(),
        session_store,
    );

    let (
        _old_run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
        _signer_state_archive,
    ) = service.into_parts();
    let restored = restore_active_run(
        &run_id,
        &mission_repo,
        &checkpoint_repo,
        &InMemorySignerStateArchive::default(),
    )
    .expect("restore runtime");

    let mut new_run_repo = InMemoryRunRepository::default();
    new_run_repo
        .insert(restored)
        .expect("insert restored runtime");
    let mut service = RuntimeHostService::new(
        new_run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
    );

    let submit = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-restart-evidence".into()),
            command: RunCommand::SubmitEvidence(SubmitEvidenceCommand {
                command_id: CommandId("cmd-restart-evidence".to_owned()),
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
    assert!(matches!(submit.response, HostCommandResponse::Inspect(_)));

    let stepped = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: Some("request-restart-step".into()),
            command: RunCommand::StepRun(StepRunCommand {
                command_id: CommandId("cmd-restart-step".to_owned()),
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
            assert_eq!(
                snapshot.status,
                ais_agent_host::inspect::RunStatus::Completed
            );
        }
        other => panic!("unexpected response: {other:?}"),
    }
}

#[tokio::test]
async fn restart_restores_patch_wait_and_completes_after_submit_plan_patch() {
    let host_session_id: HostSessionId = "session-restart-patch".into();
    let run_id = RunId("run-1".to_owned());
    let mission = sample_mission();
    let checkpoint = patch_wait_checkpoint();
    let patch = restart_patch_submission();

    let mut checkpoint_repo = InMemoryCheckpointRepository::default();
    checkpoint_repo
        .append(CheckpointArchiveEntry {
            snapshot: checkpoint,
            kind: CheckpointArchiveKind::Boundary,
        })
        .expect("append checkpoint");
    let mut mission_repo = InMemoryMissionRepository::default();
    mission_repo
        .insert(run_id.clone(), mission)
        .expect("insert mission");

    let mut service = RuntimeHostService::new(
        InMemoryRunRepository::default(),
        checkpoint_repo,
        mission_repo,
        InMemoryRunCatalogRepository::default(),
        InMemoryEventArchive::default(),
        InMemoryHostSessionStore::default(),
    );

    let inspect = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-restart-patch-inspect".into()),
            command: RunCommand::InspectRun(InspectRunCommand {
                command_id: CommandId("cmd-restart-patch-inspect".to_owned()),
                run_id: run_id.clone(),
            }),
        })
        .await;
    match inspect.response {
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(snapshot.status, ais_agent_host::inspect::RunStatus::Paused);
            assert_eq!(
                snapshot.recovery_disposition,
                Some(ais_agent_control::recovery::RecoveryDisposition::AwaitPatch)
            );
        }
        other => panic!("unexpected patch inspect response: {other:?}"),
    }

    let patched = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-restart-patch-submit".into()),
            command: RunCommand::SubmitPlanPatch(SubmitPlanPatchCommand {
                command_id: CommandId("cmd-restart-patch-submit".to_owned()),
                run_id: run_id.clone(),
                patch,
                expected_version: Some(ais_agent_control::commands::ExpectedRuntimeVersion {
                    checkpoint_seq: Some(4),
                    plan_epoch: Some(2),
                }),
            }),
        })
        .await;
    match patched.response {
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(snapshot.status, ais_agent_host::inspect::RunStatus::Running);
            assert_eq!(
                snapshot.phase,
                ais_agent_host::inspect::RunPhase::Recovering
            );
        }
        other => panic!("unexpected patch response: {other:?}"),
    }

    let stepped = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: Some("request-restart-patch-step".into()),
            command: RunCommand::StepRun(StepRunCommand {
                command_id: CommandId("cmd-restart-patch-step".to_owned()),
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
            assert_eq!(
                snapshot.status,
                ais_agent_host::inspect::RunStatus::Completed
            );
        }
        other => panic!("unexpected step response after patch: {other:?}"),
    }
}

#[tokio::test]
async fn restart_restores_awaiting_signer_and_completes_through_real_host_service() {
    let host_session_id: HostSessionId = "session-restart-signer".into();
    let run_id = RunId("run-1".to_owned());
    let mission = sample_mission();
    let signer_state = sample_signer_state();
    let checkpoint = awaiting_signer_checkpoint(&signer_state);

    let mut run_repo = InMemoryRunRepository::default();
    let mut runtime = ActiveRun::new(mission.clone(), checkpoint.clone());
    runtime.set_pending_signer_state(Some(signer_state.clone()));
    run_repo.insert(runtime).expect("insert runtime");
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
    let mut mission_repo = InMemoryMissionRepository::default();
    mission_repo
        .insert(run_id.clone(), mission.clone())
        .expect("insert mission");
    let mut signer_state_archive = InMemorySignerStateArchive::default();
    signer_state_archive
        .upsert(signer_state)
        .expect("persist signer state");
    let service = RuntimeHostService::new_with_signer_archive(
        run_repo,
        checkpoint_repo,
        mission_repo,
        InMemoryRunCatalogRepository::default(),
        InMemoryEventArchive::default(),
        session_store,
        signer_state_archive,
    );

    let (
        _old_run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
        signer_state_archive,
    ) = service.into_parts_with_signer_archive();
    let restored = restore_active_run(
        &run_id,
        &mission_repo,
        &checkpoint_repo,
        &signer_state_archive,
    )
    .expect("restore runtime");

    let mut new_run_repo = InMemoryRunRepository::default();
    new_run_repo
        .insert(restored)
        .expect("insert restored runtime");
    let mut service = RuntimeHostService::new_with_signer_archive(
        new_run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
        signer_state_archive,
    );

    let signer = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-restart-signer".into()),
            command: RunCommand::SubmitSignerDecision(SubmitSignerDecisionCommand {
                command_id: CommandId("cmd-restart-signer".to_owned()),
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
    assert!(matches!(signer.response, HostCommandResponse::Inspect(_)));

    let stepped = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: Some("request-restart-signer-step".into()),
            command: RunCommand::StepRun(StepRunCommand {
                command_id: CommandId("cmd-restart-signer-step".to_owned()),
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
async fn restart_restores_denied_signer_state_and_clears_durable_signer_archive_after_step() {
    let host_session_id: HostSessionId = "session-restart-signer-denied".into();
    let run_id = RunId("run-1".to_owned());
    let mission = sample_mission();
    let signer_state = sample_signer_state();
    let checkpoint = awaiting_signer_checkpoint(&signer_state);

    let mut run_repo = InMemoryRunRepository::default();
    let mut runtime = ActiveRun::new(mission.clone(), checkpoint.clone());
    runtime.set_pending_signer_state(Some(signer_state.clone()));
    run_repo.insert(runtime).expect("insert runtime");
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
    let mut mission_repo = InMemoryMissionRepository::default();
    mission_repo
        .insert(run_id.clone(), mission.clone())
        .expect("insert mission");
    let mut signer_state_archive = InMemorySignerStateArchive::default();
    signer_state_archive
        .upsert(signer_state)
        .expect("persist signer state");
    let service = RuntimeHostService::new_with_signer_archive(
        run_repo,
        checkpoint_repo,
        mission_repo,
        InMemoryRunCatalogRepository::default(),
        InMemoryEventArchive::default(),
        session_store,
        signer_state_archive,
    );

    let (
        _old_run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
        signer_state_archive,
    ) = service.into_parts_with_signer_archive();
    let restored = restore_active_run(
        &run_id,
        &mission_repo,
        &checkpoint_repo,
        &signer_state_archive,
    )
    .expect("restore runtime");

    let mut new_run_repo = InMemoryRunRepository::default();
    new_run_repo
        .insert(restored)
        .expect("insert restored runtime");
    let mut service = RuntimeHostService::new_with_signer_archive(
        new_run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
        signer_state_archive,
    );

    let denied = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-restart-signer-denied".into()),
            command: RunCommand::SubmitSignerDecision(SubmitSignerDecisionCommand {
                command_id: CommandId("cmd-restart-signer-denied".to_owned()),
                run_id: run_id.clone(),
                decision: SignerDecisionSubmission {
                    request_id: SignerRequestId("signer-1".to_owned()),
                    decision: SignerDecisionKind::Denied,
                    tx_hash: None,
                    details: BTreeMap::new(),
                },
                expected_version: None,
            }),
        })
        .await;
    assert!(matches!(denied.response, HostCommandResponse::Inspect(_)));

    let (
        _old_run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
        signer_state_archive,
    ) = service.into_parts_with_signer_archive();
    let restored = restore_active_run(
        &run_id,
        &mission_repo,
        &checkpoint_repo,
        &signer_state_archive,
    )
    .expect("restore denied signer state");
    assert_eq!(
        restored
            .pending_signer_state
            .as_ref()
            .map(|state| &state.status),
        Some(&ais_agent_core::runtime::SignerRequestStatus::Denied)
    );

    let mut new_run_repo = InMemoryRunRepository::default();
    new_run_repo
        .insert(restored)
        .expect("insert restored denied runtime");
    let mut service = RuntimeHostService::new_with_signer_archive(
        new_run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
        signer_state_archive,
    );

    let stepped = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: Some("request-restart-signer-denied-step".into()),
            command: RunCommand::StepRun(StepRunCommand {
                command_id: CommandId("cmd-restart-signer-denied-step".to_owned()),
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
                ais_agent_host::inspect::PauseKind::NeedUserInput
            );
            assert_eq!(
                pause.failure_context.as_ref().map(|failure| &failure.code),
                Some(&ais_agent_control::recovery::RunFailureCode::SignerDenied)
            );
        }
        other => panic!("unexpected response: {other:?}"),
    }

    let (
        _run_repo,
        _checkpoint_repo,
        _mission_repo,
        _run_catalog_repo,
        _event_archive,
        _session_store,
        signer_state_archive,
    ) = service.into_parts_with_signer_archive();
    match signer_state_archive.load(&run_id) {
        Err(crate::persistence::SignerStateArchiveError::NotFound { run_id }) => {
            assert_eq!(run_id, "run-1")
        }
        other => panic!("unexpected signer archive state after step: {other:?}"),
    }
}

#[tokio::test]
async fn restart_restores_verifying_after_broadcast_and_finishes_verification() {
    let host_session_id: HostSessionId = "session-restart-verify".into();
    let run_id = RunId("run-1".to_owned());
    let mission = sample_mission();
    let checkpoint = verifying_after_broadcast_checkpoint();

    let mut run_repo = InMemoryRunRepository::default();
    run_repo
        .insert(ActiveRun::new(mission.clone(), checkpoint.clone()))
        .expect("insert runtime");
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
    let mut mission_repo = InMemoryMissionRepository::default();
    mission_repo
        .insert(run_id.clone(), mission.clone())
        .expect("insert mission");
    let service = RuntimeHostService::new(
        run_repo,
        checkpoint_repo,
        mission_repo,
        InMemoryRunCatalogRepository::default(),
        InMemoryEventArchive::default(),
        session_store,
    );

    let (
        _old_run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
        _signer_state_archive,
    ) = service.into_parts();
    let restored = restore_active_run(
        &run_id,
        &mission_repo,
        &checkpoint_repo,
        &InMemorySignerStateArchive::default(),
    )
    .expect("restore runtime");

    let mut new_run_repo = InMemoryRunRepository::default();
    new_run_repo
        .insert(restored)
        .expect("insert restored runtime");
    let mut service = RuntimeHostService::new(
        new_run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
    );

    let stepped = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: Some("request-restart-verify-step".into()),
            command: RunCommand::StepRun(StepRunCommand {
                command_id: CommandId("cmd-restart-verify-step".to_owned()),
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
            assert_eq!(
                snapshot.status,
                ais_agent_host::inspect::RunStatus::Completed
            );
            assert_eq!(
                snapshot.effect_status,
                Some(ais_agent_host::inspect::EffectStatusView::Satisfied)
            );
        }
        other => panic!("unexpected response: {other:?}"),
    }
}

#[tokio::test]
async fn restart_preserves_cancel_pending_confirmation_wait_truth() {
    let host_session_id: HostSessionId = "session-restart-cancel-pending".into();
    let run_id = RunId("run-1".to_owned());
    let mission = sample_mission();
    let signer_state = sample_signer_state();
    let checkpoint = awaiting_signer_checkpoint(&signer_state);

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
    let mut signer_state_archive = InMemorySignerStateArchive::default();
    signer_state_archive
        .upsert(signer_state)
        .expect("persist signer state");
    let mut service = RuntimeHostService::new_with_signer_archive(
        InMemoryRunRepository::default(),
        checkpoint_repo,
        mission_repo,
        InMemoryRunCatalogRepository::default(),
        InMemoryEventArchive::default(),
        InMemoryHostSessionStore::default(),
        signer_state_archive,
    );

    let inspect = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-restart-cancel-pending-inspect-1".into()),
            command: RunCommand::InspectRun(InspectRunCommand {
                command_id: CommandId("cmd-restart-cancel-pending-inspect-1".to_owned()),
                run_id: run_id.clone(),
            }),
        })
        .await;
    assert!(matches!(inspect.response, HostCommandResponse::Inspect(_)));

    let signer = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-restart-cancel-pending-signer".into()),
            command: RunCommand::SubmitSignerDecision(SubmitSignerDecisionCommand {
                command_id: CommandId("cmd-restart-cancel-pending-signer".to_owned()),
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
    assert!(matches!(signer.response, HostCommandResponse::Inspect(_)));

    let stepped = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-restart-cancel-pending-step".into()),
            command: RunCommand::StepRun(StepRunCommand {
                command_id: CommandId("cmd-restart-cancel-pending-step".to_owned()),
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

    let cancel = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-restart-cancel-pending-request".into()),
            command: RunCommand::RequestCancelRun(RequestCancelRunCommand {
                command_id: CommandId("cmd-restart-cancel-pending-request".to_owned()),
                run_id: run_id.clone(),
                reason: Some("cancel after submission".to_owned()),
                expected_version: None,
            }),
        })
        .await;
    assert!(matches!(cancel.response, HostCommandResponse::Pause(_)));

    let (
        _old_run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        _session_store,
        signer_state_archive,
    ) = service.into_parts_with_signer_archive();
    let mut restarted = RuntimeHostService::new_with_signer_archive(
        InMemoryRunRepository::default(),
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        InMemoryHostSessionStore::default(),
        signer_state_archive,
    );

    let inspect = restarted
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: Some("request-restart-cancel-pending-inspect-2".into()),
            command: RunCommand::InspectRun(InspectRunCommand {
                command_id: CommandId("cmd-restart-cancel-pending-inspect-2".to_owned()),
                run_id,
            }),
        })
        .await;
    match inspect.response {
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(snapshot.cancel_state, Some(CancelState::Pending));
            assert_eq!(
                snapshot.interruption_class,
                Some(InterruptionClass::HostCancelRequested)
            );
            assert_eq!(
                snapshot.recovery_disposition,
                Some(RecoveryDisposition::ContinueWait)
            );
            assert_eq!(snapshot.pending_confirmations.len(), 1);
        }
        other => panic!("unexpected response: {other:?}"),
    }
}

#[tokio::test]
async fn restart_preserves_retry_ready_confirmation_timeout_truth() {
    let host_session_id: HostSessionId = "session-restart-confirm-timeout".into();
    let run_id = RunId("run-1".to_owned());
    let mission = sample_mission();
    let checkpoint = confirmation_timeout_checkpoint();

    let mut checkpoint_repo = InMemoryCheckpointRepository::default();
    checkpoint_repo
        .append(CheckpointArchiveEntry {
            snapshot: checkpoint,
            kind: CheckpointArchiveKind::Boundary,
        })
        .expect("append checkpoint");
    let mut mission_repo = InMemoryMissionRepository::default();
    mission_repo
        .insert(run_id.clone(), mission)
        .expect("insert mission");

    let mut service = RuntimeHostService::new(
        InMemoryRunRepository::default(),
        checkpoint_repo,
        mission_repo,
        InMemoryRunCatalogRepository::default(),
        InMemoryEventArchive::default(),
        InMemoryHostSessionStore::default(),
    );

    let inspect = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: Some("request-restart-confirm-timeout-inspect".into()),
            command: RunCommand::InspectRun(InspectRunCommand {
                command_id: CommandId("cmd-restart-confirm-timeout-inspect".to_owned()),
                run_id,
            }),
        })
        .await;
    match inspect.response {
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(
                snapshot.interruption_class,
                Some(InterruptionClass::ConfirmationWaitTimeout)
            );
            assert_eq!(
                snapshot.recovery_disposition,
                Some(RecoveryDisposition::RetryReady)
            );
            assert_eq!(snapshot.pending_confirmations.len(), 1);
        }
        other => panic!("unexpected response: {other:?}"),
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

fn evidence_wait_checkpoint() -> CheckpointSnapshot {
    let mut lifecycle = RunLifecycleState::new(RunId("run-1".to_owned()), "mission-1");
    lifecycle.mark_running(RunPhase::Planning);
    lifecycle.await_evidence("need quote", vec!["evidence.quote".to_owned()]);

    let mut checkpoint = base_checkpoint(lifecycle, vec![derive_terminal_node("derive-quote")]);
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
}

fn patch_wait_checkpoint() -> CheckpointSnapshot {
    let mut lifecycle = RunLifecycleState::new(RunId("run-1".to_owned()), "mission-1");
    lifecycle.phase = RunPhase::Governing;
    lifecycle.checkpoint_seq = 4;
    lifecycle.plan_epoch = 2;
    lifecycle.pause_with_failure(
        ais_agent_control::recovery::RunFailureStage::Govern,
        RunFailureCode::GovernorDenied,
        "governor requested recovery patch",
    );

    let mut checkpoint = base_checkpoint(lifecycle, vec![failed_derive_node("derive-failed")]);
    checkpoint.checkpoint_seq = 4;
    checkpoint.plan_epoch = 2;
    if let Some(failure) = checkpoint.lifecycle.failure.as_mut() {
        failure.node_refs.push("derive-failed".to_owned());
    }
    checkpoint
}

fn awaiting_signer_checkpoint(signer_state: &SignerRequestState) -> CheckpointSnapshot {
    let mut lifecycle = RunLifecycleState::new(RunId("run-1".to_owned()), "mission-1");
    lifecycle.mark_running(RunPhase::Broadcasting);
    lifecycle.await_signer_request(signer_state);
    lifecycle.bump_checkpoint();
    lifecycle.bump_plan_epoch();

    let mut checkpoint = base_checkpoint(
        lifecycle,
        vec![
            succeeded_actuate_node("swap"),
            verify_terminal_node("verify-swap", vec!["swap"]),
        ],
    );
    checkpoint.pending_requests.pending_signer_request_id = Some(signer_state.request_id.0.clone());
    checkpoint.last_completed_node_id = Some("simulate-swap".to_owned());
    checkpoint
}

fn verifying_after_broadcast_checkpoint() -> CheckpointSnapshot {
    let mut lifecycle = RunLifecycleState::new(RunId("run-1".to_owned()), "mission-1");
    lifecycle.mark_running(RunPhase::Verifying);
    lifecycle.await_confirmation("waiting for chain receipt 0xabc");
    lifecycle.bump_checkpoint();
    lifecycle.bump_plan_epoch();

    let mut checkpoint = base_checkpoint(
        lifecycle,
        vec![
            succeeded_actuate_node("swap"),
            verify_terminal_node("verify-swap", vec!["swap"]),
        ],
    );
    checkpoint.last_completed_node_id = Some("swap".to_owned());
    checkpoint.pending_requests.pending_confirmation_id = Some("0xabc".to_owned());
    checkpoint
        .effect_contracts
        .insert("effect.swap".to_owned(), sample_effect_contract());
    checkpoint.actuation_records.push(ActuationRecord {
        record_id: "actuation-1".to_owned(),
        node_id: "swap".to_owned(),
        kind: ActuationKind::BroadcastSubmitted,
        status: ActuationStatus::Succeeded,
        chain: Some("eip155:1".to_owned()),
        tx_hash: Some("0xabc".to_owned()),
        summary: "submitted broadcast before restart".to_owned(),
    });
    checkpoint
}

fn confirmation_timeout_checkpoint() -> CheckpointSnapshot {
    let mut checkpoint = verifying_after_broadcast_checkpoint();
    checkpoint.lifecycle.failure = Some(ais_agent_control::recovery::RunFailureContext::new(
        RunFailureCode::ConfirmationTimeout,
        ais_agent_control::recovery::RunFailureStage::Confirm,
        checkpoint.lifecycle.checkpoint_seq,
        checkpoint.lifecycle.plan_epoch,
        Some(ais_agent_control::recovery::StableBoundaryKind::Confirmation),
        "confirmation lookup timed out",
    ));
    checkpoint.lifecycle.record_interruption(
        InterruptionClass::ConfirmationWaitTimeout,
        Some(ais_agent_control::recovery::RunFailureStage::Confirm),
        Some(ais_agent_control::recovery::SideEffectPhase::AwaitingConfirmation),
        "confirmation lookup timed out",
    );
    if let Some(failure) = checkpoint.lifecycle.failure.as_mut() {
        failure.confirmation_refs = vec!["0xabc".to_owned()];
        failure.node_refs.push("verify-swap".to_owned());
    }
    checkpoint
}

fn completed_checkpoint() -> CheckpointSnapshot {
    let mut lifecycle = RunLifecycleState::new(RunId("run-1".to_owned()), "mission-1");
    lifecycle.mark_running(RunPhase::Verifying);
    lifecycle.bump_checkpoint();
    lifecycle.bump_plan_epoch();
    lifecycle.complete("swap completed");

    let mut checkpoint = base_checkpoint(
        lifecycle,
        vec![
            succeeded_actuate_node("swap"),
            verify_terminal_node("verify-swap", vec!["swap"]),
        ],
    );
    checkpoint.last_completed_node_id = Some("verify-swap".to_owned());
    checkpoint
}

fn archived_started_event(run_id: RunId) -> ais_agent_control::events::RunEventEnvelope {
    ais_agent_control::events::RunEventEnvelope {
        run_id: run_id.clone(),
        event_seq: 1,
        checkpoint_seq: 0,
        plan_epoch: 0,
        event: RunEvent::Started(RunStarted {
            event_id: EventId(format!("{}:started:1", run_id.0)),
            run_id,
            phase: "mission_accepted".to_owned(),
        }),
    }
}

fn sample_effect_contract() -> EffectContract {
    EffectContract {
        effect_id: "effect.swap".to_owned(),
        kind: EffectContractKind::StateTransition,
        assertions: vec![EffectAssertion {
            expression: "receipt.status == true".to_owned(),
            description: "receipt should indicate success".to_owned(),
        }],
        tolerance_hint: Some("receipt_status".to_owned()),
    }
}

fn base_checkpoint(lifecycle: RunLifecycleState, nodes: Vec<ActionNode>) -> CheckpointSnapshot {
    let terminals = nodes
        .iter()
        .filter(|node| node.kind == ActionNodeKind::Verify || node.kind == ActionNodeKind::Derive)
        .map(|node| node.node_id.clone())
        .collect();
    CheckpointSnapshot {
        run_id: "run-1".to_owned(),
        mission_id: "mission-1".to_owned(),
        checkpoint_seq: lifecycle.checkpoint_seq,
        plan_epoch: lifecycle.plan_epoch,
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
        payload: ActionPayload::Actuate(ActuateAction {
            mode: ActuateMode::DriverCall,
            actuator_hint: "derive quote".to_owned(),
            chain: None,
            envelope_ref: None,
            requires_effect_contract: false,
            live: None,
        }),
        implementation_hint: None,
        expected_effect_ref: None,
    }
}

fn failed_derive_node(node_id: &str) -> ActionNode {
    ActionNode {
        node_id: node_id.to_owned(),
        kind: ActionNodeKind::Derive,
        origin: ActionOrigin::DriverFragment,
        status: ActionNodeStatus::Failed,
        depends_on: Vec::new(),
        inputs: Vec::new(),
        evidence_refs: Vec::new(),
        payload: ActionPayload::Actuate(ActuateAction {
            mode: ActuateMode::DriverCall,
            actuator_hint: "derive retry".to_owned(),
            chain: None,
            envelope_ref: None,
            requires_effect_contract: false,
            live: None,
        }),
        implementation_hint: None,
        expected_effect_ref: None,
    }
}

fn succeeded_actuate_node(node_id: &str) -> ActionNode {
    ActionNode {
        node_id: node_id.to_owned(),
        kind: ActionNodeKind::Actuate,
        origin: ActionOrigin::DriverFragment,
        status: ActionNodeStatus::Succeeded,
        depends_on: Vec::new(),
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
            verifier_hint: "verify final effect".to_owned(),
            pre_observation_ref: None,
            post_observation_ref: None,
            live: None,
        }),
        implementation_hint: None,
        expected_effect_ref: Some("effect.swap".to_owned()),
    }
}

fn sample_signer_state() -> SignerRequestState {
    SignerRequestState::new_pending(
        SignerRequestId("signer-1".to_owned()),
        RunId("run-1".to_owned()),
        "eip155:1",
        "sign swap",
    )
    .with_node_id("swap")
}

fn restart_patch_submission() -> PlanPatchSubmission {
    PlanPatchSubmission {
        patch_id: "patch-restart-1".to_owned(),
        run_id: RunId("run-1".to_owned()),
        basis_checkpoint_seq: 4,
        basis_plan_epoch: 2,
        reason_code: RunFailureCode::GovernorDenied,
        target: PlanPatchTarget::FailedFragment {
            node_ids: vec!["derive-failed".to_owned()],
        },
        operations: vec![PlanPatchOperation::ReplaceFragment {
            fragment: json!({
                "roots": ["derive-retry"],
                "terminals": ["derive-retry"],
                "nodes": {
                    "derive-retry": {
                        "node_id": "derive-retry",
                        "kind": "derive",
                        "origin": "driver_fragment",
                        "status": "pending",
                        "depends_on": [],
                        "inputs": [],
                        "evidence_refs": [],
                        "payload": {
                            "type": "derive",
                            "derive_kind": "parameter",
                            "derivation_hint": "recover derive",
                            "output_key": "quote"
                        },
                        "implementation_hint": null,
                        "expected_effect_ref": null
                    }
                }
            }),
            preserved_effect_refs: Vec::new(),
        }],
        expected_outcome: None,
    }
}

fn sample_runtime_claim(
    run_id: &RunId,
    host_session_id: &HostSessionId,
    claim_id: &str,
    status: RunClaimStatus,
    lease_expires_at_ms: Option<u64>,
    claim_epoch: u64,
) -> RunClaim {
    RunClaim {
        claim_id: ClaimId(claim_id.to_owned()),
        run_id: run_id.clone(),
        host_session_id: host_session_id.0.clone(),
        owner_kind: RunClaimOwnerKind::InteractiveHost,
        owner_instance_id: host_session_id.0.clone(),
        lease_started_at_ms: 1,
        lease_expires_at_ms,
        last_renewed_at_ms: Some(1),
        claim_epoch,
        mode: RunClaimMode::ExclusiveMutation,
        status,
    }
}
