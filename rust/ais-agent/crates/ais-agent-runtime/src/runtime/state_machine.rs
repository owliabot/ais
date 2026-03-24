//! State-machine helpers for runtime transitions and checkpoint normalization.

use ais_agent_core::{
    action::{kinds::verify::VerifyKind, ActionNodeKind, ActionNodeStatus, ActionPayload},
    checkpoint::{CheckpointSnapshot, PendingSignerRequestSnapshot, PendingSignerTimeoutSnapshot},
    runtime::{RunStatus, SignerRequestState, SignerRequestStatus},
};

use crate::runtime::ActiveRun;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckpointPersistenceMode {
    Boundary,
    Progress,
    SideEffect,
}

#[derive(Debug, Default)]
pub struct RuntimeStateMachine;

impl RuntimeStateMachine {
    pub fn checkpoint_for_persistence(
        runtime: &ActiveRun,
        mode: CheckpointPersistenceMode,
    ) -> CheckpointSnapshot {
        let mut snapshot = runtime.checkpoint.clone();

        snapshot.run_id = runtime.run_id.0.clone();
        snapshot.mission_id = runtime.mission.mission_id.clone();
        snapshot.checkpoint_seq = runtime.checkpoint_seq();
        snapshot.plan_epoch = runtime.plan_epoch();
        snapshot.lifecycle.checkpoint_seq = runtime.checkpoint.lifecycle.checkpoint_seq;
        snapshot.lifecycle.plan_epoch = runtime.checkpoint.lifecycle.plan_epoch;

        match mode {
            CheckpointPersistenceMode::Boundary => {
                snapshot.pending_requests.pending_signer_request_id = runtime
                    .pending_signer_state
                    .as_ref()
                    .map(|state| state.request_id.0.clone());
                snapshot.pending_requests.pending_signer_request = runtime
                    .pending_signer_state
                    .as_ref()
                    .map(checkpoint_pending_signer_request);
            }
            CheckpointPersistenceMode::Progress => {
                if !matches!(
                    runtime.checkpoint.lifecycle.status,
                    RunStatus::AwaitingEvidence
                        | RunStatus::AwaitingSigner
                        | RunStatus::AwaitingConfirmation
                        | RunStatus::AwaitingArtifactContinuation
                        | RunStatus::Paused
                ) {
                    snapshot.pending_requests.pending_evidence_refs.clear();
                    snapshot.pending_requests.pending_envelope_refs.clear();
                    snapshot.pending_requests.pending_signer_request_id = None;
                    snapshot.pending_requests.pending_signer_request = None;
                    snapshot.pending_requests.pending_submission_id = None;
                } else {
                    snapshot.pending_requests.pending_signer_request_id = runtime
                        .pending_signer_state
                        .as_ref()
                        .map(|state| state.request_id.0.clone());
                    snapshot.pending_requests.pending_signer_request = runtime
                        .pending_signer_state
                        .as_ref()
                        .map(checkpoint_pending_signer_request);
                }
            }
            CheckpointPersistenceMode::SideEffect => {
                snapshot.pending_requests.pending_signer_request_id = runtime
                    .pending_signer_state
                    .as_ref()
                    .map(|state| state.request_id.0.clone());
                snapshot.pending_requests.pending_signer_request = runtime
                    .pending_signer_state
                    .as_ref()
                    .map(checkpoint_pending_signer_request);
            }
        }

        snapshot
    }

    pub fn restored_pending_signer_state(
        checkpoint: &CheckpointSnapshot,
        pending_signer_state: Option<SignerRequestState>,
    ) -> Option<SignerRequestState> {
        match (
            checkpoint
                .pending_requests
                .pending_signer_request_id
                .as_deref(),
            pending_signer_state,
        ) {
            (Some(expected_request_id), Some(state))
                if state.request_id.0 == expected_request_id =>
            {
                Some(state)
            }
            (None, Some(state)) if state.status == SignerRequestStatus::Signed => Some(state),
            (Some(_), _) => None,
            (None, _) => None,
        }
    }

    pub fn requires_confirmation_resume(checkpoint: &CheckpointSnapshot) -> bool {
        checkpoint.lifecycle.status == RunStatus::AwaitingConfirmation
    }

    pub fn missing_effect_contract_for_confirmation_resume(
        checkpoint: &CheckpointSnapshot,
    ) -> Option<(String, String)> {
        checkpoint
            .action_graph
            .nodes
            .values()
            .find(|node| {
                node.kind == ActionNodeKind::Verify
                    && matches!(
                        node.status,
                        ActionNodeStatus::Pending | ActionNodeStatus::Ready
                    )
                    && matches!(
                        &node.payload,
                        ActionPayload::Verify(verify)
                            if verify.verify_kind == VerifyKind::EffectContract
                    )
                    && node.expected_effect_ref.is_some()
            })
            .and_then(|node| {
                let effect_id = node.expected_effect_ref.as_ref()?;
                if checkpoint.effect_contracts.contains_key(effect_id) {
                    None
                } else {
                    Some((node.node_id.clone(), effect_id.clone()))
                }
            })
    }
}

fn checkpoint_pending_signer_request(state: &SignerRequestState) -> PendingSignerRequestSnapshot {
    PendingSignerRequestSnapshot {
        request_id: state.request_id.0.clone(),
        node_id: state.node_id.clone(),
        chain: Some(state.chain.clone()),
        summary: state.summary.clone(),
        payload: state.payload.clone(),
        timeout_policy: state
            .timeout
            .as_ref()
            .map(|timeout| PendingSignerTimeoutSnapshot {
                requested_at_ms: timeout.requested_at_ms,
                expires_at_ms: timeout.expires_at_ms,
            }),
    }
}
