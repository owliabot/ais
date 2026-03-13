use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    ids::{ClaimId, CommandId, IdempotencyKey, RunId, SignerRequestId},
    ownership::{RunClaimMode, RunClaimOwnerKind},
    patch::PlanPatchSubmission,
};

/// Transport-neutral command surface for driving the runtime.
///
/// This enum is intentionally coarse-grained:
/// - hosts create runs
/// - inspect runs
/// - step runs until a stable boundary
/// - inject evidence or envelopes
/// - resolve signer waits
/// - cancel runs
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RunCommand {
    BeginRun(BeginRunCommand),
    InspectRun(InspectRunCommand),
    ClaimRun(ClaimRunCommand),
    RenewRunClaim(RenewRunClaimCommand),
    ReleaseRunClaim(ReleaseRunClaimCommand),
    StepRun(StepRunCommand),
    SubmitEvidence(SubmitEvidenceCommand),
    SubmitEnvelope(SubmitEnvelopeCommand),
    SubmitSignerDecision(SubmitSignerDecisionCommand),
    SubmitPlanPatch(SubmitPlanPatchCommand),
    RequestCancelRun(RequestCancelRunCommand),
    CancelRun(CancelRunCommand),
}

impl RunCommand {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::BeginRun(_) => "begin_run",
            Self::InspectRun(_) => "inspect_run",
            Self::ClaimRun(_) => "claim_run",
            Self::RenewRunClaim(_) => "renew_run_claim",
            Self::ReleaseRunClaim(_) => "release_run_claim",
            Self::StepRun(_) => "step_run",
            Self::SubmitEvidence(_) => "submit_evidence",
            Self::SubmitEnvelope(_) => "submit_envelope",
            Self::SubmitSignerDecision(_) => "submit_signer_decision",
            Self::SubmitPlanPatch(_) => "submit_plan_patch",
            Self::RequestCancelRun(_) => "request_cancel_run",
            Self::CancelRun(_) => "cancel_run",
        }
    }

    pub fn is_mutating(&self) -> bool {
        matches!(
            self,
            Self::StepRun(_)
                | Self::ClaimRun(_)
                | Self::RenewRunClaim(_)
                | Self::ReleaseRunClaim(_)
                | Self::SubmitEvidence(_)
                | Self::SubmitEnvelope(_)
                | Self::SubmitSignerDecision(_)
                | Self::SubmitPlanPatch(_)
                | Self::RequestCancelRun(_)
                | Self::CancelRun(_)
        )
    }

    pub fn expected_runtime_version(&self) -> Option<&ExpectedRuntimeVersion> {
        match self {
            Self::StepRun(command) => command.expected_version.as_ref(),
            Self::SubmitEvidence(command) => command.expected_version.as_ref(),
            Self::SubmitEnvelope(command) => command.expected_version.as_ref(),
            Self::SubmitSignerDecision(command) => command.expected_version.as_ref(),
            Self::SubmitPlanPatch(command) => command.expected_version.as_ref(),
            Self::RequestCancelRun(command) => command.expected_version.as_ref(),
            Self::CancelRun(command) => command.expected_version.as_ref(),
            Self::BeginRun(_)
            | Self::InspectRun(_)
            | Self::ClaimRun(_)
            | Self::RenewRunClaim(_)
            | Self::ReleaseRunClaim(_) => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BeginRunCommand {
    pub command_id: CommandId,
    pub idempotency_key: IdempotencyKey,
    pub mission: MissionSubmission,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InspectRunCommand {
    pub command_id: CommandId,
    pub run_id: RunId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimRunCommand {
    pub command_id: CommandId,
    pub run_id: RunId,
    pub owner_kind: RunClaimOwnerKind,
    pub owner_instance_id: String,
    pub mode: RunClaimMode,
    #[serde(default)]
    pub requested_lease_ms: Option<u64>,
    #[serde(default)]
    pub allow_supersede: bool,
    #[serde(default)]
    pub expected_current_claim_id: Option<ClaimId>,
    #[serde(default)]
    pub expected_current_claim_epoch: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenewRunClaimCommand {
    pub command_id: CommandId,
    pub run_id: RunId,
    pub claim_id: ClaimId,
    pub claim_epoch: u64,
    #[serde(default)]
    pub requested_lease_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseRunClaimCommand {
    pub command_id: CommandId,
    pub run_id: RunId,
    pub claim_id: ClaimId,
    pub claim_epoch: u64,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExpectedRuntimeVersion {
    #[serde(default)]
    pub checkpoint_seq: Option<u64>,
    #[serde(default)]
    pub plan_epoch: Option<u64>,
}

impl ExpectedRuntimeVersion {
    pub fn is_empty(&self) -> bool {
        self.checkpoint_seq.is_none() && self.plan_epoch.is_none()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepRunCommand {
    pub command_id: CommandId,
    pub run_id: RunId,
    pub until: StepUntil,
    pub budget: Option<StepBudget>,
    #[serde(default)]
    pub expected_version: Option<ExpectedRuntimeVersion>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitEvidenceCommand {
    pub command_id: CommandId,
    pub run_id: RunId,
    pub evidence: EvidenceSubmission,
    #[serde(default)]
    pub expected_version: Option<ExpectedRuntimeVersion>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitEnvelopeCommand {
    pub command_id: CommandId,
    pub run_id: RunId,
    pub envelope: EnvelopeSubmission,
    #[serde(default)]
    pub expected_version: Option<ExpectedRuntimeVersion>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitSignerDecisionCommand {
    pub command_id: CommandId,
    pub run_id: RunId,
    pub decision: SignerDecisionSubmission,
    #[serde(default)]
    pub expected_version: Option<ExpectedRuntimeVersion>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitPlanPatchCommand {
    pub command_id: CommandId,
    pub run_id: RunId,
    pub patch: PlanPatchSubmission,
    #[serde(default)]
    pub expected_version: Option<ExpectedRuntimeVersion>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CancelRunCommand {
    pub command_id: CommandId,
    pub run_id: RunId,
    pub reason: Option<String>,
    #[serde(default)]
    pub expected_version: Option<ExpectedRuntimeVersion>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestCancelRunCommand {
    pub command_id: CommandId,
    pub run_id: RunId,
    pub reason: Option<String>,
    #[serde(default)]
    pub expected_version: Option<ExpectedRuntimeVersion>,
}

/// Host-supplied mission envelope.
///
/// This is intentionally small at the command boundary and will later be
/// normalized into richer core-domain mission objects.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissionSubmission {
    pub goal: String,
    #[serde(default)]
    pub allowed_chains: Vec<String>,
    #[serde(default)]
    pub constraints: BTreeMap<String, Value>,
    pub budget: Option<MissionBudgetSubmission>,
    #[serde(default)]
    pub metadata: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissionBudgetSubmission {
    pub max_steps: Option<u32>,
    pub max_signer_requests: Option<u32>,
    pub max_wall_clock_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepUntil {
    NextBoundary,
    CompleteOrBoundary,
    BudgetExhausted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetryIntent {
    ResumeExecution,
    PollConfirmation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepBudget {
    pub max_nodes: Option<u32>,
    pub max_wall_clock_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    Fact,
    QueryResult,
    RouteOrQuote,
    Metadata,
    ExternalObservation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceSubmission {
    pub evidence_id: String,
    pub kind: EvidenceKind,
    pub source: String,
    pub observed_at_ms: Option<u64>,
    pub chain_scope: Option<String>,
    pub payload: Value,
    pub confidence: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvelopeKind {
    EvmEnvelope,
    SolanaEnvelope,
    ExternalJob,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvelopeSubmission {
    pub envelope_id: String,
    pub kind: EnvelopeKind,
    pub chain: String,
    pub payload: Value,
    pub expected_effect: Option<Value>,
    pub provenance: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignerDecisionKind {
    Approved,
    Denied,
    Submitted,
    Expired,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignerDecisionSubmission {
    pub request_id: SignerRequestId,
    pub decision: SignerDecisionKind,
    pub tx_hash: Option<String>,
    #[serde(default)]
    pub details: BTreeMap<String, Value>,
}

#[cfg(test)]
mod tests {
    use super::{ClaimRunCommand, ReleaseRunClaimCommand, RenewRunClaimCommand, RunCommand};
    use crate::{
        ids::{ClaimId, CommandId, RunId},
        ownership::{RunClaimMode, RunClaimOwnerKind},
    };

    #[test]
    fn ownership_commands_serialize_with_stable_snake_case_tags() {
        let claim = RunCommand::ClaimRun(ClaimRunCommand {
            command_id: CommandId("cmd-claim".to_owned()),
            run_id: RunId("run-1".to_owned()),
            owner_kind: RunClaimOwnerKind::InteractiveHost,
            owner_instance_id: "host-a".to_owned(),
            mode: RunClaimMode::ExclusiveMutation,
            requested_lease_ms: Some(30_000),
            allow_supersede: false,
            expected_current_claim_id: None,
            expected_current_claim_epoch: None,
        });
        let renew = RunCommand::RenewRunClaim(RenewRunClaimCommand {
            command_id: CommandId("cmd-renew".to_owned()),
            run_id: RunId("run-1".to_owned()),
            claim_id: ClaimId("claim-1".to_owned()),
            claim_epoch: 2,
            requested_lease_ms: Some(15_000),
        });
        let release = RunCommand::ReleaseRunClaim(ReleaseRunClaimCommand {
            command_id: CommandId("cmd-release".to_owned()),
            run_id: RunId("run-1".to_owned()),
            claim_id: ClaimId("claim-1".to_owned()),
            claim_epoch: 2,
            reason: Some("handoff".to_owned()),
        });

        assert_eq!(
            serde_json::to_value(&claim).unwrap()["type"],
            serde_json::Value::String("claim_run".to_owned())
        );
        assert_eq!(
            serde_json::to_value(&renew).unwrap()["type"],
            serde_json::Value::String("renew_run_claim".to_owned())
        );
        assert_eq!(
            serde_json::to_value(&release).unwrap()["type"],
            serde_json::Value::String("release_run_claim".to_owned())
        );
    }

    #[test]
    fn ownership_commands_are_mutating_but_do_not_use_runtime_version_preconditions() {
        let claim = RunCommand::ClaimRun(ClaimRunCommand {
            command_id: CommandId("cmd-claim".to_owned()),
            run_id: RunId("run-1".to_owned()),
            owner_kind: RunClaimOwnerKind::InteractiveHost,
            owner_instance_id: "host-a".to_owned(),
            mode: RunClaimMode::ExclusiveMutation,
            requested_lease_ms: None,
            allow_supersede: true,
            expected_current_claim_id: Some(ClaimId("claim-0".to_owned())),
            expected_current_claim_epoch: Some(1),
        });

        assert!(claim.is_mutating());
        assert_eq!(claim.kind(), "claim_run");
        assert!(claim.expected_runtime_version().is_none());
    }
}
