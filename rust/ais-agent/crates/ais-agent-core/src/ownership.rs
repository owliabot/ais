use ais_agent_control::{
    ownership::{OwnershipVisibility, RunClaimMode},
    recovery::CancelState,
};

use crate::{
    checkpoint::CheckpointSnapshot,
    recovery::{classify_cancel_state, classify_side_effect_phase},
    runtime::RunStatus,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimPolicy {
    pub claim_required_for_mutation: bool,
    pub required_mutation_mode: Option<RunClaimMode>,
    pub owner_visibility: OwnershipVisibility,
    pub observer_only_claim_allowed: bool,
    pub allow_release: bool,
    pub allow_expire_reacquire: bool,
    pub allow_supersede_active_claim: bool,
    pub strict_post_side_effect_handoff: bool,
}

pub fn classify_claim_policy(checkpoint: &CheckpointSnapshot) -> ClaimPolicy {
    let status = &checkpoint.lifecycle.status;
    let terminal = matches!(
        status,
        RunStatus::Completed | RunStatus::Failed | RunStatus::Cancelled
    );
    let awaiting_host = matches!(
        checkpoint.lifecycle.phase,
        crate::runtime::RunPhase::AwaitingHost
    );
    let cancel_state = classify_cancel_state(checkpoint);
    let side_effect_phase = classify_side_effect_phase(checkpoint);
    let strict_post_side_effect_handoff = matches!(cancel_state, Some(CancelState::Pending))
        || matches!(
            side_effect_phase,
            Some(
                ais_agent_control::recovery::SideEffectPhase::BroadcastSubmitted
                    | ais_agent_control::recovery::SideEffectPhase::AwaitingConfirmation
                    | ais_agent_control::recovery::SideEffectPhase::ReceiptObserved
                    | ais_agent_control::recovery::SideEffectPhase::Verified
            )
        );

    ClaimPolicy {
        claim_required_for_mutation: !terminal,
        required_mutation_mode: (!terminal).then_some(RunClaimMode::ExclusiveMutation),
        owner_visibility: OwnershipVisibility::ObserverReadAllowed,
        observer_only_claim_allowed: true,
        allow_release: terminal || (awaiting_host && !strict_post_side_effect_handoff),
        allow_expire_reacquire: !terminal,
        allow_supersede_active_claim: awaiting_host && !strict_post_side_effect_handoff,
        strict_post_side_effect_handoff,
    }
}

#[cfg(test)]
mod tests {
    use ais_agent_control::{
        ids::RunId,
        ownership::{OwnershipVisibility, RunClaimMode},
        recovery::{CancelState, InterruptionClass, InterruptionState, SideEffectPhase},
    };

    use crate::{
        action::ActionGraph,
        checkpoint::{CheckpointSnapshot, PendingRequestsSnapshot},
        evidence::EvidenceGraph,
        runtime::{RunLifecycleState, RunPhase, RunStatus},
    };

    use super::{classify_claim_policy, ClaimPolicy};

    fn checkpoint_with_lifecycle(lifecycle: RunLifecycleState) -> CheckpointSnapshot {
        CheckpointSnapshot {
            run_id: lifecycle.run_id.0.clone(),
            mission_id: lifecycle.mission_id.clone(),
            checkpoint_seq: lifecycle.checkpoint_seq,
            plan_epoch: lifecycle.plan_epoch,
            lifecycle,
            action_graph: ActionGraph::default(),
            evidence_graph: EvidenceGraph::default(),
            effect_contracts: Default::default(),
            pending_requests: PendingRequestsSnapshot::default(),
            last_completed_node_id: None,
            actuation_records: Vec::new(),
        }
    }

    fn running_checkpoint() -> CheckpointSnapshot {
        let mut lifecycle = RunLifecycleState::new(RunId("run-1".to_owned()), "mission-1");
        lifecycle.mark_running(RunPhase::Simulating);
        checkpoint_with_lifecycle(lifecycle)
    }

    fn awaiting_confirmation_checkpoint() -> CheckpointSnapshot {
        let mut lifecycle = RunLifecycleState::new(RunId("run-1".to_owned()), "mission-1");
        lifecycle.await_confirmation("waiting");
        let mut checkpoint = checkpoint_with_lifecycle(lifecycle);
        checkpoint.pending_requests.pending_confirmation_id = Some("tx-1".to_owned());
        checkpoint
    }

    fn cancel_pending_checkpoint() -> CheckpointSnapshot {
        let mut lifecycle = RunLifecycleState::new(RunId("run-1".to_owned()), "mission-1");
        lifecycle.status = RunStatus::AwaitingConfirmation;
        lifecycle.phase = RunPhase::AwaitingHost;
        lifecycle.cancel_state = Some(CancelState::Pending);
        lifecycle.interruption = Some(InterruptionState {
            class: InterruptionClass::HostCancelRequested,
            stage: None,
            side_effect_phase: Some(SideEffectPhase::AwaitingConfirmation),
            summary: "cancel pending".to_owned(),
        });
        checkpoint_with_lifecycle(lifecycle)
    }

    fn completed_checkpoint() -> CheckpointSnapshot {
        let mut lifecycle = RunLifecycleState::new(RunId("run-1".to_owned()), "mission-1");
        lifecycle.complete("done");
        checkpoint_with_lifecycle(lifecycle)
    }

    #[test]
    fn running_checkpoint_requires_exclusive_mutation_claim() {
        let policy = classify_claim_policy(&running_checkpoint());

        assert_eq!(
            policy,
            ClaimPolicy {
                claim_required_for_mutation: true,
                required_mutation_mode: Some(RunClaimMode::ExclusiveMutation),
                owner_visibility: OwnershipVisibility::ObserverReadAllowed,
                observer_only_claim_allowed: true,
                allow_release: false,
                allow_expire_reacquire: true,
                allow_supersede_active_claim: false,
                strict_post_side_effect_handoff: false,
            }
        );
    }

    #[test]
    fn paused_pre_side_effect_checkpoint_allows_release_and_supersede() {
        let mut lifecycle = RunLifecycleState::new(RunId("run-1".to_owned()), "mission-1");
        lifecycle.pause("await host");
        let policy = classify_claim_policy(&checkpoint_with_lifecycle(lifecycle));

        assert!(policy.allow_release);
        assert!(policy.allow_supersede_active_claim);
        assert!(!policy.strict_post_side_effect_handoff);
    }

    #[test]
    fn awaiting_confirmation_checkpoint_is_strict_handoff() {
        let policy = classify_claim_policy(&awaiting_confirmation_checkpoint());

        assert!(policy.strict_post_side_effect_handoff);
        assert_eq!(
            policy.required_mutation_mode,
            Some(RunClaimMode::ExclusiveMutation)
        );
    }

    #[test]
    fn cancel_pending_checkpoint_is_strict_handoff() {
        let policy = classify_claim_policy(&cancel_pending_checkpoint());

        assert!(policy.strict_post_side_effect_handoff);
        assert!(policy.claim_required_for_mutation);
    }

    #[test]
    fn completed_checkpoint_no_longer_requires_mutation_claim() {
        let policy = classify_claim_policy(&completed_checkpoint());

        assert!(!policy.claim_required_for_mutation);
        assert_eq!(policy.required_mutation_mode, None);
        assert!(policy.allow_release);
        assert!(!policy.allow_expire_reacquire);
        assert!(!policy.strict_post_side_effect_handoff);
    }
}
