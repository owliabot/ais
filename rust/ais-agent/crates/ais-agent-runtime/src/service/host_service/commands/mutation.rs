use ais_agent_control::commands::{
    CancelRunCommand, RequestCancelRunCommand, StepRunCommand, SubmitEnvelopeCommand,
    SubmitEvidenceCommand, SubmitExecutionArtifactContinuationCommand, SubmitPlanPatchCommand,
    SubmitSignerDecisionCommand,
};
use ais_agent_control::recovery::{CancelState, SideEffectPhase};
use ais_agent_core::{
    checkpoint::PendingRequestsSnapshot,
    recovery::{classify_cancel_request, CancelRequestResolution},
};
use ais_agent_host::{control::HostCommandOutcome, session::HostSessionId};

use crate::{
    concurrency::guard_run_command_version,
    persistence::{DurableMutationKind, MissionWriteMode},
    runtime::apply_plan_patch,
    stepper::{StepBudget, StepScheduler},
};

use super::super::{
    artifact_planner::seed_execution_artifact_checkpoint, conversion,
    persist::PendingCheckpointRecorder, DurableCheckpointWrite, RuntimeHostService,
    RuntimeHostServiceError, RuntimeHostServiceResult,
};

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
    pub async fn step_run(
        &mut self,
        host_session_id: HostSessionId,
        command: StepRunCommand,
    ) -> RuntimeHostServiceResult {
        self.ensure_mutation_session_link(&host_session_id, &command.run_id)?;
        let _claim = self.ensure_mutation_claim(&host_session_id, &command.run_id)?;
        let mut runtime = self.load_or_restore_active_run(&command.run_id)?;
        guard_run_command_version(
            &ais_agent_control::commands::RunCommand::StepRun(command.clone()),
            &runtime,
        )
        .map_err(RuntimeHostServiceError::VersionConflict)?;
        let base_revision = runtime.revision;
        let base_checkpoint = runtime.checkpoint.clone();
        runtime.record_command(command.command_id, None);

        let mut checkpoint_recorder = PendingCheckpointRecorder::default();
        let result = StepScheduler::step_until_boundary(
            &mut runtime,
            &mut checkpoint_recorder,
            conversion::map_until(command.until),
            StepBudget {
                max_transitions: command.budget.as_ref().and_then(|budget| budget.max_nodes),
                max_wall_clock_ms: command.budget.and_then(|budget| budget.max_wall_clock_ms),
            },
        )
        .await?;
        let checkpoint_entry = checkpoint_recorder.into_latest_entry().ok_or_else(|| {
            RuntimeHostServiceError::Checkpoint(
                crate::persistence::CheckpointRepositoryError::Storage {
                    message: "step scheduler produced no checkpoint entry".to_owned(),
                },
            )
        })?;

        if let Err(error) = self.commit_existing_run_state(
            &runtime,
            Some(base_revision),
            match checkpoint_entry.kind {
                crate::persistence::CheckpointArchiveKind::SideEffect => {
                    DurableMutationKind::SideEffect
                }
                crate::persistence::CheckpointArchiveKind::Boundary => {
                    if matches!(
                        runtime.checkpoint.lifecycle.status,
                        ais_agent_core::runtime::RunStatus::Completed
                            | ais_agent_core::runtime::RunStatus::Failed
                            | ais_agent_core::runtime::RunStatus::Cancelled
                    ) {
                        DurableMutationKind::Terminal
                    } else {
                        DurableMutationKind::Progress
                    }
                }
                crate::persistence::CheckpointArchiveKind::Progress => {
                    DurableMutationKind::Progress
                }
            },
            Some(checkpoint_entry),
            &result.events,
            None,
        ) {
            self.invalidate_hot_runtime_if_durable_checkpoint_advanced(
                &runtime.run_id,
                &base_checkpoint,
            );
            return Err(error);
        }

        Ok(HostCommandOutcome {
            response: self.inspect_or_pause_response(&host_session_id, &runtime)?,
            events: result.events,
        })
    }

    pub async fn submit_evidence(
        &mut self,
        host_session_id: HostSessionId,
        command: SubmitEvidenceCommand,
    ) -> RuntimeHostServiceResult {
        self.ensure_mutation_session_link(&host_session_id, &command.run_id)?;
        let _claim = self.ensure_mutation_claim(&host_session_id, &command.run_id)?;
        let mut runtime = self.load_or_restore_active_run(&command.run_id)?;
        guard_run_command_version(
            &ais_agent_control::commands::RunCommand::SubmitEvidence(command.clone()),
            &runtime,
        )
        .map_err(RuntimeHostServiceError::VersionConflict)?;
        let base_revision = runtime.revision;
        let base_checkpoint = runtime.checkpoint.clone();
        runtime.record_command(command.command_id, None);

        let submission =
            conversion::host_evidence_submission(command.run_id.clone(), command.evidence);
        let record = submission.into_evidence_record();
        runtime
            .checkpoint
            .evidence_graph
            .records
            .insert(record.evidence_id.clone(), record);
        runtime.touch_transition();

        if let Err(error) = self.commit_existing_run_state(
            &runtime,
            Some(base_revision),
            DurableMutationKind::Progress,
            Some(self.capture_checkpoint_entry(&runtime, DurableCheckpointWrite::Progress)?),
            &[],
            None,
        ) {
            self.invalidate_hot_runtime_if_durable_checkpoint_advanced(
                &runtime.run_id,
                &base_checkpoint,
            );
            return Err(error);
        }

        self.inspect_outcome(&host_session_id, &runtime, Vec::new())
    }

    pub async fn submit_envelope(
        &mut self,
        host_session_id: HostSessionId,
        command: SubmitEnvelopeCommand,
    ) -> RuntimeHostServiceResult {
        self.ensure_mutation_session_link(&host_session_id, &command.run_id)?;
        let _claim = self.ensure_mutation_claim(&host_session_id, &command.run_id)?;
        let mut runtime = self.load_or_restore_active_run(&command.run_id)?;
        guard_run_command_version(
            &ais_agent_control::commands::RunCommand::SubmitEnvelope(command.clone()),
            &runtime,
        )
        .map_err(RuntimeHostServiceError::VersionConflict)?;
        let base_revision = runtime.revision;
        let base_checkpoint = runtime.checkpoint.clone();
        runtime.record_command(command.command_id, None);

        let submission =
            conversion::host_envelope_submission(command.run_id.clone(), command.envelope)
                .map_err(RuntimeHostServiceError::EnvelopeRejected)?;
        if !runtime
            .checkpoint
            .pending_requests
            .pending_envelope_refs
            .is_empty()
            && !runtime
                .checkpoint
                .pending_requests
                .pending_envelope_refs
                .iter()
                .any(|envelope_ref| envelope_ref == &submission.envelope_id)
        {
            return Err(RuntimeHostServiceError::EnvelopeRejected(format!(
                "submitted envelope `{}` does not satisfy pending envelope refs {:?}",
                submission.envelope_id, runtime.checkpoint.pending_requests.pending_envelope_refs
            )));
        }
        if let Some(effect_contract) = submission.expected_effect_contract.clone() {
            runtime
                .checkpoint
                .effect_contracts
                .insert(effect_contract.effect_id.clone(), effect_contract);
        }
        let runtime_envelope = submission.into_runtime_envelope();
        let envelope_id = runtime_envelope.envelope_id.clone();
        runtime
            .envelopes
            .insert(runtime_envelope.envelope_id.clone(), runtime_envelope);
        conversion::resolve_pending_envelope_recovery(&mut runtime, &envelope_id);
        runtime.touch_transition();

        if let Err(error) = self.commit_existing_run_state(
            &runtime,
            Some(base_revision),
            DurableMutationKind::Progress,
            Some(self.capture_checkpoint_entry(&runtime, DurableCheckpointWrite::Progress)?),
            &[],
            None,
        ) {
            self.invalidate_hot_runtime_if_durable_checkpoint_advanced(
                &runtime.run_id,
                &base_checkpoint,
            );
            return Err(error);
        }

        self.inspect_outcome(&host_session_id, &runtime, Vec::new())
    }

    pub async fn submit_signer_decision(
        &mut self,
        host_session_id: HostSessionId,
        command: SubmitSignerDecisionCommand,
    ) -> RuntimeHostServiceResult {
        self.ensure_mutation_session_link(&host_session_id, &command.run_id)?;
        let _claim = self.ensure_mutation_claim(&host_session_id, &command.run_id)?;
        let mut runtime = self.load_or_restore_active_run(&command.run_id)?;
        guard_run_command_version(
            &ais_agent_control::commands::RunCommand::SubmitSignerDecision(command.clone()),
            &runtime,
        )
        .map_err(RuntimeHostServiceError::VersionConflict)?;
        let base_revision = runtime.revision;
        let base_checkpoint = runtime.checkpoint.clone();
        runtime.record_command(command.command_id, None);

        let host_decision =
            conversion::host_signer_decision(command.run_id.clone(), command.decision);
        let decision = host_decision.clone().into_runtime_decision();
        let Some(signer_state) = runtime.pending_signer_state.as_mut() else {
            return Err(RuntimeHostServiceError::SignerDecisionMismatch);
        };
        if signer_state.request_id != host_decision.request_id {
            return Err(RuntimeHostServiceError::SignerDecisionMismatch);
        }
        signer_state.apply_decision(decision);
        runtime.touch_transition();

        if let Err(error) = self.commit_existing_run_state(
            &runtime,
            Some(base_revision),
            DurableMutationKind::Progress,
            Some(self.capture_checkpoint_entry(&runtime, DurableCheckpointWrite::Progress)?),
            &[],
            None,
        ) {
            self.invalidate_hot_runtime_if_durable_checkpoint_advanced(
                &runtime.run_id,
                &base_checkpoint,
            );
            return Err(error);
        }

        self.inspect_outcome(&host_session_id, &runtime, Vec::new())
    }

    pub async fn submit_plan_patch(
        &mut self,
        host_session_id: HostSessionId,
        command: SubmitPlanPatchCommand,
    ) -> RuntimeHostServiceResult {
        self.ensure_mutation_session_link(&host_session_id, &command.run_id)?;
        let _claim = self.ensure_mutation_claim(&host_session_id, &command.run_id)?;
        let mut runtime = self.load_or_restore_active_run(&command.run_id)?;
        let base_revision = runtime.revision;
        let base_checkpoint = runtime.checkpoint.clone();
        if let Err(conflict) = guard_run_command_version(
            &ais_agent_control::commands::RunCommand::SubmitPlanPatch(command.clone()),
            &runtime,
        ) {
            self.record_rejected_plan_patch_audit(
                &mut runtime,
                &command.patch,
                conflict.code.clone(),
                Some(base_revision),
            )?;
            return Err(RuntimeHostServiceError::VersionConflict(conflict));
        }
        if let Err(error) = ais_agent_core::patch::validate_submit_plan_patch_command(&command) {
            self.record_rejected_plan_patch_audit(
                &mut runtime,
                &command.patch,
                error.to_string(),
                Some(base_revision),
            )?;
            return Err(RuntimeHostServiceError::PlanPatchLegality(
                error.to_string(),
            ));
        }
        runtime.record_command(command.command_id, None);
        let runtime_patch_outcome = match apply_plan_patch(&mut runtime, &command.patch) {
            Ok(outcome) => outcome,
            Err(error) => {
                self.record_rejected_plan_patch_audit(
                    &mut runtime,
                    &command.patch,
                    error.to_string(),
                    Some(base_revision),
                )?;
                return Err(RuntimeHostServiceError::PlanPatchLegality(
                    error.to_string(),
                ));
            }
        };
        let patch_outcome =
            conversion::patch_audit_outcome(&runtime, &command.patch, &runtime_patch_outcome);
        let patch_events = vec![
            crate::events::RuntimeEventEmitter::emit_plan_patch_submitted(
                &mut runtime,
                &command.patch,
            ),
            crate::events::RuntimeEventEmitter::emit_plan_patch_applied(
                &mut runtime,
                &command.patch,
                Some(patch_outcome),
            ),
        ];

        if let Err(error) = self.commit_existing_run_state(
            &runtime,
            Some(base_revision),
            DurableMutationKind::Progress,
            Some(self.capture_checkpoint_entry(&runtime, DurableCheckpointWrite::Progress)?),
            &patch_events,
            Some(MissionWriteMode::Upsert),
        ) {
            self.invalidate_hot_runtime_if_durable_checkpoint_advanced(
                &runtime.run_id,
                &base_checkpoint,
            );
            return Err(error);
        }

        self.inspect_outcome(&host_session_id, &runtime, patch_events)
    }

    pub async fn submit_execution_artifact_continuation(
        &mut self,
        host_session_id: HostSessionId,
        command: SubmitExecutionArtifactContinuationCommand,
    ) -> RuntimeHostServiceResult {
        self.ensure_mutation_session_link(&host_session_id, &command.run_id)?;
        let _claim = self.ensure_mutation_claim(&host_session_id, &command.run_id)?;
        let mut runtime = self.load_or_restore_active_run(&command.run_id)?;
        guard_run_command_version(
            &ais_agent_control::commands::RunCommand::SubmitExecutionArtifactContinuation(
                command.clone(),
            ),
            &runtime,
        )
        .map_err(RuntimeHostServiceError::VersionConflict)?;
        let base_revision = runtime.revision;
        let base_checkpoint = runtime.checkpoint.clone();
        runtime.record_command(command.command_id, None);

        let current_snapshot = runtime
            .checkpoint
            .execution_artifact
            .as_ref()
            .ok_or_else(|| {
                RuntimeHostServiceError::ContinuationRejected(
                    "run has no active execution artifact state".to_owned(),
                )
            })?;
        let continuation = current_snapshot
            .awaiting_continuation
            .as_ref()
            .ok_or_else(|| {
                RuntimeHostServiceError::ContinuationRejected(
                    "run is not waiting for an execution artifact continuation".to_owned(),
                )
            })?;
        if continuation.package_entry != command.package_entry {
            return Err(RuntimeHostServiceError::ContinuationRejected(format!(
                "submitted package_entry `{}` does not match pending continuation `{}`",
                command.package_entry, continuation.package_entry
            )));
        }

        let exported_outputs = current_snapshot.exported_outputs.clone();
        for required_output in &continuation.required_outputs {
            if !exported_outputs.contains_key(required_output) {
                return Err(RuntimeHostServiceError::ContinuationRejected(format!(
                    "pending continuation requires exported output `{required_output}` but it is missing from runtime state"
                )));
            }
        }

        runtime.pending_signer_state = None;
        runtime.checkpoint.pending_requests = PendingRequestsSnapshot::default();
        runtime.checkpoint.last_completed_node_id = None;
        seed_execution_artifact_checkpoint(
            &mut runtime.checkpoint,
            &self.execution_wiring,
            &command.artifact,
        )
        .map_err(RuntimeHostServiceError::ContinuationRejected)?;
        if let Some(snapshot) = runtime.checkpoint.execution_artifact.as_mut() {
            snapshot.exported_outputs = exported_outputs;
            snapshot.awaiting_continuation = None;
        }
        runtime
            .checkpoint
            .lifecycle
            .mark_running(ais_agent_core::runtime::RunPhase::Planning);
        runtime.touch_transition();

        if let Err(error) = self.commit_existing_run_state(
            &runtime,
            Some(base_revision),
            DurableMutationKind::Progress,
            Some(self.capture_checkpoint_entry(&runtime, DurableCheckpointWrite::Progress)?),
            &[],
            None,
        ) {
            self.invalidate_hot_runtime_if_durable_checkpoint_advanced(
                &runtime.run_id,
                &base_checkpoint,
            );
            return Err(error);
        }

        self.inspect_outcome(&host_session_id, &runtime, Vec::new())
    }

    pub async fn cancel_run(
        &mut self,
        host_session_id: HostSessionId,
        command: CancelRunCommand,
    ) -> RuntimeHostServiceResult {
        self.request_cancel_run(
            host_session_id,
            RequestCancelRunCommand {
                command_id: command.command_id,
                run_id: command.run_id,
                reason: command.reason,
                expected_version: command.expected_version,
            },
        )
        .await
    }

    pub async fn request_cancel_run(
        &mut self,
        host_session_id: HostSessionId,
        command: RequestCancelRunCommand,
    ) -> RuntimeHostServiceResult {
        self.ensure_mutation_session_link(&host_session_id, &command.run_id)?;
        let _claim = self.ensure_mutation_claim(&host_session_id, &command.run_id)?;
        let mut runtime = self.load_or_restore_active_run(&command.run_id)?;
        guard_run_command_version(
            &ais_agent_control::commands::RunCommand::RequestCancelRun(command.clone()),
            &runtime,
        )
        .map_err(RuntimeHostServiceError::VersionConflict)?;
        let base_revision = runtime.revision;
        let base_checkpoint = runtime.checkpoint.clone();
        runtime.record_command(command.command_id, None);
        let cancel_reason = command
            .reason
            .unwrap_or_else(|| "cancelled by host".to_owned());
        match classify_cancel_request(&runtime.checkpoint) {
            CancelRequestResolution::CancelImmediately => {
                runtime.pending_signer_state = None;
                runtime.checkpoint.pending_requests = PendingRequestsSnapshot::default();
                runtime.checkpoint.lifecycle.cancel(cancel_reason);
            }
            CancelRequestResolution::CancelPending => {
                runtime
                    .checkpoint
                    .lifecycle
                    .request_cancel_pending(cancel_reason, cancel_side_effect_phase(&runtime));
            }
            CancelRequestResolution::Reject(reason) => {
                return Err(RuntimeHostServiceError::CancelRejected(reason));
            }
        }
        runtime.touch_transition();

        if let Err(error) = self.commit_existing_run_state(
            &runtime,
            Some(base_revision),
            match runtime.checkpoint.lifecycle.cancel_state {
                Some(CancelState::Pending) => DurableMutationKind::Progress,
                _ => DurableMutationKind::Terminal,
            },
            Some(self.capture_checkpoint_entry(&runtime, DurableCheckpointWrite::Boundary)?),
            &[],
            None,
        ) {
            self.invalidate_hot_runtime_if_durable_checkpoint_advanced(
                &runtime.run_id,
                &base_checkpoint,
            );
            return Err(error);
        }

        Ok(HostCommandOutcome {
            response: self.inspect_or_pause_response(&host_session_id, &runtime)?,
            events: Vec::new(),
        })
    }
}

fn cancel_side_effect_phase(runtime: &crate::runtime::ActiveRun) -> Option<SideEffectPhase> {
    crate::runtime::classify_recovery_view(&runtime.checkpoint).side_effect_phase
}
