use ais_agent_control::{events::RunEventEnvelope, ids::RunId, patch::PlanPatchSubmission};
use ais_agent_core::checkpoint::CheckpointSnapshot;
use tracing::{debug, debug_span, info, warn};

use crate::{
    events::RuntimeEventEmitter,
    persistence::{
        persist_boundary_checkpoint, persist_progress_checkpoint,
        signer_state_into_wait_state_record, CheckpointArchiveEntry, CheckpointArchiveKind,
        CheckpointRepository, DurableMutationExecutor, DurableMutationKind, DurableMutationUnit,
        LinearDurableMutationExecutor, MissionWrite, MissionWriteMode, RunWaitStateWrite,
        RuntimeAuditQuery,
    },
    runtime::ActiveRun,
};

use super::{conversion, DurableCheckpointWrite, RuntimeHostService, RuntimeHostServiceError};

#[derive(Debug, Default)]
pub(super) struct PendingCheckpointRecorder {
    entries: Vec<CheckpointArchiveEntry>,
}

impl PendingCheckpointRecorder {
    pub(super) fn into_latest_entry(self) -> Option<CheckpointArchiveEntry> {
        self.entries.into_iter().last()
    }
}

impl CheckpointRepository for PendingCheckpointRecorder {
    fn latest(
        &self,
        run_id: &str,
    ) -> Result<
        ais_agent_core::checkpoint::CheckpointSnapshot,
        crate::persistence::CheckpointRepositoryError,
    > {
        self.entries
            .iter()
            .rev()
            .find(|entry| entry.snapshot.run_id == run_id)
            .map(|entry| entry.snapshot.clone())
            .ok_or_else(|| crate::persistence::CheckpointRepositoryError::NotFound {
                run_id: run_id.to_owned(),
            })
    }

    fn append(
        &mut self,
        entry: CheckpointArchiveEntry,
    ) -> Result<(), crate::persistence::CheckpointRepositoryError> {
        self.entries.push(entry);
        Ok(())
    }

    fn history(
        &self,
        run_id: &str,
    ) -> Result<Vec<CheckpointArchiveEntry>, crate::persistence::CheckpointRepositoryError> {
        let entries = self
            .entries
            .iter()
            .filter(|entry| entry.snapshot.run_id == run_id)
            .cloned()
            .collect::<Vec<_>>();
        if entries.is_empty() {
            Err(crate::persistence::CheckpointRepositoryError::NotFound {
                run_id: run_id.to_owned(),
            })
        } else {
            Ok(entries)
        }
    }
}

impl<R, C, M, K, E, S, G, A, Q> RuntimeHostService<R, C, M, K, E, S, G, A, Q>
where
    R: crate::runtime::RunRepository + Send,
    C: crate::persistence::CheckpointRepository + Send,
    M: crate::persistence::MissionRepository + Send,
    K: crate::persistence::RunCatalogRepository + Send,
    E: crate::persistence::EventArchive + Send,
    S: ais_agent_host::session::HostSessionStore + Send,
    G: crate::persistence::SignerStateStore + Send,
    A: crate::persistence::RuntimeAuditArchive + Send,
    Q: crate::persistence::RunClaimRepository + Send,
{
    fn durable_executor(
        &mut self,
    ) -> LinearDurableMutationExecutor<&mut M, &mut C, &mut E, &mut K, &mut G, &mut A> {
        LinearDurableMutationExecutor::new(
            &mut self.mission_repo,
            &mut self.checkpoint_repo,
            &mut self.event_archive,
            &mut self.run_catalog_repo,
            &mut self.signer_state_store,
            &mut self.audit_archive,
        )
    }

    fn archived_latest_audit_seq(
        &self,
        run_id: &RunId,
    ) -> Result<Option<u64>, RuntimeHostServiceError> {
        match self.audit_archive.read(RuntimeAuditQuery {
            run_id: run_id.clone(),
            after_audit_seq: None,
            limit: Some(1),
        }) {
            Ok(slice) => Ok(slice.latest_audit_seq),
            Err(crate::persistence::RuntimeAuditArchiveError::NotFound { .. }) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    pub(super) fn capture_checkpoint_entry(
        &self,
        runtime: &ActiveRun,
        checkpoint_write: DurableCheckpointWrite,
    ) -> Result<CheckpointArchiveEntry, RuntimeHostServiceError> {
        let mut recorder = PendingCheckpointRecorder::default();
        match checkpoint_write {
            DurableCheckpointWrite::Boundary => {
                persist_boundary_checkpoint(&mut recorder, runtime)?;
            }
            DurableCheckpointWrite::Progress => {
                persist_progress_checkpoint(&mut recorder, runtime)?;
            }
        }
        recorder.into_latest_entry().ok_or_else(|| {
            RuntimeHostServiceError::Checkpoint(
                crate::persistence::CheckpointRepositoryError::Storage {
                    message: "checkpoint recorder captured no entry".to_owned(),
                },
            )
        })
    }

    fn build_durable_mutation_unit(
        &self,
        runtime: &ActiveRun,
        mutation_kind: DurableMutationKind,
        mission_write_mode: Option<MissionWriteMode>,
        checkpoint_entry: CheckpointArchiveEntry,
        events: &[RunEventEnvelope],
    ) -> Result<DurableMutationUnit, RuntimeHostServiceError> {
        let latest_event_seq = events
            .last()
            .map(|event| event.event_seq)
            .or_else(|| self.archived_latest_event_seq(&runtime.run_id).ok())
            .filter(|seq| *seq > 0);
        let latest_audit_seq = self.archived_latest_audit_seq(&runtime.run_id)?;
        let audit_records = conversion::runtime_audit_records(events, latest_audit_seq);

        Ok(DurableMutationUnit {
            run_id: runtime.run_id.clone(),
            kind: mutation_kind,
            mission_write: mission_write_mode.map(|mode| MissionWrite {
                run_id: runtime.run_id.clone(),
                mode,
                mission: runtime.mission.clone(),
            }),
            checkpoint_write: crate::persistence::CheckpointWrite {
                entry: checkpoint_entry,
            },
            event_write: crate::persistence::EventWriteBatch {
                events: events.to_vec(),
            },
            catalog_write: crate::persistence::CatalogWrite {
                entry: conversion::run_catalog_entry(runtime, latest_event_seq),
            },
            wait_state_write: Some(match runtime.pending_signer_state.clone() {
                Some(signer_state) => RunWaitStateWrite::Upsert {
                    wait_state: signer_state_into_wait_state_record(signer_state)?,
                },
                None => RunWaitStateWrite::Clear {
                    run_id: runtime.run_id.clone(),
                },
            }),
            audit_write: crate::persistence::AuditWriteBatch {
                records: audit_records,
            },
        })
    }

    fn commit_grouped_run_state(
        &mut self,
        runtime: &ActiveRun,
        unit: DurableMutationUnit,
    ) -> Result<crate::persistence::DurableCommitReceipt, RuntimeHostServiceError> {
        let _span = debug_span!(
            "runtime.host.commit_grouped_run_state",
            run_id = %runtime.run_id.0,
            command_id = runtime
                .last_command_id
                .as_ref()
                .map(|id| id.0.as_str())
                .unwrap_or("<none>"),
            checkpoint_seq = runtime.checkpoint.checkpoint_seq,
            plan_epoch = runtime.checkpoint.plan_epoch,
            revision = runtime.revision,
        )
        .entered();
        let mutation_kind = unit.kind;
        let receipt = self.durable_executor().commit(unit).map_err(|error| {
            warn!(
                run_id = %runtime.run_id.0,
                mutation_kind = ?mutation_kind,
                checkpoint_seq = runtime.checkpoint.checkpoint_seq,
                plan_epoch = runtime.checkpoint.plan_epoch,
                revision = runtime.revision,
                error = %error,
                "runtime.host.grouped_commit_failed"
            );
            RuntimeHostServiceError::DurableCommit(error)
        })?;
        debug!(
            run_id = %runtime.run_id.0,
            mutation_kind = ?mutation_kind,
            checkpoint_seq = receipt.checkpoint_seq,
            plan_epoch = receipt.plan_epoch,
            latest_event_seq = ?receipt.latest_event_seq,
            latest_audit_seq = ?receipt.latest_audit_seq,
            revision = runtime.revision,
            "runtime.host.grouped_commit_succeeded"
        );
        Ok(receipt)
    }

    pub(super) fn persist_new_run(
        &mut self,
        run_id: &RunId,
        _mission: &ais_agent_core::mission::Mission,
        _checkpoint: &CheckpointSnapshot,
        runtime: &ActiveRun,
        events: &[RunEventEnvelope],
    ) -> Result<(), RuntimeHostServiceError> {
        let _span = debug_span!(
            "runtime.host.persist_new_run_scope",
            run_id = %run_id.0,
            command_id = runtime
                .last_command_id
                .as_ref()
                .map(|id| id.0.as_str())
                .unwrap_or("<none>"),
            checkpoint_seq = runtime.checkpoint.checkpoint_seq,
            plan_epoch = runtime.checkpoint.plan_epoch,
            revision = runtime.revision,
        )
        .entered();
        debug!(
            run_id = %run_id.0,
            checkpoint_seq = runtime.checkpoint.checkpoint_seq,
            plan_epoch = runtime.checkpoint.plan_epoch,
            event_count = events.len(),
            "runtime.host.persist_new_run"
        );
        let unit = self.build_durable_mutation_unit(
            runtime,
            DurableMutationKind::RunBegin,
            Some(MissionWriteMode::Insert),
            conversion::checkpoint_entry(runtime, CheckpointArchiveKind::Boundary),
            events,
        )?;
        self.commit_grouped_run_state(runtime, unit)?;
        if let Err(error) = self.run_repo.insert(runtime.clone()) {
            warn!(run_id = %run_id.0, stage = "hot_insert", error = %error, "runtime.host.persist_new_run_failed");
            return Err(error.into());
        }
        debug!(run_id = %run_id.0, "runtime.host.persist_new_run_committed");
        Ok(())
    }

    pub(super) fn commit_existing_run_state(
        &mut self,
        runtime: &ActiveRun,
        base_revision: Option<u64>,
        mutation_kind: DurableMutationKind,
        checkpoint_entry: Option<CheckpointArchiveEntry>,
        events: &[RunEventEnvelope],
        mission_write_mode: Option<MissionWriteMode>,
    ) -> Result<(), RuntimeHostServiceError> {
        let _span = debug_span!(
            "runtime.host.commit_existing_run_state_scope",
            run_id = %runtime.run_id.0,
            command_id = runtime
                .last_command_id
                .as_ref()
                .map(|id| id.0.as_str())
                .unwrap_or("<none>"),
            checkpoint_seq = runtime.checkpoint.checkpoint_seq,
            plan_epoch = runtime.checkpoint.plan_epoch,
            revision = runtime.revision,
        )
        .entered();
        debug!(
            run_id = %runtime.run_id.0,
            mutation_kind = ?mutation_kind,
            has_checkpoint_entry = checkpoint_entry.is_some(),
            event_count = events.len(),
            mission_write_mode = ?mission_write_mode,
            base_revision,
            "runtime.host.commit_existing_run_state"
        );

        if let Some(checkpoint_entry) = checkpoint_entry {
            let unit = self.build_durable_mutation_unit(
                runtime,
                mutation_kind,
                mission_write_mode,
                checkpoint_entry,
                events,
            )?;
            self.commit_grouped_run_state(runtime, unit)?;
            self.run_repo.save(runtime.clone(), base_revision)?;
            return Ok(());
        }

        if let Some(mode) = mission_write_mode {
            let result = match mode {
                MissionWriteMode::Insert => self
                    .mission_repo
                    .insert(runtime.run_id.clone(), runtime.mission.clone()),
                MissionWriteMode::Upsert => self
                    .mission_repo
                    .upsert(runtime.run_id.clone(), runtime.mission.clone()),
            };
            if let Err(error) = result {
                warn!(run_id = %runtime.run_id.0, stage = "mission_write", error = %error, "runtime.host.commit_existing_run_state_failed");
                return Err(error.into());
            }
        }

        for event in events.iter().cloned() {
            self.event_archive.append(event)?;
        }
        self.sync_wait_state_archive(runtime)?;
        self.persist_run_catalog(
            runtime,
            if events.is_empty() {
                None
            } else {
                Some(events)
            },
        )?;
        self.run_repo.save(runtime.clone(), base_revision)?;
        Ok(())
    }

    pub(super) fn invalidate_hot_runtime_if_durable_checkpoint_advanced(
        &mut self,
        run_id: &RunId,
        base_checkpoint: &CheckpointSnapshot,
    ) {
        let durable_is_newer = match self.checkpoint_repo.latest(&run_id.0) {
            Ok(checkpoint) => conversion::checkpoint_is_newer(&checkpoint, base_checkpoint),
            Err(crate::persistence::CheckpointRepositoryError::NotFound { .. }) => false,
            Err(_) => false,
        };
        if !durable_is_newer {
            return;
        }

        info!(
            run_id = %run_id.0,
            hot_checkpoint_seq = base_checkpoint.checkpoint_seq,
            hot_plan_epoch = base_checkpoint.plan_epoch,
            "runtime.host.invalidate_hot_runtime_after_durable_advance"
        );
        let _ = match self.run_repo.delete(run_id) {
            Ok(()) | Err(crate::runtime::RunRepositoryError::NotFound { .. }) => Ok(()),
            Err(other) => Err(other),
        };
    }

    pub(super) fn sync_wait_state_archive(
        &mut self,
        runtime: &ActiveRun,
    ) -> Result<(), RuntimeHostServiceError> {
        match runtime.pending_signer_state.clone() {
            Some(state) => {
                self.signer_state_store
                    .upsert_wait_state(signer_state_into_wait_state_record(state)?)?;
            }
            None => {
                self.signer_state_store.clear_wait_state(&runtime.run_id)?;
            }
        }
        Ok(())
    }

    pub(super) fn record_rejected_plan_patch_audit(
        &mut self,
        runtime: &mut ActiveRun,
        patch: &PlanPatchSubmission,
        message: String,
        base_revision: Option<u64>,
    ) -> Result<(), RuntimeHostServiceError> {
        let event = RuntimeEventEmitter::emit_plan_patch_rejected(runtime, patch, message);
        self.commit_existing_run_state(
            runtime,
            base_revision,
            DurableMutationKind::Progress,
            None,
            std::slice::from_ref(&event),
            None,
        )?;
        Ok(())
    }

    pub(super) fn persist_run_catalog(
        &mut self,
        runtime: &ActiveRun,
        newly_appended_events: Option<&[RunEventEnvelope]>,
    ) -> Result<(), RuntimeHostServiceError> {
        let latest_event_seq = newly_appended_events
            .and_then(|events| events.last().map(|event| event.event_seq))
            .or_else(|| self.archived_latest_event_seq(&runtime.run_id).ok())
            .filter(|seq| *seq > 0);

        self.run_catalog_repo
            .upsert(conversion::run_catalog_entry(runtime, latest_event_seq))?;
        Ok(())
    }
}
