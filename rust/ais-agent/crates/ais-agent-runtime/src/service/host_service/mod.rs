//! Runtime-backed host service.

mod api_native_evm_common;
pub(crate) mod artifact_planner;
mod commands;
mod conversion;
mod error;
mod launch_binding;
mod launch_spec;
mod launch_validation;
mod load;
mod persist;

use std::{future::Future, pin::Pin};

use ais_agent_control::ids::RunId;
use ais_agent_host::{
    control::{HostCommandOutcome, HostCommandResponse, HostCommandService},
    events::{HostEventServiceError, HostRunEventBatch, HostRunEventQuery, HostRunEventService},
    session::{HostSessionStore, HostedRunCommand},
};
use tracing::{debug, info_span, warn, Instrument};

use crate::{
    persistence::{
        CheckpointRepository, EventArchive, MissionRepository, RunCatalogRepository,
        RunClaimRepository, RuntimeAuditArchive, SignerStateStore,
    },
    runtime::RunRepository,
    service::RuntimeCommandRouter,
};

#[allow(unused_imports)]
pub use api_native_evm_common::{
    RuntimeChainConnection, RuntimeChainConnectionRef, RuntimeChainProviderEntry,
    RuntimeExecutionWiring, RuntimeProviderRegistry,
};
pub use error::RuntimeHostServiceError;
#[cfg(test)]
pub(crate) use launch_spec::seed_launch_spec_checkpoint;

pub type RuntimeHostServiceResult = Result<HostCommandOutcome, RuntimeHostServiceError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DurableCheckpointWrite {
    Boundary,
    Progress,
}

#[derive(Debug)]
pub struct RuntimeHostService<
    R,
    C,
    M,
    K,
    E,
    S,
    G = crate::persistence::InMemorySignerStateStore,
    A = crate::persistence::InMemoryRuntimeAuditArchive,
    Q = crate::persistence::InMemoryRunClaimRepository,
> {
    run_repo: R,
    checkpoint_repo: C,
    mission_repo: M,
    run_catalog_repo: K,
    event_archive: E,
    session_store: S,
    signer_state_store: G,
    audit_archive: A,
    claim_repo: Q,
    execution_wiring: RuntimeExecutionWiring,
    next_run_seq: u64,
}

impl<R, C, M, K, E, S>
    RuntimeHostService<
        R,
        C,
        M,
        K,
        E,
        S,
        crate::persistence::InMemorySignerStateStore,
        crate::persistence::InMemoryRuntimeAuditArchive,
        crate::persistence::InMemoryRunClaimRepository,
    >
where
    R: RunRepository + Send,
    C: CheckpointRepository + Send,
    M: MissionRepository + Send,
    K: RunCatalogRepository + Send,
    E: EventArchive + Send,
    S: HostSessionStore + Send,
{
    pub fn new(
        run_repo: R,
        checkpoint_repo: C,
        mission_repo: M,
        run_catalog_repo: K,
        event_archive: E,
        session_store: S,
    ) -> Self {
        Self::new_with_signer_state_store(
            run_repo,
            checkpoint_repo,
            mission_repo,
            run_catalog_repo,
            event_archive,
            session_store,
            crate::persistence::InMemorySignerStateStore::default(),
        )
    }

    pub fn into_parts(
        self,
    ) -> (
        R,
        C,
        M,
        K,
        E,
        S,
        crate::persistence::InMemorySignerStateStore,
    ) {
        (
            self.run_repo,
            self.checkpoint_repo,
            self.mission_repo,
            self.run_catalog_repo,
            self.event_archive,
            self.session_store,
            self.signer_state_store,
        )
    }
}

impl<R, C, M, K, E, S, G>
    RuntimeHostService<
        R,
        C,
        M,
        K,
        E,
        S,
        G,
        crate::persistence::InMemoryRuntimeAuditArchive,
        crate::persistence::InMemoryRunClaimRepository,
    >
where
    R: RunRepository + Send,
    C: CheckpointRepository + Send,
    M: MissionRepository + Send,
    K: RunCatalogRepository + Send,
    E: EventArchive + Send,
    S: HostSessionStore + Send,
    G: SignerStateStore + Send,
{
    pub fn new_with_signer_state_store(
        run_repo: R,
        checkpoint_repo: C,
        mission_repo: M,
        run_catalog_repo: K,
        event_archive: E,
        session_store: S,
        signer_state_store: G,
    ) -> Self {
        Self::new_with_archives_and_claim_repo(
            run_repo,
            checkpoint_repo,
            mission_repo,
            run_catalog_repo,
            event_archive,
            session_store,
            signer_state_store,
            crate::persistence::InMemoryRuntimeAuditArchive::default(),
            crate::persistence::InMemoryRunClaimRepository::default(),
        )
    }
}

impl<R, C, M, K, E, S, G, A>
    RuntimeHostService<R, C, M, K, E, S, G, A, crate::persistence::InMemoryRunClaimRepository>
where
    R: RunRepository + Send,
    C: CheckpointRepository + Send,
    M: MissionRepository + Send,
    K: RunCatalogRepository + Send,
    E: EventArchive + Send,
    S: HostSessionStore + Send,
    G: SignerStateStore + Send,
    A: RuntimeAuditArchive + Send,
{
    pub fn new_with_archives(
        run_repo: R,
        checkpoint_repo: C,
        mission_repo: M,
        run_catalog_repo: K,
        event_archive: E,
        session_store: S,
        signer_state_store: G,
        audit_archive: A,
    ) -> Self {
        Self::new_with_archives_and_claim_repo(
            run_repo,
            checkpoint_repo,
            mission_repo,
            run_catalog_repo,
            event_archive,
            session_store,
            signer_state_store,
            audit_archive,
            crate::persistence::InMemoryRunClaimRepository::default(),
        )
    }
}

impl<R, C, M, K, E, S, G, A, Q> RuntimeHostService<R, C, M, K, E, S, G, A, Q>
where
    R: RunRepository + Send,
    C: CheckpointRepository + Send,
    M: MissionRepository + Send,
    K: RunCatalogRepository + Send,
    E: EventArchive + Send,
    S: HostSessionStore + Send,
    G: SignerStateStore + Send,
    A: RuntimeAuditArchive + Send,
    Q: RunClaimRepository + Send,
{
    pub fn new_with_archives_and_claim_repo(
        run_repo: R,
        checkpoint_repo: C,
        mission_repo: M,
        run_catalog_repo: K,
        event_archive: E,
        session_store: S,
        signer_state_store: G,
        audit_archive: A,
        claim_repo: Q,
    ) -> Self {
        Self {
            run_repo,
            checkpoint_repo,
            mission_repo,
            run_catalog_repo,
            event_archive,
            session_store,
            signer_state_store,
            audit_archive,
            claim_repo,
            execution_wiring: RuntimeExecutionWiring::default(),
            next_run_seq: 1,
        }
    }

    pub fn with_execution_wiring(mut self, execution_wiring: RuntimeExecutionWiring) -> Self {
        self.execution_wiring = execution_wiring;
        self
    }

    pub fn into_parts_with_signer_state_store(self) -> (R, C, M, K, E, S, G) {
        (
            self.run_repo,
            self.checkpoint_repo,
            self.mission_repo,
            self.run_catalog_repo,
            self.event_archive,
            self.session_store,
            self.signer_state_store,
        )
    }

    pub fn into_parts_with_claim_repo(self) -> (R, C, M, K, E, S, G, A, Q) {
        (
            self.run_repo,
            self.checkpoint_repo,
            self.mission_repo,
            self.run_catalog_repo,
            self.event_archive,
            self.session_store,
            self.signer_state_store,
            self.audit_archive,
            self.claim_repo,
        )
    }
}

impl<R, C, M, K, E, S, G, A, Q> HostCommandService for RuntimeHostService<R, C, M, K, E, S, G, A, Q>
where
    R: RunRepository + Send,
    C: CheckpointRepository + Send,
    M: MissionRepository + Send,
    K: RunCatalogRepository + Send,
    E: EventArchive + Send,
    S: HostSessionStore + Send,
    G: SignerStateStore + Send,
    A: RuntimeAuditArchive + Send,
    Q: RunClaimRepository + Send,
{
    fn handle(
        &mut self,
        command: HostedRunCommand,
    ) -> Pin<Box<dyn Future<Output = HostCommandOutcome> + Send + '_>> {
        let command_id = conversion::command_id(&command.command).clone();
        let run_id = conversion::command_run_id(&command.command);
        let command_kind = command.command.kind().to_owned();
        let host_command_span = info_span!(
            "host.command",
            host_session_id = %command.host_session_id.0,
            host_request_id = ?command.host_request_id.as_ref().map(|id| id.0.as_str()),
            command_id = %command_id.0,
            run_id = ?run_id.as_ref().map(|id| id.0.as_str()),
        );
        Box::pin(
            async move {
                debug!(
                    host_session_id = %command.host_session_id.0,
                    host_request_id = ?command.host_request_id.as_ref().map(|id| id.0.as_str()),
                    command_id = %command_id.0,
                    command_kind = %command_kind,
                    run_id = ?run_id.as_ref().map(|id| id.0.as_str()),
                    "runtime.host.command.start"
                );
                let replay_key = conversion::replay_key(&command);
                let replay_claim_id = match replay_key.as_ref() {
                    Some(_) => Some(self.idempotency_claim_id_for_command(&command.command)),
                    None => None,
                };

                let replay_claim_id = match replay_claim_id {
                    Some(Ok(claim_id)) => Some(claim_id),
                    Some(Err(error)) => return error.into_outcome(),
                    None => None,
                };

                if let Some(key) = replay_key.as_ref() {
                    match self.session_store.register_idempotency(
                        command.host_session_id.clone(),
                        key.clone(),
                        command_id.clone(),
                        run_id.clone(),
                        replay_claim_id.clone().flatten(),
                    ) {
                        ais_agent_host::session::IdempotencyResolution::Accepted => {}
                        ais_agent_host::session::IdempotencyResolution::Replay {
                            outcome, ..
                        } => {
                            return match outcome {
                                Some(outcome) => outcome,
                                None => RuntimeHostServiceError::IdempotencyReplayIncomplete
                                    .into_outcome(),
                            };
                        }
                        ais_agent_host::session::IdempotencyResolution::Conflict { .. } => {
                            return RuntimeHostServiceError::IdempotencyConflict.into_outcome();
                        }
                    }
                }

                let host_session_id = command.host_session_id.clone();
                let result = RuntimeCommandRouter::into_outcome(
                    RuntimeCommandRouter::route(self, command).await,
                );
                log_command_outcome(&command_kind, &result, run_id.as_ref());

                if let Some(key) = replay_key {
                    match &result.response {
                        HostCommandResponse::Error(_) => {
                            self.session_store.clear_idempotency(&host_session_id, &key);
                        }
                        _ => {
                            let resolved_run_id = conversion::outcome_run_id(&result).or(run_id);
                            let resolved_replay_claim_id = conversion::completed_replay_claim_id(
                                &result,
                                replay_claim_id.flatten(),
                            );
                            self.session_store.complete_idempotency(
                                &host_session_id,
                                &key,
                                result.clone(),
                                resolved_run_id,
                                resolved_replay_claim_id,
                            );
                        }
                    }
                }

                result
            }
            .instrument(host_command_span),
        )
    }
}

fn log_command_outcome(
    command_kind: &str,
    result: &HostCommandOutcome,
    requested_run_id: Option<&RunId>,
) {
    let run_id = conversion::outcome_run_id(result)
        .or_else(|| requested_run_id.cloned())
        .map(|id| id.0);
    let checkpoint_seq = response_checkpoint_seq(&result.response);
    let response_kind = response_kind(&result.response);

    match &result.response {
        HostCommandResponse::Error(error) => {
            warn!(
                command_kind = command_kind,
                response_kind,
                run_id = ?run_id,
                checkpoint_seq = ?checkpoint_seq,
                error_code = %error.code,
                event_count = result.events.len(),
                "runtime.host.command_rejected"
            );
        }
        _ => {
            debug!(
                command_kind = command_kind,
                response_kind,
                run_id = ?run_id,
                checkpoint_seq = ?checkpoint_seq,
                event_count = result.events.len(),
                "runtime.host.command_accepted"
            );
        }
    }
}

fn response_kind(response: &HostCommandResponse) -> &'static str {
    match response {
        HostCommandResponse::Accepted(_) => "accepted",
        HostCommandResponse::Inspect(_) => "inspect",
        HostCommandResponse::Pause(_) => "pause",
        HostCommandResponse::Session(_) => "session",
        HostCommandResponse::Error(_) => "error",
    }
}

fn response_checkpoint_seq(response: &HostCommandResponse) -> Option<u64> {
    match response {
        HostCommandResponse::Inspect(snapshot) => Some(snapshot.checkpoint_seq),
        HostCommandResponse::Accepted(_)
        | HostCommandResponse::Pause(_)
        | HostCommandResponse::Session(_)
        | HostCommandResponse::Error(_) => None,
    }
}

impl<R, C, M, K, E, S, G, A, Q> HostRunEventService
    for RuntimeHostService<R, C, M, K, E, S, G, A, Q>
where
    R: RunRepository + Send,
    C: CheckpointRepository + Send,
    M: MissionRepository + Send,
    K: RunCatalogRepository + Send,
    E: EventArchive + Send,
    S: HostSessionStore + Send,
    G: SignerStateStore + Send,
    A: RuntimeAuditArchive + Send,
    Q: RunClaimRepository + Send,
{
    fn list_events(
        &self,
        query: HostRunEventQuery,
    ) -> Pin<Box<dyn Future<Output = Result<HostRunEventBatch, HostEventServiceError>> + Send + '_>>
    {
        let response = self.load_event_batch(query);
        Box::pin(async move { response })
    }
}

#[cfg(test)]
mod tests {
    use ais_agent_control::ids::RunId;
    use ais_agent_host::control::{
        HostAcceptedResponse, HostCommandError, HostCommandOutcome, HostCommandResponse,
    };

    use crate::tests::tracing_capture::capture_tracing_output_at_level;

    #[test]
    fn response_kind_classifies_host_responses() {
        assert_eq!(
            super::response_kind(&HostCommandResponse::Accepted(HostAcceptedResponse {
                run_id: None,
                message: None,
                session: None,
            })),
            "accepted"
        );
        assert_eq!(
            super::response_kind(&HostCommandResponse::Error(HostCommandError {
                code: "claim_conflict".to_owned(),
                message: "x".to_owned(),
            })),
            "error"
        );
    }

    #[test]
    fn response_checkpoint_seq_is_only_available_for_inspect() {
        assert_eq!(
            super::response_checkpoint_seq(&HostCommandResponse::Accepted(HostAcceptedResponse {
                run_id: None,
                message: None,
                session: None,
            })),
            None
        );
    }

    #[test]
    fn log_command_outcome_demotes_success_to_debug() {
        let outcome = HostCommandOutcome {
            response: HostCommandResponse::Accepted(HostAcceptedResponse {
                run_id: Some(RunId("run-accepted".to_owned())),
                message: None,
                session: None,
            }),
            events: Vec::new(),
        };

        let (info_output, ()) = capture_tracing_output_at_level(tracing::Level::INFO, || {
            super::log_command_outcome("step_run", &outcome, None);
        });
        let (debug_output, ()) = capture_tracing_output_at_level(tracing::Level::DEBUG, || {
            super::log_command_outcome("step_run", &outcome, None);
        });

        assert!(!info_output.contains("runtime.host.command_accepted"));
        assert!(debug_output.contains("runtime.host.command_accepted"));
        assert!(debug_output.contains("run-accepted"));
    }

    #[test]
    fn log_command_outcome_keeps_rejection_visible() {
        let outcome = HostCommandOutcome {
            response: HostCommandResponse::Error(HostCommandError {
                code: "claim_conflict".to_owned(),
                message: "run claimed elsewhere".to_owned(),
            }),
            events: Vec::new(),
        };

        let (output, ()) = capture_tracing_output_at_level(tracing::Level::INFO, || {
            super::log_command_outcome(
                "step_run",
                &outcome,
                Some(&RunId("run-rejected".to_owned())),
            );
        });

        assert!(output.contains("runtime.host.command_rejected"));
        assert!(output.contains("error_code=claim_conflict"));
        assert!(output.contains("run_id=Some(\"run-rejected\")"));
    }
}
