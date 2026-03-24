use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::execution_artifact::{ExecutionOutputKey, ExecutionPackageEntry, ExecutionStageId};
use crate::ids::{ChainSubmissionId, EventId, RunId, SignerRequestId};
use crate::{
    patch::{PatchOutcome, PlanPatchSubmission},
    recovery::{
        RecoveryActionKind, RecoveryDisposition, RecoverySuggestion, RunFailureCode,
        RunFailureContext,
    },
};

pub use crate::audit::{GovernorDecisionAuditKind, PlanPatchAuditStatus};

pub const RUN_EVENT_SCHEMA_V2: &str = "ais-agent/runtime_event/v2";
pub const RUN_EVENT_VERSION_V2: u16 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunEventFamily {
    Lifecycle,
    Pause,
    Transition,
    SideEffect,
    Signer,
    Recovery,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunEventDescriptor {
    pub schema: &'static str,
    pub family: RunEventFamily,
    pub event_type: &'static str,
    pub event_version: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunEventTraceContext {
    pub trace_id: String,
    pub span_id: String,
}

impl RunEventDescriptor {
    pub const fn new(family: RunEventFamily, event_type: &'static str) -> Self {
        Self {
            schema: RUN_EVENT_SCHEMA_V2,
            family,
            event_type,
            event_version: RUN_EVENT_VERSION_V2,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunEventEnvelope {
    pub run_id: RunId,
    pub event_seq: u64,
    pub checkpoint_seq: u64,
    pub plan_epoch: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_context: Option<RunEventTraceContext>,
    pub event: RunEvent,
}

impl RunEventEnvelope {
    pub fn descriptor(&self) -> RunEventDescriptor {
        self.event.descriptor()
    }
}

/// Host-visible runtime event stream.
///
/// These events intentionally describe stable lifecycle milestones instead of
/// leaking internal planner or executor details.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RunEvent {
    Started(RunStarted),
    Progress(RunProgress),
    RecoveryAudit(RunRecoveryAudit),
    GovernorDecision(RunGovernorDecision),
    PlanPatchAudit(RunPlanPatchAudit),
    Paused(RunPaused),
    AwaitingEvidence(RunAwaitingEvidence),
    AwaitingConfirm(RunAwaitingConfirm),
    AwaitingSigner(RunAwaitingSigner),
    AwaitingContinuation(RunAwaitingContinuation),
    BroadcastSubmitted(RunBroadcastSubmitted),
    VerifyPassed(RunVerifyPassed),
    VerifyFailed(RunVerifyFailed),
    Completed(RunCompleted),
    Failed(RunFailed),
}

impl RunEvent {
    pub fn descriptor(&self) -> RunEventDescriptor {
        match self {
            Self::Started(_) => {
                RunEventDescriptor::new(RunEventFamily::Lifecycle, "run.lifecycle.started")
            }
            Self::Progress(_) => {
                RunEventDescriptor::new(RunEventFamily::Transition, "run.transition.progress")
            }
            Self::RecoveryAudit(_) => {
                RunEventDescriptor::new(RunEventFamily::Recovery, "run.recovery.classified")
            }
            Self::GovernorDecision(_) => RunEventDescriptor::new(
                RunEventFamily::Transition,
                "run.transition.governor_decision",
            ),
            Self::PlanPatchAudit(audit) => match audit.status {
                PlanPatchAuditStatus::Submitted => RunEventDescriptor::new(
                    RunEventFamily::Recovery,
                    "run.recovery.patch_submitted",
                ),
                PlanPatchAuditStatus::Applied => {
                    RunEventDescriptor::new(RunEventFamily::Recovery, "run.recovery.patch_applied")
                }
                PlanPatchAuditStatus::Rejected => {
                    RunEventDescriptor::new(RunEventFamily::Recovery, "run.recovery.patch_rejected")
                }
            },
            Self::Paused(_) => RunEventDescriptor::new(RunEventFamily::Pause, "run.pause.paused"),
            Self::AwaitingEvidence(_) => {
                RunEventDescriptor::new(RunEventFamily::Pause, "run.pause.awaiting_evidence")
            }
            Self::AwaitingConfirm(_) => {
                RunEventDescriptor::new(RunEventFamily::Pause, "run.pause.awaiting_confirmation")
            }
            Self::AwaitingSigner(_) => {
                RunEventDescriptor::new(RunEventFamily::Signer, "run.signer.request_created")
            }
            Self::AwaitingContinuation(_) => {
                RunEventDescriptor::new(RunEventFamily::Pause, "run.pause.awaiting_continuation")
            }
            Self::BroadcastSubmitted(_) => RunEventDescriptor::new(
                RunEventFamily::SideEffect,
                "run.side_effect.broadcast_submitted",
            ),
            Self::VerifyPassed(_) => {
                RunEventDescriptor::new(RunEventFamily::SideEffect, "run.side_effect.verify_passed")
            }
            Self::VerifyFailed(_) => {
                RunEventDescriptor::new(RunEventFamily::SideEffect, "run.side_effect.verify_failed")
            }
            Self::Completed(_) => {
                RunEventDescriptor::new(RunEventFamily::Lifecycle, "run.lifecycle.completed")
            }
            Self::Failed(_) => {
                RunEventDescriptor::new(RunEventFamily::Lifecycle, "run.lifecycle.failed")
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunStarted {
    pub event_id: EventId,
    pub run_id: RunId,
    pub phase: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunProgress {
    pub event_id: EventId,
    pub run_id: RunId,
    pub phase: String,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunRecoveryAudit {
    pub event_id: EventId,
    pub run_id: RunId,
    pub recovery_disposition: Option<RecoveryDisposition>,
    pub failure_context: Option<RunFailureContext>,
    #[serde(default)]
    pub recovery_suggestions: Vec<RecoverySuggestion>,
    #[serde(default)]
    pub allowed_recovery_actions: Vec<RecoveryActionKind>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunGovernorDecision {
    pub event_id: EventId,
    pub run_id: RunId,
    pub node_id: Option<String>,
    pub decision: GovernorDecisionAuditKind,
    pub reason: String,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
    pub signer_request_id: Option<SignerRequestId>,
    pub rejection_code: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunPlanPatchAudit {
    pub event_id: EventId,
    pub run_id: RunId,
    pub patch_id: String,
    pub status: PlanPatchAuditStatus,
    pub patch: PlanPatchSubmission,
    pub outcome: Option<PatchOutcome>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunPaused {
    pub event_id: EventId,
    pub run_id: RunId,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunAwaitingEvidence {
    pub event_id: EventId,
    pub run_id: RunId,
    pub reason: String,
    pub missing_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunAwaitingConfirm {
    pub event_id: EventId,
    pub run_id: RunId,
    pub submission_id: Option<ChainSubmissionId>,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunAwaitingSigner {
    pub event_id: EventId,
    pub run_id: RunId,
    pub request_id: SignerRequestId,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunAwaitingContinuation {
    pub event_id: EventId,
    pub run_id: RunId,
    pub stage_id: Option<ExecutionStageId>,
    pub package_entry: Option<ExecutionPackageEntry>,
    #[serde(default)]
    pub required_outputs: Vec<ExecutionOutputKey>,
    #[serde(default)]
    pub resolved_outputs: std::collections::BTreeMap<ExecutionOutputKey, Value>,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunBroadcastSubmitted {
    pub event_id: EventId,
    pub run_id: RunId,
    pub node_id: String,
    pub chain: Option<String>,
    pub submission_id: Option<ChainSubmissionId>,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunVerifyPassed {
    pub event_id: EventId,
    pub run_id: RunId,
    pub node_id: String,
    pub submission_id: Option<ChainSubmissionId>,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunVerifyFailed {
    pub event_id: EventId,
    pub run_id: RunId,
    pub node_id: String,
    pub submission_id: Option<ChainSubmissionId>,
    pub code: Option<RunFailureCode>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunCompleted {
    pub event_id: EventId,
    pub run_id: RunId,
    pub summary: String,
    pub result: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunFailed {
    pub event_id: EventId,
    pub run_id: RunId,
    pub phase: String,
    pub code: RunFailureCode,
    pub message: String,
    pub failure_context: Option<RunFailureContext>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn awaiting_signer_descriptor_uses_signer_taxonomy() {
        let event = RunEvent::AwaitingSigner(RunAwaitingSigner {
            event_id: EventId("event-1".to_owned()),
            run_id: RunId("run-1".to_owned()),
            request_id: SignerRequestId("signer-1".to_owned()),
            reason: "signer required".to_owned(),
        });

        let descriptor = event.descriptor();
        assert_eq!(descriptor.schema, RUN_EVENT_SCHEMA_V2);
        assert_eq!(descriptor.family, RunEventFamily::Signer);
        assert_eq!(descriptor.event_type, "run.signer.request_created");
        assert_eq!(descriptor.event_version, RUN_EVENT_VERSION_V2);
    }

    #[test]
    fn plan_patch_descriptor_tracks_status_specific_taxonomy() {
        let event = RunEvent::PlanPatchAudit(RunPlanPatchAudit {
            event_id: EventId("event-1".to_owned()),
            run_id: RunId("run-1".to_owned()),
            patch_id: "patch-1".to_owned(),
            status: PlanPatchAuditStatus::Rejected,
            patch: PlanPatchSubmission {
                patch_id: "patch-1".to_owned(),
                run_id: RunId("run-1".to_owned()),
                basis_checkpoint_seq: 1,
                basis_plan_epoch: 0,
                reason_code: RunFailureCode::GovernorDenied,
                target: crate::patch::PlanPatchTarget::ActiveFrontier,
                operations: Vec::new(),
                expected_outcome: None,
            },
            outcome: None,
            message: Some("no".to_owned()),
        });

        let descriptor = event.descriptor();
        assert_eq!(descriptor.family, RunEventFamily::Recovery);
        assert_eq!(descriptor.event_type, "run.recovery.patch_rejected");
    }

    #[test]
    fn trace_context_round_trips_on_event_envelope() {
        let envelope = RunEventEnvelope {
            run_id: RunId("run-1".to_owned()),
            event_seq: 1,
            checkpoint_seq: 2,
            plan_epoch: 3,
            trace_context: Some(RunEventTraceContext {
                trace_id: "run:run-1:cmd:cmd-1:ckpt:2:epoch:3".to_owned(),
                span_id: "run.side_effect.broadcast_submitted:broadcast.swap:1".to_owned(),
            }),
            event: RunEvent::Started(RunStarted {
                event_id: EventId("event-1".to_owned()),
                run_id: RunId("run-1".to_owned()),
                phase: "running".to_owned(),
            }),
        };

        let json = serde_json::to_value(&envelope).expect("serialize");
        assert_eq!(
            json.get("trace_context")
                .and_then(|value| value.get("trace_id"))
                .and_then(|value| value.as_str()),
            Some("run:run-1:cmd:cmd-1:ckpt:2:epoch:3")
        );

        let decoded: RunEventEnvelope = serde_json::from_value(json).expect("deserialize");
        assert_eq!(
            decoded
                .trace_context
                .as_ref()
                .map(|context| context.span_id.as_str()),
            Some("run.side_effect.broadcast_submitted:broadcast.swap:1")
        );
    }
}
