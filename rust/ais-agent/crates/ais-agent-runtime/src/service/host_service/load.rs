use ais_agent_control::ids::RunId;
use ais_agent_core::{checkpoint::CheckpointSnapshot, mission::Mission};
use ais_agent_host::{
    events::{HostEventServiceError, HostRunEventBatch, HostRunEventQuery},
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

impl<R, C, M, K, E, S, G, A> RuntimeHostService<R, C, M, K, E, S, G, A>
where
    R: crate::runtime::RunRepository + Send,
    C: crate::persistence::CheckpointRepository + Send,
    M: crate::persistence::MissionRepository + Send,
    K: crate::persistence::RunCatalogRepository + Send,
    E: crate::persistence::EventArchive + Send,
    S: ais_agent_host::session::HostSessionStore + Send,
    G: crate::persistence::SignerStateArchive + Send,
    A: crate::persistence::RuntimeAuditArchive + Send,
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
}
