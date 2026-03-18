//! Signer-boundary transition.

use ais_agent_control::recovery::{RunFailureCode, RunFailureStage};
use ais_agent_core::{
    action::ActionNodeStatus,
    actuation::ActuationKind,
    runtime::{RunPhase, RunStatus, SignerRequestStatus},
};

use crate::{
    runtime::ActiveRun,
    stepper::{
        transitions::{add_actuation_record, mark_node_status},
        StepTransition, StepTransitionKind,
    },
};

pub(crate) fn apply_signer_transition(runtime: &mut ActiveRun) -> Option<StepTransition> {
    let signer_state = runtime.pending_signer_state.clone()?;
    let node_id = signer_state.node_id.clone();

    let transition = match signer_state.status {
        SignerRequestStatus::Pending => return None,
        SignerRequestStatus::Submitted | SignerRequestStatus::Reconciled => {
            if let Some(node_id) = node_id.as_deref() {
                mark_node_status(runtime, node_id, ActionNodeStatus::Succeeded);
                add_actuation_record(
                    runtime,
                    node_id,
                    ActuationKind::BroadcastSubmitted,
                    Some(signer_state.chain.clone()),
                    signer_state.submitted_tx_hash.clone(),
                    "signer submitted transaction",
                );
            }
            runtime.checkpoint.pending_requests.pending_confirmation_id =
                signer_state.submitted_tx_hash.clone();
            runtime.pending_signer_state = None;
            runtime
                .checkpoint
                .pending_requests
                .pending_signer_request_id = None;
            runtime.checkpoint.pending_requests.pending_signer_request = None;
            runtime.checkpoint.lifecycle.await_confirmation(
                signer_state
                    .submitted_tx_hash
                    .as_ref()
                    .map(|tx_hash| {
                        format!("waiting for chain receipt after signer submission {tx_hash}")
                    })
                    .unwrap_or_else(|| {
                        "waiting for chain receipt after signer submission".to_owned()
                    }),
            );
            StepTransition {
                kind: StepTransitionKind::Signer,
                node_id,
                summary: "signer submission accepted and moved to confirmation wait".to_owned(),
            }
        }
        SignerRequestStatus::Signed => {
            if runtime.checkpoint.lifecycle.status != RunStatus::AwaitingSigner {
                return None;
            }
            if let Some(node_id) = node_id.as_deref() {
                mark_node_status(runtime, node_id, ActionNodeStatus::Ready);
            }
            runtime
                .checkpoint
                .lifecycle
                .resolve_signer_wait(RunPhase::Broadcasting);
            StepTransition {
                kind: StepTransitionKind::Signer,
                node_id,
                summary: "signer provided a signed transaction for runtime broadcast".to_owned(),
            }
        }
        SignerRequestStatus::Denied
        | SignerRequestStatus::Expired
        | SignerRequestStatus::TimedOut => {
            if let Some(node_id) = node_id.as_deref() {
                mark_node_status(runtime, node_id, ActionNodeStatus::Failed);
            }
            let failure_code = match signer_state.status {
                SignerRequestStatus::Denied => RunFailureCode::SignerDenied,
                SignerRequestStatus::Expired | SignerRequestStatus::TimedOut => {
                    RunFailureCode::SignerExpired
                }
                _ => RunFailureCode::RuntimeInvariantViolation,
            };
            runtime.pending_signer_state = None;
            runtime
                .checkpoint
                .pending_requests
                .pending_signer_request_id = None;
            runtime.checkpoint.pending_requests.pending_signer_request = None;
            runtime.checkpoint.lifecycle.pause_with_failure(
                RunFailureStage::Signer,
                failure_code,
                format!(
                    "signer request {} did not complete",
                    signer_state.request_id.0
                ),
            );
            if let Some(failure) = runtime.checkpoint.lifecycle.failure.as_mut() {
                failure.signer_request_ref = Some(signer_state.request_id.clone());
                if let Some(node_id) = signer_state.node_id.clone() {
                    failure.node_refs.push(node_id);
                }
            }
            StepTransition {
                kind: StepTransitionKind::Signer,
                node_id,
                summary: "signer boundary failed".to_owned(),
            }
        }
    };

    runtime.touch_transition();
    Some(transition)
}
