use serde::{Deserialize, Serialize};

use ais_agent_control::{
    ids::RunId,
    ownership::RunOwnershipSnapshot,
    recovery::{
        CancelState, InterruptionClass, RecoveryActionKind, RecoveryDisposition,
        RecoverySuggestion, RunFailureContext, SideEffectPhase,
    },
};

use crate::inspect::{PendingConfirmationView, PendingSignerRequestView, ProgressView};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Created,
    Running,
    Paused,
    AwaitingEvidence,
    AwaitingSigner,
    AwaitingConfirm,
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
    pub tx_hash: Option<String>,
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
pub struct RunResultView {
    pub summary: String,
    pub terminal_failure_context: Option<RunFailureContext>,
    pub final_recovery_disposition: Option<RecoveryDisposition>,
    #[serde(default)]
    pub final_recovery_suggestions: Vec<RecoverySuggestion>,
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
    pub pending_signer_requests: Vec<PendingSignerRequestView>,
    #[serde(default)]
    pub recent_side_effects: Vec<SideEffectView>,
    pub effect_status: Option<EffectStatusView>,
    pub ownership: RunOwnershipSnapshot,
    pub run_result: Option<RunResultView>,
    pub progress: ProgressView,
}
