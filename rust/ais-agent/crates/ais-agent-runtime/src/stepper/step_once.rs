//! Single local transition execution.

use serde::{Deserialize, Serialize};

use crate::{
    runtime::ActiveRun,
    stepper::transitions::{
        apply_broadcast_transition, apply_complete_transition, apply_derive_transition,
        apply_execution_artifact_transition, apply_govern_transition, apply_ingest_transition,
        apply_observe_transition, apply_recover_transition, apply_signer_transition,
        apply_simulate_transition, apply_verify_transition,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepTransitionKind {
    Ingest,
    Observe,
    Derive,
    Artifact,
    Simulate,
    Govern,
    Signer,
    Broadcast,
    Verify,
    Recover,
    Complete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepTransition {
    pub kind: StepTransitionKind,
    pub node_id: Option<String>,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepOnceResult {
    pub applied_transition: Option<StepTransition>,
    pub checkpoint_seq: u64,
    pub plan_epoch: u64,
    pub revision: u64,
}

#[derive(Debug, Default)]
pub struct StepOnce {
    _private: (),
}

impl StepOnce {
    pub async fn apply(runtime: &mut ActiveRun) -> StepOnceResult {
        let applied_transition = if let Some(transition) = apply_ingest_transition(runtime) {
            Some(transition)
        } else if let Some(transition) = apply_observe_transition(runtime).await {
            Some(transition)
        } else if let Some(transition) = apply_derive_transition(runtime) {
            Some(transition)
        } else if let Some(transition) = apply_execution_artifact_transition(runtime) {
            Some(transition)
        } else if let Some(transition) = apply_simulate_transition(runtime).await {
            Some(transition)
        } else if let Some(transition) = apply_govern_transition(runtime) {
            Some(transition)
        } else if let Some(transition) = apply_signer_transition(runtime) {
            Some(transition)
        } else if let Some(transition) = apply_broadcast_transition(runtime).await {
            Some(transition)
        } else if let Some(transition) = apply_verify_transition(runtime).await {
            Some(transition)
        } else if let Some(transition) = apply_recover_transition(runtime) {
            Some(transition)
        } else {
            apply_complete_transition(runtime)
        };

        StepOnceResult {
            applied_transition,
            checkpoint_seq: runtime.checkpoint_seq(),
            plan_epoch: runtime.plan_epoch(),
            revision: runtime.revision,
        }
    }
}
