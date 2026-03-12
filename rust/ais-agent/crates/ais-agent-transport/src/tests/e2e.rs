use std::{
    io::Cursor,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use tower::util::ServiceExt;

use ais_agent_control::{events::RunEvent, ids::RunId};
use ais_agent_host::{
    control::{HostCommandOutcome, HostCommandResponse, HostCommandService},
    events::{HostRunEventBatch, HostRunEventQuery},
    session::InMemoryHostSessionStore,
};
use ais_agent_runtime::{runtime::InMemoryRunRepository, service::RuntimeHostService};
use ais_agent_store_sqlite::SqliteStore;

use crate::{
    http::build_http_router,
    jsonl::{JsonlInboundFrame, JsonlOutboundFrame, JsonlServer},
    tests::{
        commands::{
            cancel_command, envelope_command, evidence_command, illegal_patch_command,
            inspect_command, patch_command, request_cancel_command, sample_begin_command,
            signer_approved_command, signer_command, stale_patch_command, step_command,
        },
        runtime_host::{
            build_preloaded_envelope_wait_service, build_preloaded_evidence_wait_service,
            build_preloaded_patch_wait_service, build_preloaded_signer_wait_service,
            build_preloaded_solana_signer_wait_service, build_runtime_host_service,
        },
    },
};

#[tokio::test]
async fn jsonl_runtime_e2e_drives_begin_inspect_and_cancel() {
    let commands = [
        sample_begin_command(),
        inspect_command(&RunId("run-1".to_owned()), "request-inspect-1"),
        cancel_command(&RunId("run-1".to_owned())),
        inspect_command(&RunId("run-1".to_owned()), "request-inspect-2"),
    ];
    let input = commands
        .into_iter()
        .map(|command| {
            serde_json::to_string(&JsonlInboundFrame::Command { command }).expect("encode command")
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";

    let mut service = build_runtime_host_service();
    let mut output = Vec::new();

    JsonlServer
        .serve(Cursor::new(input), &mut output, &mut service)
        .await
        .expect("serve");

    let frames = parse_jsonl_frames(&output);
    assert!(frames.iter().any(|frame| matches!(
        frame,
        JsonlOutboundFrame::Event {
            event: ais_agent_control::events::RunEventEnvelope {
                event: RunEvent::Started(_),
                ..
            }
        }
    )));

    let responses = response_views(&frames);
    assert_eq!(responses.len(), 4);
    match responses[1] {
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(snapshot.status, ais_agent_host::inspect::RunStatus::Running);
        }
        other => panic!("unexpected inspect response: {other:?}"),
    }
    match responses[3] {
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(
                snapshot.status,
                ais_agent_host::inspect::RunStatus::Cancelled
            );
        }
        other => panic!("unexpected cancel inspect response: {other:?}"),
    }
}

#[tokio::test]
async fn jsonl_runtime_e2e_marks_confirmation_wait_cancel_as_pending() {
    let run_id = RunId("run-1".to_owned());
    let commands = [
        inspect_command(&run_id, "request-cancel-pending-inspect-1"),
        signer_command(
            &run_id,
            &ais_agent_control::ids::SignerRequestId("signer-1".to_owned()),
        ),
        step_command(&run_id, "request-cancel-pending-step"),
        request_cancel_command(&run_id),
        inspect_command(&run_id, "request-cancel-pending-inspect-2"),
    ];
    let input = commands
        .into_iter()
        .map(|command| {
            serde_json::to_string(&JsonlInboundFrame::Command { command }).expect("encode command")
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";

    let mut service = build_preloaded_signer_wait_service();
    let mut output = Vec::new();

    JsonlServer
        .serve(Cursor::new(input), &mut output, &mut service)
        .await
        .expect("serve");

    let frames = parse_jsonl_frames(&output);
    let responses = response_views(&frames);
    assert_eq!(responses.len(), 5);

    match responses[3] {
        HostCommandResponse::Pause(pause) => {
            assert_eq!(
                pause.kind,
                ais_agent_host::inspect::PauseKind::NeedConfirmation
            );
            assert_eq!(
                pause.cancel_state,
                Some(ais_agent_control::recovery::CancelState::Pending)
            );
            assert_eq!(
                pause.interruption_class,
                Some(ais_agent_control::recovery::InterruptionClass::HostCancelRequested)
            );
            assert_eq!(
                pause.recovery_disposition,
                ais_agent_control::recovery::RecoveryDisposition::ContinueWait
            );
            assert!(!pause
                .allowed_recovery_actions
                .contains(&ais_agent_control::recovery::RecoveryActionKind::CancelRun));
        }
        other => panic!("unexpected cancel-pending response: {other:?}"),
    }
    match responses[4] {
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(
                snapshot.cancel_state,
                Some(ais_agent_control::recovery::CancelState::Pending)
            );
            assert_eq!(
                snapshot.interruption_class,
                Some(ais_agent_control::recovery::InterruptionClass::HostCancelRequested)
            );
        }
        other => panic!("unexpected post-cancel inspect response: {other:?}"),
    }
}

#[tokio::test]
async fn jsonl_runtime_e2e_drives_preloaded_evidence_wait_to_completion() {
    let run_id = RunId("run-1".to_owned());
    let commands = [
        inspect_command(&run_id, "request-inspect-1"),
        evidence_command(&run_id),
        step_command(&run_id, "request-step-1"),
    ];
    let input = commands
        .into_iter()
        .map(|command| {
            serde_json::to_string(&JsonlInboundFrame::Command { command }).expect("encode command")
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";

    let mut service = build_preloaded_evidence_wait_service();
    let mut output = Vec::new();

    JsonlServer
        .serve(Cursor::new(input), &mut output, &mut service)
        .await
        .expect("serve");

    let frames = parse_jsonl_frames(&output);
    assert!(frames.iter().any(|frame| matches!(
        frame,
        JsonlOutboundFrame::Event {
            event: ais_agent_control::events::RunEventEnvelope {
                event: RunEvent::Completed(_),
                ..
            }
        }
    )));

    let responses = response_views(&frames);
    match responses[0] {
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(
                snapshot.status,
                ais_agent_host::inspect::RunStatus::AwaitingEvidence
            );
            assert_eq!(
                snapshot.recovery_disposition,
                Some(ais_agent_control::recovery::RecoveryDisposition::AwaitEvidence)
            );
            assert!(snapshot
                .allowed_recovery_actions
                .contains(&ais_agent_control::recovery::RecoveryActionKind::SubmitEvidence));
            assert_eq!(snapshot.recovery_suggestions.len(), 1);
        }
        other => panic!("unexpected initial inspect: {other:?}"),
    }
    match responses[2] {
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
        other => panic!("unexpected completion inspect: {other:?}"),
    }
}

#[tokio::test]
async fn jsonl_runtime_e2e_replays_same_mutating_request_without_advancing_twice() {
    let run_id = RunId("run-1".to_owned());
    let commands = [
        evidence_command(&run_id),
        evidence_command(&run_id),
        step_command(&run_id, "request-step-replay"),
        step_command(&run_id, "request-step-replay"),
    ];
    let input = commands
        .into_iter()
        .map(|command| {
            serde_json::to_string(&JsonlInboundFrame::Command { command }).expect("encode command")
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";

    let mut service = build_preloaded_evidence_wait_service();
    let mut output = Vec::new();

    JsonlServer
        .serve(Cursor::new(input), &mut output, &mut service)
        .await
        .expect("serve");

    let frames = parse_jsonl_frames(&output);
    assert!(frames.iter().any(|frame| matches!(
        frame,
        JsonlOutboundFrame::Event {
            event: ais_agent_control::events::RunEventEnvelope {
                event: RunEvent::Completed(_),
                ..
            }
        }
    )));
    let responses = response_views(&frames);
    assert_eq!(responses.len(), 4);

    match (responses[2], responses[3]) {
        (HostCommandResponse::Inspect(first), HostCommandResponse::Inspect(replay)) => {
            assert_eq!(first.status, ais_agent_host::inspect::RunStatus::Completed);
            assert_eq!(replay.status, ais_agent_host::inspect::RunStatus::Completed);
            assert_eq!(first.checkpoint_seq, replay.checkpoint_seq);
            assert_eq!(first.plan_epoch, replay.plan_epoch);
        }
        other => panic!("unexpected replay responses: {other:?}"),
    }
}

#[tokio::test]
async fn jsonl_runtime_e2e_can_poll_event_batches_from_real_runtime_service() {
    let input = serde_json::to_string(&JsonlInboundFrame::PollEvents {
        query: HostRunEventQuery {
            run_id: RunId("run-1".to_owned()),
            after_event_seq: Some(0),
            limit: Some(10),
        },
    })
    .expect("encode poll")
        + "\n";

    let mut service = build_preloaded_signer_wait_service();
    let mut output = Vec::new();

    JsonlServer
        .serve(Cursor::new(input), &mut output, &mut service)
        .await
        .expect("serve");

    let frames = parse_jsonl_frames(&output);
    assert_eq!(frames.len(), 1);
    match &frames[0] {
        JsonlOutboundFrame::EventBatch { batch } => {
            assert_eq!(batch.run_id.0, "run-1");
            assert_eq!(batch.events.len(), 0);
        }
        other => panic!("unexpected frame: {other:?}"),
    }
}

#[tokio::test]
async fn jsonl_runtime_e2e_can_poll_event_batches_after_sqlite_backed_restart() {
    let sqlite_path = sqlite_test_path("jsonl-events");
    let mut service = sqlite_runtime_host_service(
        &sqlite_path,
        InMemoryRunRepository::default(),
        InMemoryHostSessionStore::default(),
    );

    let begin = service.handle(sample_begin_command()).await;
    let run_id = match begin.response {
        HostCommandResponse::Accepted(response) => response.run_id.expect("run id"),
        other => panic!("unexpected begin response: {other:?}"),
    };

    let (
        _run_repo,
        _checkpoint_repo,
        _mission_repo,
        _run_catalog_repo,
        _event_archive,
        session_store,
        _signer_state_archive,
    ) = service.into_parts();
    let mut restarted = sqlite_runtime_host_service(
        &sqlite_path,
        InMemoryRunRepository::default(),
        session_store,
    );
    let input = serde_json::to_string(&JsonlInboundFrame::PollEvents {
        query: HostRunEventQuery {
            run_id: run_id.clone(),
            after_event_seq: Some(0),
            limit: Some(10),
        },
    })
    .expect("encode poll")
        + "\n";
    let mut output = Vec::new();

    JsonlServer
        .serve(Cursor::new(input), &mut output, &mut restarted)
        .await
        .expect("serve");

    let frames = parse_jsonl_frames(&output);
    assert_eq!(frames.len(), 1);
    match &frames[0] {
        JsonlOutboundFrame::EventBatch { batch } => {
            assert_eq!(batch.run_id, run_id);
            assert_eq!(batch.latest_event_seq, Some(1));
            assert_eq!(batch.events.len(), 1);
            assert!(matches!(batch.events[0].event, RunEvent::Started(_)));
        }
        other => panic!("unexpected frame: {other:?}"),
    }
}

#[tokio::test]
async fn jsonl_runtime_e2e_requires_inspect_relink_after_restart_before_mutation() {
    let run_id = RunId("run-1".to_owned());
    let commands = [
        evidence_command(&run_id),
        inspect_command(&run_id, "request-relink-inspect"),
        evidence_command(&run_id),
        step_command(&run_id, "request-relink-step"),
    ];
    let input = commands
        .into_iter()
        .map(|command| {
            serde_json::to_string(&JsonlInboundFrame::Command { command }).expect("encode command")
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";

    let service = build_preloaded_evidence_wait_service();
    let (
        _run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        _session_store,
        _signer_state_archive,
    ) = service.into_parts();
    let mut restarted = RuntimeHostService::new(
        InMemoryRunRepository::default(),
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        InMemoryHostSessionStore::default(),
    );
    let mut output = Vec::new();

    JsonlServer
        .serve(Cursor::new(input), &mut output, &mut restarted)
        .await
        .expect("serve");

    let frames = parse_jsonl_frames(&output);
    let responses = response_views(&frames);
    assert_eq!(responses.len(), 4);
    match responses[0] {
        HostCommandResponse::Error(error) => {
            assert_eq!(error.code, "session_relink_required");
        }
        other => panic!("unexpected pre-relink response: {other:?}"),
    }
    match responses[1] {
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(
                snapshot.status,
                ais_agent_host::inspect::RunStatus::AwaitingEvidence
            );
        }
        other => panic!("unexpected relink inspect response: {other:?}"),
    }
    match responses[3] {
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(
                snapshot.status,
                ais_agent_host::inspect::RunStatus::Completed
            );
        }
        other => panic!("unexpected completion response after relink: {other:?}"),
    }
}

#[tokio::test]
async fn jsonl_runtime_e2e_can_patch_and_continue_after_restart_relink() {
    let run_id = RunId("run-1".to_owned());
    let commands = [
        inspect_command(&run_id, "request-restart-patch-inspect"),
        patch_command(&run_id, "request-restart-patch-submit"),
        step_command(&run_id, "request-restart-patch-step"),
    ];
    let input = commands
        .into_iter()
        .map(|command| {
            serde_json::to_string(&JsonlInboundFrame::Command { command }).expect("encode command")
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";

    let service = build_preloaded_patch_wait_service();
    let (
        _run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        _session_store,
        _signer_state_archive,
    ) = service.into_parts();
    let mut restarted = RuntimeHostService::new(
        InMemoryRunRepository::default(),
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        InMemoryHostSessionStore::default(),
    );
    let mut output = Vec::new();

    JsonlServer
        .serve(Cursor::new(input), &mut output, &mut restarted)
        .await
        .expect("serve");

    let frames = parse_jsonl_frames(&output);
    let responses = response_views(&frames);
    assert_eq!(responses.len(), 3);
    match responses[0] {
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(snapshot.status, ais_agent_host::inspect::RunStatus::Paused);
            assert_eq!(
                snapshot.recovery_disposition,
                Some(ais_agent_control::recovery::RecoveryDisposition::AwaitPatch)
            );
        }
        other => panic!("unexpected restart patch inspect response: {other:?}"),
    }
    match responses[2] {
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(
                snapshot.status,
                ais_agent_host::inspect::RunStatus::Completed
            );
        }
        other => panic!("unexpected restart patch completion response: {other:?}"),
    }
}

#[tokio::test]
async fn jsonl_runtime_e2e_can_submit_patch_after_recovery_pause_and_continue() {
    let run_id = RunId("run-1".to_owned());
    let commands = [
        inspect_command(&run_id, "request-patch-inspect-1"),
        patch_command(&run_id, "request-patch-submit-1"),
        step_command(&run_id, "request-patch-step-1"),
    ];
    let input = commands
        .into_iter()
        .map(|command| {
            serde_json::to_string(&JsonlInboundFrame::Command { command }).expect("encode command")
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";

    let mut service = build_preloaded_patch_wait_service();
    let mut output = Vec::new();

    JsonlServer
        .serve(Cursor::new(input), &mut output, &mut service)
        .await
        .expect("serve");

    let frames = parse_jsonl_frames(&output);
    assert!(frames.iter().any(|frame| matches!(
        frame,
        JsonlOutboundFrame::Event {
            event: ais_agent_control::events::RunEventEnvelope {
                event: RunEvent::PlanPatchAudit(ref audit),
                ..
            }
        } if audit.status == ais_agent_control::events::PlanPatchAuditStatus::Submitted
    )));
    assert!(frames.iter().any(|frame| matches!(
        frame,
        JsonlOutboundFrame::Event {
            event: ais_agent_control::events::RunEventEnvelope {
                event: RunEvent::PlanPatchAudit(ref audit),
                ..
            }
        } if audit.status == ais_agent_control::events::PlanPatchAuditStatus::Applied
    )));
    let responses = response_views(&frames);
    assert_eq!(responses.len(), 3);

    match responses[0] {
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(
                snapshot.recovery_disposition,
                Some(ais_agent_control::recovery::RecoveryDisposition::AwaitPatch)
            );
            assert!(snapshot
                .allowed_recovery_actions
                .contains(&ais_agent_control::recovery::RecoveryActionKind::SubmitPlanPatch));
        }
        other => panic!("unexpected patch inspect: {other:?}"),
    }
    match responses[1] {
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(snapshot.status, ais_agent_host::inspect::RunStatus::Running);
            assert_eq!(
                snapshot.phase,
                ais_agent_host::inspect::RunPhase::Recovering
            );
        }
        other => panic!("unexpected patch submit response: {other:?}"),
    }
    match responses[2] {
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(
                snapshot.status,
                ais_agent_host::inspect::RunStatus::Completed
            );
        }
        other => panic!("unexpected patch step response: {other:?}"),
    }
}

#[tokio::test]
async fn jsonl_runtime_e2e_surfaces_stale_and_illegal_patch_rejections() {
    let run_id = RunId("run-1".to_owned());
    let input = [
        serde_json::to_string(&JsonlInboundFrame::Command {
            command: stale_patch_command(&run_id, "request-patch-stale"),
        })
        .expect("encode stale patch"),
        serde_json::to_string(&JsonlInboundFrame::Command {
            command: illegal_patch_command(&run_id, "request-patch-illegal"),
        })
        .expect("encode illegal patch"),
        serde_json::to_string(&JsonlInboundFrame::PollEvents {
            query: HostRunEventQuery {
                run_id: run_id.clone(),
                after_event_seq: Some(0),
                limit: Some(10),
            },
        })
        .expect("encode poll"),
    ]
    .join("\n")
        + "\n";

    let mut service = build_preloaded_patch_wait_service();
    let mut output = Vec::new();

    JsonlServer
        .serve(Cursor::new(input), &mut output, &mut service)
        .await
        .expect("serve");

    let frames = parse_jsonl_frames(&output);
    let responses = response_views(&frames);
    assert_eq!(responses.len(), 2);

    match responses[0] {
        HostCommandResponse::Error(error) => {
            assert_eq!(error.code, "stale_command_conflict");
        }
        other => panic!("unexpected stale patch response: {other:?}"),
    }
    match responses[1] {
        HostCommandResponse::Error(error) => {
            assert_eq!(error.code, "plan_patch_illegal");
        }
        other => panic!("unexpected illegal patch response: {other:?}"),
    }

    let batch = frames
        .iter()
        .find_map(|frame| match frame {
            JsonlOutboundFrame::EventBatch { batch } => Some(batch),
            _ => None,
        })
        .expect("rejected patch audit batch");
    let rejected_count = batch
        .events
        .iter()
        .filter(|event| {
            matches!(
                event.event,
                RunEvent::PlanPatchAudit(ref audit)
                    if audit.status == ais_agent_control::events::PlanPatchAuditStatus::Rejected
            )
        })
        .count();
    assert_eq!(rejected_count, 2);
}

#[tokio::test]
async fn http_runtime_e2e_drives_preloaded_signer_wait_to_completion() {
    let app = build_http_router(build_preloaded_signer_wait_service());
    let run_id = RunId("run-1".to_owned());

    let inspect_1 = send_http_command(&app, inspect_command(&run_id, "http-inspect-1")).await;
    let signer_request_id = match inspect_1.response {
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(
                snapshot.status,
                ais_agent_host::inspect::RunStatus::AwaitingSigner
            );
            assert_eq!(
                snapshot.recovery_disposition,
                Some(ais_agent_control::recovery::RecoveryDisposition::AwaitSigner)
            );
            assert!(snapshot
                .allowed_recovery_actions
                .contains(&ais_agent_control::recovery::RecoveryActionKind::SubmitSignerDecision));
            snapshot.pending_signer_requests[0].request_id.clone()
        }
        other => panic!("unexpected inspect response: {other:?}"),
    };

    let signer = send_http_command(&app, signer_command(&run_id, &signer_request_id)).await;
    match signer.response {
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(
                snapshot.status,
                ais_agent_host::inspect::RunStatus::AwaitingSigner
            );
        }
        other => panic!("unexpected signer submission response: {other:?}"),
    }

    let stepped = send_http_command(&app, step_command(&run_id, "http-step-1")).await;
    match stepped.response {
        HostCommandResponse::Pause(pause) => {
            assert_eq!(
                pause.kind,
                ais_agent_host::inspect::PauseKind::NeedConfirmation
            );
            assert_eq!(
                pause.recovery_disposition,
                ais_agent_control::recovery::RecoveryDisposition::ContinueWait
            );
            assert_eq!(pause.pending_confirmations.len(), 1);
            assert!(pause
                .allowed_recovery_actions
                .contains(&ais_agent_control::recovery::RecoveryActionKind::AwaitConfirmation));
        }
        other => panic!("unexpected step response: {other:?}"),
    }
}

#[tokio::test]
async fn http_runtime_e2e_marks_confirmation_wait_cancel_as_pending() {
    let app = build_http_router(build_preloaded_signer_wait_service());
    let run_id = RunId("run-1".to_owned());

    let inspect_1 = send_http_command(
        &app,
        inspect_command(&run_id, "http-cancel-pending-inspect-1"),
    )
    .await;
    let signer_request_id = match inspect_1.response {
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(
                snapshot.status,
                ais_agent_host::inspect::RunStatus::AwaitingSigner
            );
            snapshot.pending_signer_requests[0].request_id.clone()
        }
        other => panic!("unexpected inspect response: {other:?}"),
    };

    let _signer = send_http_command(&app, signer_command(&run_id, &signer_request_id)).await;
    let _step = send_http_command(&app, step_command(&run_id, "http-cancel-pending-step")).await;

    let cancel = send_http_command(&app, request_cancel_command(&run_id)).await;
    match cancel.response {
        HostCommandResponse::Pause(pause) => {
            assert_eq!(
                pause.kind,
                ais_agent_host::inspect::PauseKind::NeedConfirmation
            );
            assert_eq!(
                pause.cancel_state,
                Some(ais_agent_control::recovery::CancelState::Pending)
            );
            assert_eq!(
                pause.interruption_class,
                Some(ais_agent_control::recovery::InterruptionClass::HostCancelRequested)
            );
            assert!(!pause
                .allowed_recovery_actions
                .contains(&ais_agent_control::recovery::RecoveryActionKind::CancelRun));
        }
        other => panic!("unexpected cancel response: {other:?}"),
    }

    let inspect_2 = send_http_command(
        &app,
        inspect_command(&run_id, "http-cancel-pending-inspect-2"),
    )
    .await;
    match inspect_2.response {
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(
                snapshot.cancel_state,
                Some(ais_agent_control::recovery::CancelState::Pending)
            );
            assert_eq!(
                snapshot.interruption_class,
                Some(ais_agent_control::recovery::InterruptionClass::HostCancelRequested)
            );
        }
        other => panic!("unexpected inspect response: {other:?}"),
    }
}

#[tokio::test]
async fn http_runtime_e2e_can_complete_after_host_approved_signer_via_commands_inspect_and_events()
{
    let app = build_http_router(build_preloaded_signer_wait_service());
    let run_id = RunId("run-1".to_owned());

    let inspect_1 =
        send_http_command(&app, inspect_command(&run_id, "http-inspect-approved-1")).await;
    let signer_request_id = match inspect_1.response {
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(
                snapshot.status,
                ais_agent_host::inspect::RunStatus::AwaitingSigner
            );
            snapshot.pending_signer_requests[0].request_id.clone()
        }
        other => panic!("unexpected inspect response: {other:?}"),
    };

    let signer =
        send_http_command(&app, signer_approved_command(&run_id, &signer_request_id)).await;
    match signer.response {
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(
                snapshot.status,
                ais_agent_host::inspect::RunStatus::AwaitingSigner
            );
        }
        other => panic!("unexpected signer approval response: {other:?}"),
    }

    let stepped = send_http_command(&app, step_command(&run_id, "http-step-approved-1")).await;
    match stepped.response {
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(
                snapshot.status,
                ais_agent_host::inspect::RunStatus::Completed
            );
        }
        other => panic!("unexpected step response: {other:?}"),
    }

    let batch = send_http_event_poll(&app, &run_id, Some(0), Some(50)).await;
    assert!(batch
        .events
        .iter()
        .any(|event| matches!(event.event, RunEvent::Completed(_))));
}

#[tokio::test]
async fn http_runtime_e2e_can_poll_event_batches() {
    let app = build_http_router(build_runtime_host_service());

    let begin = send_http_command(&app, sample_begin_command()).await;
    let run_id = match begin.response {
        HostCommandResponse::Accepted(response) => response.run_id.expect("run id"),
        other => panic!("unexpected begin response: {other:?}"),
    };

    let batch = send_http_event_poll(&app, &run_id, Some(0), Some(10)).await;
    assert_eq!(batch.run_id, run_id);
    assert_eq!(batch.latest_event_seq, Some(1));
    assert_eq!(batch.next_after_event_seq, Some(1));
    assert_eq!(batch.events.len(), 1);
    assert!(matches!(batch.events[0].event, RunEvent::Started(_)));
}

#[tokio::test]
async fn http_runtime_e2e_returns_404_for_missing_run_event_poll() {
    let app = build_http_router(build_runtime_host_service());

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/runs/run-missing/events?after_event_seq=0&limit=10")
                .method("GET")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let body: serde_json::Value = serde_json::from_slice(&bytes).expect("decode error body");
    assert_eq!(body["code"], "run_not_found");
}

#[tokio::test]
async fn http_runtime_e2e_supports_host_collaboration_via_inspect_and_event_polling() {
    let app = build_http_router(build_preloaded_signer_wait_service());
    let run_id = RunId("run-1".to_owned());

    let inspect_before =
        send_http_command(&app, inspect_command(&run_id, "http-collab-inspect-1")).await;
    let signer_request_id = match inspect_before.response {
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(
                snapshot.status,
                ais_agent_host::inspect::RunStatus::AwaitingSigner
            );
            snapshot.pending_signer_requests[0].request_id.clone()
        }
        other => panic!("unexpected inspect response: {other:?}"),
    };

    let initial_batch = send_http_event_poll(&app, &run_id, Some(0), Some(10)).await;
    assert!(initial_batch.events.is_empty());

    let signer = send_http_command(&app, signer_command(&run_id, &signer_request_id)).await;
    match signer.response {
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(
                snapshot.status,
                ais_agent_host::inspect::RunStatus::AwaitingSigner
            );
        }
        other => panic!("unexpected signer response: {other:?}"),
    }

    let stepped = send_http_command(&app, step_command(&run_id, "http-collab-step")).await;
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

    let completed_batch = send_http_event_poll(&app, &run_id, Some(0), Some(10)).await;
    assert!(completed_batch
        .events
        .iter()
        .any(|event| matches!(event.event, RunEvent::AwaitingConfirm(_))));
}

#[tokio::test]
async fn http_runtime_e2e_supports_minimal_solana_guarded_execution_to_confirmation_pause() {
    let app = build_http_router(build_preloaded_solana_signer_wait_service());
    let run_id = RunId("run-1".to_owned());

    let inspect_1 = send_http_command(&app, inspect_command(&run_id, "http-sol-inspect-1")).await;
    let signer_request_id = match inspect_1.response {
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(
                snapshot.status,
                ais_agent_host::inspect::RunStatus::AwaitingSigner
            );
            snapshot.pending_signer_requests[0].request_id.clone()
        }
        other => panic!("unexpected inspect response: {other:?}"),
    };

    let initial_batch = send_http_event_poll(&app, &run_id, Some(0), Some(10)).await;
    assert!(initial_batch.events.is_empty());

    let signer = send_http_command(&app, signer_command(&run_id, &signer_request_id)).await;
    match signer.response {
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(
                snapshot.status,
                ais_agent_host::inspect::RunStatus::AwaitingSigner
            );
        }
        other => panic!("unexpected signer submission response: {other:?}"),
    }

    let stepped = send_http_command(&app, step_command(&run_id, "http-sol-step-1")).await;
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

    let batch = send_http_event_poll(&app, &run_id, Some(0), Some(20)).await;
    assert!(batch
        .events
        .iter()
        .any(|event| matches!(event.event, RunEvent::AwaitingConfirm(_))));
}

#[tokio::test]
async fn http_runtime_e2e_can_replace_envelope_after_recovery_pause_and_continue() {
    let app = build_http_router(build_preloaded_envelope_wait_service());
    let run_id = RunId("run-1".to_owned());

    let inspect_1 =
        send_http_command(&app, inspect_command(&run_id, "http-envelope-inspect-1")).await;
    match inspect_1.response {
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(
                snapshot.recovery_disposition,
                Some(ais_agent_control::recovery::RecoveryDisposition::AwaitEnvelope)
            );
            assert!(snapshot
                .allowed_recovery_actions
                .contains(&ais_agent_control::recovery::RecoveryActionKind::SubmitEnvelope));
            assert_eq!(
                snapshot.recovery_suggestions[0].required_inputs[0]
                    .value
                    .as_deref(),
                Some("env.swap")
            );
        }
        other => panic!("unexpected envelope inspect response: {other:?}"),
    }

    let wrong = send_http_command(
        &app,
        envelope_command(&run_id, "env.other", "http-envelope-wrong"),
    )
    .await;
    match wrong.response {
        HostCommandResponse::Error(error) => {
            assert_eq!(error.code, "envelope_invalid");
        }
        other => panic!("unexpected wrong envelope response: {other:?}"),
    }

    let submit = send_http_command(
        &app,
        envelope_command(&run_id, "env.swap", "http-envelope-submit"),
    )
    .await;
    match submit.response {
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(snapshot.status, ais_agent_host::inspect::RunStatus::Running);
            assert_eq!(
                snapshot.phase,
                ais_agent_host::inspect::RunPhase::Recovering
            );
        }
        other => panic!("unexpected envelope submit response: {other:?}"),
    }

    let stepped = send_http_command(&app, step_command(&run_id, "http-envelope-step")).await;
    match stepped.response {
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(
                snapshot.status,
                ais_agent_host::inspect::RunStatus::Completed
            );
        }
        other => panic!("unexpected envelope step response: {other:?}"),
    }
}

fn parse_jsonl_frames(output: &[u8]) -> Vec<JsonlOutboundFrame> {
    std::str::from_utf8(output)
        .expect("utf8")
        .lines()
        .map(|line| serde_json::from_str::<JsonlOutboundFrame>(line).expect("frame"))
        .collect()
}

fn response_views(frames: &[JsonlOutboundFrame]) -> Vec<&HostCommandResponse> {
    frames
        .iter()
        .filter_map(|frame| match frame {
            JsonlOutboundFrame::Response(response) => Some(&response.response),
            _ => None,
        })
        .collect()
}

async fn send_http_command(
    app: &axum::Router,
    command: ais_agent_host::session::HostedRunCommand,
) -> HostCommandOutcome {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/commands")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&command).expect("encode command"),
                ))
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    serde_json::from_slice(&bytes).expect("decode outcome")
}

async fn send_http_event_poll(
    app: &axum::Router,
    run_id: &RunId,
    after_event_seq: Option<u64>,
    limit: Option<usize>,
) -> HostRunEventBatch {
    let mut uri = format!("/runs/{}/events", run_id.0);
    let mut pairs = Vec::new();
    if let Some(after_event_seq) = after_event_seq {
        pairs.push(format!("after_event_seq={after_event_seq}"));
    }
    if let Some(limit) = limit {
        pairs.push(format!("limit={limit}"));
    }
    if !pairs.is_empty() {
        uri.push('?');
        uri.push_str(&pairs.join("&"));
    }

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(uri)
                .method("GET")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    serde_json::from_slice(&bytes).expect("decode event batch")
}

fn sqlite_runtime_host_service(
    sqlite_path: &Path,
    run_repo: InMemoryRunRepository,
    session_store: InMemoryHostSessionStore,
) -> RuntimeHostService<
    InMemoryRunRepository,
    SqliteStore,
    SqliteStore,
    SqliteStore,
    SqliteStore,
    InMemoryHostSessionStore,
> {
    RuntimeHostService::new(
        run_repo,
        SqliteStore::open_path(sqlite_path).expect("checkpoint store"),
        SqliteStore::open_path(sqlite_path).expect("mission store"),
        SqliteStore::open_path(sqlite_path).expect("catalog store"),
        SqliteStore::open_path(sqlite_path).expect("event store"),
        session_store,
    )
}

fn sqlite_test_path(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "ais-agent-transport-{label}-{}-{nanos}.sqlite",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    path
}
