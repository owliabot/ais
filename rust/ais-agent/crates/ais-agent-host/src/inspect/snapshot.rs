use serde::{Deserialize, Serialize};

use ais_agent_control::{
    events::{RunEventFamily, RunEventTraceContext},
    ids::{ChainSubmissionId, RunId},
    ownership::RunOwnershipSnapshot,
    recovery::{
        CancelState, InterruptionClass, RecoveryActionKind, RecoveryDisposition,
        RecoverySuggestion, RunFailureContext, SideEffectPhase,
    },
};

use crate::inspect::{
    PendingConfirmationView, PendingContinuationView, PendingSignerRequestView, ProgressView,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Created,
    Running,
    Paused,
    AwaitingEvidence,
    AwaitingSigner,
    AwaitingConfirm,
    AwaitingContinuation,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunPhase {
    MissionAccepted,
    Planning,
    Simulating,
    Governing,
    AwaitingHost,
    Broadcasting,
    Verifying,
    Recovering,
    Finalized,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoundaryKind {
    Pause,
    Evidence,
    Signer,
    Confirmation,
    ArtifactContinuation,
    Completion,
    Failure,
    Cancellation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveBoundaryView {
    pub kind: BoundaryKind,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissionSummaryView {
    pub goal: String,
    #[serde(default)]
    pub allowed_chains: Vec<String>,
    pub policy_mode: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequiredInputView {
    pub reference: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SideEffectView {
    pub kind: String,
    pub summary: String,
    pub submission_id: Option<ChainSubmissionId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecentEventView {
    pub event_seq: u64,
    pub checkpoint_seq: u64,
    pub plan_epoch: u64,
    pub family: RunEventFamily,
    pub event_type: String,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_context: Option<RunEventTraceContext>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectStatusView {
    Pending,
    Satisfied,
    Violated,
    Unknown,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RecoveryView {
    pub recovery_disposition: Option<RecoveryDisposition>,
    pub failure_context: Option<RunFailureContext>,
    #[serde(default)]
    pub recovery_suggestions: Vec<RecoverySuggestion>,
    #[serde(default)]
    pub allowed_recovery_actions: Vec<RecoveryActionKind>,
    pub interruption_class: Option<InterruptionClass>,
    pub cancel_state: Option<CancelState>,
    pub side_effect_phase: Option<SideEffectPhase>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchTraceView {
    pub branch_stage_id: String,
    #[serde(default)]
    pub available_targets: Vec<String>,
    pub selected_target: String,
    pub predicate_value: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunResultView {
    pub summary: String,
    pub terminal_failure_context: Option<RunFailureContext>,
    pub final_recovery_disposition: Option<RecoveryDisposition>,
    #[serde(default)]
    pub final_recovery_suggestions: Vec<RecoverySuggestion>,
    #[serde(default)]
    pub branch_trace: Vec<BranchTraceView>,
    pub ownership: RunOwnershipSnapshot,
    pub interruption_class: Option<InterruptionClass>,
    pub cancel_state: Option<CancelState>,
    pub side_effect_phase: Option<SideEffectPhase>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InspectSnapshot {
    pub schema: String,
    pub run_id: RunId,
    pub status: RunStatus,
    pub phase: RunPhase,
    pub checkpoint_seq: u64,
    pub plan_epoch: u64,
    pub active_boundary: Option<ActiveBoundaryView>,
    pub interruption_class: Option<InterruptionClass>,
    pub cancel_state: Option<CancelState>,
    pub side_effect_phase: Option<SideEffectPhase>,
    pub recovery_disposition: Option<RecoveryDisposition>,
    pub failure_context: Option<RunFailureContext>,
    #[serde(default)]
    pub recovery_suggestions: Vec<RecoverySuggestion>,
    #[serde(default)]
    pub allowed_recovery_actions: Vec<RecoveryActionKind>,
    pub mission_summary: MissionSummaryView,
    #[serde(default)]
    pub required_inputs: Vec<RequiredInputView>,
    #[serde(default)]
    pub pending_confirmations: Vec<PendingConfirmationView>,
    #[serde(default)]
    pub pending_continuations: Vec<PendingContinuationView>,
    #[serde(default)]
    pub pending_signer_requests: Vec<PendingSignerRequestView>,
    #[serde(default)]
    pub recent_side_effects: Vec<SideEffectView>,
    #[serde(default)]
    pub recent_events: Vec<RecentEventView>,
    pub effect_status: Option<EffectStatusView>,
    #[serde(default)]
    pub branch_trace: Vec<BranchTraceView>,
    pub ownership: RunOwnershipSnapshot,
    pub run_result: Option<RunResultView>,
    pub progress: ProgressView,
}
