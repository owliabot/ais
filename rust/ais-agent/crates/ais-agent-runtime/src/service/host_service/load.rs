use std::time::{SystemTime, UNIX_EPOCH};

use ais_agent_control::{
    commands::RunCommand,
    ids::{ClaimId, RunId},
    ownership::{ClaimTransitionKind, OwnershipErrorCode, RunClaim, RunClaimMode, RunClaimStatus},
};
use ais_agent_core::{checkpoint::CheckpointSnapshot, mission::Mission, ownership::ClaimPolicy};
use ais_agent_host::{
    events::{HostEventServiceError, HostRunEventBatch, HostRunEventQuery},
    inspect::{InspectSnapshot, PauseBundle},
    session::{HostRunLink, HostSessionId},
};
use tracing::{debug, info};

use crate::{
    persistence::{
        restore_active_run, EventArchiveError, EventArchiveQuery, MissionRepositoryError,
    },
    runtime::{ActiveRun, RunRepositoryError},
};

use super::{conversion, RuntimeHostService, RuntimeHostServiceError};

impl<R, C, M, K, E, S, G, A, Q> RuntimeHostService<R, C, M, K, E, S, G, A, Q>
where
    R: crate::runtime::RunRepository + Send,
    C: crate::persistence::CheckpointRepository + Send,
    M: crate::persistence::MissionRepository + Send,
    K: crate::persistence::RunCatalogRepository + Send,
    E: crate::persistence::EventArchive + Send,
    S: ais_agent_host::session::HostSessionStore + Send,
    G: crate::persistence::SignerStateArchive + Send,
    A: crate::persistence::RuntimeAuditArchive + Send,
    Q: crate::persistence::RunClaimRepository + Send,
{
    pub(super) fn establish_inspect_session_link(
        &mut self,
        host_session_id: &HostSessionId,
        run_id: &RunId,
        mission: &Mission,
    ) -> Result<(), RuntimeHostServiceError> {
        match self.session_store.run_link(run_id) {
            Some(link) if &link.host_session_id == host_session_id => Ok(()),
            Some(_) => Err(RuntimeHostServiceError::SessionRunMismatch {
                host_session_id: host_session_id.0.clone(),
                run_id: run_id.0.clone(),
            }),
            None => {
                self.session_store.link_run(HostRunLink::new(
                    host_session_id.clone(),
                    run_id.clone(),
                    mission.goal.clone(),
                    mission.allowed_chains.clone(),
                ));
                info!(
                    run_id = %run_id.0,
                    host_session_id = %host_session_id.0,
                    "runtime.host.session_relinked_on_inspect"
                );
                Ok(())
            }
        }
    }

    pub(super) fn ensure_mutation_session_link(
        &self,
        host_session_id: &HostSessionId,
        run_id: &RunId,
    ) -> Result<(), RuntimeHostServiceError> {
        let Some(link) = self.session_store.run_link(run_id) else {
            return if self.run_identity_exists(run_id)? {
                Err(RuntimeHostServiceError::SessionRelinkRequired {
                    host_session_id: host_session_id.0.clone(),
                    run_id: run_id.0.clone(),
                })
            } else {
                Err(RuntimeHostServiceError::RunNotFound {
                    run_id: run_id.0.clone(),
                })
            };
        };
        if &link.host_session_id != host_session_id {
            return Err(RuntimeHostServiceError::SessionRunMismatch {
                host_session_id: host_session_id.0.clone(),
                run_id: run_id.0.clone(),
            });
        }
        Ok(())
    }

    pub(super) fn force_session_link(
        &mut self,
        host_session_id: &HostSessionId,
        run_id: &RunId,
        mission: &Mission,
    ) {
        self.session_store.link_run(HostRunLink::new(
            host_session_id.clone(),
            run_id.clone(),
            mission.goal.clone(),
            mission.allowed_chains.clone(),
        ));
    }

    pub(super) fn load_or_restore_active_run(
        &mut self,
        run_id: &RunId,
    ) -> Result<ActiveRun, RuntimeHostServiceError> {
        let hot = self.load_hot_runtime(run_id)?;
        if let Some(runtime) = hot.as_ref() {
            if !self.durable_checkpoint_is_newer(run_id, &runtime.checkpoint)? {
                debug!(
                    run_id = %run_id.0,
                    source = "hot_cache",
                    checkpoint_seq = runtime.checkpoint.checkpoint_seq,
                    plan_epoch = runtime.checkpoint.plan_epoch,
                    revision = runtime.revision,
                    "runtime.host.load_or_restore_active_run"
                );
                return Ok(runtime.clone());
            }
            info!(
                run_id = %run_id.0,
                hot_checkpoint_seq = runtime.checkpoint.checkpoint_seq,
                hot_plan_epoch = runtime.checkpoint.plan_epoch,
                "runtime.host.durable_checkpoint_newer_than_hot"
            );
        } else {
            info!(run_id = %run_id.0, "runtime.host.hot_runtime_miss");
        }

        self.restore_and_cache_active_run(run_id, hot.is_some())
    }

    pub(super) fn load_inspect_projection_input(
        &mut self,
        run_id: &RunId,
    ) -> Result<(Mission, CheckpointSnapshot), RuntimeHostServiceError> {
        let hot = self.load_hot_runtime(run_id)?;
        if let Some(runtime) = hot {
            if !self.durable_checkpoint_is_newer(run_id, &runtime.checkpoint)? {
                debug!(
                    run_id = %run_id.0,
                    source = "hot_cache",
                    checkpoint_seq = runtime.checkpoint.checkpoint_seq,
                    plan_epoch = runtime.checkpoint.plan_epoch,
                    "runtime.host.inspect_projection_input_hot"
                );
                return Ok((runtime.mission, runtime.checkpoint));
            }
            info!(
                run_id = %run_id.0,
                hot_checkpoint_seq = runtime.checkpoint.checkpoint_seq,
                hot_plan_epoch = runtime.checkpoint.plan_epoch,
                "runtime.host.inspect_projection_input_durable"
            );
        } else {
            info!(run_id = %run_id.0, "runtime.host.inspect_projection_input_hot_miss");
        }

        Ok((
            self.mission_repo.load(run_id)?,
            self.checkpoint_repo.latest(&run_id.0)?,
        ))
    }

    pub(super) fn allocate_next_run_seq(&mut self) -> Result<u64, RuntimeHostServiceError> {
        loop {
            let run_seq = self.next_run_seq;
            self.next_run_seq = self.next_run_seq.saturating_add(1);
            let run_id = RunId(format!("run-{run_seq}"));
            if !self.run_identity_exists(&run_id)? {
                return Ok(run_seq);
            }
        }
    }

    pub(super) fn run_identity_exists(
        &self,
        run_id: &RunId,
    ) -> Result<bool, RuntimeHostServiceError> {
        let hot_exists = match self.run_repo.load(run_id) {
            Ok(_) => true,
            Err(RunRepositoryError::NotFound { .. }) => false,
            Err(other) => return Err(RuntimeHostServiceError::Repository(other)),
        };
        if hot_exists {
            return Ok(true);
        }

        match self.mission_repo.load(run_id) {
            Ok(_) => Ok(true),
            Err(MissionRepositoryError::NotFound { .. }) => Ok(false),
            Err(other) => Err(RuntimeHostServiceError::Mission(other)),
        }
    }

    pub(super) fn load_hot_runtime(
        &self,
        run_id: &RunId,
    ) -> Result<Option<ActiveRun>, RuntimeHostServiceError> {
        match self.run_repo.load(run_id) {
            Ok(runtime) => Ok(Some(runtime)),
            Err(RunRepositoryError::NotFound { .. }) => Ok(None),
            Err(other) => Err(RuntimeHostServiceError::Repository(other)),
        }
    }

    pub(super) fn durable_checkpoint_is_newer(
        &self,
        run_id: &RunId,
        hot_checkpoint: &CheckpointSnapshot,
    ) -> Result<bool, RuntimeHostServiceError> {
        match self.checkpoint_repo.latest(&run_id.0) {
            Ok(checkpoint) => Ok(conversion::checkpoint_is_newer(&checkpoint, hot_checkpoint)),
            Err(crate::persistence::CheckpointRepositoryError::NotFound { .. }) => Ok(false),
            Err(other) => Err(RuntimeHostServiceError::Checkpoint(other)),
        }
    }

    pub(super) fn restore_and_cache_active_run(
        &mut self,
        run_id: &RunId,
        replace_existing_hot: bool,
    ) -> Result<ActiveRun, RuntimeHostServiceError> {
        let mut restored = restore_active_run(
            run_id,
            &self.mission_repo,
            &self.checkpoint_repo,
            &self.signer_state_archive,
        )?;
        restored.event_seq = self.archived_latest_event_seq(run_id)?;
        if replace_existing_hot {
            self.run_repo.save(restored.clone(), None)?;
        } else {
            self.run_repo.insert(restored.clone())?;
        }
        info!(
            run_id = %run_id.0,
            checkpoint_seq = restored.checkpoint.checkpoint_seq,
            plan_epoch = restored.checkpoint.plan_epoch,
            revision = restored.revision,
            event_seq = restored.event_seq,
            replace_existing_hot,
            "runtime.host.restore_and_cache_active_run"
        );
        Ok(restored)
    }

    pub(super) fn archived_latest_event_seq(
        &self,
        run_id: &RunId,
    ) -> Result<u64, RuntimeHostServiceError> {
        match self.event_archive.read(EventArchiveQuery {
            run_id: run_id.clone(),
            after_event_seq: None,
            limit: Some(1),
        }) {
            Ok(slice) => Ok(slice.latest_event_seq.unwrap_or_default()),
            Err(EventArchiveError::NotFound { .. }) => Ok(0),
            Err(other) => Err(RuntimeHostServiceError::EventArchive(other)),
        }
    }

    pub(super) fn load_event_batch(
        &self,
        query: HostRunEventQuery,
    ) -> Result<HostRunEventBatch, HostEventServiceError> {
        match self.event_archive.read(EventArchiveQuery {
            run_id: query.run_id.clone(),
            after_event_seq: query.after_event_seq,
            limit: query.limit,
        }) {
            Ok(slice) => Ok(conversion::host_event_batch(query, slice)),
            Err(EventArchiveError::NotFound { .. }) => {
                self.mission_repo
                    .load(&query.run_id)
                    .map_err(|error| match error {
                        MissionRepositoryError::NotFound { run_id } => HostEventServiceError {
                            code: "run_not_found".to_owned(),
                            message: format!("run `{run_id}` not found"),
                        },
                        other => HostEventServiceError {
                            code: "event_query_failed".to_owned(),
                            message: other.to_string(),
                        },
                    })?;

                Ok(HostRunEventBatch {
                    run_id: query.run_id,
                    after_event_seq: query.after_event_seq,
                    latest_event_seq: None,
                    next_after_event_seq: None,
                    truncated: false,
                    events: Vec::new(),
                })
            }
            Err(other) => Err(HostEventServiceError {
                code: "event_query_failed".to_owned(),
                message: other.to_string(),
            }),
        }
    }

    pub(super) fn claim_now_ms() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }

    pub(super) fn default_claim_lease_ms() -> u64 {
        5 * 60 * 1_000
    }

    pub(super) fn requested_claim_lease_ms(requested_lease_ms: Option<u64>) -> u64 {
        requested_lease_ms.unwrap_or_else(Self::default_claim_lease_ms)
    }

    pub(super) fn seed_initial_claim(
        &mut self,
        host_session_id: &HostSessionId,
        run_id: &RunId,
    ) -> Result<RunClaim, RuntimeHostServiceError> {
        let lease_started_at_ms = Self::claim_now_ms();
        self.acquire_claim(
            host_session_id,
            RunClaim {
                claim_id: ClaimId(format!("claim-{}-1", run_id.0)),
                run_id: run_id.clone(),
                host_session_id: host_session_id.0.clone(),
                owner_kind: ais_agent_control::ownership::RunClaimOwnerKind::InteractiveHost,
                owner_instance_id: host_session_id.0.clone(),
                lease_started_at_ms,
                lease_expires_at_ms: Some(lease_started_at_ms + Self::default_claim_lease_ms()),
                last_renewed_at_ms: Some(lease_started_at_ms),
                claim_epoch: 1,
                mode: ais_agent_control::ownership::RunClaimMode::ExclusiveMutation,
                status: RunClaimStatus::Active,
            },
        )
    }

    pub(super) fn acquire_claim(
        &mut self,
        host_session_id: &HostSessionId,
        claim: RunClaim,
    ) -> Result<RunClaim, RuntimeHostServiceError> {
        let run_id = claim.run_id.0.clone();
        self.claim_repo
            .acquire(claim)
            .map_err(|error| Self::map_claim_error(host_session_id, &run_id, error))
    }

    pub(super) fn next_claim_id(&self, run_id: &RunId, host_session_id: &HostSessionId) -> ClaimId {
        ClaimId(format!(
            "claim-{}-{}-{}",
            run_id.0,
            host_session_id.0,
            Self::claim_now_ms()
        ))
    }

    pub(super) fn build_claim(
        &self,
        host_session_id: &HostSessionId,
        run_id: &RunId,
        owner_kind: ais_agent_control::ownership::RunClaimOwnerKind,
        owner_instance_id: String,
        mode: RunClaimMode,
        requested_lease_ms: Option<u64>,
    ) -> RunClaim {
        let lease_started_at_ms = Self::claim_now_ms();
        let lease_ms = Self::requested_claim_lease_ms(requested_lease_ms);
        RunClaim {
            claim_id: self.next_claim_id(run_id, host_session_id),
            run_id: run_id.clone(),
            host_session_id: host_session_id.0.clone(),
            owner_kind,
            owner_instance_id,
            lease_started_at_ms,
            lease_expires_at_ms: Some(lease_started_at_ms.saturating_add(lease_ms)),
            last_renewed_at_ms: Some(lease_started_at_ms),
            claim_epoch: 1,
            mode,
            status: RunClaimStatus::Active,
        }
    }

    pub(super) fn claim_policy(
        &mut self,
        run_id: &RunId,
    ) -> Result<ClaimPolicy, RuntimeHostServiceError> {
        let (_, checkpoint) = self.load_inspect_projection_input(run_id)?;
        Ok(ais_agent_core::ownership::classify_claim_policy(
            &checkpoint,
        ))
    }

    pub(super) fn expire_stale_claim_if_needed(
        &mut self,
        host_session_id: &HostSessionId,
        run_id: &RunId,
    ) -> Result<Option<RunClaim>, RuntimeHostServiceError> {
        self.claim_repo
            .expire_stale(crate::persistence::ClaimExpireRequest {
                run_id: run_id.clone(),
                now_ms: Self::claim_now_ms(),
            })
            .map_err(|error| Self::map_claim_error(host_session_id, &run_id.0, error))
    }

    fn map_claim_error(
        host_session_id: &HostSessionId,
        run_id: &str,
        error: crate::persistence::RunClaimRepositoryError,
    ) -> RuntimeHostServiceError {
        let code = match error {
            crate::persistence::RunClaimRepositoryError::ActiveClaimConflict { .. } => {
                OwnershipErrorCode::ClaimConflict
            }
            crate::persistence::RunClaimRepositoryError::ClaimEpochConflict { .. } => {
                OwnershipErrorCode::ClaimEpochStale
            }
            crate::persistence::RunClaimRepositoryError::ClaimNotFound { .. } => {
                OwnershipErrorCode::ClaimRequired
            }
            crate::persistence::RunClaimRepositoryError::InvalidStatus { .. } => {
                OwnershipErrorCode::ClaimTransferRequired
            }
            crate::persistence::RunClaimRepositoryError::InvalidClaim { .. }
            | crate::persistence::RunClaimRepositoryError::Storage { .. } => {
                OwnershipErrorCode::ClaimRequired
            }
        };
        RuntimeHostServiceError::OwnershipViolation {
            code,
            run_id: run_id.to_owned(),
            message: format!("session `{}`: {error}", host_session_id.0),
        }
    }

    pub(super) fn load_effective_claim(
        &self,
        run_id: &RunId,
    ) -> Result<Option<RunClaim>, RuntimeHostServiceError> {
        let Some(mut claim) = self.claim_repo.load_active(run_id).map_err(|error| {
            RuntimeHostServiceError::OwnershipViolation {
                code: OwnershipErrorCode::ClaimRequired,
                run_id: run_id.0.clone(),
                message: error.to_string(),
            }
        })?
        else {
            return Ok(None);
        };

        if claim
            .lease_expires_at_ms
            .map(|expires| expires <= Self::claim_now_ms())
            .unwrap_or(false)
        {
            claim.status = RunClaimStatus::Expired;
            return Ok(Some(claim));
        }

        Ok(Some(claim))
    }

    fn bootstrap_legacy_mutation_claim(
        &mut self,
        host_session_id: &HostSessionId,
        run_id: &RunId,
    ) -> Result<RunClaim, RuntimeHostServiceError> {
        let lease_started_at_ms = Self::claim_now_ms();
        self.acquire_claim(
            host_session_id,
            RunClaim {
                claim_id: ClaimId(format!(
                    "claim-{}-bootstrap-{}",
                    run_id.0, lease_started_at_ms
                )),
                run_id: run_id.clone(),
                host_session_id: host_session_id.0.clone(),
                owner_kind: ais_agent_control::ownership::RunClaimOwnerKind::InteractiveHost,
                owner_instance_id: host_session_id.0.clone(),
                lease_started_at_ms,
                lease_expires_at_ms: Some(lease_started_at_ms + Self::default_claim_lease_ms()),
                last_renewed_at_ms: Some(lease_started_at_ms),
                claim_epoch: 1,
                mode: ais_agent_control::ownership::RunClaimMode::ExclusiveMutation,
                status: RunClaimStatus::Active,
            },
        )
    }

    pub(super) fn ensure_mutation_claim(
        &mut self,
        host_session_id: &HostSessionId,
        run_id: &RunId,
    ) -> Result<RunClaim, RuntimeHostServiceError> {
        if let Some(expired) = self.expire_stale_claim_if_needed(host_session_id, run_id)? {
            return Err(RuntimeHostServiceError::OwnershipViolation {
                code: OwnershipErrorCode::ClaimExpired,
                run_id: run_id.0.clone(),
                message: format!("active claim `{}` has expired", expired.claim_id.0),
            });
        }
        let Some(claim) = self.load_effective_claim(run_id)? else {
            return match self.latest_claim_for_run(run_id)? {
                Some(previous_claim) => Err(RuntimeHostServiceError::OwnershipViolation {
                    code: OwnershipErrorCode::ClaimRequired,
                    run_id: run_id.0.clone(),
                    message: format!(
                        "run `{}` requires an explicit claim after {:?} `{}`",
                        run_id.0, previous_claim.status, previous_claim.claim_id.0
                    ),
                }),
                None => self.bootstrap_legacy_mutation_claim(host_session_id, run_id),
            };
        };

        if claim.status == RunClaimStatus::Expired {
            return Err(RuntimeHostServiceError::OwnershipViolation {
                code: OwnershipErrorCode::ClaimExpired,
                run_id: run_id.0.clone(),
                message: format!("active claim `{}` has expired", claim.claim_id.0),
            });
        }
        if !claim.mode.allows_mutation() {
            return Err(RuntimeHostServiceError::OwnershipViolation {
                code: OwnershipErrorCode::ObserverOnly,
                run_id: run_id.0.clone(),
                message: format!("claim `{}` is observer_only", claim.claim_id.0),
            });
        }
        if claim.host_session_id != host_session_id.0 {
            return Err(RuntimeHostServiceError::OwnershipViolation {
                code: OwnershipErrorCode::ClaimConflict,
                run_id: run_id.0.clone(),
                message: format!(
                    "active claim `{}` belongs to session `{}`",
                    claim.claim_id.0, claim.host_session_id
                ),
            });
        }

        Ok(claim)
    }

    pub(super) fn hydrate_inspect_ownership(
        &self,
        snapshot: &mut InspectSnapshot,
    ) -> Result<(), RuntimeHostServiceError> {
        if let Some(claim) = self.load_effective_claim(&snapshot.run_id)? {
            if claim.status == RunClaimStatus::Expired {
                snapshot.ownership.last_terminal_claim_id = Some(claim.claim_id.clone());
                snapshot.ownership.last_claim_transition = Some(ClaimTransitionKind::ClaimExpired);
            }
            snapshot.ownership.current_claim = Some(claim);
        } else if let Some(claim) = self.latest_claim_for_run(&snapshot.run_id)? {
            snapshot.ownership.last_terminal_claim_id = Some(claim.claim_id.clone());
            snapshot.ownership.last_claim_transition =
                Some(claim_status_transition_kind(claim.status.clone()));
        }
        if let Some(run_result) = snapshot.run_result.as_mut() {
            run_result.ownership = snapshot.ownership.clone();
        }
        Ok(())
    }

    pub(super) fn hydrate_pause_ownership(
        &self,
        pause: &mut PauseBundle,
    ) -> Result<(), RuntimeHostServiceError> {
        if let Some(claim) = self.load_effective_claim(&pause.run_id)? {
            if claim.status == RunClaimStatus::Expired {
                pause.ownership.last_terminal_claim_id = Some(claim.claim_id.clone());
                pause.ownership.last_claim_transition = Some(ClaimTransitionKind::ClaimExpired);
            }
            pause.ownership.current_claim = Some(claim);
        } else if let Some(claim) = self.latest_claim_for_run(&pause.run_id)? {
            pause.ownership.last_terminal_claim_id = Some(claim.claim_id.clone());
            pause.ownership.last_claim_transition =
                Some(claim_status_transition_kind(claim.status.clone()));
        }
        Ok(())
    }

    fn latest_claim_for_run(
        &self,
        run_id: &RunId,
    ) -> Result<Option<RunClaim>, RuntimeHostServiceError> {
        self.claim_repo
            .load_latest_for_run(run_id)
            .map_err(|error| RuntimeHostServiceError::OwnershipViolation {
                code: OwnershipErrorCode::ClaimRequired,
                run_id: run_id.0.clone(),
                message: error.to_string(),
            })
    }

    pub(super) fn idempotency_claim_id_for_command(
        &self,
        command: &RunCommand,
    ) -> Result<Option<ClaimId>, RuntimeHostServiceError> {
        match command {
            RunCommand::BeginRun(_) | RunCommand::InspectRun(_) => Ok(None),
            RunCommand::ClaimRun(command) => Ok(self
                .load_effective_claim(&command.run_id)?
                .map(|claim| claim.claim_id)),
            RunCommand::RenewRunClaim(command) => Ok(Some(command.claim_id.clone())),
            RunCommand::ReleaseRunClaim(command) => Ok(Some(command.claim_id.clone())),
            RunCommand::StepRun(command) => Ok(self
                .load_effective_claim(&command.run_id)?
                .map(|claim| claim.claim_id)),
            RunCommand::SubmitEvidence(command) => Ok(self
                .load_effective_claim(&command.run_id)?
                .map(|claim| claim.claim_id)),
            RunCommand::SubmitEnvelope(command) => Ok(self
                .load_effective_claim(&command.run_id)?
                .map(|claim| claim.claim_id)),
            RunCommand::SubmitSignerDecision(command) => Ok(self
                .load_effective_claim(&command.run_id)?
                .map(|claim| claim.claim_id)),
            RunCommand::SubmitPlanPatch(command) => Ok(self
                .load_effective_claim(&command.run_id)?
                .map(|claim| claim.claim_id)),
            RunCommand::SubmitExecutionArtifactContinuation(command) => Ok(self
                .load_effective_claim(&command.run_id)?
                .map(|claim| claim.claim_id)),
            RunCommand::RequestCancelRun(command) => Ok(self
                .load_effective_claim(&command.run_id)?
                .map(|claim| claim.claim_id)),
            RunCommand::CancelRun(command) => Ok(self
                .load_effective_claim(&command.run_id)?
                .map(|claim| claim.claim_id)),
        }
    }
}

fn claim_status_transition_kind(status: RunClaimStatus) -> ClaimTransitionKind {
    match status {
        RunClaimStatus::Active => ClaimTransitionKind::ClaimAcquired,
        RunClaimStatus::Expired => ClaimTransitionKind::ClaimExpired,
        RunClaimStatus::Released => ClaimTransitionKind::ClaimReleased,
        RunClaimStatus::Superseded => ClaimTransitionKind::ClaimSuperseded,
    }
}
