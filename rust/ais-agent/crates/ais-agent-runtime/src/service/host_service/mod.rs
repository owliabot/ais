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

use ais_agent_host::{
    control::{HostCommandOutcome, HostCommandResponse, HostCommandService},
    events::{HostEventServiceError, HostRunEventBatch, HostRunEventQuery, HostRunEventService},
    session::{HostSessionStore, HostedRunCommand},
};

use crate::{
    persistence::{
        CheckpointRepository, EventArchive, MissionRepository, RunCatalogRepository,
        RunClaimRepository, RuntimeAuditArchive, SignerStateArchive,
    },
    runtime::RunRepository,
    service::RuntimeCommandRouter,
};

pub use api_native_evm_common::RuntimeExecutionWiring;
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
    G = crate::persistence::InMemorySignerStateArchive,
    A = crate::persistence::InMemoryRuntimeAuditArchive,
    Q = crate::persistence::InMemoryRunClaimRepository,
> {
    run_repo: R,
    checkpoint_repo: C,
    mission_repo: M,
    run_catalog_repo: K,
    event_archive: E,
    session_store: S,
    signer_state_archive: G,
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
        crate::persistence::InMemorySignerStateArchive,
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
        Self::new_with_archives_and_claim_repo(
            run_repo,
            checkpoint_repo,
            mission_repo,
            run_catalog_repo,
            event_archive,
            session_store,
            signer_state_archive,
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
        Self::new_with_archives_and_claim_repo(
            run_repo,
            checkpoint_repo,
            mission_repo,
            run_catalog_repo,
            event_archive,
            session_store,
            signer_state_archive,
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
    G: SignerStateArchive + Send,
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
        signer_state_archive: G,
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
            signer_state_archive,
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

    pub fn into_parts_with_claim_repo(self) -> (R, C, M, K, E, S, G, A, Q) {
        (
            self.run_repo,
            self.checkpoint_repo,
            self.mission_repo,
            self.run_catalog_repo,
            self.event_archive,
            self.session_store,
            self.signer_state_archive,
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
    G: SignerStateArchive + Send,
    A: RuntimeAuditArchive + Send,
    Q: RunClaimRepository + Send,
{
    fn handle(
        &mut self,
        command: HostedRunCommand,
    ) -> Pin<Box<dyn Future<Output = HostCommandOutcome> + Send + '_>> {
        Box::pin(async move {
            let replay_key = conversion::replay_key(&command);
            let command_id = conversion::command_id(&command.command).clone();
            let run_id = conversion::command_run_id(&command.command);
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
        })
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
    G: SignerStateArchive + Send,
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
