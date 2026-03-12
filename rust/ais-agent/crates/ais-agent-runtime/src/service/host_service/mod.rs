//! Runtime-backed host service.

mod commands;
mod conversion;
mod error;
mod load;
mod persist;

use std::{future::Future, pin::Pin};

use ais_agent_host::{
    control::{HostCommandOutcome, HostCommandResponse, HostCommandService},
    events::{HostEventServiceError, HostRunEventBatch, HostRunEventQuery, HostRunEventService},
    session::{HostSessionStore, HostedRunCommand},
};

use crate::{
    persistence::{
        CheckpointRepository, EventArchive, MissionRepository, RunCatalogRepository,
        RuntimeAuditArchive, SignerStateArchive,
    },
    runtime::RunRepository,
    service::RuntimeCommandRouter,
};

pub use error::RuntimeHostServiceError;

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
    G = crate::persistence::InMemorySignerStateArchive,
    A = crate::persistence::InMemoryRuntimeAuditArchive,
> {
    run_repo: R,
    checkpoint_repo: C,
    mission_repo: M,
    run_catalog_repo: K,
    event_archive: E,
    session_store: S,
    signer_state_archive: G,
    audit_archive: A,
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
        crate::persistence::InMemorySignerStateArchive,
        crate::persistence::InMemoryRuntimeAuditArchive,
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
        Self::new_with_signer_archive(
            run_repo,
            checkpoint_repo,
            mission_repo,
            run_catalog_repo,
            event_archive,
            session_store,
            crate::persistence::InMemorySignerStateArchive::default(),
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
        crate::persistence::InMemorySignerStateArchive,
    ) {
        (
            self.run_repo,
            self.checkpoint_repo,
            self.mission_repo,
            self.run_catalog_repo,
            self.event_archive,
            self.session_store,
            self.signer_state_archive,
        )
    }
}

impl<R, C, M, K, E, S, G>
    RuntimeHostService<R, C, M, K, E, S, G, crate::persistence::InMemoryRuntimeAuditArchive>
where
    R: RunRepository + Send,
    C: CheckpointRepository + Send,
    M: MissionRepository + Send,
    K: RunCatalogRepository + Send,
    E: EventArchive + Send,
    S: HostSessionStore + Send,
    G: SignerStateArchive + Send,
{
    pub fn new_with_signer_archive(
        run_repo: R,
        checkpoint_repo: C,
        mission_repo: M,
        run_catalog_repo: K,
        event_archive: E,
        session_store: S,
        signer_state_archive: G,
    ) -> Self {
        Self::new_with_archives(
            run_repo,
            checkpoint_repo,
            mission_repo,
            run_catalog_repo,
            event_archive,
            session_store,
            signer_state_archive,
            crate::persistence::InMemoryRuntimeAuditArchive::default(),
        )
    }
}

impl<R, C, M, K, E, S, G, A> RuntimeHostService<R, C, M, K, E, S, G, A>
where
    R: RunRepository + Send,
    C: CheckpointRepository + Send,
    M: MissionRepository + Send,
    K: RunCatalogRepository + Send,
    E: EventArchive + Send,
    S: HostSessionStore + Send,
    G: SignerStateArchive + Send,
    A: RuntimeAuditArchive + Send,
{
    pub fn new_with_archives(
        run_repo: R,
        checkpoint_repo: C,
        mission_repo: M,
        run_catalog_repo: K,
        event_archive: E,
        session_store: S,
        signer_state_archive: G,
        audit_archive: A,
    ) -> Self {
        Self {
            run_repo,
            checkpoint_repo,
            mission_repo,
            run_catalog_repo,
            event_archive,
            session_store,
            signer_state_archive,
            audit_archive,
            next_run_seq: 1,
        }
    }

    pub fn into_parts_with_signer_archive(self) -> (R, C, M, K, E, S, G) {
        (
            self.run_repo,
            self.checkpoint_repo,
            self.mission_repo,
            self.run_catalog_repo,
            self.event_archive,
            self.session_store,
            self.signer_state_archive,
        )
    }
}

impl<R, C, M, K, E, S, G, A> HostCommandService for RuntimeHostService<R, C, M, K, E, S, G, A>
where
    R: RunRepository + Send,
    C: CheckpointRepository + Send,
    M: MissionRepository + Send,
    K: RunCatalogRepository + Send,
    E: EventArchive + Send,
    S: HostSessionStore + Send,
    G: SignerStateArchive + Send,
    A: RuntimeAuditArchive + Send,
{
    fn handle(
        &mut self,
        command: HostedRunCommand,
    ) -> Pin<Box<dyn Future<Output = HostCommandOutcome> + Send + '_>> {
        Box::pin(async move {
            let replay_key = conversion::replay_key(&command);
            let command_id = conversion::command_id(&command.command).clone();
            let run_id = conversion::command_run_id(&command.command);

            if let Some(key) = replay_key.as_ref() {
                match self.session_store.register_idempotency(
                    command.host_session_id.clone(),
                    key.clone(),
                    command_id.clone(),
                    run_id.clone(),
                ) {
                    ais_agent_host::session::IdempotencyResolution::Accepted => {}
                    ais_agent_host::session::IdempotencyResolution::Replay { outcome, .. } => {
                        return match outcome {
                            Some(outcome) => outcome,
                            None => {
                                RuntimeHostServiceError::IdempotencyReplayIncomplete.into_outcome()
                            }
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

            if let Some(key) = replay_key {
                match &result.response {
                    HostCommandResponse::Error(_) => {
                        self.session_store.clear_idempotency(&host_session_id, &key);
                    }
                    _ => {
                        let resolved_run_id = conversion::outcome_run_id(&result).or(run_id);
                        self.session_store.complete_idempotency(
                            &host_session_id,
                            &key,
                            result.clone(),
                            resolved_run_id,
                        );
                    }
                }
            }

            result
        })
    }
}

impl<R, C, M, K, E, S, G, A> HostRunEventService for RuntimeHostService<R, C, M, K, E, S, G, A>
where
    R: RunRepository + Send,
    C: CheckpointRepository + Send,
    M: MissionRepository + Send,
    K: RunCatalogRepository + Send,
    E: EventArchive + Send,
    S: HostSessionStore + Send,
    G: SignerStateArchive + Send,
    A: RuntimeAuditArchive + Send,
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
