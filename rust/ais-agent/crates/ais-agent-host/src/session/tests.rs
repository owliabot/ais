use ais_agent_control::{
    ids::{ClaimId, CommandId, IdempotencyKey, RunId},
    ownership::{OwnershipVisibility, RunOwnershipSnapshot},
};

use crate::{
    control::{HostCommandOutcome, HostCommandResponse},
    inspect::{
        ActionStatusCountsView, InspectSnapshot, MissionSummaryView, ProgressView, RunPhase,
        RunStatus,
    },
    session::{
        HostRunLink, HostSessionId, HostSessionSnapshot, HostSessionStore, IdempotencyResolution,
        InMemoryHostSessionStore,
    },
};

#[test]
fn host_session_identity_stays_separate_from_run_identity() {
    let mut store = InMemoryHostSessionStore::default();
    let session_id: HostSessionId = "session.telegram.user-1".into();
    let run_id: RunId = "run-1".into();

    store.link_run(HostRunLink::new(
        session_id.clone(),
        run_id.clone(),
        "swap usdc to weth".to_owned(),
        vec!["eip155:1".to_owned()],
    ));

    let snapshot = store
        .session_snapshot(&session_id)
        .expect("session snapshot");
    assert_eq!(snapshot.host_session_id, session_id);
    assert_eq!(snapshot.active_run_id, Some(run_id.clone()));
    assert_eq!(snapshot.linked_runs[0].run_id, run_id);
}

#[test]
fn idempotency_is_scoped_to_host_session() {
    let mut store = InMemoryHostSessionStore::default();

    let first = store.register_idempotency(
        "session-a".into(),
        IdempotencyKey("key-1".to_owned()),
        CommandId("cmd-1".to_owned()),
        Some(RunId("run-1".to_owned())),
        None,
    );
    assert!(matches!(first, IdempotencyResolution::Accepted));

    let replay = store.register_idempotency(
        "session-a".into(),
        IdempotencyKey("key-1".to_owned()),
        CommandId("cmd-1".to_owned()),
        Some(RunId("run-1".to_owned())),
        None,
    );
    match replay {
        IdempotencyResolution::Replay {
            existing_command_id,
            run_id,
            outcome,
        } => {
            assert_eq!(existing_command_id, CommandId("cmd-1".to_owned()));
            assert_eq!(run_id, Some(RunId("run-1".to_owned())));
            assert!(outcome.is_none());
        }
        other => panic!("unexpected resolution: {other:?}"),
    }

    let different_session = store.register_idempotency(
        "session-b".into(),
        IdempotencyKey("key-1".to_owned()),
        CommandId("cmd-9".to_owned()),
        Some(RunId("run-9".to_owned())),
        None,
    );
    assert!(matches!(different_session, IdempotencyResolution::Accepted));
}

#[test]
fn completed_idempotency_replays_cached_outcome() {
    let mut store = InMemoryHostSessionStore::default();
    let session_id: HostSessionId = "session-a".into();
    let key = IdempotencyKey("key-1".to_owned());
    let run_id = RunId("run-1".to_owned());

    let accepted = store.register_idempotency(
        session_id.clone(),
        key.clone(),
        CommandId("cmd-1".to_owned()),
        Some(run_id.clone()),
        None,
    );
    assert!(matches!(accepted, IdempotencyResolution::Accepted));

    let outcome = HostCommandOutcome {
        response: HostCommandResponse::Session(HostSessionSnapshot {
            host_session_id: session_id.clone(),
            active_run_id: Some(run_id.clone()),
            linked_runs: Vec::new(),
        }),
        events: Vec::new(),
    };
    store.complete_idempotency(
        &session_id,
        &key,
        outcome.clone(),
        Some(run_id.clone()),
        None,
    );

    let replay = store.register_idempotency(
        session_id,
        key,
        CommandId("cmd-1".to_owned()),
        Some(run_id.clone()),
        None,
    );
    match replay {
        IdempotencyResolution::Replay {
            existing_command_id,
            run_id: replay_run_id,
            outcome: Some(replayed_outcome),
        } => {
            assert_eq!(existing_command_id, CommandId("cmd-1".to_owned()));
            assert_eq!(replay_run_id, Some(run_id));
            match replayed_outcome.response {
                HostCommandResponse::Session(snapshot) => {
                    assert_eq!(snapshot.host_session_id.0, "session-a");
                    assert_eq!(snapshot.active_run_id, Some(RunId("run-1".to_owned())));
                }
                other => panic!("unexpected replayed outcome: {other:?}"),
            }
        }
        other => panic!("unexpected resolution: {other:?}"),
    }
}

#[test]
fn idempotency_conflicts_when_claim_lineage_changes() {
    let mut store = InMemoryHostSessionStore::default();
    let session_id: HostSessionId = "session-a".into();
    let key = IdempotencyKey("key-claim".to_owned());
    let run_id = RunId("run-1".to_owned());

    let accepted = store.register_idempotency(
        session_id.clone(),
        key.clone(),
        CommandId("cmd-step".to_owned()),
        Some(run_id.clone()),
        Some(ClaimId("claim-1".to_owned())),
    );
    assert!(matches!(accepted, IdempotencyResolution::Accepted));

    let conflict = store.register_idempotency(
        session_id,
        key,
        CommandId("cmd-step".to_owned()),
        Some(run_id),
        Some(ClaimId("claim-2".to_owned())),
    );
    assert!(matches!(conflict, IdempotencyResolution::Conflict { .. }));
}

#[test]
fn completed_idempotency_can_update_claim_lineage_for_replay() {
    let mut store = InMemoryHostSessionStore::default();
    let session_id: HostSessionId = "session-a".into();
    let key = IdempotencyKey("key-claim".to_owned());
    let run_id = RunId("run-1".to_owned());

    let accepted = store.register_idempotency(
        session_id.clone(),
        key.clone(),
        CommandId("cmd-step".to_owned()),
        Some(run_id.clone()),
        None,
    );
    assert!(matches!(accepted, IdempotencyResolution::Accepted));

    store.complete_idempotency(
        &session_id,
        &key,
        HostCommandOutcome {
            response: HostCommandResponse::Session(HostSessionSnapshot {
                host_session_id: session_id.clone(),
                active_run_id: Some(run_id.clone()),
                linked_runs: Vec::new(),
            }),
            events: Vec::new(),
        },
        Some(run_id.clone()),
        Some(ClaimId("claim-1".to_owned())),
    );

    let replay = store.register_idempotency(
        session_id,
        key,
        CommandId("cmd-step".to_owned()),
        Some(run_id),
        Some(ClaimId("claim-1".to_owned())),
    );
    assert!(matches!(replay, IdempotencyResolution::Replay { .. }));
}

#[test]
fn inspect_projection_updates_run_link_cursor() {
    let mut store = InMemoryHostSessionStore::default();
    let session_id: HostSessionId = "session.discord.room-1".into();
    let run_id: RunId = "run-1".into();

    store.link_run(HostRunLink::new(
        session_id.clone(),
        run_id.clone(),
        "observe and act".to_owned(),
        vec!["solana:mainnet".to_owned()],
    ));

    let inspect = InspectSnapshot {
        schema: "ais-agent/inspect_snapshot/v2".to_owned(),
        run_id: run_id.clone(),
        status: RunStatus::AwaitingEvidence,
        phase: RunPhase::AwaitingHost,
        checkpoint_seq: 3,
        plan_epoch: 2,
        active_boundary: None,
        interruption_class: None,
        cancel_state: None,
        side_effect_phase: None,
        recovery_disposition: None,
        failure_context: None,
        recovery_suggestions: Vec::new(),
        allowed_recovery_actions: Vec::new(),
        mission_summary: MissionSummaryView {
            goal: "observe and act".to_owned(),
            allowed_chains: vec!["solana:mainnet".to_owned()],
            policy_mode: None,
        },
        required_inputs: Vec::new(),
        pending_confirmations: Vec::new(),
        pending_continuations: Vec::new(),
        pending_signer_requests: Vec::new(),
        recent_events: Vec::new(),
        recent_side_effects: Vec::new(),
        effect_status: None,
        branch_trace: Vec::new(),
        ownership: RunOwnershipSnapshot {
            run_id: run_id.clone(),
            current_claim: None,
            last_terminal_claim_id: None,
            last_claim_transition: None,
            claim_required_for_mutation: true,
            owner_visibility: OwnershipVisibility::ObserverReadAllowed,
        },
        run_result: None,
        progress: ProgressView {
            graph_id: None,
            total_nodes: 0,
            roots: 0,
            terminals: 0,
            status_counts: ActionStatusCountsView::default(),
            active_node_ids: Vec::new(),
            blocked_node_ids: Vec::new(),
            last_completed_node_id: None,
            required_evidence_count: 1,
            actuation_record_count: 0,
        },
    };

    let updated = store
        .apply_inspect(&session_id, &inspect)
        .expect("updated link");
    let cursor = updated.inspect_cursor.expect("inspect cursor");
    assert_eq!(cursor.checkpoint_seq, 3);
    assert_eq!(cursor.plan_epoch, 2);
    assert_eq!(cursor.status, RunStatus::AwaitingEvidence);
    assert_eq!(cursor.phase, RunPhase::AwaitingHost);
}

#[test]
fn relinking_run_moves_it_to_the_new_host_session() {
    let mut store = InMemoryHostSessionStore::default();
    let first_session_id: HostSessionId = "session-a".into();
    let second_session_id: HostSessionId = "session-b".into();
    let run_id: RunId = "run-1".into();

    store.link_run(HostRunLink::new(
        first_session_id.clone(),
        run_id.clone(),
        "swap usdc to weth".to_owned(),
        vec!["eip155:1".to_owned()],
    ));
    store.link_run(HostRunLink::new(
        second_session_id.clone(),
        run_id.clone(),
        "swap usdc to weth".to_owned(),
        vec!["eip155:1".to_owned()],
    ));

    assert!(store.session_snapshot(&first_session_id).is_none());
    let second_snapshot = store
        .session_snapshot(&second_session_id)
        .expect("second session snapshot");
    assert_eq!(second_snapshot.active_run_id, Some(run_id.clone()));
    assert_eq!(second_snapshot.linked_runs.len(), 1);
    assert_eq!(
        second_snapshot.linked_runs[0].host_session_id,
        second_session_id
    );
    assert_eq!(
        store.run_link(&run_id).expect("run link").host_session_id.0,
        "session-b"
    );
}
