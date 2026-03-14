use ais_agent_control::{
    commands::RetryIntent,
    ownership::{OwnershipVisibility, RunOwnershipSnapshot},
    recovery::{RecoveryActionKind, RecoveryDisposition},
};
use ais_agent_core::{
    action::ActionNodeStatus,
    actuation::ActuationStatus,
    checkpoint::CheckpointSnapshot,
    mission::Mission,
    ownership::classify_claim_policy,
    recovery::{classify_recovery_view as classify_core_recovery_view, RecoveryProjection},
    runtime::{
        BoundaryKind as CoreBoundaryKind, RunPhase as CoreRunPhase, RunStatus as CoreRunStatus,
    },
};

use crate::inspect::{
    progress::ActionStatusCountsView, ActiveBoundaryView, BoundaryKind, EffectStatusView,
    InspectSnapshot, MissionSummaryView, PauseActionView, PauseBundle, PauseKind,
    PendingConfirmationView, PendingContinuationView, PendingSignerRequestView, ProgressView,
    RecoveryView, RequiredInputView, RunPhase, RunResultView, RunStatus, SideEffectView,
};

pub fn project_inspect_snapshot(
    mission: &Mission,
    checkpoint: &CheckpointSnapshot,
) -> InspectSnapshot {
    project_inspect_snapshot_with_recovery(mission, checkpoint, default_recovery_view(checkpoint))
}

pub fn project_inspect_snapshot_with_recovery(
    mission: &Mission,
    checkpoint: &CheckpointSnapshot,
    recovery: RecoveryView,
) -> InspectSnapshot {
    let lifecycle = &checkpoint.lifecycle;
    let pending_signer_requests = project_pending_signer_requests(checkpoint);
    let pending_confirmations = project_pending_confirmations(checkpoint);
    let pending_continuations = project_pending_continuations(checkpoint);
    let ownership = project_ownership_snapshot(checkpoint);

    InspectSnapshot {
        schema: "ais-agent/inspect_snapshot/v1".to_owned(),
        run_id: lifecycle.run_id.clone(),
        status: map_run_status(&lifecycle.status),
        phase: map_run_phase(&lifecycle.phase),
        checkpoint_seq: checkpoint.checkpoint_seq,
        plan_epoch: checkpoint.plan_epoch,
        active_boundary: lifecycle
            .active_boundary
            .as_ref()
            .map(|boundary| ActiveBoundaryView {
                kind: map_boundary_kind(&boundary.kind),
                summary: boundary.summary.clone(),
            }),
        interruption_class: recovery.interruption_class.clone(),
        cancel_state: recovery.cancel_state.clone(),
        side_effect_phase: recovery.side_effect_phase.clone(),
        recovery_disposition: recovery.recovery_disposition.clone(),
        failure_context: recovery.failure_context.clone(),
        recovery_suggestions: recovery.recovery_suggestions.clone(),
        allowed_recovery_actions: recovery.allowed_recovery_actions.clone(),
        mission_summary: MissionSummaryView {
            goal: mission.goal.clone(),
            allowed_chains: mission.allowed_chains.clone(),
            policy_mode: mission.policy.policy_mode.clone(),
        },
        required_inputs: checkpoint
            .evidence_graph
            .requirements
            .iter()
            .filter(|requirement| requirement.satisfied_by_evidence_id.is_none())
            .map(|requirement| RequiredInputView {
                reference: requirement.reference.clone(),
                reason: requirement.reason.clone(),
            })
            .collect(),
        pending_confirmations,
        pending_continuations,
        pending_signer_requests,
        recent_side_effects: checkpoint
            .actuation_records
            .iter()
            .rev()
            .take(5)
            .map(|record| SideEffectView {
                kind: format!("{:?}", record.kind).to_lowercase(),
                summary: record.summary.clone(),
                tx_hash: record.tx_hash.clone(),
            })
            .collect(),
        effect_status: Some(project_effect_status(checkpoint)),
        ownership: ownership.clone(),
        run_result: project_run_result(checkpoint, &recovery, ownership),
        progress: project_progress_view(checkpoint),
    }
}

pub fn project_pause_bundle(checkpoint: &CheckpointSnapshot) -> Option<PauseBundle> {
    project_pause_bundle_with_recovery(checkpoint, default_recovery_view(checkpoint))
}

pub fn project_pause_bundle_with_recovery(
    checkpoint: &CheckpointSnapshot,
    recovery: RecoveryView,
) -> Option<PauseBundle> {
    let lifecycle = &checkpoint.lifecycle;
    let boundary = lifecycle.active_boundary.as_ref()?;
    let ownership = project_ownership_snapshot(checkpoint);

    let kind = match (&lifecycle.status, &boundary.kind) {
        (CoreRunStatus::AwaitingEvidence, _) => PauseKind::NeedEvidence,
        (CoreRunStatus::AwaitingSigner, _) => PauseKind::NeedSigner,
        (CoreRunStatus::AwaitingConfirmation, _) | (_, CoreBoundaryKind::Confirmation)
            if checkpoint
                .pending_requests
                .pending_confirmation_id
                .is_some() =>
        {
            PauseKind::NeedConfirmation
        }
        (CoreRunStatus::AwaitingArtifactContinuation, _)
        | (_, CoreBoundaryKind::ArtifactContinuation) => PauseKind::NeedContinuation,
        (CoreRunStatus::Paused, _) => PauseKind::NeedUserInput,
        (CoreRunStatus::Failed, _) | (_, CoreBoundaryKind::Failure) => PauseKind::RuntimeFailure,
        _ => return None,
    };

    Some(PauseBundle {
        schema: "ais-agent/pause_bundle/v2".to_owned(),
        run_id: lifecycle.run_id.clone(),
        kind,
        interruption_class: recovery.interruption_class.clone(),
        cancel_state: recovery.cancel_state.clone(),
        side_effect_phase: recovery.side_effect_phase.clone(),
        recovery_disposition: recovery
            .recovery_disposition
            .clone()
            .unwrap_or(RecoveryDisposition::AwaitUserInput),
        summary: boundary.summary.clone(),
        ownership: ownership.clone(),
        blocking_refs: boundary.blocking_refs.clone(),
        required_actions: project_pause_actions(&recovery, &ownership),
        failure_context: recovery.failure_context.clone(),
        recovery_suggestions: recovery.recovery_suggestions.clone(),
        allowed_recovery_actions: recovery.allowed_recovery_actions.clone(),
        pending_signer_requests: project_pending_signer_requests(checkpoint),
        pending_confirmations: project_pending_confirmations(checkpoint),
        pending_continuations: project_pending_continuations(checkpoint),
        notes: project_pause_notes(checkpoint),
    })
}

pub fn project_progress_view(checkpoint: &CheckpointSnapshot) -> ProgressView {
    let mut counts = ActionStatusCountsView::default();
    let mut active_node_ids = Vec::new();
    let mut blocked_node_ids = Vec::new();

    for (node_id, node) in &checkpoint.action_graph.nodes {
        match node.status {
            ActionNodeStatus::Pending => counts.pending += 1,
            ActionNodeStatus::Ready => counts.ready += 1,
            ActionNodeStatus::Running => {
                counts.running += 1;
                active_node_ids.push(node_id.clone());
            }
            ActionNodeStatus::Blocked => {
                counts.blocked += 1;
                blocked_node_ids.push(node_id.clone());
            }
            ActionNodeStatus::Succeeded => counts.succeeded += 1,
            ActionNodeStatus::Failed => counts.failed += 1,
            ActionNodeStatus::Skipped => counts.skipped += 1,
        }
    }

    ProgressView {
        graph_id: checkpoint.action_graph.graph_id.clone(),
        total_nodes: checkpoint.action_graph.nodes.len() as u32,
        roots: checkpoint.action_graph.roots.len() as u32,
        terminals: checkpoint.action_graph.terminals.len() as u32,
        status_counts: counts,
        active_node_ids,
        blocked_node_ids,
        last_completed_node_id: checkpoint.last_completed_node_id.clone(),
        required_evidence_count: checkpoint
            .evidence_graph
            .requirements
            .iter()
            .filter(|requirement| requirement.satisfied_by_evidence_id.is_none())
            .count() as u32,
        actuation_record_count: checkpoint.actuation_records.len() as u32,
    }
}

fn project_pause_actions(
    recovery: &RecoveryView,
    ownership: &RunOwnershipSnapshot,
) -> Vec<PauseActionView> {
    recovery
        .allowed_recovery_actions
        .clone()
        .into_iter()
        .map(|action| PauseActionView {
            action_kind: action.clone(),
            action: map_recovery_action_command(&action).to_owned(),
            description: map_recovery_action_description(&action).to_owned(),
            requires_mutation_claim: ownership.claim_required_for_mutation
                && action_requires_mutation_claim(&action),
            retry_intent: map_recovery_action_retry_intent(&action),
        })
        .collect()
}

fn default_recovery_view(checkpoint: &CheckpointSnapshot) -> RecoveryView {
    classify_core_recovery_view(checkpoint).into()
}

fn project_pause_notes(checkpoint: &CheckpointSnapshot) -> Vec<String> {
    let mut notes = Vec::new();
    if checkpoint
        .pending_requests
        .pending_confirmation_id
        .is_some()
    {
        notes.push("chain confirmation is still pending".to_owned());
    }
    if checkpoint
        .actuation_records
        .iter()
        .any(|record| record.status == ActuationStatus::Failed)
    {
        notes.push("one or more actuation records are marked failed".to_owned());
    }
    notes
}

fn project_pending_signer_requests(
    checkpoint: &CheckpointSnapshot,
) -> Vec<PendingSignerRequestView> {
    let boundary = checkpoint.lifecycle.active_boundary.as_ref();
    let signer_request_id = checkpoint
        .pending_requests
        .pending_signer_request_id
        .clone()
        .or_else(|| {
            boundary.and_then(|current| current.signer_request_id.as_ref().map(|id| id.0.clone()))
        });

    let Some(request_id) = signer_request_id else {
        return Vec::new();
    };

    vec![PendingSignerRequestView {
        request_id: request_id.into(),
        chain: None,
        summary: boundary
            .map(|current| current.summary.clone())
            .unwrap_or_else(|| "signer decision required".to_owned()),
    }]
}

fn project_pending_confirmations(checkpoint: &CheckpointSnapshot) -> Vec<PendingConfirmationView> {
    checkpoint
        .pending_requests
        .pending_confirmation_id
        .clone()
        .into_iter()
        .map(|confirmation_id| PendingConfirmationView {
            confirmation_id,
            kind: "chain_confirmation".to_owned(),
            summary: "transaction broadcasted; waiting for chain receipt/confirmation".to_owned(),
        })
        .collect()
}

fn project_pending_continuations(checkpoint: &CheckpointSnapshot) -> Vec<PendingContinuationView> {
    let summary = checkpoint
        .lifecycle
        .active_boundary
        .as_ref()
        .map(|boundary| boundary.summary.clone())
        .unwrap_or_else(|| "artifact continuation required".to_owned());
    checkpoint
        .execution_artifact
        .as_ref()
        .and_then(|snapshot| snapshot.awaiting_continuation.as_ref())
        .into_iter()
        .map(|continuation| {
            let resolved_outputs = checkpoint
                .execution_artifact
                .as_ref()
                .map(|snapshot| {
                    continuation
                        .required_outputs
                        .iter()
                        .filter_map(|output_key| {
                            snapshot
                                .exported_outputs
                                .get(output_key)
                                .cloned()
                                .map(|value| (output_key.clone(), value))
                        })
                        .collect()
                })
                .unwrap_or_default();
            PendingContinuationView {
                stage_id: continuation.stage_id.clone(),
                package_entry: continuation.package_entry.clone(),
                required_outputs: continuation.required_outputs.clone(),
                resolved_outputs,
                summary: summary.clone(),
            }
        })
        .collect()
}

fn project_effect_status(checkpoint: &CheckpointSnapshot) -> EffectStatusView {
    if checkpoint
        .action_graph
        .nodes
        .values()
        .any(|node| node.status == ActionNodeStatus::Failed)
    {
        return EffectStatusView::Violated;
    }

    if checkpoint.lifecycle.status == CoreRunStatus::Completed {
        return EffectStatusView::Satisfied;
    }

    if checkpoint.action_graph.nodes.values().any(|node| {
        matches!(
            node.status,
            ActionNodeStatus::Running | ActionNodeStatus::Ready
        )
    }) {
        return EffectStatusView::Pending;
    }

    EffectStatusView::Unknown
}

fn project_run_result(
    checkpoint: &CheckpointSnapshot,
    recovery: &RecoveryView,
    ownership: RunOwnershipSnapshot,
) -> Option<RunResultView> {
    let lifecycle = &checkpoint.lifecycle;
    if !matches!(
        lifecycle.status,
        CoreRunStatus::Completed | CoreRunStatus::Failed | CoreRunStatus::Cancelled
    ) {
        return None;
    }

    let summary = lifecycle
        .active_boundary
        .as_ref()
        .map(|boundary| boundary.summary.clone())
        .or_else(|| {
            lifecycle
                .failure
                .as_ref()
                .map(|failure| failure.summary.clone())
        })
        .unwrap_or_else(|| "run finalized".to_owned());

    Some(RunResultView {
        summary,
        terminal_failure_context: recovery.failure_context.clone(),
        final_recovery_disposition: recovery.recovery_disposition.clone(),
        final_recovery_suggestions: recovery.recovery_suggestions.clone(),
        ownership,
        interruption_class: recovery.interruption_class.clone(),
        cancel_state: recovery.cancel_state.clone(),
        side_effect_phase: recovery.side_effect_phase.clone(),
    })
}

fn project_ownership_snapshot(checkpoint: &CheckpointSnapshot) -> RunOwnershipSnapshot {
    let policy = classify_claim_policy(checkpoint);
    RunOwnershipSnapshot {
        run_id: checkpoint.lifecycle.run_id.clone(),
        current_claim: None,
        last_terminal_claim_id: None,
        last_claim_transition: None,
        claim_required_for_mutation: policy.claim_required_for_mutation,
        owner_visibility: match policy.owner_visibility {
            OwnershipVisibility::SameSessionOnly => OwnershipVisibility::SameSessionOnly,
            OwnershipVisibility::ObserverReadAllowed => OwnershipVisibility::ObserverReadAllowed,
        },
    }
}

fn action_requires_mutation_claim(action: &RecoveryActionKind) -> bool {
    matches!(
        action,
        RecoveryActionKind::SubmitEvidence
            | RecoveryActionKind::SubmitEnvelope
            | RecoveryActionKind::SubmitSignerDecision
            | RecoveryActionKind::SubmitExecutionArtifactContinuation
            | RecoveryActionKind::SubmitPlanPatch
            | RecoveryActionKind::RetryStep
            | RecoveryActionKind::CancelRun
            | RecoveryActionKind::AwaitConfirmation
    )
}

fn map_recovery_action_retry_intent(action: &RecoveryActionKind) -> Option<RetryIntent> {
    match action {
        RecoveryActionKind::RetryStep => Some(RetryIntent::ResumeExecution),
        RecoveryActionKind::AwaitConfirmation => Some(RetryIntent::PollConfirmation),
        _ => None,
    }
}

fn map_recovery_action_command(action: &RecoveryActionKind) -> &'static str {
    match action {
        RecoveryActionKind::SubmitEvidence => "submit_evidence",
        RecoveryActionKind::SubmitEnvelope => "submit_envelope",
        RecoveryActionKind::SubmitSignerDecision => "submit_signer_decision",
        RecoveryActionKind::SubmitExecutionArtifactContinuation => {
            "submit_execution_artifact_continuation"
        }
        RecoveryActionKind::SubmitPlanPatch => "submit_plan_patch",
        RecoveryActionKind::RetryStep => "step_run",
        RecoveryActionKind::CancelRun => "cancel_run",
        RecoveryActionKind::AwaitConfirmation => "step_run",
        RecoveryActionKind::EscalateUserReview => "escalate_user_review",
    }
}

fn map_recovery_action_description(action: &RecoveryActionKind) -> &'static str {
    match action {
        RecoveryActionKind::SubmitEvidence => {
            "Provide the missing or refreshed evidence needed by the blocked frontier."
        }
        RecoveryActionKind::SubmitEnvelope => {
            "Submit a replacement envelope that satisfies the current runtime constraints."
        }
        RecoveryActionKind::SubmitSignerDecision => {
            "Resolve the pending signer request so execution can continue."
        }
        RecoveryActionKind::SubmitExecutionArtifactContinuation => {
            "Submit the package-built continuation artifact needed to resume execution."
        }
        RecoveryActionKind::SubmitPlanPatch => {
            "Submit a bounded patch for the active frontier before retrying execution."
        }
        RecoveryActionKind::RetryStep => {
            "Run the stepper again when retry or confirmation polling is allowed."
        }
        RecoveryActionKind::CancelRun => "Abort the run instead of attempting further recovery.",
        RecoveryActionKind::AwaitConfirmation => {
            "Wait for more chain confirmation information before making a new decision."
        }
        RecoveryActionKind::EscalateUserReview => {
            "Escalate to the user or outer host review before proceeding."
        }
    }
}

impl From<RecoveryProjection> for RecoveryView {
    fn from(value: RecoveryProjection) -> Self {
        Self {
            recovery_disposition: value.recovery_disposition,
            failure_context: value.failure_context,
            recovery_suggestions: value.recovery_suggestions,
            allowed_recovery_actions: value.allowed_recovery_actions,
            interruption_class: value.interruption_class,
            cancel_state: value.cancel_state,
            side_effect_phase: value.side_effect_phase,
        }
    }
}

fn map_run_status(status: &CoreRunStatus) -> RunStatus {
    match status {
        CoreRunStatus::Created => RunStatus::Created,
        CoreRunStatus::Running => RunStatus::Running,
        CoreRunStatus::Paused => RunStatus::Paused,
        CoreRunStatus::AwaitingEvidence => RunStatus::AwaitingEvidence,
        CoreRunStatus::AwaitingSigner => RunStatus::AwaitingSigner,
        CoreRunStatus::AwaitingConfirmation => RunStatus::AwaitingConfirm,
        CoreRunStatus::AwaitingArtifactContinuation => RunStatus::AwaitingContinuation,
        CoreRunStatus::Completed => RunStatus::Completed,
        CoreRunStatus::Failed => RunStatus::Failed,
        CoreRunStatus::Cancelled => RunStatus::Cancelled,
    }
}

fn map_run_phase(phase: &CoreRunPhase) -> RunPhase {
    match phase {
        CoreRunPhase::MissionAccepted => RunPhase::MissionAccepted,
        CoreRunPhase::Planning => RunPhase::Planning,
        CoreRunPhase::Simulating => RunPhase::Simulating,
        CoreRunPhase::Governing => RunPhase::Governing,
        CoreRunPhase::AwaitingHost => RunPhase::AwaitingHost,
        CoreRunPhase::Broadcasting => RunPhase::Broadcasting,
        CoreRunPhase::Verifying => RunPhase::Verifying,
        CoreRunPhase::Recovering => RunPhase::Recovering,
        CoreRunPhase::Finalized => RunPhase::Finalized,
    }
}

fn map_boundary_kind(kind: &CoreBoundaryKind) -> BoundaryKind {
    match kind {
        CoreBoundaryKind::Pause => BoundaryKind::Pause,
        CoreBoundaryKind::Evidence => BoundaryKind::Evidence,
        CoreBoundaryKind::Signer => BoundaryKind::Signer,
        CoreBoundaryKind::Confirmation => BoundaryKind::Confirmation,
        CoreBoundaryKind::ArtifactContinuation => BoundaryKind::ArtifactContinuation,
        CoreBoundaryKind::Completion => BoundaryKind::Completion,
        CoreBoundaryKind::Failure => BoundaryKind::Failure,
        CoreBoundaryKind::Cancellation => BoundaryKind::Cancellation,
    }
}
