use serde::{Deserialize, Serialize};

use crate::ids::{AuditId, ClaimId, RunId, SignerRequestId};
use crate::{
    ownership::{ClaimTransitionKind, RunClaimOwnerKind},
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
pub struct ClaimTransitionAuditRecord {
    pub claim_id: ClaimId,
    pub previous_claim_id: Option<ClaimId>,
    pub host_session_id: String,
    pub transition_kind: ClaimTransitionKind,
    pub transition_reason: String,
    pub actor_or_initiator_kind: RunClaimOwnerKind,
    pub effective_timestamp_ms: u64,
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
    ClaimTransition(ClaimTransitionAuditRecord),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claim_transition_audit_round_trips_as_snake_case_variant() {
        let record = RuntimeAuditRecord {
            audit_id: AuditId("audit-1".to_owned()),
            run_id: RunId("run-1".to_owned()),
            audit_seq: 1,
            checkpoint_seq: 0,
            plan_epoch: 0,
            audit: RuntimeAudit::ClaimTransition(ClaimTransitionAuditRecord {
                claim_id: ClaimId("claim-1".to_owned()),
                previous_claim_id: Some(ClaimId("claim-0".to_owned())),
                host_session_id: "session-1".to_owned(),
                transition_kind: ClaimTransitionKind::ClaimSuperseded,
                transition_reason: "handoff".to_owned(),
                actor_or_initiator_kind: RunClaimOwnerKind::InteractiveHost,
                effective_timestamp_ms: 123,
            }),
        };

        let json = serde_json::to_value(&record).expect("serialize audit");
        assert_eq!(json["audit"]["type"], "claim_transition");
        assert_eq!(json["audit"]["transition_kind"], "claim_superseded");
        let decoded: RuntimeAuditRecord = serde_json::from_value(json).expect("decode audit");
        match decoded.audit {
            RuntimeAudit::ClaimTransition(payload) => {
                assert_eq!(payload.claim_id.0, "claim-1");
                assert_eq!(
                    payload
                        .previous_claim_id
                        .as_ref()
                        .map(|claim_id| claim_id.0.as_str()),
                    Some("claim-0")
                );
            }
            other => panic!("unexpected audit payload: {other:?}"),
        }
    }
}
