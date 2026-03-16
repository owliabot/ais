//! Runtime event emission shell.

use ais_agent_control::{
    events::{
        GovernorDecisionAuditKind, PlanPatchAuditStatus, RunAwaitingContinuation,
        RunAwaitingEvidence, RunAwaitingSigner, RunCompleted, RunEvent, RunEventEnvelope,
        RunFailed, RunGovernorDecision, RunPaused, RunPlanPatchAudit, RunProgress,
        RunRecoveryAudit, RunStarted,
    },
    ids::EventId,
    patch::{PatchOutcome, PlanPatchSubmission},
    recovery::RunFailureCode,
};
use ais_agent_core::runtime::{BoundaryKind, RunStatus};

use crate::{
    runtime::{classify_recovery_view, ActiveRun},
    stepper::{StepTransition, StepTransitionKind},
};

#[derive(Debug, Default)]
pub struct RuntimeEventEmitter;

impl RuntimeEventEmitter {
    pub fn emit_started(
        runtime: &mut ActiveRun,
        phase: impl Into<String>,
    ) -> Vec<RunEventEnvelope> {
        vec![envelope(
            runtime,
            RunEvent::Started(RunStarted {
                event_id: event_id(runtime, "started"),
                run_id: runtime.run_id.clone(),
                phase: phase.into(),
            }),
        )]
    }

    pub fn emit_after_step(
        runtime: &mut ActiveRun,
        transition: &StepTransition,
    ) -> Vec<RunEventEnvelope> {
        let mut events = vec![envelope(
            runtime,
            RunEvent::Progress(RunProgress {
                event_id: event_id(runtime, "progress"),
                run_id: runtime.run_id.clone(),
                phase: format!("{:?}", runtime.checkpoint.lifecycle.phase).to_lowercase(),
                summary: transition.summary.clone(),
            }),
        )];

        if let Some(governor_event) = governor_decision_event(runtime, transition) {
            events.push(envelope(runtime, governor_event));
        }

        match runtime.checkpoint.lifecycle.status {
            RunStatus::AwaitingEvidence => events.push(envelope(
                runtime,
                RunEvent::AwaitingEvidence(RunAwaitingEvidence {
                    event_id: event_id(runtime, "awaiting_evidence"),
                    run_id: runtime.run_id.clone(),
                    reason: runtime
                        .checkpoint
                        .lifecycle
                        .active_boundary
                        .as_ref()
                        .map(|boundary| boundary.summary.clone())
                        .unwrap_or_else(|| "additional evidence required".to_owned()),
                    missing_refs: runtime
                        .checkpoint
                        .pending_requests
                        .pending_evidence_refs
                        .clone(),
                }),
            )),
            RunStatus::AwaitingSigner => {
                let request_id = runtime
                    .pending_signer_state
                    .as_ref()
                    .map(|request| request.request_id.clone())
                    .or_else(|| {
                        runtime
                            .checkpoint
                            .lifecycle
                            .active_boundary
                            .as_ref()
                            .and_then(|boundary| boundary.signer_request_id.clone())
                    });
                if let Some(request_id) = request_id {
                    events.push(envelope(
                        runtime,
                        RunEvent::AwaitingSigner(RunAwaitingSigner {
                            event_id: event_id(runtime, "awaiting_signer"),
                            run_id: runtime.run_id.clone(),
                            request_id,
                            reason: runtime
                                .checkpoint
                                .lifecycle
                                .active_boundary
                                .as_ref()
                                .map(|boundary| boundary.summary.clone())
                                .unwrap_or_else(|| "signer decision required".to_owned()),
                        }),
                    ));
                }
            }
            RunStatus::AwaitingConfirmation => events.push(envelope(
                runtime,
                RunEvent::AwaitingConfirm(ais_agent_control::events::RunAwaitingConfirm {
                    event_id: event_id(runtime, "awaiting_confirm"),
                    run_id: runtime.run_id.clone(),
                    confirmation_id: runtime
                        .checkpoint
                        .pending_requests
                        .pending_confirmation_id
                        .clone(),
                    reason: runtime
                        .checkpoint
                        .lifecycle
                        .active_boundary
                        .as_ref()
                        .map(|boundary| boundary.summary.clone())
                        .unwrap_or_else(|| "waiting for chain confirmation".to_owned()),
                }),
            )),
            RunStatus::AwaitingArtifactContinuation => events.push(envelope(
                runtime,
                RunEvent::AwaitingContinuation(RunAwaitingContinuation {
                    event_id: event_id(runtime, "awaiting_artifact_continuation"),
                    run_id: runtime.run_id.clone(),
                    stage_id: runtime
                        .checkpoint
                        .execution_artifact
                        .as_ref()
                        .and_then(|artifact| artifact.awaiting_continuation.as_ref())
                        .map(|continuation| continuation.stage_id.clone()),
                    package_entry: runtime
                        .checkpoint
                        .execution_artifact
                        .as_ref()
                        .and_then(|artifact| artifact.awaiting_continuation.as_ref())
                        .map(|continuation| continuation.package_entry.clone()),
                    required_outputs: runtime
                        .checkpoint
                        .execution_artifact
                        .as_ref()
                        .and_then(|artifact| artifact.awaiting_continuation.as_ref())
                        .map(|continuation| continuation.required_outputs.clone())
                        .unwrap_or_default(),
                    resolved_outputs: runtime
                        .checkpoint
                        .execution_artifact
                        .as_ref()
                        .and_then(|artifact| {
                            artifact
                                .awaiting_continuation
                                .as_ref()
                                .map(|continuation| (artifact, continuation))
                        })
                        .map(|(artifact, continuation)| {
                            continuation
                                .required_outputs
                                .iter()
                                .filter_map(|output_key| {
                                    artifact
                                        .exported_outputs
                                        .get(output_key)
                                        .cloned()
                                        .map(|value| (output_key.clone(), value))
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                    reason: runtime
                        .checkpoint
                        .lifecycle
                        .active_boundary
                        .as_ref()
                        .map(|boundary| boundary.summary.clone())
                        .unwrap_or_else(|| "artifact continuation required".to_owned()),
                }),
            )),
            RunStatus::Paused => events.push(envelope(
                runtime,
                RunEvent::Paused(RunPaused {
                    event_id: event_id(runtime, "paused"),
                    run_id: runtime.run_id.clone(),
                    reason: runtime
                        .checkpoint
                        .lifecycle
                        .active_boundary
                        .as_ref()
                        .map(|boundary| boundary.summary.clone())
                        .unwrap_or_else(|| "run paused".to_owned()),
                }),
            )),
            RunStatus::Completed => events.push(envelope(
                runtime,
                RunEvent::Completed(RunCompleted {
                    event_id: event_id(runtime, "completed"),
                    run_id: runtime.run_id.clone(),
                    summary: runtime
                        .checkpoint
                        .lifecycle
                        .active_boundary
                        .as_ref()
                        .map(|boundary| boundary.summary.clone())
                        .unwrap_or_else(|| "run completed".to_owned()),
                    result: None,
                }),
            )),
            RunStatus::Failed => {
                let failure = runtime.checkpoint.lifecycle.failure.as_ref();
                events.push(envelope(
                    runtime,
                    RunEvent::Failed(RunFailed {
                        event_id: event_id(runtime, "failed"),
                        run_id: runtime.run_id.clone(),
                        phase: format!("{:?}", runtime.checkpoint.lifecycle.phase).to_lowercase(),
                        code: failure
                            .map(|failure| failure.code.clone())
                            .unwrap_or(RunFailureCode::RuntimeInvariantViolation),
                        message: failure
                            .map(|failure| failure.summary.clone())
                            .unwrap_or_else(|| "runtime failed".to_owned()),
                        failure_context: failure.cloned(),
                    }),
                ));
            }
            RunStatus::Cancelled | RunStatus::Created | RunStatus::Running => {}
        }

        if matches!(
            runtime
                .checkpoint
                .lifecycle
                .active_boundary
                .as_ref()
                .map(|boundary| &boundary.kind),
            Some(BoundaryKind::Pause)
        ) && !matches!(runtime.checkpoint.lifecycle.status, RunStatus::Paused)
        {
            events.push(envelope(
                runtime,
                RunEvent::Paused(RunPaused {
                    event_id: event_id(runtime, "paused"),
                    run_id: runtime.run_id.clone(),
                    reason: runtime
                        .checkpoint
                        .lifecycle
                        .active_boundary
                        .as_ref()
                        .map(|boundary| boundary.summary.clone())
                        .unwrap_or_else(|| "run paused".to_owned()),
                }),
            ));
        }

        if let Some(recovery_event) = recovery_audit_event(runtime) {
            events.push(envelope(runtime, recovery_event));
        }

        events
    }

    pub fn emit_plan_patch_submitted(
        runtime: &mut ActiveRun,
        patch: &PlanPatchSubmission,
    ) -> RunEventEnvelope {
        envelope(
            runtime,
            RunEvent::PlanPatchAudit(RunPlanPatchAudit {
                event_id: event_id(runtime, "plan_patch_submitted"),
                run_id: runtime.run_id.clone(),
                patch_id: patch.patch_id.clone(),
                status: PlanPatchAuditStatus::Submitted,
                patch: patch.clone(),
                outcome: None,
                message: None,
            }),
        )
    }

    pub fn emit_plan_patch_applied(
        runtime: &mut ActiveRun,
        patch: &PlanPatchSubmission,
        outcome: Option<PatchOutcome>,
    ) -> RunEventEnvelope {
        envelope(
            runtime,
            RunEvent::PlanPatchAudit(RunPlanPatchAudit {
                event_id: event_id(runtime, "plan_patch_applied"),
                run_id: runtime.run_id.clone(),
                patch_id: patch.patch_id.clone(),
                status: PlanPatchAuditStatus::Applied,
                patch: patch.clone(),
                outcome,
                message: None,
            }),
        )
    }

    pub fn emit_plan_patch_rejected(
        runtime: &mut ActiveRun,
        patch: &PlanPatchSubmission,
        message: impl Into<String>,
    ) -> RunEventEnvelope {
        envelope(
            runtime,
            RunEvent::PlanPatchAudit(RunPlanPatchAudit {
                event_id: event_id(runtime, "plan_patch_rejected"),
                run_id: runtime.run_id.clone(),
                patch_id: patch.patch_id.clone(),
                status: PlanPatchAuditStatus::Rejected,
                patch: patch.clone(),
                outcome: None,
                message: Some(message.into()),
            }),
        )
    }
}

fn recovery_audit_event(runtime: &ActiveRun) -> Option<RunEvent> {
    let recovery = classify_recovery_view(&runtime.checkpoint);
    if recovery.recovery_disposition.is_none()
        && recovery.failure_context.is_none()
        && recovery.recovery_suggestions.is_empty()
        && recovery.allowed_recovery_actions.is_empty()
    {
        return None;
    }

    Some(RunEvent::RecoveryAudit(RunRecoveryAudit {
        event_id: event_id(runtime, "recovery_audit"),
        run_id: runtime.run_id.clone(),
        recovery_disposition: recovery.recovery_disposition,
        failure_context: recovery.failure_context,
        recovery_suggestions: recovery.recovery_suggestions,
        allowed_recovery_actions: recovery.allowed_recovery_actions,
    }))
}

fn governor_decision_event(runtime: &ActiveRun, transition: &StepTransition) -> Option<RunEvent> {
    if transition.kind != StepTransitionKind::Govern {
        return None;
    }

    let lifecycle = &runtime.checkpoint.lifecycle;
    let failure = lifecycle.failure.as_ref();
    let (decision, evidence_refs, signer_request_id, rejection_code) = match lifecycle.status {
        RunStatus::AwaitingSigner => (
            GovernorDecisionAuditKind::AllowWithSigner,
            Vec::new(),
            runtime
                .pending_signer_state
                .as_ref()
                .map(|request| request.request_id.clone())
                .or_else(|| {
                    runtime
                        .checkpoint
                        .pending_requests
                        .pending_signer_request_id
                        .clone()
                        .map(ais_agent_control::ids::SignerRequestId)
                }),
            None,
        ),
        RunStatus::AwaitingEvidence => (
            GovernorDecisionAuditKind::RequireMoreEvidence,
            runtime
                .checkpoint
                .pending_requests
                .pending_evidence_refs
                .clone(),
            None,
            failure.map(|failure| format!("{:?}", failure.code).to_lowercase()),
        ),
        RunStatus::Paused if matches!(failure.map(|failure| &failure.stage), Some(stage) if *stage == ais_agent_control::recovery::RunFailureStage::Govern) => {
            (
                GovernorDecisionAuditKind::Reject,
                runtime
                    .checkpoint
                    .pending_requests
                    .pending_evidence_refs
                    .clone(),
                None,
                failure.map(|failure| format!("{:?}", failure.code).to_lowercase()),
            )
        }
        _ => (GovernorDecisionAuditKind::Allow, Vec::new(), None, None),
    };

    Some(RunEvent::GovernorDecision(RunGovernorDecision {
        event_id: event_id(runtime, "governor_decision"),
        run_id: runtime.run_id.clone(),
        node_id: transition.node_id.clone(),
        decision,
        reason: transition.summary.clone(),
        evidence_refs,
        signer_request_id,
        rejection_code,
    }))
}

fn envelope(runtime: &mut ActiveRun, event: RunEvent) -> RunEventEnvelope {
    let event_seq = runtime.next_event_seq();
    let envelope = RunEventEnvelope {
        run_id: runtime.run_id.clone(),
        event_seq,
        checkpoint_seq: runtime.checkpoint_seq(),
        plan_epoch: runtime.plan_epoch(),
        event,
    };
    runtime.record_event(envelope.clone());
    envelope
}

fn event_id(runtime: &ActiveRun, kind: &str) -> EventId {
    EventId(format!(
        "{}:{kind}:{}",
        runtime.run_id.0,
        runtime.event_seq + 1
    ))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use ais_agent_control::{
        events::RunEvent,
        execution_artifact::{ExecutionArtifactLaunchSpec, ExecutionChainFamily},
        ids::RunId,
    };
    use ais_agent_core::{
        action::{
            kinds::observe::{ObserveAction, ObserveSourceKind},
            ActionGraph, ActionNode, ActionNodeKind, ActionNodeStatus, ActionOrigin, ActionPayload,
        },
        checkpoint::{
            CheckpointSnapshot, ExecutionArtifactRuntimeSnapshot, PendingRequestsSnapshot,
        },
        evidence::{
            EvidenceFreshness, EvidenceGraph, EvidenceKind, EvidenceProvenance, EvidenceRecord,
        },
        mission::{Mission, MissionBudget, MissionPolicy},
        runtime::{RunLifecycleState, RunPhase},
    };
    use serde_json::json;

    use crate::{
        runtime::ActiveRun,
        stepper::{StepTransition, StepTransitionKind},
    };

    use super::RuntimeEventEmitter;

    #[test]
    fn observe_only_completion_reuses_generic_completed_event_contract() {
        let mut lifecycle = RunLifecycleState::new(RunId("run-observe".to_owned()), "mission-1");
        lifecycle.mark_running(RunPhase::Planning);
        lifecycle.bump_checkpoint();
        lifecycle.bump_plan_epoch();
        lifecycle.complete("observe-only artifact completed");

        let checkpoint = CheckpointSnapshot {
            run_id: "run-observe".to_owned(),
            mission_id: "mission-1".to_owned(),
            checkpoint_seq: lifecycle.checkpoint_seq,
            plan_epoch: lifecycle.plan_epoch,
            lifecycle,
            action_graph: ActionGraph {
                graph_id: Some("artifact.stage.quote".to_owned()),
                roots: vec!["artifact.stage.quote.observe".to_owned()],
                terminals: vec!["artifact.stage.quote.observe".to_owned()],
                nodes: BTreeMap::from([(
                    "artifact.stage.quote.observe".to_owned(),
                    ActionNode {
                        node_id: "artifact.stage.quote.observe".to_owned(),
                        kind: ActionNodeKind::Observe,
                        origin: ActionOrigin::DriverFragment,
                        status: ActionNodeStatus::Succeeded,
                        depends_on: Vec::new(),
                        inputs: Vec::new(),
                        evidence_refs: vec!["query.quote".to_owned()],
                        payload: ActionPayload::Observe(ObserveAction {
                            source_kind: ObserveSourceKind::ChainRead,
                            source_hint: "observe-only query completed".to_owned(),
                            output_key: Some("query.quote".to_owned()),
                            live: None,
                        }),
                        implementation_hint: Some("execution_artifact".to_owned()),
                        expected_effect_ref: None,
                    },
                )]),
            },
            evidence_graph: EvidenceGraph {
                records: BTreeMap::from([(
                    "query.quote".to_owned(),
                    EvidenceRecord {
                        evidence_id: "query.quote".to_owned(),
                        kind: EvidenceKind::ExternalObservation,
                        provenance: EvidenceProvenance {
                            source: "evm.alloy.live_read".to_owned(),
                            chain_scope: Some("eip155:1".to_owned()),
                            trace_hint: Some("artifact.stage.quote.observe".to_owned()),
                        },
                        freshness: EvidenceFreshness {
                            observed_at_ms: Some(1_000),
                            expires_at_ms: None,
                            max_age_ms: None,
                        },
                        confidence_ppm: Some(1_000_000),
                        payload: json!({"decoded_u256": "10000000000000000"}),
                    },
                )]),
                requirements: Vec::new(),
                usages: Vec::new(),
            },
            effect_contracts: Default::default(),
            pending_requests: PendingRequestsSnapshot::default(),
            last_completed_node_id: Some("artifact.stage.quote.observe".to_owned()),
            actuation_records: Vec::new(),
            execution_artifact: Some(ExecutionArtifactRuntimeSnapshot {
                launch_spec: ExecutionArtifactLaunchSpec {
                    protocol_package_id: "owliabot.uniswap_v3".to_owned(),
                    action_key: "quote_exact_in_single".to_owned(),
                    chain_family: ExecutionChainFamily::Evm,
                    allowed_chains: vec!["eip155:1".to_owned()],
                    entry_stage_id: "stage.quote".into(),
                    actor: None,
                    transactions: Vec::new(),
                    stages: Vec::new(),
                    observations: Vec::new(),
                    preconditions: Vec::new(),
                    postconditions: Vec::new(),
                    expected_effects: Vec::new(),
                    execution_policy: None,
                    risk_class: None,
                    risk_tags: Vec::new(),
                    decoded_intent: None,
                    candidate_envelopes: Vec::new(),
                    decode_spec: None,
                    validation_plan: None,
                    evidence: json!({}),
                    metadata: BTreeMap::new(),
                },
                active_stage_id: None,
                planned_stage_graphs: BTreeMap::new(),
                exported_outputs: BTreeMap::from([(
                    "quote.amount_out_atomic".into(),
                    json!("10000000000000000"),
                )]),
                branch_trace: Vec::new(),
                awaiting_continuation: None,
            }),
        };
        let mission = Mission {
            mission_id: "mission-1".to_owned(),
            goal: "quote".to_owned(),
            allowed_chains: vec!["eip155:1".to_owned()],
            budget: MissionBudget {
                max_steps: Some(4),
                max_signer_requests: Some(0),
                max_wall_clock_ms: Some(5_000),
            },
            policy: MissionPolicy {
                policy_mode: Some("guarded".to_owned()),
                allow_raw_envelopes: false,
                require_effect_contract_for_writes: false,
            },
            constraints: BTreeMap::new(),
            metadata: BTreeMap::new(),
        };
        let mut runtime = ActiveRun::new(mission, checkpoint);

        let events = RuntimeEventEmitter::emit_after_step(
            &mut runtime,
            &StepTransition {
                kind: StepTransitionKind::Complete,
                node_id: None,
                summary: "observe-only artifact completed".to_owned(),
            },
        );

        assert!(matches!(events[0].event, RunEvent::Progress(_)));
        assert!(events
            .iter()
            .any(|event| matches!(event.event, RunEvent::Completed(_))));
        assert!(!events
            .iter()
            .any(|event| matches!(event.event, RunEvent::Paused(_))));
    }
}
