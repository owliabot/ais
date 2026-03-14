use std::io::{self, BufRead, BufReader, Write};

use ais_agent_host::{control::HostCommandService, events::HostRunEventService};
use ais_agent_transport::jsonl::JsonlServer;

pub async fn local_jsonl<S>(service: &mut S) -> io::Result<()>
where
    S: HostCommandService + HostRunEventService,
{
    let stdin = io::stdin();
    let stdout = io::stdout();
    let reader = BufReader::new(stdin.lock());
    let writer = stdout.lock();

    serve_jsonl_with_service(reader, writer, service).await
}

pub async fn serve_jsonl_with_service<R, W, S>(
    reader: R,
    writer: W,
    service: &mut S,
) -> io::Result<()>
where
    R: BufRead,
    W: Write,
    S: HostCommandService + HostRunEventService,
{
    JsonlServer.serve(reader, writer, service).await
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use ais_agent_control::{
        commands::{BeginRunCommand, MissionSubmission, RunCommand},
        events::{RunEvent, RunEventEnvelope, RunStarted},
        ids::{CommandId, EventId, IdempotencyKey, RunId},
        ownership::{OwnershipVisibility, RunOwnershipSnapshot},
        recovery::{
            RecoveryActionKind, RecoveryDisposition, RecoveryPriority, RecoverySuggestion,
            RunFailureCode, RunFailureContext, RunFailureStage, StableBoundaryKind,
        },
    };
    use ais_agent_host::{
        control::{
            HostAcceptedResponse, HostCommandOutcome, HostCommandResponse, HostCommandService,
        },
        events::{
            HostEventServiceError, HostRunEventBatch, HostRunEventQuery, HostRunEventService,
        },
        inspect::{PauseActionView, PauseBundle, PauseKind},
        session::{
            HostCommandEnvelope, HostRequestId, HostSessionId, HostSessionSnapshot,
            HostedRunCommand,
        },
    };
    use ais_agent_transport::jsonl::{JsonlInboundFrame, JsonlOutboundFrame};

    use super::serve_jsonl_with_service;

    #[tokio::test]
    async fn local_jsonl_shell_can_delegate_to_an_injected_host_service() {
        let mut service = ShellHarnessHostService;
        let command = HostCommandEnvelope {
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
                launch_spec: None,
            }),
        };
        let input =
            serde_json::to_string(&JsonlInboundFrame::Command { command }).expect("encode command");
        let mut output = Vec::new();

        serve_jsonl_with_service(Cursor::new(format!("{input}\n")), &mut output, &mut service)
            .await
            .expect("serve");

        let frames = std::str::from_utf8(&output)
            .expect("utf8")
            .lines()
            .map(|line| serde_json::from_str::<JsonlOutboundFrame>(line).expect("frame"))
            .collect::<Vec<_>>();

        assert_eq!(frames.len(), 2);
        match &frames[0] {
            JsonlOutboundFrame::Response(frame) => match &frame.response {
                HostCommandResponse::Accepted(value) => {
                    assert_eq!(value.run_id.as_ref().map(|id| id.0.as_str()), Some("run-1"));
                }
                other => panic!("unexpected response: {other:?}"),
            },
            other => panic!("unexpected frame: {other:?}"),
        }
    }

    #[tokio::test]
    async fn local_jsonl_shell_preserves_recovery_aware_pause_payloads() {
        let mut service = RecoveryPauseShellHostService;
        let command = HostCommandEnvelope {
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
                launch_spec: None,
            }),
        };
        let input =
            serde_json::to_string(&JsonlInboundFrame::Command { command }).expect("encode command");
        let mut output = Vec::new();

        serve_jsonl_with_service(Cursor::new(format!("{input}\n")), &mut output, &mut service)
            .await
            .expect("serve");

        let frames = std::str::from_utf8(&output)
            .expect("utf8")
            .lines()
            .map(|line| serde_json::from_str::<JsonlOutboundFrame>(line).expect("frame"))
            .collect::<Vec<_>>();

        assert_eq!(frames.len(), 1);
        match &frames[0] {
            JsonlOutboundFrame::Response(frame) => match &frame.response {
                HostCommandResponse::Pause(pause) => {
                    assert_eq!(pause.kind, PauseKind::NeedUserInput);
                    assert_eq!(pause.recovery_disposition, RecoveryDisposition::AwaitPatch);
                    assert_eq!(
                        pause.failure_context.as_ref().map(|failure| &failure.code),
                        Some(&RunFailureCode::GovernorDenied)
                    );
                    assert_eq!(pause.recovery_suggestions.len(), 1);
                    assert!(pause
                        .allowed_recovery_actions
                        .contains(&RecoveryActionKind::SubmitPlanPatch));
                }
                other => panic!("unexpected response: {other:?}"),
            },
            other => panic!("unexpected frame: {other:?}"),
        }
    }

    struct ShellHarnessHostService;
    struct RecoveryPauseShellHostService;

    impl HostCommandService for ShellHarnessHostService {
        fn handle(
            &mut self,
            _command: HostedRunCommand,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = HostCommandOutcome> + Send + '_>>
        {
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

    impl HostRunEventService for ShellHarnessHostService {
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

    impl HostCommandService for RecoveryPauseShellHostService {
        fn handle(
            &mut self,
            _command: HostedRunCommand,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = HostCommandOutcome> + Send + '_>>
        {
            Box::pin(async move {
                let mut failure = RunFailureContext::new(
                    RunFailureCode::GovernorDenied,
                    RunFailureStage::Govern,
                    4,
                    2,
                    Some(StableBoundaryKind::Pause),
                    "governor requested patch",
                );
                failure.node_refs.push("govern.swap".to_owned());

                HostCommandOutcome {
                    response: HostCommandResponse::Pause(PauseBundle {
                        schema: "ais-agent/pause_bundle/v2".to_owned(),
                        run_id: RunId("run-1".to_owned()),
                        kind: PauseKind::NeedUserInput,
                        interruption_class: None,
                        cancel_state: None,
                        side_effect_phase: None,
                        recovery_disposition: RecoveryDisposition::AwaitPatch,
                        summary: "governor requested patch".to_owned(),
                        ownership: RunOwnershipSnapshot {
                            run_id: RunId("run-1".to_owned()),
                            current_claim: None,
                            last_terminal_claim_id: None,
                            last_claim_transition: None,
                            claim_required_for_mutation: true,
                            owner_visibility: OwnershipVisibility::ObserverReadAllowed,
                        },
                        blocking_refs: vec!["govern.swap".to_owned()],
                        required_actions: vec![PauseActionView {
                            action_kind: RecoveryActionKind::SubmitPlanPatch,
                            action: "submit_plan_patch".to_owned(),
                            description: "submit a bounded plan patch".to_owned(),
                            requires_mutation_claim: true,
                            retry_intent: None,
                        }],
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
                        pending_signer_requests: Vec::new(),
                        pending_confirmations: Vec::new(),
                        notes: vec!["host review required".to_owned()],
                    }),
                    events: Vec::new(),
                }
            })
        }
    }

    impl HostRunEventService for RecoveryPauseShellHostService {
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
}
