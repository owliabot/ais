use std::collections::BTreeMap;

use ais_agent_control::{
    audit::{
        GovernorDecisionAuditRecord, PlanPatchAuditRecord, RecoveryAuditRecord, RuntimeAudit,
        RuntimeAuditRecord,
    },
    commands::{MissionBudgetSubmission, MissionSubmission, RunCommand, StepUntil},
    events::RunEvent,
    ids::{AuditId, ClaimId, IdempotencyKey, RunId},
    launch_spec::LaunchSpecSubmission,
    patch::{PatchOutcome, PlanPatchOperation, PlanPatchSubmission},
};
use ais_agent_core::{
    action::{ActionGraph, ActionNodeStatus},
    checkpoint::{CheckpointSnapshot, PendingRequestsSnapshot},
    evidence::EvidenceGraph,
    mission::{Mission, MissionBudget, MissionPolicy},
    runtime::{RunLifecycleState, RunPhase, RunStatus},
};
use ais_agent_host::{
    control::{HostCommandOutcome, HostCommandResponse},
    envelope::HostEnvelopeSubmission,
    events::{HostRunEventBatch, HostRunEventQuery},
    evidence::HostEvidenceSubmission,
    session::HostedRunCommand,
    signer::HostSignerResolution,
};

use crate::{
    persistence::{
        CheckpointArchiveEntry, CheckpointArchiveKind, EventArchiveSlice, RunCatalogEntry,
    },
    runtime::{classify_recovery_disposition, ActiveRun, RuntimePatchOutcome},
    stepper::StepUntilBoundary,
};

pub(super) fn checkpoint_is_newer(
    durable_checkpoint: &CheckpointSnapshot,
    hot_checkpoint: &CheckpointSnapshot,
) -> bool {
    (
        durable_checkpoint.checkpoint_seq,
        durable_checkpoint.plan_epoch,
    ) > (hot_checkpoint.checkpoint_seq, hot_checkpoint.plan_epoch)
}

pub(super) fn replay_key(command: &HostedRunCommand) -> Option<IdempotencyKey> {
    match &command.command {
        RunCommand::BeginRun(begin) => Some(begin.idempotency_key.clone()),
        RunCommand::InspectRun(_) => None,
        RunCommand::ClaimRun(_)
        | RunCommand::RenewRunClaim(_)
        | RunCommand::ReleaseRunClaim(_)
        | RunCommand::StepRun(_)
        | RunCommand::SubmitEvidence(_)
        | RunCommand::SubmitEnvelope(_)
        | RunCommand::SubmitSignerResolution(_)
        | RunCommand::SubmitPlanPatch(_)
        | RunCommand::SubmitExecutionArtifactContinuation(_)
        | RunCommand::RequestCancelRun(_)
        | RunCommand::CancelRun(_) => command
            .host_request_id
            .as_ref()
            .map(|request_id| IdempotencyKey(request_id.0.clone())),
    }
}

pub(super) fn command_id(command: &RunCommand) -> &ais_agent_control::ids::CommandId {
    match command {
        RunCommand::BeginRun(command) => &command.command_id,
        RunCommand::InspectRun(command) => &command.command_id,
        RunCommand::ClaimRun(command) => &command.command_id,
        RunCommand::RenewRunClaim(command) => &command.command_id,
        RunCommand::ReleaseRunClaim(command) => &command.command_id,
        RunCommand::StepRun(command) => &command.command_id,
        RunCommand::SubmitEvidence(command) => &command.command_id,
        RunCommand::SubmitEnvelope(command) => &command.command_id,
        RunCommand::SubmitSignerResolution(command) => &command.command_id,
        RunCommand::SubmitPlanPatch(command) => &command.command_id,
        RunCommand::SubmitExecutionArtifactContinuation(command) => &command.command_id,
        RunCommand::RequestCancelRun(command) => &command.command_id,
        RunCommand::CancelRun(command) => &command.command_id,
    }
}

pub(super) fn command_run_id(command: &RunCommand) -> Option<RunId> {
    match command {
        RunCommand::BeginRun(_) => None,
        RunCommand::InspectRun(command) => Some(command.run_id.clone()),
        RunCommand::ClaimRun(command) => Some(command.run_id.clone()),
        RunCommand::RenewRunClaim(command) => Some(command.run_id.clone()),
        RunCommand::ReleaseRunClaim(command) => Some(command.run_id.clone()),
        RunCommand::StepRun(command) => Some(command.run_id.clone()),
        RunCommand::SubmitEvidence(command) => Some(command.run_id.clone()),
        RunCommand::SubmitEnvelope(command) => Some(command.run_id.clone()),
        RunCommand::SubmitSignerResolution(command) => Some(command.run_id.clone()),
        RunCommand::SubmitPlanPatch(command) => Some(command.run_id.clone()),
        RunCommand::SubmitExecutionArtifactContinuation(command) => Some(command.run_id.clone()),
        RunCommand::RequestCancelRun(command) => Some(command.run_id.clone()),
        RunCommand::CancelRun(command) => Some(command.run_id.clone()),
    }
}

pub(super) fn outcome_run_id(outcome: &HostCommandOutcome) -> Option<RunId> {
    match &outcome.response {
        HostCommandResponse::Accepted(response) => response.run_id.clone(),
        HostCommandResponse::Inspect(snapshot) => Some(snapshot.run_id.clone()),
        HostCommandResponse::Pause(bundle) => Some(bundle.run_id.clone()),
        HostCommandResponse::Session(snapshot) => snapshot.active_run_id.clone(),
        HostCommandResponse::Error(_) => None,
    }
}

pub(super) fn completed_replay_claim_id(
    outcome: &HostCommandOutcome,
    registered_claim_id: Option<ClaimId>,
) -> Option<ClaimId> {
    outcome_claim_id(outcome).or(registered_claim_id)
}

pub(super) fn outcome_claim_id(outcome: &HostCommandOutcome) -> Option<ClaimId> {
    match &outcome.response {
        HostCommandResponse::Inspect(snapshot) => snapshot
            .ownership
            .current_claim
            .as_ref()
            .map(|claim| claim.claim_id.clone()),
        HostCommandResponse::Pause(bundle) => bundle
            .ownership
            .current_claim
            .as_ref()
            .map(|claim| claim.claim_id.clone()),
        HostCommandResponse::Accepted(_)
        | HostCommandResponse::Session(_)
        | HostCommandResponse::Error(_) => None,
    }
}

pub(super) fn normalize_mission(
    submission: MissionSubmission,
    run_seq: u64,
    launch_spec: &LaunchSpecSubmission,
) -> Mission {
    Mission {
        mission_id: format!("mission-{run_seq}"),
        goal: submission.goal,
        allowed_chains: submission.allowed_chains,
        budget: normalize_budget(submission.budget),
        policy: MissionPolicy {
            policy_mode: Some("guarded".to_owned()),
            allow_raw_envelopes: true,
            require_effect_contract_for_writes: requires_effect_contract_for_launch_spec(
                launch_spec,
            ),
        },
        constraints: submission.constraints,
        metadata: submission.metadata,
    }
}

fn requires_effect_contract_for_launch_spec(launch_spec: &LaunchSpecSubmission) -> bool {
    match launch_spec {
        LaunchSpecSubmission::ExecutionArtifact(_) => false,
        LaunchSpecSubmission::PrebuiltFragment(_) | LaunchSpecSubmission::ReflectionRequest(_) => {
            true
        }
    }
}

fn normalize_budget(submission: Option<MissionBudgetSubmission>) -> MissionBudget {
    MissionBudget {
        max_steps: submission.as_ref().and_then(|budget| budget.max_steps),
        max_signer_requests: submission
            .as_ref()
            .and_then(|budget| budget.max_signer_requests),
        max_wall_clock_ms: submission.and_then(|budget| budget.max_wall_clock_ms),
    }
}

pub(super) fn initial_checkpoint(run_id: RunId, mission: &Mission) -> CheckpointSnapshot {
    let mut lifecycle = RunLifecycleState::new(run_id, mission.mission_id.clone());
    lifecycle.mark_running(RunPhase::MissionAccepted);

    CheckpointSnapshot {
        run_id: lifecycle.run_id.0.clone(),
        mission_id: mission.mission_id.clone(),
        checkpoint_seq: lifecycle.checkpoint_seq,
        plan_epoch: lifecycle.plan_epoch,
        lifecycle,
        action_graph: ActionGraph {
            graph_id: Some(format!("graph-{}", mission.mission_id)),
            roots: Vec::new(),
            terminals: Vec::new(),
            nodes: BTreeMap::new(),
        },
        evidence_graph: EvidenceGraph::default(),
        effect_contracts: Default::default(),
        pending_requests: PendingRequestsSnapshot::default(),
        last_completed_node_id: None,
        actuation_records: Vec::new(),
        execution_artifact: None,
    }
}

pub(super) fn map_until(until: StepUntil) -> StepUntilBoundary {
    match until {
        StepUntil::NextBoundary => StepUntilBoundary::NextBoundary,
        StepUntil::CompleteOrBoundary => StepUntilBoundary::CompleteOrBoundary,
        StepUntil::BudgetExhausted => StepUntilBoundary::BudgetExhausted,
    }
}

pub(super) fn host_evidence_submission(
    run_id: RunId,
    evidence: ais_agent_control::commands::EvidenceSubmission,
) -> HostEvidenceSubmission {
    HostEvidenceSubmission {
        run_id,
        evidence_id: evidence.evidence_id,
        kind: match evidence.kind {
            ais_agent_control::commands::EvidenceKind::Fact => {
                ais_agent_core::evidence::EvidenceKind::Fact
            }
            ais_agent_control::commands::EvidenceKind::QueryResult => {
                ais_agent_core::evidence::EvidenceKind::QueryResult
            }
            ais_agent_control::commands::EvidenceKind::RouteOrQuote => {
                ais_agent_core::evidence::EvidenceKind::RouteOrQuote
            }
            ais_agent_control::commands::EvidenceKind::Metadata => {
                ais_agent_core::evidence::EvidenceKind::Metadata
            }
            ais_agent_control::commands::EvidenceKind::ExternalObservation => {
                ais_agent_core::evidence::EvidenceKind::ExternalObservation
            }
        },
        source: evidence.source,
        observed_at_ms: evidence.observed_at_ms,
        expires_at_ms: None,
        max_age_ms: None,
        chain_scope: evidence.chain_scope,
        trace_hint: None,
        confidence_ppm: evidence
            .confidence
            .map(|confidence| (confidence.clamp(0.0, 1.0) * 1_000_000.0) as u32),
        payload: evidence.payload,
    }
}

pub(super) fn host_envelope_submission(
    run_id: RunId,
    envelope: ais_agent_control::commands::EnvelopeSubmission,
) -> Result<HostEnvelopeSubmission, String> {
    let expected_effect_contract = envelope
        .expected_effect
        .clone()
        .map(serde_json::from_value::<ais_agent_core::effect::EffectContract>)
        .transpose()
        .map_err(|error| format!("invalid expected_effect contract: {error}"))?;

    Ok(HostEnvelopeSubmission {
        run_id,
        envelope_id: envelope.envelope_id,
        kind: match envelope.kind {
            ais_agent_control::commands::EnvelopeKind::EvmEnvelope => {
                ais_agent_host::envelope::HostEnvelopeKind::EvmEnvelope
            }
            ais_agent_control::commands::EnvelopeKind::SolanaEnvelope => {
                ais_agent_host::envelope::HostEnvelopeKind::SolanaEnvelope
            }
            ais_agent_control::commands::EnvelopeKind::ExternalJob => {
                ais_agent_host::envelope::HostEnvelopeKind::ExternalJob
            }
        },
        chain: envelope.chain,
        payload: envelope.payload,
        expected_effect_ref: expected_effect_contract
            .as_ref()
            .map(|contract| contract.effect_id.clone()),
        expected_effect_contract,
        provenance: envelope.provenance,
    })
}

pub(super) fn resolve_pending_envelope_recovery(runtime: &mut ActiveRun, envelope_id: &str) {
    if !runtime
        .checkpoint
        .pending_requests
        .pending_envelope_refs
        .iter()
        .any(|pending| pending == envelope_id)
    {
        return;
    }

    runtime
        .checkpoint
        .pending_requests
        .pending_envelope_refs
        .retain(|pending| pending != envelope_id);

    if !runtime
        .checkpoint
        .pending_requests
        .pending_envelope_refs
        .is_empty()
    {
        if let Some(boundary) = runtime.checkpoint.lifecycle.active_boundary.as_mut() {
            boundary.blocking_refs = runtime
                .checkpoint
                .pending_requests
                .pending_envelope_refs
                .clone();
        }
        return;
    }

    let recovery_nodes = runtime
        .checkpoint
        .lifecycle
        .failure
        .as_ref()
        .filter(|failure| {
            matches!(
                failure.code,
                ais_agent_control::recovery::RunFailureCode::EnvelopeInvalid
            )
        })
        .map(|failure| failure.node_refs.clone())
        .unwrap_or_default();

    for node_id in recovery_nodes {
        if let Some(node) = runtime.checkpoint.action_graph.nodes.get_mut(&node_id) {
            if matches!(
                node.status,
                ActionNodeStatus::Blocked | ActionNodeStatus::Failed
            ) {
                node.status = ActionNodeStatus::Ready;
            }
        }
    }

    runtime
        .checkpoint
        .lifecycle
        .mark_running(RunPhase::Recovering);
}

pub(super) fn host_signer_resolution(
    run_id: RunId,
    resolution: ais_agent_control::commands::SignerResolutionSubmission,
) -> HostSignerResolution {
    HostSignerResolution {
        run_id,
        request_id: resolution.request_id,
        kind: match resolution.kind {
            ais_agent_control::commands::SignerResolutionKind::Denied => {
                ais_agent_host::signer::HostSignerResolutionKind::Denied
            }
            ais_agent_control::commands::SignerResolutionKind::Submitted => {
                ais_agent_host::signer::HostSignerResolutionKind::Submitted
            }
            ais_agent_control::commands::SignerResolutionKind::Signed => {
                ais_agent_host::signer::HostSignerResolutionKind::Signed
            }
            ais_agent_control::commands::SignerResolutionKind::Expired => {
                ais_agent_host::signer::HostSignerResolutionKind::Expired
            }
        },
        resolved_at_ms: None,
        submission_id: resolution.submission_id,
        signed_payload: resolution.signed_payload,
        details: resolution.details,
    }
}

pub(super) fn host_event_batch(
    query: HostRunEventQuery,
    slice: EventArchiveSlice,
) -> HostRunEventBatch {
    HostRunEventBatch {
        run_id: query.run_id,
        after_event_seq: query.after_event_seq,
        latest_event_seq: slice.latest_event_seq,
        next_after_event_seq: slice.next_after_event_seq,
        truncated: slice.truncated,
        events: slice.events,
    }
}

pub(super) fn patch_audit_outcome(
    runtime: &ActiveRun,
    patch: &PlanPatchSubmission,
    outcome: &RuntimePatchOutcome,
) -> PatchOutcome {
    let mut preserved_effect_refs = outcome.updated_effect_refs.clone();
    for operation in &patch.operations {
        match operation {
            PlanPatchOperation::ReplaceFragment {
                preserved_effect_refs: refs,
                ..
            }
            | PlanPatchOperation::AppendFragment {
                preserved_effect_refs: refs,
                ..
            } => {
                preserved_effect_refs.extend(refs.clone());
            }
            PlanPatchOperation::ReplaceEffectContract { effect_ref, .. } => {
                preserved_effect_refs.push(effect_ref.clone());
            }
            PlanPatchOperation::DropBranch { .. }
            | PlanPatchOperation::TightenConstraints { .. } => {}
        }
    }
    preserved_effect_refs.sort();
    preserved_effect_refs.dedup();

    PatchOutcome {
        next_recovery_disposition: classify_recovery_disposition(&runtime.checkpoint),
        touched_node_refs: outcome.patched_node_refs.clone(),
        preserved_effect_refs,
    }
}

pub(super) fn run_catalog_entry(
    runtime: &ActiveRun,
    latest_event_seq: Option<u64>,
) -> RunCatalogEntry {
    let status = runtime.checkpoint.lifecycle.status.clone();
    RunCatalogEntry {
        run_id: runtime.run_id.clone(),
        mission_id: runtime.mission.mission_id.clone(),
        status: status.clone(),
        phase: runtime.checkpoint.lifecycle.phase.clone(),
        active_boundary_kind: runtime
            .checkpoint
            .lifecycle
            .active_boundary
            .as_ref()
            .map(|boundary| boundary.kind.clone()),
        latest_checkpoint_seq: runtime.checkpoint.checkpoint_seq,
        latest_event_seq,
        latest_revision: runtime.revision,
        created_at_ms: None,
        updated_at_ms: runtime.last_updated_at_ms,
        terminal_at_ms: match status {
            RunStatus::Completed | RunStatus::Failed | RunStatus::Cancelled => {
                runtime.last_updated_at_ms
            }
            _ => None,
        },
    }
}

pub(super) fn checkpoint_entry(
    runtime: &ActiveRun,
    kind: CheckpointArchiveKind,
) -> CheckpointArchiveEntry {
    CheckpointArchiveEntry {
        snapshot: runtime.checkpoint.clone(),
        kind,
    }
}

pub(super) fn runtime_audit_records(
    events: &[ais_agent_control::events::RunEventEnvelope],
    latest_audit_seq: Option<u64>,
) -> Vec<RuntimeAuditRecord> {
    let mut next_audit_seq = latest_audit_seq.unwrap_or(0);
    let mut records = Vec::new();

    for event in events {
        let audit = match &event.event {
            RunEvent::RecoveryAudit(event) => RuntimeAudit::Recovery(RecoveryAuditRecord {
                recovery_disposition: event.recovery_disposition.clone(),
                failure_context: event.failure_context.clone(),
                recovery_suggestions: event.recovery_suggestions.clone(),
                allowed_recovery_actions: event.allowed_recovery_actions.clone(),
            }),
            RunEvent::GovernorDecision(event) => {
                RuntimeAudit::GovernorDecision(GovernorDecisionAuditRecord {
                    node_id: event.node_id.clone(),
                    decision: event.decision.clone(),
                    reason: event.reason.clone(),
                    evidence_refs: event.evidence_refs.clone(),
                    signer_request_id: event.signer_request_id.clone(),
                    rejection_code: event.rejection_code.clone(),
                })
            }
            RunEvent::PlanPatchAudit(event) => RuntimeAudit::PlanPatch(PlanPatchAuditRecord {
                patch_id: event.patch_id.clone(),
                status: event.status.clone(),
                patch: event.patch.clone(),
                outcome: event.outcome.clone(),
                message: event.message.clone(),
            }),
            _ => continue,
        };

        next_audit_seq = next_audit_seq.saturating_add(1);
        records.push(RuntimeAuditRecord {
            audit_id: AuditId(format!("{}:audit:{next_audit_seq}", event.run_id.0)),
            run_id: event.run_id.clone(),
            audit_seq: next_audit_seq,
            checkpoint_seq: event.checkpoint_seq,
            plan_epoch: event.plan_epoch,
            audit,
        });
    }

    records
}
