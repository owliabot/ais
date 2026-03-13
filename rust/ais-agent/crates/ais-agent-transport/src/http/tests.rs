use ais_agent_control::{
    commands::{
        BeginRunCommand, ClaimRunCommand, MissionSubmission, ReleaseRunClaimCommand,
        RenewRunClaimCommand, RunCommand,
    },
    events::{RunEvent, RunEventEnvelope, RunStarted},
    ids::{ClaimId, CommandId, EventId, IdempotencyKey, RunId},
    ownership::{OwnershipVisibility, RunClaimMode, RunClaimOwnerKind, RunOwnershipSnapshot},
    recovery::{
        InterruptionClass, RecoveryActionKind, RecoveryDisposition, RecoveryPriority,
        RecoverySuggestion, RunFailureCode, RunFailureContext, RunFailureStage, SideEffectPhase,
        StableBoundaryKind,
    },
};
use ais_agent_host::{
    control::{HostAcceptedResponse, HostCommandOutcome, HostCommandResponse, HostCommandService},
    events::{HostEventServiceError, HostRunEventBatch, HostRunEventQuery, HostRunEventService},
    inspect::{
        ActionStatusCountsView, BoundaryKind, InspectSnapshot, MissionSummaryView, PauseActionView,
        PauseBundle, PauseKind, PendingConfirmationView, ProgressView, RunPhase, RunStatus,
    },
    session::{
        HostCommandEnvelope, HostRequestId, HostSessionId, HostSessionSnapshot, HostedRunCommand,
    },
};
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use tower::util::ServiceExt;

use crate::http::build_http_router;

#[tokio::test]
async fn http_transport_maps_directly_to_host_command_outcome() {
    let app = build_http_router(MockHostService);
    let body = serde_json::to_vec(&sample_command()).expect("encode command");

    let response = app
        .oneshot(
            Request::builder()
                .uri("/commands")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let outcome: HostCommandOutcome = serde_json::from_slice(&bytes).expect("decode outcome");

    match outcome.response {
        HostCommandResponse::Accepted(value) => {
            assert_eq!(value.run_id.expect("run id").0, "run-1");
        }
        other => panic!("unexpected response: {other:?}"),
    }
    assert_eq!(outcome.events.len(), 1);
    assert_eq!(outcome.events[0].event_seq, 1);
    assert_eq!(outcome.events[0].checkpoint_seq, 0);
    assert_eq!(outcome.events[0].plan_epoch, 0);
}

#[tokio::test]
async fn http_transport_can_poll_event_batches() {
    let app = build_http_router(MockHostService);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/runs/run-1/events?after_event_seq=0&limit=10")
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
    let batch: HostRunEventBatch = serde_json::from_slice(&bytes).expect("decode batch");
    assert_eq!(batch.run_id.0, "run-1");
    assert_eq!(batch.latest_event_seq, Some(1));
    assert_eq!(batch.next_after_event_seq, Some(1));
    assert_eq!(batch.events.len(), 1);
}

#[tokio::test]
async fn http_transport_maps_event_poll_not_found_to_404() {
    let app = build_http_router(EventNotFoundMockHostService);

    let response = app
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
    let body = decode_json_body(response).await;
    assert_eq!(body["code"], "run_not_found");
}

#[tokio::test]
async fn http_transport_maps_event_poll_conflict_to_409() {
    let app = build_http_router(EventConflictMockHostService);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/runs/run-1/events?after_event_seq=0&limit=10")
                .method("GET")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = decode_json_body(response).await;
    assert_eq!(body["code"], "session_run_mismatch");
}

#[tokio::test]
async fn http_transport_maps_event_poll_backend_failure_to_503() {
    let app = build_http_router(EventBackendFailureMockHostService);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/runs/run-1/events?after_event_seq=0&limit=10")
                .method("GET")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = decode_json_body(response).await;
    assert_eq!(body["code"], "event_query_failed");
}

#[tokio::test]
async fn http_transport_preserves_recovery_aware_inspect_fields() {
    let app = build_http_router(RecoveryInspectMockHostService);
    let body = serde_json::to_vec(&sample_command()).expect("encode command");

    let response = app
        .oneshot(
            Request::builder()
                .uri("/commands")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let outcome: HostCommandOutcome = serde_json::from_slice(&bytes).expect("decode outcome");

    match outcome.response {
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(snapshot.run_id.0, "run-1");
            assert_eq!(
                snapshot.recovery_disposition,
                Some(RecoveryDisposition::AwaitPatch)
            );
            assert_eq!(
                snapshot
                    .failure_context
                    .as_ref()
                    .map(|failure| &failure.code),
                Some(&RunFailureCode::GovernorDenied)
            );
            assert_eq!(snapshot.recovery_suggestions.len(), 1);
            assert_eq!(
                snapshot.recovery_suggestions[0].action_kind,
                RecoveryActionKind::SubmitPlanPatch
            );
            assert_eq!(
                snapshot.allowed_recovery_actions,
                vec![
                    RecoveryActionKind::SubmitPlanPatch,
                    RecoveryActionKind::CancelRun
                ]
            );
        }
        other => panic!("unexpected response: {other:?}"),
    }
}

#[tokio::test]
async fn http_transport_serializes_confirmation_pause_actions_with_typed_kinds() {
    let app = build_http_router(ConfirmationPauseMockHostService);
    let body = serde_json::to_vec(&sample_command()).expect("encode command");

    let response = app
        .oneshot(
            Request::builder()
                .uri("/commands")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    let body = decode_json_body(response).await;
    assert_eq!(body["response"]["type"], "pause");
    let actions = body["response"]["required_actions"]
        .as_array()
        .expect("required actions array");
    assert_eq!(actions[0]["action"], "step_run");
    assert_eq!(actions[0]["action_kind"], "retry_step");
    assert_eq!(actions[1]["action"], "step_run");
    assert_eq!(actions[1]["action_kind"], "await_confirmation");
}

#[tokio::test]
async fn http_transport_preserves_retry_ready_inspect_fields() {
    let app = build_http_router(RetryReadyInspectMockHostService);
    let body = serde_json::to_vec(&sample_command()).expect("encode command");

    let response = app
        .oneshot(
            Request::builder()
                .uri("/commands")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let outcome: HostCommandOutcome = serde_json::from_slice(&bytes).expect("decode outcome");

    match outcome.response {
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(
                snapshot.recovery_disposition,
                Some(RecoveryDisposition::RetryReady)
            );
            assert_eq!(
                snapshot.interruption_class,
                Some(InterruptionClass::ConfirmationWaitTimeout)
            );
            assert_eq!(
                snapshot.side_effect_phase,
                Some(SideEffectPhase::AwaitingConfirmation)
            );
        }
        other => panic!("unexpected response: {other:?}"),
    }
}

#[tokio::test]
async fn http_transport_preserves_await_user_input_pause_fields() {
    let app = build_http_router(AwaitUserInputPauseMockHostService);
    let body = serde_json::to_vec(&sample_command()).expect("encode command");

    let response = app
        .oneshot(
            Request::builder()
                .uri("/commands")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let outcome: HostCommandOutcome = serde_json::from_slice(&bytes).expect("decode outcome");

    match outcome.response {
        HostCommandResponse::Pause(pause) => {
            assert_eq!(
                pause.recovery_disposition,
                RecoveryDisposition::AwaitUserInput
            );
            assert_eq!(
                pause.interruption_class,
                Some(InterruptionClass::BroadcastOutcomeUncertain)
            );
            assert_eq!(
                pause.side_effect_phase,
                Some(SideEffectPhase::BroadcastSubmitted)
            );
        }
        other => panic!("unexpected response: {other:?}"),
    }
}

#[tokio::test]
async fn http_transport_passes_through_claim_run_command() {
    let app = build_http_router(OwnershipPassthroughMockHostService);
    let body = serde_json::to_vec(&sample_claim_command()).expect("encode command");

    let response = app
        .oneshot(
            Request::builder()
                .uri("/commands")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let outcome: HostCommandOutcome = serde_json::from_slice(&bytes).expect("decode outcome");
    match outcome.response {
        HostCommandResponse::Accepted(value) => {
            assert_eq!(value.run_id, Some(RunId("run-1".to_owned())));
            assert_eq!(value.message.as_deref(), Some("claim_run:worker-a"));
        }
        other => panic!("unexpected response: {other:?}"),
    }
}

#[tokio::test]
async fn http_transport_passes_through_renew_and_release_claim_commands() {
    let app = build_http_router(OwnershipPassthroughMockHostService);

    for (command, expected_message) in [
        (sample_renew_claim_command(), "renew_run_claim:claim-1:3"),
        (
            sample_release_claim_command(),
            "release_run_claim:claim-1:3",
        ),
    ] {
        let body = serde_json::to_vec(&command).expect("encode command");
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/commands")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let outcome: HostCommandOutcome = serde_json::from_slice(&bytes).expect("decode outcome");
        match outcome.response {
            HostCommandResponse::Accepted(value) => {
                assert_eq!(value.message.as_deref(), Some(expected_message));
            }
            other => panic!("unexpected response: {other:?}"),
        }
    }
}

#[tokio::test]
async fn http_transport_surfaces_ownership_error_codes_in_host_outcomes() {
    for code in [
        "claim_required",
        "claim_conflict",
        "claim_expired",
        "observer_only",
    ] {
        let app = build_http_router(OwnershipErrorMockHostService { code });
        let body = serde_json::to_vec(&sample_claim_command()).expect("encode command");
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/commands")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let outcome: HostCommandOutcome = serde_json::from_slice(&bytes).expect("decode outcome");
        match outcome.response {
            HostCommandResponse::Error(error) => assert_eq!(error.code, code),
            other => panic!("unexpected response: {other:?}"),
        }
    }
}

struct MockHostService;

struct RecoveryInspectMockHostService;
struct ConfirmationPauseMockHostService;
struct RetryReadyInspectMockHostService;
struct AwaitUserInputPauseMockHostService;
struct OwnershipPassthroughMockHostService;
struct OwnershipErrorMockHostService {
    code: &'static str,
}
struct EventNotFoundMockHostService;
struct EventConflictMockHostService;
struct EventBackendFailureMockHostService;

impl HostCommandService for MockHostService {
    fn handle(
        &mut self,
        _command: HostedRunCommand,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = HostCommandOutcome> + Send + '_>> {
        Box::pin(async move {
            HostCommandOutcome {
                response: HostCommandResponse::Accepted(HostAcceptedResponse {
                    run_id: Some(RunId("run-1".to_owned())),
                    message: Some("accepted".to_owned()),
                    session: Some(HostSessionSnapshot {
                        host_session_id: HostSessionId("session-1".to_owned()),
                        active_run_id: Some(RunId("run-1".to_owned())),
                        linked_runs: Vec::new(),
                    }),
                }),
                events: vec![RunEventEnvelope {
                    run_id: RunId("run-1".to_owned()),
                    event_seq: 1,
                    checkpoint_seq: 0,
                    plan_epoch: 0,
                    event: RunEvent::Started(RunStarted {
                        event_id: EventId("event-1".to_owned()),
                        run_id: RunId("run-1".to_owned()),
                        phase: "mission_accepted".to_owned(),
                    }),
                }],
            }
        })
    }
}

impl HostCommandService for RecoveryInspectMockHostService {
    fn handle(
        &mut self,
        _command: HostedRunCommand,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = HostCommandOutcome> + Send + '_>> {
        Box::pin(async move {
            HostCommandOutcome {
                response: HostCommandResponse::Inspect(sample_recovery_aware_inspect()),
                events: Vec::new(),
            }
        })
    }
}

impl HostCommandService for ConfirmationPauseMockHostService {
    fn handle(
        &mut self,
        _command: HostedRunCommand,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = HostCommandOutcome> + Send + '_>> {
        Box::pin(async move {
            HostCommandOutcome {
                response: HostCommandResponse::Pause(sample_confirmation_pause()),
                events: Vec::new(),
            }
        })
    }
}

impl HostCommandService for RetryReadyInspectMockHostService {
    fn handle(
        &mut self,
        _command: HostedRunCommand,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = HostCommandOutcome> + Send + '_>> {
        Box::pin(async move {
            HostCommandOutcome {
                response: HostCommandResponse::Inspect(sample_retry_ready_inspect()),
                events: Vec::new(),
            }
        })
    }
}

impl HostCommandService for AwaitUserInputPauseMockHostService {
    fn handle(
        &mut self,
        _command: HostedRunCommand,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = HostCommandOutcome> + Send + '_>> {
        Box::pin(async move {
            HostCommandOutcome {
                response: HostCommandResponse::Pause(sample_await_user_input_pause()),
                events: Vec::new(),
            }
        })
    }
}

impl HostCommandService for OwnershipPassthroughMockHostService {
    fn handle(
        &mut self,
        command: HostedRunCommand,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = HostCommandOutcome> + Send + '_>> {
        Box::pin(async move {
            let message = match command.command {
                RunCommand::ClaimRun(command) => {
                    assert_eq!(command.run_id.0, "run-1");
                    assert_eq!(command.owner_instance_id, "worker-a");
                    assert_eq!(command.mode, RunClaimMode::ExclusiveMutation);
                    format!("claim_run:{}", command.owner_instance_id)
                }
                RunCommand::RenewRunClaim(command) => {
                    assert_eq!(command.claim_id.0, "claim-1");
                    assert_eq!(command.claim_epoch, 3);
                    format!(
                        "renew_run_claim:{}:{}",
                        command.claim_id.0, command.claim_epoch
                    )
                }
                RunCommand::ReleaseRunClaim(command) => {
                    assert_eq!(command.claim_id.0, "claim-1");
                    assert_eq!(command.claim_epoch, 3);
                    format!(
                        "release_run_claim:{}:{}",
                        command.claim_id.0, command.claim_epoch
                    )
                }
                other => panic!("unexpected command: {other:?}"),
            };

            HostCommandOutcome {
                response: HostCommandResponse::Accepted(HostAcceptedResponse {
                    run_id: Some(RunId("run-1".to_owned())),
                    message: Some(message),
                    session: None,
                }),
                events: Vec::new(),
            }
        })
    }
}

impl HostCommandService for OwnershipErrorMockHostService {
    fn handle(
        &mut self,
        _command: HostedRunCommand,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = HostCommandOutcome> + Send + '_>> {
        let code = self.code;
        Box::pin(async move {
            HostCommandOutcome {
                response: HostCommandResponse::Error(ais_agent_host::control::HostCommandError {
                    code: code.to_owned(),
                    message: format!("ownership error: {code}"),
                }),
                events: Vec::new(),
            }
        })
    }
}

impl HostRunEventService for ConfirmationPauseMockHostService {
    fn list_events(
        &self,
        query: HostRunEventQuery,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<HostRunEventBatch, HostEventServiceError>>
                + Send
                + '_,
        >,
    > {
        Box::pin(async move {
            Ok(HostRunEventBatch {
                run_id: query.run_id,
                after_event_seq: query.after_event_seq,
                latest_event_seq: Some(1),
                next_after_event_seq: Some(1),
                truncated: false,
                events: Vec::new(),
            })
        })
    }
}

impl HostRunEventService for RetryReadyInspectMockHostService {
    fn list_events(
        &self,
        query: HostRunEventQuery,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<HostRunEventBatch, HostEventServiceError>>
                + Send
                + '_,
        >,
    > {
        Box::pin(async move {
            Ok(HostRunEventBatch {
                run_id: query.run_id,
                after_event_seq: query.after_event_seq,
                latest_event_seq: None,
                next_after_event_seq: None,
                truncated: false,
                events: Vec::new(),
            })
        })
    }
}

impl HostRunEventService for AwaitUserInputPauseMockHostService {
    fn list_events(
        &self,
        query: HostRunEventQuery,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<HostRunEventBatch, HostEventServiceError>>
                + Send
                + '_,
        >,
    > {
        Box::pin(async move {
            Ok(HostRunEventBatch {
                run_id: query.run_id,
                after_event_seq: query.after_event_seq,
                latest_event_seq: None,
                next_after_event_seq: None,
                truncated: false,
                events: Vec::new(),
            })
        })
    }
}

impl HostRunEventService for OwnershipPassthroughMockHostService {
    fn list_events(
        &self,
        query: HostRunEventQuery,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<HostRunEventBatch, HostEventServiceError>>
                + Send
                + '_,
        >,
    > {
        Box::pin(async move {
            Ok(HostRunEventBatch {
                run_id: query.run_id,
                after_event_seq: query.after_event_seq,
                latest_event_seq: None,
                next_after_event_seq: None,
                truncated: false,
                events: Vec::new(),
            })
        })
    }
}

impl HostRunEventService for OwnershipErrorMockHostService {
    fn list_events(
        &self,
        query: HostRunEventQuery,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<HostRunEventBatch, HostEventServiceError>>
                + Send
                + '_,
        >,
    > {
        Box::pin(async move {
            Ok(HostRunEventBatch {
                run_id: query.run_id,
                after_event_seq: query.after_event_seq,
                latest_event_seq: None,
                next_after_event_seq: None,
                truncated: false,
                events: Vec::new(),
            })
        })
    }
}

impl HostCommandService for EventNotFoundMockHostService {
    fn handle(
        &mut self,
        _command: HostedRunCommand,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = HostCommandOutcome> + Send + '_>> {
        Box::pin(async move { panic!("command path should not be used in event test") })
    }
}

impl HostCommandService for EventConflictMockHostService {
    fn handle(
        &mut self,
        _command: HostedRunCommand,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = HostCommandOutcome> + Send + '_>> {
        Box::pin(async move { panic!("command path should not be used in event test") })
    }
}

impl HostCommandService for EventBackendFailureMockHostService {
    fn handle(
        &mut self,
        _command: HostedRunCommand,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = HostCommandOutcome> + Send + '_>> {
        Box::pin(async move { panic!("command path should not be used in event test") })
    }
}

impl HostRunEventService for MockHostService {
    fn list_events(
        &self,
        query: HostRunEventQuery,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<HostRunEventBatch, HostEventServiceError>>
                + Send
                + '_,
        >,
    > {
        Box::pin(async move {
            Ok(HostRunEventBatch {
                run_id: query.run_id,
                after_event_seq: query.after_event_seq,
                latest_event_seq: Some(1),
                next_after_event_seq: Some(1),
                truncated: false,
                events: vec![RunEventEnvelope {
                    run_id: RunId("run-1".to_owned()),
                    event_seq: 1,
                    checkpoint_seq: 0,
                    plan_epoch: 0,
                    event: RunEvent::Started(RunStarted {
                        event_id: EventId("event-1".to_owned()),
                        run_id: RunId("run-1".to_owned()),
                        phase: "mission_accepted".to_owned(),
                    }),
                }],
            })
        })
    }
}

impl HostRunEventService for RecoveryInspectMockHostService {
    fn list_events(
        &self,
        query: HostRunEventQuery,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<HostRunEventBatch, HostEventServiceError>>
                + Send
                + '_,
        >,
    > {
        Box::pin(async move {
            Ok(HostRunEventBatch {
                run_id: query.run_id,
                after_event_seq: query.after_event_seq,
                latest_event_seq: None,
                next_after_event_seq: None,
                truncated: false,
                events: Vec::new(),
            })
        })
    }
}

impl HostRunEventService for EventNotFoundMockHostService {
    fn list_events(
        &self,
        _query: HostRunEventQuery,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<HostRunEventBatch, HostEventServiceError>>
                + Send
                + '_,
        >,
    > {
        Box::pin(async move {
            Err(HostEventServiceError {
                code: "run_not_found".to_owned(),
                message: "run `run-missing` not found".to_owned(),
            })
        })
    }
}

impl HostRunEventService for EventConflictMockHostService {
    fn list_events(
        &self,
        _query: HostRunEventQuery,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<HostRunEventBatch, HostEventServiceError>>
                + Send
                + '_,
        >,
    > {
        Box::pin(async move {
            Err(HostEventServiceError {
                code: "session_run_mismatch".to_owned(),
                message: "session is not linked to run".to_owned(),
            })
        })
    }
}

impl HostRunEventService for EventBackendFailureMockHostService {
    fn list_events(
        &self,
        _query: HostRunEventQuery,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<HostRunEventBatch, HostEventServiceError>>
                + Send
                + '_,
        >,
    > {
        Box::pin(async move {
            Err(HostEventServiceError {
                code: "event_query_failed".to_owned(),
                message: "event archive storage error".to_owned(),
            })
        })
    }
}

fn sample_command() -> HostedRunCommand {
    HostCommandEnvelope {
        host_session_id: HostSessionId("session-1".to_owned()),
        host_request_id: Some(HostRequestId("request-1".to_owned())),
        command: RunCommand::BeginRun(BeginRunCommand {
            command_id: CommandId("cmd-1".to_owned()),
            idempotency_key: IdempotencyKey("idem-1".to_owned()),
            mission: MissionSubmission {
                goal: "swap".to_owned(),
                allowed_chains: vec!["eip155:1".to_owned()],
                constraints: Default::default(),
                budget: None,
                metadata: Default::default(),
            },
        }),
    }
}

fn sample_claim_command() -> HostedRunCommand {
    HostCommandEnvelope {
        host_session_id: HostSessionId("session-1".to_owned()),
        host_request_id: Some(HostRequestId("request-claim".to_owned())),
        command: RunCommand::ClaimRun(ClaimRunCommand {
            command_id: CommandId("cmd-claim".to_owned()),
            run_id: RunId("run-1".to_owned()),
            owner_kind: RunClaimOwnerKind::InteractiveHost,
            owner_instance_id: "worker-a".to_owned(),
            mode: RunClaimMode::ExclusiveMutation,
            requested_lease_ms: Some(30_000),
            allow_supersede: false,
            expected_current_claim_id: None,
            expected_current_claim_epoch: None,
        }),
    }
}

fn sample_renew_claim_command() -> HostedRunCommand {
    HostCommandEnvelope {
        host_session_id: HostSessionId("session-1".to_owned()),
        host_request_id: Some(HostRequestId("request-renew".to_owned())),
        command: RunCommand::RenewRunClaim(RenewRunClaimCommand {
            command_id: CommandId("cmd-renew".to_owned()),
            run_id: RunId("run-1".to_owned()),
            claim_id: ClaimId("claim-1".to_owned()),
            claim_epoch: 3,
            requested_lease_ms: Some(30_000),
        }),
    }
}

fn sample_release_claim_command() -> HostedRunCommand {
    HostCommandEnvelope {
        host_session_id: HostSessionId("session-1".to_owned()),
        host_request_id: Some(HostRequestId("request-release".to_owned())),
        command: RunCommand::ReleaseRunClaim(ReleaseRunClaimCommand {
            command_id: CommandId("cmd-release".to_owned()),
            run_id: RunId("run-1".to_owned()),
            claim_id: ClaimId("claim-1".to_owned()),
            claim_epoch: 3,
            reason: Some("transport passthrough".to_owned()),
        }),
    }
}

async fn decode_json_body(response: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    serde_json::from_slice(&bytes).expect("decode json body")
}

fn sample_ownership(claim_required_for_mutation: bool) -> RunOwnershipSnapshot {
    RunOwnershipSnapshot {
        run_id: RunId("run-1".to_owned()),
        current_claim: None,
        last_terminal_claim_id: None,
        last_claim_transition: None,
        claim_required_for_mutation,
        owner_visibility: OwnershipVisibility::ObserverReadAllowed,
    }
}

fn sample_recovery_aware_inspect() -> InspectSnapshot {
    let mut failure = RunFailureContext::new(
        RunFailureCode::GovernorDenied,
        RunFailureStage::Govern,
        4,
        2,
        Some(StableBoundaryKind::Pause),
        "governor requested patch",
    );
    failure.node_refs.push("govern.swap".to_owned());

    InspectSnapshot {
        schema: "ais-agent/inspect_snapshot/v1".to_owned(),
        run_id: RunId("run-1".to_owned()),
        status: RunStatus::Paused,
        phase: RunPhase::AwaitingHost,
        checkpoint_seq: 4,
        plan_epoch: 2,
        active_boundary: Some(ais_agent_host::inspect::ActiveBoundaryView {
            kind: BoundaryKind::Pause,
            summary: "governor requested patch".to_owned(),
        }),
        interruption_class: None,
        cancel_state: None,
        side_effect_phase: None,
        recovery_disposition: Some(RecoveryDisposition::AwaitPatch),
        failure_context: Some(failure),
        recovery_suggestions: vec![RecoverySuggestion {
            suggestion_id: "run-1:recovery:4:submit_plan_patch".to_owned(),
            action_kind: RecoveryActionKind::SubmitPlanPatch,
            reason_code: RunFailureCode::GovernorDenied,
            priority: RecoveryPriority::HostReview,
            basis_checkpoint_seq: 4,
            basis_plan_epoch: 2,
            retry_intent: None,
            target_refs: vec!["govern.swap".to_owned()],
            required_inputs: Vec::new(),
            constraints: Vec::new(),
        }],
        allowed_recovery_actions: vec![
            RecoveryActionKind::SubmitPlanPatch,
            RecoveryActionKind::CancelRun,
        ],
        mission_summary: MissionSummaryView {
            goal: "swap".to_owned(),
            allowed_chains: vec!["eip155:1".to_owned()],
            policy_mode: Some("guarded".to_owned()),
        },
        required_inputs: Vec::new(),
        pending_confirmations: Vec::new(),
        pending_signer_requests: Vec::new(),
        recent_side_effects: Vec::new(),
        effect_status: None,
        ownership: sample_ownership(true),
        run_result: None,
        progress: ProgressView {
            graph_id: Some("graph-1".to_owned()),
            total_nodes: 1,
            roots: 1,
            terminals: 1,
            status_counts: ActionStatusCountsView::default(),
            active_node_ids: Vec::new(),
            blocked_node_ids: vec!["govern.swap".to_owned()],
            last_completed_node_id: None,
            required_evidence_count: 0,
            actuation_record_count: 0,
        },
    }
}

fn sample_confirmation_pause() -> PauseBundle {
    PauseBundle {
        schema: "ais-agent/pause_bundle/v2".to_owned(),
        run_id: RunId("run-1".to_owned()),
        kind: PauseKind::NeedConfirmation,
        interruption_class: None,
        cancel_state: None,
        side_effect_phase: Some(ais_agent_control::recovery::SideEffectPhase::AwaitingConfirmation),
        recovery_disposition: RecoveryDisposition::ContinueWait,
        summary: "waiting for chain receipt".to_owned(),
        ownership: sample_ownership(true),
        blocking_refs: vec!["confirm-1".to_owned()],
        required_actions: vec![
            PauseActionView {
                action_kind: RecoveryActionKind::RetryStep,
                action: "step_run".to_owned(),
                description: "Run the stepper again when retry or confirmation polling is allowed."
                    .to_owned(),
                requires_mutation_claim: true,
                retry_intent: Some(ais_agent_control::commands::RetryIntent::ResumeExecution),
            },
            PauseActionView {
                action_kind: RecoveryActionKind::AwaitConfirmation,
                action: "step_run".to_owned(),
                description:
                    "Wait for more chain confirmation information before making a new decision."
                        .to_owned(),
                requires_mutation_claim: true,
                retry_intent: Some(ais_agent_control::commands::RetryIntent::PollConfirmation),
            },
        ],
        failure_context: None,
        recovery_suggestions: Vec::new(),
        allowed_recovery_actions: vec![
            RecoveryActionKind::RetryStep,
            RecoveryActionKind::AwaitConfirmation,
            RecoveryActionKind::CancelRun,
        ],
        pending_signer_requests: Vec::new(),
        pending_confirmations: vec![PendingConfirmationView {
            confirmation_id: "confirm-1".to_owned(),
            kind: "chain_confirmation".to_owned(),
            summary: "waiting for confirmation".to_owned(),
        }],
        notes: vec!["confirmation polling in progress".to_owned()],
    }
}

fn sample_retry_ready_inspect() -> InspectSnapshot {
    let mut failure = RunFailureContext::new(
        RunFailureCode::ConfirmationTimeout,
        RunFailureStage::Confirm,
        5,
        2,
        Some(StableBoundaryKind::Confirmation),
        "confirmation lookup timed out",
    );
    failure.confirmation_refs.push("confirm-1".to_owned());

    InspectSnapshot {
        schema: "ais-agent/inspect_snapshot/v1".to_owned(),
        run_id: RunId("run-1".to_owned()),
        status: RunStatus::AwaitingConfirm,
        phase: RunPhase::AwaitingHost,
        checkpoint_seq: 5,
        plan_epoch: 2,
        active_boundary: Some(ais_agent_host::inspect::ActiveBoundaryView {
            kind: BoundaryKind::Confirmation,
            summary: "waiting for chain receipt".to_owned(),
        }),
        interruption_class: Some(InterruptionClass::ConfirmationWaitTimeout),
        cancel_state: None,
        side_effect_phase: Some(SideEffectPhase::AwaitingConfirmation),
        recovery_disposition: Some(RecoveryDisposition::RetryReady),
        failure_context: Some(failure),
        recovery_suggestions: vec![RecoverySuggestion {
            suggestion_id: "run-1:recovery:5:retry_step".to_owned(),
            action_kind: RecoveryActionKind::RetryStep,
            reason_code: RunFailureCode::ConfirmationTimeout,
            priority: RecoveryPriority::Automatic,
            basis_checkpoint_seq: 5,
            basis_plan_epoch: 2,
            retry_intent: Some(ais_agent_control::commands::RetryIntent::ResumeExecution),
            target_refs: vec!["confirm-1".to_owned()],
            required_inputs: Vec::new(),
            constraints: Vec::new(),
        }],
        allowed_recovery_actions: vec![
            RecoveryActionKind::RetryStep,
            RecoveryActionKind::CancelRun,
        ],
        mission_summary: MissionSummaryView {
            goal: "swap".to_owned(),
            allowed_chains: vec!["eip155:1".to_owned()],
            policy_mode: Some("guarded".to_owned()),
        },
        required_inputs: Vec::new(),
        pending_confirmations: vec![PendingConfirmationView {
            confirmation_id: "confirm-1".to_owned(),
            kind: "chain_confirmation".to_owned(),
            summary: "waiting for confirmation".to_owned(),
        }],
        pending_signer_requests: Vec::new(),
        recent_side_effects: Vec::new(),
        effect_status: None,
        ownership: sample_ownership(true),
        run_result: None,
        progress: ProgressView {
            graph_id: Some("graph-1".to_owned()),
            total_nodes: 1,
            roots: 1,
            terminals: 1,
            status_counts: ActionStatusCountsView::default(),
            active_node_ids: Vec::new(),
            blocked_node_ids: Vec::new(),
            last_completed_node_id: Some("swap".to_owned()),
            required_evidence_count: 0,
            actuation_record_count: 1,
        },
    }
}

fn sample_await_user_input_pause() -> PauseBundle {
    let mut failure = RunFailureContext::new(
        RunFailureCode::BroadcastUncertain,
        RunFailureStage::Broadcast,
        6,
        2,
        Some(StableBoundaryKind::Pause),
        "provider accepted submission but tx outcome is uncertain",
    );
    failure
        .confirmation_refs
        .push("confirm-uncertain".to_owned());

    PauseBundle {
        schema: "ais-agent/pause_bundle/v2".to_owned(),
        run_id: RunId("run-1".to_owned()),
        kind: PauseKind::NeedUserInput,
        interruption_class: Some(InterruptionClass::BroadcastOutcomeUncertain),
        cancel_state: None,
        side_effect_phase: Some(SideEffectPhase::BroadcastSubmitted),
        recovery_disposition: RecoveryDisposition::AwaitUserInput,
        summary: "provider accepted submission but tx outcome is uncertain".to_owned(),
        ownership: sample_ownership(true),
        blocking_refs: vec!["confirm-uncertain".to_owned()],
        required_actions: vec![PauseActionView {
            action_kind: RecoveryActionKind::EscalateUserReview,
            action: "escalate_user_review".to_owned(),
            description: "escalate to host/user review".to_owned(),
            requires_mutation_claim: false,
            retry_intent: None,
        }],
        failure_context: Some(failure),
        recovery_suggestions: vec![RecoverySuggestion {
            suggestion_id: "run-1:recovery:6:user_review".to_owned(),
            action_kind: RecoveryActionKind::EscalateUserReview,
            reason_code: RunFailureCode::BroadcastUncertain,
            priority: RecoveryPriority::UserReview,
            basis_checkpoint_seq: 6,
            basis_plan_epoch: 2,
            retry_intent: None,
            target_refs: vec!["confirm-uncertain".to_owned()],
            required_inputs: Vec::new(),
            constraints: Vec::new(),
        }],
        allowed_recovery_actions: vec![
            RecoveryActionKind::EscalateUserReview,
            RecoveryActionKind::CancelRun,
        ],
        pending_signer_requests: Vec::new(),
        pending_confirmations: vec![PendingConfirmationView {
            confirmation_id: "confirm-uncertain".to_owned(),
            kind: "chain_confirmation".to_owned(),
            summary: "submission outcome uncertain".to_owned(),
        }],
        notes: vec!["manual review required".to_owned()],
    }
}
