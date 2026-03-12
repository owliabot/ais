use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ids::{EventId, RunId, SignerRequestId};
use crate::{
    patch::{PatchOutcome, PlanPatchSubmission},
    recovery::{
        RecoveryActionKind, RecoveryDisposition, RecoverySuggestion, RunFailureCode,
        RunFailureContext,
    },
};

pub use crate::audit::{GovernorDecisionAuditKind, PlanPatchAuditStatus};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunEventEnvelope {
    pub run_id: RunId,
    pub event_seq: u64,
    pub checkpoint_seq: u64,
    pub plan_epoch: u64,
    pub event: RunEvent,
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
    Completed(RunCompleted),
    Failed(RunFailed),
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
    pub confirmation_id: Option<String>,
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
