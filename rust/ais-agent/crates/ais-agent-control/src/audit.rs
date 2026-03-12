use serde::{Deserialize, Serialize};

use crate::ids::{AuditId, RunId, SignerRequestId};
use crate::{
    patch::{PatchOutcome, PlanPatchSubmission},
    recovery::{
        InterruptionClass, RecoveryActionKind, RecoveryDisposition, RecoverySuggestion,
        RunFailureContext, RunFailureStage,
    },
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GovernorDecisionAuditKind {
    Allow,
    AllowWithSigner,
    RequireMoreEvidence,
    Reject,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanPatchAuditStatus {
    Submitted,
    Applied,
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeCancellationAuditState {
    Requested,
    Acknowledged,
    Rejected,
    Finalized,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeDurableCommitOutcome {
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryAuditRecord {
    pub recovery_disposition: Option<RecoveryDisposition>,
    pub failure_context: Option<RunFailureContext>,
    #[serde(default)]
    pub recovery_suggestions: Vec<RecoverySuggestion>,
    #[serde(default)]
    pub allowed_recovery_actions: Vec<RecoveryActionKind>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GovernorDecisionAuditRecord {
    pub node_id: Option<String>,
    pub decision: GovernorDecisionAuditKind,
    pub reason: String,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
    pub signer_request_id: Option<SignerRequestId>,
    pub rejection_code: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanPatchAuditRecord {
    pub patch_id: String,
    pub status: PlanPatchAuditStatus,
    pub patch: PlanPatchSubmission,
    pub outcome: Option<PatchOutcome>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CancellationAuditRecord {
    pub state: RuntimeCancellationAuditState,
    pub reason: Option<String>,
    pub side_effect_submitted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterruptionAuditRecord {
    pub class: InterruptionClass,
    pub stage: Option<RunFailureStage>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DurableCommitAuditRecord {
    pub mutation_kind: String,
    pub outcome: RuntimeDurableCommitOutcome,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RuntimeAudit {
    Recovery(RecoveryAuditRecord),
    GovernorDecision(GovernorDecisionAuditRecord),
    PlanPatch(PlanPatchAuditRecord),
    Cancellation(CancellationAuditRecord),
    Interruption(InterruptionAuditRecord),
    DurableCommit(DurableCommitAuditRecord),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeAuditRecord {
    pub audit_id: AuditId,
    pub run_id: RunId,
    pub audit_seq: u64,
    pub checkpoint_seq: u64,
    pub plan_epoch: u64,
    pub audit: RuntimeAudit,
}
