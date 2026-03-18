use crate::runtime::{
    RunLifecycleState, RunPhase, RunStatus, SignerRequestState, SignerRequestStatus,
    SignerResolution, SignerResolutionKind,
};
use ais_agent_control::{
    ids::{RunId, SignerRequestId},
    recovery::{RunFailureCode, RunFailureStage},
};

#[test]
fn signer_request_tracks_submitted_and_reconciled_states() {
    let mut request = SignerRequestState::new_pending(
        SignerRequestId("signer-1".to_owned()),
        RunId("run-1".to_owned()),
        "eip155:1",
        "submit swap transaction",
    )
    .with_node_id("swap");

    request.apply_resolution(SignerResolution {
        request_id: SignerRequestId("signer-1".to_owned()),
        kind: SignerResolutionKind::Submitted,
        resolved_at_ms: Some(100),
        tx_hash: Some("0xabc".to_owned()),
        signed_payload: None,
    });

    assert_eq!(request.status, SignerRequestStatus::Submitted);
    assert_eq!(request.submitted_tx_hash.as_deref(), Some("0xabc"));
    assert!(request.reconcile_required);

    request.mark_reconciled(Some("0xdef".to_owned()));
    assert_eq!(request.status, SignerRequestStatus::Reconciled);
    assert_eq!(request.submitted_tx_hash.as_deref(), Some("0xdef"));
    assert!(!request.reconcile_required);
}

#[test]
fn signer_request_times_out_only_while_pending() {
    let mut request = SignerRequestState::new_pending(
        SignerRequestId("signer-1".to_owned()),
        RunId("run-1".to_owned()),
        "eip155:1",
        "submit swap transaction",
    )
    .with_timeout(100, Some(200));

    assert!(request.mark_timed_out(300));
    assert_eq!(request.status, SignerRequestStatus::TimedOut);
    assert!(!request.mark_timed_out(400));
}

#[test]
fn lifecycle_can_enter_and_resolve_awaiting_signer_from_request_state() {
    let mut lifecycle = RunLifecycleState::new(RunId("run-1".to_owned()), "mission-1");
    let request = SignerRequestState::new_pending(
        SignerRequestId("signer-1".to_owned()),
        RunId("run-1".to_owned()),
        "eip155:1",
        "submit swap transaction",
    );

    lifecycle.await_signer_request(&request);
    assert_eq!(lifecycle.status, RunStatus::AwaitingSigner);
    assert_eq!(
        lifecycle
            .active_boundary
            .as_ref()
            .and_then(|boundary| boundary.signer_request_id.as_ref())
            .map(|id| id.0.as_str()),
        Some("signer-1")
    );

    lifecycle.resolve_signer_wait(RunPhase::Broadcasting);
    assert_eq!(lifecycle.status, RunStatus::Running);
    assert_eq!(lifecycle.phase, RunPhase::Broadcasting);
    assert!(lifecycle.active_boundary.is_none());
}

#[test]
fn lifecycle_fail_records_typed_failure_context() {
    let mut lifecycle = RunLifecycleState::new(RunId("run-1".to_owned()), "mission-1");
    lifecycle.bump_checkpoint();
    lifecycle.bump_plan_epoch();

    lifecycle.fail(
        RunFailureStage::Verify,
        RunFailureCode::VerifyMismatch,
        "post-state balance mismatch",
    );

    let failure = lifecycle.failure.expect("typed failure");
    assert_eq!(failure.code, RunFailureCode::VerifyMismatch);
    assert_eq!(failure.stage, RunFailureStage::Verify);
    assert_eq!(failure.observed_at_checkpoint_seq, 1);
    assert_eq!(failure.observed_at_plan_epoch, 1);
    assert_eq!(failure.summary, "post-state balance mismatch");
}

#[test]
fn lifecycle_can_enter_and_resolve_awaiting_artifact_continuation() {
    let mut lifecycle = RunLifecycleState::new(RunId("run-1".to_owned()), "mission-1");

    lifecycle.await_artifact_continuation(
        "awaiting artifact continuation `build_next_stage`",
        vec!["swap.tx_hash".to_owned()],
    );

    assert_eq!(lifecycle.status, RunStatus::AwaitingArtifactContinuation);
    assert_eq!(lifecycle.phase, RunPhase::AwaitingHost);
    assert_eq!(
        lifecycle
            .active_boundary
            .as_ref()
            .map(|boundary| &boundary.kind),
        Some(&crate::runtime::BoundaryKind::ArtifactContinuation)
    );
    assert_eq!(
        lifecycle
            .active_boundary
            .as_ref()
            .map(|boundary| boundary.blocking_refs.clone()),
        Some(vec!["swap.tx_hash".to_owned()])
    );

    lifecycle.mark_running(RunPhase::Planning);
    assert_eq!(lifecycle.status, RunStatus::Running);
    assert!(lifecycle.active_boundary.is_none());
}
