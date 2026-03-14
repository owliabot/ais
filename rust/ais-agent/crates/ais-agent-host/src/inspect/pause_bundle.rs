use serde::{Deserialize, Serialize};

use ais_agent_control::{
    commands::RetryIntent,
    ids::RunId,
    ownership::RunOwnershipSnapshot,
    recovery::{
        CancelState, InterruptionClass, RecoveryActionKind, RecoveryDisposition,
        RecoverySuggestion, RunFailureContext, SideEffectPhase,
    },
};

use crate::inspect::{PendingConfirmationView, PendingContinuationView, PendingSignerRequestView};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PauseKind {
    NeedUserInput,
    NeedEvidence,
    NeedSigner,
    NeedConfirmation,
    NeedContinuation,
    RuntimeFailure,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PauseActionView {
    pub action_kind: RecoveryActionKind,
    pub action: String,
    pub description: String,
    pub requires_mutation_claim: bool,
    pub retry_intent: Option<RetryIntent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PauseBundle {
    pub schema: String,
    pub run_id: RunId,
    pub kind: PauseKind,
    pub interruption_class: Option<InterruptionClass>,
    pub cancel_state: Option<CancelState>,
    pub side_effect_phase: Option<SideEffectPhase>,
    pub recovery_disposition: RecoveryDisposition,
    pub summary: String,
    pub ownership: RunOwnershipSnapshot,
    #[serde(default)]
    pub blocking_refs: Vec<String>,
    #[serde(default)]
    pub required_actions: Vec<PauseActionView>,
    pub failure_context: Option<RunFailureContext>,
    #[serde(default)]
    pub recovery_suggestions: Vec<RecoverySuggestion>,
    #[serde(default)]
    pub allowed_recovery_actions: Vec<RecoveryActionKind>,
    #[serde(default)]
    pub pending_signer_requests: Vec<PendingSignerRequestView>,
    #[serde(default)]
    pub pending_confirmations: Vec<PendingConfirmationView>,
    #[serde(default)]
    pub pending_continuations: Vec<PendingContinuationView>,
    #[serde(default)]
    pub notes: Vec<String>,
}
