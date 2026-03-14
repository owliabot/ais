use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use ais_agent_control::{
    audit::{
        GovernorDecisionAuditKind, GovernorDecisionAuditRecord, RuntimeAudit, RuntimeAuditRecord,
    },
    commands::{
        BeginRunCommand, ClaimRunCommand, EvidenceKind, EvidenceSubmission, InspectRunCommand,
        MissionBudgetSubmission, MissionSubmission, RequestCancelRunCommand, RunCommand,
        SignerDecisionKind, SignerDecisionSubmission, StepBudget, StepRunCommand, StepUntil,
        SubmitEvidenceCommand, SubmitSignerDecisionCommand,
    },
    events::{RunEvent, RunEventEnvelope, RunProgress},
    ids::{AuditId, ClaimId, CommandId, EventId, RunId, SignerRequestId},
    launch_spec::{LaunchSpecSubmission, PrebuiltFragmentLaunchSpec},
    ownership::{ClaimTransitionKind, RunClaim, RunClaimMode, RunClaimOwnerKind, RunClaimStatus},
    recovery::{CancelState, InterruptionClass},
};
use ais_agent_core::{
    action::{
        kinds::{
            actuate::{ActuateAction, ActuateMode},
            derive::{DeriveAction, DeriveKind},
            verify::{VerifyAction, VerifyKind},
        },
        ActionGraph, ActionNode, ActionNodeKind, ActionNodeStatus, ActionOrigin, ActionPayload,
    },
    checkpoint::{CheckpointSnapshot, PendingRequestsSnapshot},
    effect::{EffectAssertion, EffectContract, EffectContractKind},
    evidence::{
        EvidenceFreshness, EvidenceGraph, EvidenceKind as CoreEvidenceKind, EvidenceProvenance,
        EvidenceRecord, EvidenceRequirement,
    },
    mission::{Mission, MissionBudget, MissionPolicy},
    runtime::{
        BoundaryKind, RunLifecycleState, RunPhase, RunStatus, SignerRequestState,
        SignerRequestStatus,
    },
};
use ais_agent_host::{
    control::{HostCommandResponse, HostCommandService},
    events::{HostRunEventQuery, HostRunEventService},
    session::{
        HostCommandEnvelope, HostRunLink, HostSessionId, HostSessionStore, InMemoryHostSessionStore,
    },
};
use ais_agent_runtime::persistence::{
    restore_active_run, CheckpointArchive, CheckpointArchiveEntry, CheckpointArchiveKind,
    ClaimExpireRequest, ClaimRenewRequest, ClaimSupersedeRequest, DurableCommitError,
    DurableMutationExecutor, DurableMutationKind, DurableMutationUnit, EventArchive,
    EventArchiveQuery, MissionRepository, MissionWrite, MissionWriteMode, RunCatalogEntry,
    RunCatalogRepository, RunClaimRepository, RunClaimRepositoryError, RuntimeAuditArchive,
    RuntimeAuditQuery, SignerStateArchive, SignerStateArchiveError, SignerStateWrite,
};
use ais_agent_runtime::{
    runtime::{InMemoryRunRepository, RunRepository},
    service::RuntimeHostService,
};
use serde_json::json;

use crate::{migrate_connection, SqliteStore, SCHEMA_VERSION};

type SqliteHostService = RuntimeHostService<
    InMemoryRunRepository,
    SqliteStore,
    SqliteStore,
    SqliteStore,
    SqliteStore,
    InMemoryHostSessionStore,
    SqliteStore,
    SqliteStore,
    SqliteStore,
>;

#[test]
fn sqlite_store_bootstraps_schema_version() {
    let store = SqliteStore::open_in_memory().expect("open sqlite store");
    let version: i32 = store
        .connection()
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .expect("query version");
    assert_eq!(version, SCHEMA_VERSION);
}

#[test]
fn sqlite_store_open_path_applies_operational_pragmas() {
    let sqlite_path = sqlite_test_path("pragmas");
    let store = SqliteStore::open_path(&sqlite_path).expect("open sqlite store");

    let journal_mode: String = store
        .connection()
        .pragma_query_value(None, "journal_mode", |row| row.get(0))
        .expect("query journal mode");
    let foreign_keys: i32 = store
        .connection()
        .pragma_query_value(None, "foreign_keys", |row| row.get(0))
        .expect("query foreign_keys");
    let synchronous: i32 = store
        .connection()
        .pragma_query_value(None, "synchronous", |row| row.get(0))
        .expect("query synchronous");
    let busy_timeout: i32 = store
        .connection()
        .pragma_query_value(None, "busy_timeout", |row| row.get(0))
        .expect("query busy_timeout");

    assert_eq!(journal_mode.to_lowercase(), "wal");
    assert_eq!(foreign_keys, 1);
    assert_eq!(synchronous, 1);
    assert_eq!(busy_timeout, 5_000);
}

#[test]
fn sqlite_store_round_trips_mission_catalog_checkpoint_and_event_archives() {
    let mut store = SqliteStore::open_in_memory().expect("open sqlite store");
    let run_id = RunId("run-1".to_owned());

    store
        .insert(run_id.clone(), sample_mission())
        .expect("insert mission");
    RunCatalogRepository::upsert(&mut store, sample_run_catalog_entry(run_id.clone()))
        .expect("upsert catalog");
    CheckpointArchive::append(
        &mut store,
        CheckpointArchiveEntry {
            snapshot: sample_checkpoint(),
            kind: CheckpointArchiveKind::Boundary,
        },
    )
    .expect("append checkpoint");
    EventArchive::append(&mut store, sample_event(run_id.clone(), 1)).expect("append event");

    let mission = MissionRepository::load(&store, &run_id).expect("load mission");
    assert_eq!(mission.mission_id, "mission-1");

    let catalog = RunCatalogRepository::load(&store, &run_id).expect("load catalog");
    assert_eq!(catalog.active_boundary_kind, Some(BoundaryKind::Evidence));

    let checkpoint = CheckpointArchive::latest(&store, &run_id.0).expect("latest checkpoint");
    assert_eq!(checkpoint.checkpoint_seq, 1);

    let events = EventArchive::read(
        &store,
        EventArchiveQuery {
            run_id,
            after_event_seq: None,
            limit: Some(10),
        },
    )
    .expect("read events");
    assert_eq!(events.events.len(), 1);
    assert_eq!(events.latest_event_seq, Some(1));
}

#[test]
fn sqlite_store_round_trips_pending_signer_state() {
    let mut store = SqliteStore::open_in_memory().expect("open sqlite store");
    let signer_state = sample_signer_state();

    SignerStateArchive::upsert(&mut store, signer_state.clone()).expect("upsert signer state");

    let loaded = SignerStateArchive::load(&store, &signer_state.run_id).expect("load signer state");
    assert_eq!(loaded, signer_state);

    let stored_request_id: String = store
        .connection()
        .query_row(
            "SELECT request_id FROM signer_state_archive WHERE run_id = ?1",
            [&loaded.run_id.0],
            |row| row.get(0),
        )
        .expect("load signer request id");
    assert_eq!(stored_request_id, loaded.request_id.0);
}

#[test]
fn sqlite_run_claim_repository_round_trips_active_claim_and_history() {
    let mut store = SqliteStore::open_in_memory().expect("open sqlite store");
    let run_id = RunId("run-claim-1".to_owned());
    let claim = sample_claim("claim-1", run_id.clone(), 1, Some(20));

    let acquired = RunClaimRepository::acquire(&mut store, claim.clone()).expect("acquire claim");
    assert_eq!(acquired, claim);
    assert_eq!(
        RunClaimRepository::load_active(&store, &run_id).expect("load active"),
        Some(claim.clone())
    );
    assert_eq!(
        RunClaimRepository::load_claim(&store, &ClaimId("claim-1".to_owned())).expect("load claim"),
        claim
    );
}

#[test]
fn sqlite_run_claim_repository_reports_conflict_and_epoch_mismatch() {
    let mut store = SqliteStore::open_in_memory().expect("open sqlite store");
    let run_id = RunId("run-claim-2".to_owned());
    RunClaimRepository::acquire(
        &mut store,
        sample_claim("claim-1", run_id.clone(), 1, Some(20)),
    )
    .expect("acquire claim");

    let conflict = RunClaimRepository::acquire(
        &mut store,
        sample_claim("claim-2", run_id.clone(), 1, Some(30)),
    )
    .expect_err("active conflict");
    assert_eq!(
        conflict,
        RunClaimRepositoryError::ActiveClaimConflict {
            run_id: "run-claim-2".to_owned(),
            claim_id: "claim-1".to_owned(),
        }
    );

    let stale = RunClaimRepository::renew(
        &mut store,
        ClaimRenewRequest {
            run_id: run_id.clone(),
            claim_id: ClaimId("claim-1".to_owned()),
            claim_epoch: 9,
            renewed_at_ms: 15,
            lease_expires_at_ms: Some(25),
        },
    )
    .expect_err("stale epoch");
    assert_eq!(
        stale,
        RunClaimRepositoryError::ClaimEpochConflict {
            claim_id: "claim-1".to_owned(),
            expected_claim_epoch: 9,
            actual_claim_epoch: 1,
        }
    );
}

#[test]
fn sqlite_run_claim_repository_expires_and_supersedes_claims() {
    let mut store = SqliteStore::open_in_memory().expect("open sqlite store");
    let run_id = RunId("run-claim-3".to_owned());
    RunClaimRepository::acquire(
        &mut store,
        sample_claim("claim-1", run_id.clone(), 1, Some(20)),
    )
    .expect("acquire claim");

    let expired = RunClaimRepository::expire_stale(
        &mut store,
        ClaimExpireRequest {
            run_id: run_id.clone(),
            now_ms: 21,
        },
    )
    .expect("expire stale")
    .expect("expired claim");
    assert_eq!(expired.status, RunClaimStatus::Expired);
    assert!(RunClaimRepository::load_active(&store, &run_id)
        .expect("load active")
        .is_none());

    RunClaimRepository::acquire(
        &mut store,
        sample_claim("claim-2", run_id.clone(), 1, Some(40)),
    )
    .expect("re-acquire claim");
    let superseded = RunClaimRepository::supersede(
        &mut store,
        ClaimSupersedeRequest {
            run_id: run_id.clone(),
            predecessor_claim_id: ClaimId("claim-2".to_owned()),
            predecessor_claim_epoch: 1,
            successor_claim: sample_claim("claim-3", run_id.clone(), 1, Some(60)),
        },
    )
    .expect("supersede claim");
    assert_eq!(superseded.predecessor.status, RunClaimStatus::Superseded);
    assert_eq!(superseded.successor.claim_id.0, "claim-3");
}

#[test]
fn sqlite_run_claim_repository_active_claim_survives_restart() {
    let sqlite_path = sqlite_test_path("claims-restart");
    {
        let mut store = SqliteStore::open_path(&sqlite_path).expect("open sqlite store");
        RunClaimRepository::acquire(
            &mut store,
            sample_claim("claim-1", RunId("run-claim-4".to_owned()), 1, Some(20)),
        )
        .expect("acquire claim");
    }

    let store = SqliteStore::open_path(&sqlite_path).expect("reopen sqlite store");
    let active = RunClaimRepository::load_active(&store, &RunId("run-claim-4".to_owned()))
        .expect("load active after restart")
        .expect("active claim");
    assert_eq!(active.claim_id.0, "claim-1");
    assert_eq!(active.status, RunClaimStatus::Active);
}

#[test]
fn sqlite_runtime_audit_archive_round_trips_cursor_reads() {
    let mut store = SqliteStore::open_in_memory().expect("open sqlite store");
    let first = sample_runtime_audit(1);
    let second = sample_runtime_audit(2);

    RuntimeAuditArchive::append(&mut store, first.clone()).expect("append first audit");
    RuntimeAuditArchive::append(&mut store, second.clone()).expect("append second audit");

    let slice = RuntimeAuditArchive::read(
        &store,
        RuntimeAuditQuery {
            run_id: first.run_id.clone(),
            after_audit_seq: Some(1),
            limit: Some(10),
        },
    )
    .expect("read audit slice");

    assert_eq!(slice.latest_audit_seq, Some(2));
    assert_eq!(slice.next_after_audit_seq, Some(2));
    assert!(!slice.truncated);
    assert_eq!(slice.records.len(), 1);
    assert_eq!(slice.records[0].audit_seq, second.audit_seq);
    assert_eq!(slice.records[0].audit_id.0, second.audit_id.0);
}

#[test]
fn sqlite_grouped_commit_round_trips_all_members() {
    let mut store = SqliteStore::open_in_memory().expect("open sqlite store");
    let unit = sample_grouped_commit_unit();

    let receipt =
        DurableMutationExecutor::commit(&mut store, unit.clone()).expect("grouped commit succeeds");

    assert_eq!(receipt.run_id, unit.run_id);
    assert_eq!(receipt.kind, DurableMutationKind::Progress);
    assert_eq!(receipt.checkpoint_seq, 1);
    assert_eq!(receipt.latest_event_seq, Some(1));
    assert_eq!(receipt.latest_audit_seq, Some(1));

    let mission = MissionRepository::load(&store, &unit.run_id).expect("load mission");
    assert_eq!(mission.mission_id, "mission-1");

    let checkpoint = CheckpointArchive::latest(&store, &unit.run_id.0).expect("load checkpoint");
    assert_eq!(checkpoint.checkpoint_seq, 1);

    let events = EventArchive::read(
        &store,
        EventArchiveQuery {
            run_id: unit.run_id.clone(),
            after_event_seq: None,
            limit: Some(8),
        },
    )
    .expect("load events");
    assert_eq!(events.latest_event_seq, Some(1));
    assert_eq!(events.events.len(), 1);

    let catalog = RunCatalogRepository::load(&store, &unit.run_id).expect("load catalog");
    assert_eq!(catalog.latest_checkpoint_seq, 1);
    assert_eq!(catalog.latest_event_seq, Some(1));

    let signer = SignerStateArchive::load(&store, &unit.run_id).expect("load signer state");
    assert_eq!(signer.request_id.0, "signer-1");

    let audits = RuntimeAuditArchive::read(
        &store,
        RuntimeAuditQuery {
            run_id: unit.run_id,
            after_audit_seq: None,
            limit: Some(8),
        },
    )
    .expect("load audits");
    assert_eq!(audits.latest_audit_seq, Some(1));
    assert_eq!(audits.records.len(), 1);
}

#[test]
fn sqlite_grouped_commit_rolls_back_earlier_members_on_audit_failure() {
    let mut store = SqliteStore::open_in_memory().expect("open sqlite store");
    let unit = sample_grouped_commit_unit();
    RuntimeAuditArchive::append(&mut store, sample_runtime_audit(1)).expect("seed audit conflict");

    let error = DurableMutationExecutor::commit(&mut store, unit.clone())
        .expect_err("duplicate audit should fail grouped commit");

    match error {
        DurableCommitError::MemberWrite { member, .. } => {
            assert_eq!(
                member,
                ais_agent_runtime::persistence::DurableMutationMember::Audit
            );
        }
        other => panic!("unexpected grouped commit error: {other:?}"),
    }

    assert!(MissionRepository::load(&store, &unit.run_id).is_err());
    assert!(CheckpointArchive::latest(&store, &unit.run_id.0).is_err());
    assert!(RunCatalogRepository::load(&store, &unit.run_id).is_err());
    assert!(SignerStateArchive::load(&store, &unit.run_id).is_err());
    assert!(EventArchive::read(
        &store,
        EventArchiveQuery {
            run_id: unit.run_id.clone(),
            after_event_seq: None,
            limit: Some(8),
        },
    )
    .is_err());

    let audits = RuntimeAuditArchive::read(
        &store,
        RuntimeAuditQuery {
            run_id: unit.run_id,
            after_audit_seq: None,
            limit: Some(8),
        },
    )
    .expect("seeded audit remains");
    assert_eq!(audits.latest_audit_seq, Some(1));
    assert_eq!(audits.records.len(), 1);
}

#[test]
fn sqlite_event_archive_reads_full_batch_when_limit_is_none() {
    let mut store = SqliteStore::open_in_memory().expect("open sqlite store");
    let run_id = RunId("run-events-none".to_owned());
    EventArchive::append(&mut store, sample_event(run_id.clone(), 1)).expect("append event 1");
    EventArchive::append(&mut store, sample_event(run_id.clone(), 2)).expect("append event 2");

    let slice = EventArchive::read(
        &store,
        EventArchiveQuery {
            run_id,
            after_event_seq: None,
            limit: None,
        },
    )
    .expect("read unlimited batch");

    assert_eq!(slice.latest_event_seq, Some(2));
    assert_eq!(slice.next_after_event_seq, Some(2));
    assert!(!slice.truncated);
    assert_eq!(slice.events.len(), 2);
}

#[test]
fn sqlite_event_archive_handles_max_limit_without_overflow() {
    let mut store = SqliteStore::open_in_memory().expect("open sqlite store");
    let run_id = RunId("run-events-max".to_owned());
    EventArchive::append(&mut store, sample_event(run_id.clone(), 1)).expect("append event 1");
    EventArchive::append(&mut store, sample_event(run_id.clone(), 2)).expect("append event 2");

    let slice = EventArchive::read(
        &store,
        EventArchiveQuery {
            run_id,
            after_event_seq: None,
            limit: Some(usize::MAX),
        },
    )
    .expect("read huge-limit batch");

    assert_eq!(slice.latest_event_seq, Some(2));
    assert_eq!(slice.next_after_event_seq, Some(2));
    assert!(!slice.truncated);
    assert_eq!(slice.events.len(), 2);
}

#[test]
fn sqlite_store_updates_and_clears_pending_signer_state() {
    let mut store = SqliteStore::open_in_memory().expect("open sqlite store");
    let original = sample_signer_state();
    let mut updated = SignerRequestState::new_pending(
        SignerRequestId("signer-2".to_owned()),
        original.run_id.clone(),
        "eip155:1",
        "re-sign swap",
    )
    .with_node_id("swap-retry")
    .with_timeout(100, Some(250));
    updated.apply_decision(ais_agent_core::runtime::SignerDecision {
        request_id: updated.request_id.clone(),
        kind: ais_agent_core::runtime::SignerDecisionKind::Approved,
        decision_at_ms: Some(150),
        tx_hash: None,
    });

    SignerStateArchive::upsert(&mut store, original).expect("upsert original signer state");
    SignerStateArchive::upsert(&mut store, updated.clone()).expect("upsert updated signer state");

    let loaded =
        SignerStateArchive::load(&store, &updated.run_id).expect("load updated signer state");
    assert_eq!(loaded, updated);

    SignerStateArchive::clear(&mut store, &updated.run_id).expect("clear signer state");
    match SignerStateArchive::load(&store, &updated.run_id) {
        Err(SignerStateArchiveError::NotFound { run_id }) => assert_eq!(run_id, updated.run_id.0),
        other => panic!("unexpected signer archive result after clear: {other:?}"),
    }

    SignerStateArchive::clear(&mut store, &updated.run_id)
        .expect("clearing absent signer state stays idempotent");
}

#[test]
fn migrate_connection_is_idempotent() {
    let conn = rusqlite::Connection::open_in_memory().expect("open connection");
    migrate_connection(&conn).expect("migrate once");
    migrate_connection(&conn).expect("migrate twice");
}

#[test]
fn sqlite_checkpoint_latest_prefers_highest_checkpoint_truth_over_append_order() {
    let mut store = SqliteStore::open_in_memory().expect("open sqlite store");

    let mut newest = sample_checkpoint();
    newest.checkpoint_seq = 2;
    newest.lifecycle.checkpoint_seq = 2;
    newest.last_completed_node_id = Some("node-2".to_owned());
    CheckpointArchive::append(
        &mut store,
        CheckpointArchiveEntry {
            snapshot: newest,
            kind: CheckpointArchiveKind::Progress,
        },
    )
    .expect("append newest checkpoint");

    let mut stale = sample_checkpoint();
    stale.checkpoint_seq = 1;
    stale.lifecycle.checkpoint_seq = 1;
    stale.last_completed_node_id = Some("node-1".to_owned());
    CheckpointArchive::append(
        &mut store,
        CheckpointArchiveEntry {
            snapshot: stale,
            kind: CheckpointArchiveKind::Boundary,
        },
    )
    .expect("append stale checkpoint later");

    let latest = CheckpointArchive::latest(&store, "run-1").expect("latest checkpoint");
    assert_eq!(latest.checkpoint_seq, 2);
    assert_eq!(latest.last_completed_node_id.as_deref(), Some("node-2"));
}

#[test]
fn sqlite_checkpoint_archive_rejects_duplicate_checkpoint_identity() {
    let mut store = SqliteStore::open_in_memory().expect("open sqlite store");
    let checkpoint = sample_checkpoint();
    let duplicate = checkpoint.clone();

    CheckpointArchive::append(
        &mut store,
        CheckpointArchiveEntry {
            snapshot: checkpoint,
            kind: CheckpointArchiveKind::Boundary,
        },
    )
    .expect("append first checkpoint");

    let error = CheckpointArchive::append(
        &mut store,
        CheckpointArchiveEntry {
            snapshot: duplicate,
            kind: CheckpointArchiveKind::Boundary,
        },
    )
    .expect_err("duplicate checkpoint identity should fail");
    match error {
        ais_agent_runtime::persistence::CheckpointArchiveError::Storage { message } => {
            assert!(message.contains("UNIQUE"));
        }
        other => panic!("unexpected duplicate checkpoint error: {other:?}"),
    }
}

#[test]
fn sqlite_query_plans_use_supporting_indexes_for_hot_archive_reads() {
    let mut store = SqliteStore::open_in_memory().expect("open sqlite store");
    let checkpoint = sample_checkpoint();

    CheckpointArchive::append(
        &mut store,
        CheckpointArchiveEntry {
            snapshot: checkpoint.clone(),
            kind: CheckpointArchiveKind::Boundary,
        },
    )
    .expect("append checkpoint");
    let mut next = checkpoint;
    next.checkpoint_seq = 2;
    next.lifecycle.checkpoint_seq = 2;
    CheckpointArchive::append(
        &mut store,
        CheckpointArchiveEntry {
            snapshot: next,
            kind: CheckpointArchiveKind::Progress,
        },
    )
    .expect("append second checkpoint");

    let latest_plan = explain_query_plan(
        store.connection(),
        r#"
        SELECT snapshot_json
        FROM checkpoint_archive
        WHERE run_id = ?1
        ORDER BY checkpoint_seq DESC, plan_epoch DESC, archive_id DESC
        LIMIT 1
        "#,
        rusqlite::params!["run-1"],
    );
    assert!(
        latest_plan.iter().any(|detail| {
            detail.contains("idx_checkpoint_archive_latest_lookup")
                || detail.contains("idx_checkpoint_archive_run_seq_epoch")
        }),
        "latest plan: {latest_plan:?}"
    );

    EventArchive::append(&mut store, sample_event(RunId("run-1".to_owned()), 1))
        .expect("append event");
    EventArchive::append(&mut store, sample_event(RunId("run-1".to_owned()), 2))
        .expect("append event");

    let event_plan = explain_query_plan(
        store.connection(),
        r#"
        SELECT event_json
        FROM event_archive
        WHERE run_id = ?1
          AND event_seq > ?2
        ORDER BY event_seq ASC
        LIMIT ?3
        "#,
        rusqlite::params!["run-1", 0_u64, 10_u64],
    );
    assert!(
        event_plan
            .iter()
            .any(|detail| detail.contains("event_archive") && detail.contains("INDEX")),
        "event plan: {event_plan:?}"
    );
}

#[tokio::test]
async fn sqlite_host_service_can_inspect_after_restart_from_durable_archives() {
    let sqlite_path = sqlite_test_path("inspect");
    let host_session_id: HostSessionId = "session-sqlite-inspect".into();
    let run_id = RunId("run-1".to_owned());
    let mission = sample_mission();
    let checkpoint = completed_checkpoint();

    seed_sqlite_mission_checkpoint(&sqlite_path, run_id.clone(), mission.clone(), checkpoint);
    let mut session_store = InMemoryHostSessionStore::default();
    session_store.link_run(HostRunLink::new(
        host_session_id.clone(),
        run_id.clone(),
        mission.goal.clone(),
        mission.allowed_chains.clone(),
    ));

    let mut service = sqlite_host_service(
        &sqlite_path,
        InMemoryRunRepository::default(),
        session_store,
    );
    let inspect = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: Some("request-sqlite-inspect".into()),
            command: RunCommand::InspectRun(InspectRunCommand {
                command_id: CommandId("cmd-sqlite-inspect".to_owned()),
                run_id: run_id.clone(),
            }),
        })
        .await;

    match inspect.response {
        HostCommandResponse::Inspect(snapshot) => assert_eq!(
            snapshot.status,
            ais_agent_host::inspect::RunStatus::Completed
        ),
        other => panic!("unexpected response: {other:?}"),
    }

    let (
        run_repo,
        _checkpoint_repo,
        _mission_repo,
        _run_catalog_repo,
        _event_archive,
        _session_store,
        _signer_state_archive,
    ) = service.into_parts_with_signer_archive();
    assert!(run_repo.load(&run_id).is_err());
}

#[tokio::test]
async fn sqlite_host_service_replays_grouped_begin_run_truth_after_restart() {
    let sqlite_path = sqlite_test_path("begin-restart");
    let host_session_id: HostSessionId = "session-sqlite-begin-restart".into();
    let mut service = sqlite_host_service(
        &sqlite_path,
        InMemoryRunRepository::default(),
        InMemoryHostSessionStore::default(),
    );

    let begin = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: None,
            command: RunCommand::BeginRun(BeginRunCommand {
                command_id: CommandId("cmd-sqlite-begin-restart".to_owned()),
                idempotency_key: "idem-sqlite-begin-restart".into(),
                mission: MissionSubmission {
                    goal: "swap".to_owned(),
                    allowed_chains: vec!["eip155:1".to_owned()],
                    constraints: BTreeMap::new(),
                    budget: Some(MissionBudgetSubmission {
                        max_steps: Some(8),
                        max_signer_requests: Some(1),
                        max_wall_clock_ms: Some(30_000),
                    }),
                    metadata: BTreeMap::new(),
                },
                launch_spec: Some(LaunchSpecSubmission::PrebuiltFragment(
                    PrebuiltFragmentLaunchSpec::default(),
                )),
            }),
        })
        .await;

    let run_id = match begin.response {
        HostCommandResponse::Accepted(response) => response.run_id.expect("run id"),
        other => panic!("unexpected response: {other:?}"),
    };

    let (
        _run_repo,
        _checkpoint_repo,
        _mission_repo,
        _run_catalog_repo,
        _event_archive,
        session_store,
        _signer_state_archive,
    ) = service.into_parts_with_signer_archive();
    let mut restarted = sqlite_host_service(
        &sqlite_path,
        InMemoryRunRepository::default(),
        session_store,
    );

    let inspect = restarted
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-sqlite-begin-restart-inspect".into()),
            command: RunCommand::InspectRun(InspectRunCommand {
                command_id: CommandId("cmd-sqlite-begin-restart-inspect".to_owned()),
                run_id: run_id.clone(),
            }),
        })
        .await;
    match inspect.response {
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(snapshot.status, ais_agent_host::inspect::RunStatus::Running);
        }
        other => panic!("unexpected inspect response: {other:?}"),
    }

    let events = restarted
        .list_events(HostRunEventQuery {
            run_id: run_id.clone(),
            after_event_seq: Some(0),
            limit: Some(8),
        })
        .await
        .expect("started event batch");
    assert_eq!(events.latest_event_seq, Some(1));
    assert_eq!(events.events.len(), 1);
    assert!(matches!(events.events[0].event, RunEvent::Started(_)));

    let (
        run_repo,
        _checkpoint_repo,
        _mission_repo,
        _run_catalog_repo,
        _event_archive,
        _session_store,
        _signer_state_archive,
    ) = restarted.into_parts_with_signer_archive();
    assert!(run_repo.load(&run_id).is_err());
}

#[tokio::test]
async fn sqlite_host_service_can_resume_awaiting_evidence_after_restart() {
    let sqlite_path = sqlite_test_path("evidence");
    let host_session_id: HostSessionId = "session-sqlite-evidence".into();
    let run_id = RunId("run-1".to_owned());
    let mission = sample_mission();
    let checkpoint = evidence_wait_checkpoint();

    seed_sqlite_mission_checkpoint(&sqlite_path, run_id.clone(), mission.clone(), checkpoint);
    let mut session_store = InMemoryHostSessionStore::default();
    session_store.link_run(HostRunLink::new(
        host_session_id.clone(),
        run_id.clone(),
        mission.goal.clone(),
        mission.allowed_chains.clone(),
    ));

    let mut service = sqlite_host_service(
        &sqlite_path,
        InMemoryRunRepository::default(),
        session_store,
    );
    let submit = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-sqlite-evidence".into()),
            command: RunCommand::SubmitEvidence(SubmitEvidenceCommand {
                command_id: CommandId("cmd-sqlite-evidence".to_owned()),
                run_id: run_id.clone(),
                evidence: EvidenceSubmission {
                    evidence_id: "quote".to_owned(),
                    kind: EvidenceKind::RouteOrQuote,
                    source: "quote-api".to_owned(),
                    observed_at_ms: Some(42),
                    chain_scope: Some("eip155:1".to_owned()),
                    payload: json!({"amount_out":"1000"}),
                    confidence: Some(0.95),
                },
                expected_version: None,
            }),
        })
        .await;
    assert!(matches!(submit.response, HostCommandResponse::Inspect(_)));

    let stepped = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: Some("request-sqlite-step".into()),
            command: RunCommand::StepRun(StepRunCommand {
                command_id: CommandId("cmd-sqlite-step".to_owned()),
                run_id,
                until: StepUntil::CompleteOrBoundary,
                budget: Some(StepBudget {
                    max_nodes: Some(8),
                    max_wall_clock_ms: None,
                }),
                expected_version: None,
            }),
        })
        .await;
    match stepped.response {
        HostCommandResponse::Inspect(snapshot) => assert_eq!(
            snapshot.status,
            ais_agent_host::inspect::RunStatus::Completed
        ),
        other => panic!("unexpected response: {other:?}"),
    }
}

#[tokio::test]
async fn sqlite_host_service_can_resume_awaiting_signer_with_restored_pending_state() {
    let sqlite_path = sqlite_test_path("signer");
    let host_session_id: HostSessionId = "session-sqlite-signer".into();
    let run_id = RunId("run-1".to_owned());
    let mission = sample_mission();
    let signer_state = sample_signer_state();
    let checkpoint = awaiting_signer_checkpoint(&signer_state);

    seed_sqlite_mission_checkpoint(&sqlite_path, run_id.clone(), mission.clone(), checkpoint);
    let mut signer_store = SqliteStore::open_path(&sqlite_path).expect("signer state store");
    SignerStateArchive::upsert(&mut signer_store, signer_state.clone())
        .expect("persist signer state");
    let restored = restore_active_run(
        &run_id,
        &SqliteStore::open_path(&sqlite_path).expect("mission store"),
        &SqliteStore::open_path(&sqlite_path).expect("checkpoint store"),
        &SqliteStore::open_path(&sqlite_path).expect("signer state store"),
    )
    .expect("restore runtime");

    let mut run_repo = InMemoryRunRepository::default();
    run_repo.insert(restored).expect("insert restored runtime");
    let mut session_store = InMemoryHostSessionStore::default();
    session_store.link_run(HostRunLink::new(
        host_session_id.clone(),
        run_id.clone(),
        mission.goal.clone(),
        mission.allowed_chains.clone(),
    ));
    let mut service = sqlite_host_service(&sqlite_path, run_repo, session_store);

    let signer = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-sqlite-signer".into()),
            command: RunCommand::SubmitSignerDecision(SubmitSignerDecisionCommand {
                command_id: CommandId("cmd-sqlite-signer".to_owned()),
                run_id: run_id.clone(),
                decision: SignerDecisionSubmission {
                    request_id: SignerRequestId("signer-1".to_owned()),
                    decision: SignerDecisionKind::Submitted,
                    tx_hash: Some("0xabc".to_owned()),
                    details: BTreeMap::new(),
                },
                expected_version: None,
            }),
        })
        .await;
    assert!(matches!(signer.response, HostCommandResponse::Inspect(_)));

    let checkpoint_store = SqliteStore::open_path(&sqlite_path).expect("checkpoint store");
    let signer_store = SqliteStore::open_path(&sqlite_path).expect("signer store");
    let catalog_store = SqliteStore::open_path(&sqlite_path).expect("catalog store");
    let checkpoint = CheckpointArchive::latest(&checkpoint_store, &run_id.0)
        .expect("latest checkpoint after signer decision");
    let signer_state = SignerStateArchive::load(&signer_store, &run_id)
        .expect("signer state after signer decision");
    let catalog =
        RunCatalogRepository::load(&catalog_store, &run_id).expect("catalog after signer decision");
    assert_eq!(checkpoint.checkpoint_seq, catalog.latest_checkpoint_seq);
    assert_eq!(signer_state.status, SignerRequestStatus::Submitted);
    assert_eq!(
        checkpoint
            .pending_requests
            .pending_signer_request_id
            .as_deref(),
        Some("signer-1")
    );

    let stepped = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: Some("request-sqlite-signer-step".into()),
            command: RunCommand::StepRun(StepRunCommand {
                command_id: CommandId("cmd-sqlite-signer-step".to_owned()),
                run_id,
                until: StepUntil::CompleteOrBoundary,
                budget: Some(StepBudget {
                    max_nodes: Some(8),
                    max_wall_clock_ms: None,
                }),
                expected_version: None,
            }),
        })
        .await;
    match stepped.response {
        HostCommandResponse::Pause(pause) => assert_eq!(
            pause.kind,
            ais_agent_host::inspect::PauseKind::NeedConfirmation
        ),
        other => panic!("unexpected response: {other:?}"),
    }
}

#[tokio::test]
async fn sqlite_host_service_preserves_cancel_pending_after_restart() {
    let sqlite_path = sqlite_test_path("cancel-pending");
    let host_session_id: HostSessionId = "session-sqlite-cancel-pending".into();
    let run_id = RunId("run-1".to_owned());
    let mission = sample_mission();
    let signer_state = sample_signer_state();
    let checkpoint = awaiting_signer_checkpoint(&signer_state);

    seed_sqlite_mission_checkpoint(&sqlite_path, run_id.clone(), mission.clone(), checkpoint);
    let mut signer_store = SqliteStore::open_path(&sqlite_path).expect("signer state store");
    SignerStateArchive::upsert(&mut signer_store, signer_state.clone())
        .expect("persist signer state");
    let restored = restore_active_run(
        &run_id,
        &SqliteStore::open_path(&sqlite_path).expect("mission store"),
        &SqliteStore::open_path(&sqlite_path).expect("checkpoint store"),
        &SqliteStore::open_path(&sqlite_path).expect("signer state store"),
    )
    .expect("restore runtime");

    let mut run_repo = InMemoryRunRepository::default();
    run_repo.insert(restored).expect("insert restored runtime");
    let mut session_store = InMemoryHostSessionStore::default();
    session_store.link_run(HostRunLink::new(
        host_session_id.clone(),
        run_id.clone(),
        mission.goal.clone(),
        mission.allowed_chains.clone(),
    ));
    let mut service = sqlite_host_service(&sqlite_path, run_repo, session_store);

    let signer = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-sqlite-cancel-pending-signer".into()),
            command: RunCommand::SubmitSignerDecision(SubmitSignerDecisionCommand {
                command_id: CommandId("cmd-sqlite-cancel-pending-signer".to_owned()),
                run_id: run_id.clone(),
                decision: SignerDecisionSubmission {
                    request_id: SignerRequestId("signer-1".to_owned()),
                    decision: SignerDecisionKind::Submitted,
                    tx_hash: Some("0xabc".to_owned()),
                    details: BTreeMap::new(),
                },
                expected_version: None,
            }),
        })
        .await;
    assert!(matches!(signer.response, HostCommandResponse::Inspect(_)));

    let stepped = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-sqlite-cancel-pending-step".into()),
            command: RunCommand::StepRun(StepRunCommand {
                command_id: CommandId("cmd-sqlite-cancel-pending-step".to_owned()),
                run_id: run_id.clone(),
                until: StepUntil::CompleteOrBoundary,
                budget: Some(StepBudget {
                    max_nodes: Some(8),
                    max_wall_clock_ms: None,
                }),
                expected_version: None,
            }),
        })
        .await;
    assert!(matches!(stepped.response, HostCommandResponse::Pause(_)));

    let cancel = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-sqlite-cancel-pending-request".into()),
            command: RunCommand::RequestCancelRun(RequestCancelRunCommand {
                command_id: CommandId("cmd-sqlite-cancel-pending-request".to_owned()),
                run_id: run_id.clone(),
                reason: Some("cancel after submission".to_owned()),
                expected_version: None,
            }),
        })
        .await;
    match cancel.response {
        HostCommandResponse::Pause(pause) => {
            assert_eq!(pause.cancel_state, Some(CancelState::Pending));
        }
        other => panic!("unexpected response: {other:?}"),
    }

    let (
        _run_repo,
        _checkpoint_repo,
        _mission_repo,
        _run_catalog_repo,
        _event_archive,
        _session_store,
        _signer_state_archive,
    ) = service.into_parts_with_signer_archive();
    let mut restarted = sqlite_host_service(
        &sqlite_path,
        InMemoryRunRepository::default(),
        InMemoryHostSessionStore::default(),
    );

    let inspect = restarted
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: Some("request-sqlite-cancel-pending-inspect".into()),
            command: RunCommand::InspectRun(InspectRunCommand {
                command_id: CommandId("cmd-sqlite-cancel-pending-inspect".to_owned()),
                run_id: run_id.clone(),
            }),
        })
        .await;
    match inspect.response {
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(snapshot.cancel_state, Some(CancelState::Pending));
            assert_eq!(
                snapshot.interruption_class,
                Some(InterruptionClass::HostCancelRequested)
            );
            assert_eq!(snapshot.pending_confirmations.len(), 1);
        }
        other => panic!("unexpected response: {other:?}"),
    }

    let checkpoint_store = SqliteStore::open_path(&sqlite_path).expect("checkpoint store");
    let checkpoint = CheckpointArchive::latest(&checkpoint_store, &run_id.0)
        .expect("latest checkpoint after restart");
    assert_eq!(
        checkpoint.lifecycle.cancel_state,
        Some(CancelState::Pending)
    );
}

#[tokio::test]
async fn sqlite_host_service_requires_reacquire_after_expired_claim_restart() {
    let sqlite_path = sqlite_test_path("claim-expired-restart");
    let host_session_id: HostSessionId = "session-sqlite-claim-expired".into();
    let run_id = RunId("run-1".to_owned());
    let mission = sample_mission();
    let checkpoint = evidence_wait_checkpoint();

    seed_sqlite_mission_checkpoint(&sqlite_path, run_id.clone(), mission.clone(), checkpoint);
    let mut claim_store = SqliteStore::open_path(&sqlite_path).expect("claim store");
    RunClaimRepository::acquire(
        &mut claim_store,
        sample_claim("claim-expired-1", run_id.clone(), 1, Some(10)),
    )
    .expect("acquire expired claim");

    let mut service = sqlite_host_service(
        &sqlite_path,
        InMemoryRunRepository::default(),
        InMemoryHostSessionStore::default(),
    );

    let inspect = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-sqlite-claim-expired-inspect".into()),
            command: RunCommand::InspectRun(InspectRunCommand {
                command_id: CommandId("cmd-sqlite-claim-expired-inspect".to_owned()),
                run_id: run_id.clone(),
            }),
        })
        .await;
    match inspect.response {
        HostCommandResponse::Inspect(snapshot) => {
            let current_claim = snapshot
                .ownership
                .current_claim
                .expect("expired claim in inspect");
            assert_eq!(current_claim.claim_id.0, "claim-expired-1");
            assert_eq!(current_claim.status, RunClaimStatus::Expired);
        }
        other => panic!("unexpected inspect response: {other:?}"),
    }

    let submit = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-sqlite-claim-expired-evidence".into()),
            command: RunCommand::SubmitEvidence(SubmitEvidenceCommand {
                command_id: CommandId("cmd-sqlite-claim-expired-evidence".to_owned()),
                run_id: run_id.clone(),
                evidence: EvidenceSubmission {
                    evidence_id: "quote".to_owned(),
                    kind: EvidenceKind::RouteOrQuote,
                    source: "quote-api".to_owned(),
                    observed_at_ms: Some(42),
                    chain_scope: Some("eip155:1".to_owned()),
                    payload: json!({"amount_out":"1000"}),
                    confidence: Some(0.95),
                },
                expected_version: None,
            }),
        })
        .await;
    match submit.response {
        HostCommandResponse::Error(error) => assert_eq!(error.code, "claim_expired"),
        other => panic!("unexpected expired mutation response: {other:?}"),
    }

    let claim = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: Some("request-sqlite-claim-expired-reacquire".into()),
            command: RunCommand::ClaimRun(ClaimRunCommand {
                command_id: CommandId("cmd-sqlite-claim-expired-reacquire".to_owned()),
                run_id: run_id.clone(),
                owner_kind: RunClaimOwnerKind::InteractiveHost,
                owner_instance_id: "session-sqlite-claim-expired".to_owned(),
                mode: RunClaimMode::ExclusiveMutation,
                requested_lease_ms: None,
                allow_supersede: false,
                expected_current_claim_id: None,
                expected_current_claim_epoch: None,
            }),
        })
        .await;
    match claim.response {
        HostCommandResponse::Inspect(snapshot) => {
            let current_claim = snapshot.ownership.current_claim.expect("reacquired claim");
            assert_ne!(current_claim.claim_id.0, "claim-expired-1");
            assert_eq!(current_claim.status, RunClaimStatus::Active);
        }
        other => panic!("unexpected reacquire response: {other:?}"),
    }
}

#[tokio::test]
async fn sqlite_host_service_keeps_released_claim_readable_but_not_mutable_after_restart() {
    let sqlite_path = sqlite_test_path("claim-released-restart");
    let host_session_id: HostSessionId = "session-sqlite-claim-released".into();
    let run_id = RunId("run-1".to_owned());
    let mission = sample_mission();
    let checkpoint = evidence_wait_checkpoint();

    seed_sqlite_mission_checkpoint(&sqlite_path, run_id.clone(), mission.clone(), checkpoint);
    let mut claim_store = SqliteStore::open_path(&sqlite_path).expect("claim store");
    RunClaimRepository::acquire(
        &mut claim_store,
        sample_claim("claim-released-1", run_id.clone(), 1, Some(u64::MAX / 2)),
    )
    .expect("acquire claim");
    RunClaimRepository::release(
        &mut claim_store,
        ais_agent_runtime::persistence::ClaimReleaseRequest {
            run_id: run_id.clone(),
            claim_id: ClaimId("claim-released-1".to_owned()),
            claim_epoch: 1,
        },
    )
    .expect("release claim");

    let mut service = sqlite_host_service(
        &sqlite_path,
        InMemoryRunRepository::default(),
        InMemoryHostSessionStore::default(),
    );

    let inspect = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-sqlite-claim-released-inspect".into()),
            command: RunCommand::InspectRun(InspectRunCommand {
                command_id: CommandId("cmd-sqlite-claim-released-inspect".to_owned()),
                run_id: run_id.clone(),
            }),
        })
        .await;
    match inspect.response {
        HostCommandResponse::Inspect(snapshot) => {
            assert!(snapshot.ownership.current_claim.is_none());
            assert_eq!(
                snapshot.ownership.last_terminal_claim_id,
                Some(ClaimId("claim-released-1".to_owned()))
            );
            assert_eq!(
                snapshot.ownership.last_claim_transition,
                Some(ClaimTransitionKind::ClaimReleased)
            );
        }
        other => panic!("unexpected inspect response: {other:?}"),
    }

    let submit = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: Some("request-sqlite-claim-released-evidence".into()),
            command: RunCommand::SubmitEvidence(SubmitEvidenceCommand {
                command_id: CommandId("cmd-sqlite-claim-released-evidence".to_owned()),
                run_id,
                evidence: EvidenceSubmission {
                    evidence_id: "quote".to_owned(),
                    kind: EvidenceKind::RouteOrQuote,
                    source: "quote-api".to_owned(),
                    observed_at_ms: Some(42),
                    chain_scope: Some("eip155:1".to_owned()),
                    payload: json!({"amount_out":"1000"}),
                    confidence: Some(0.95),
                },
                expected_version: None,
            }),
        })
        .await;
    match submit.response {
        HostCommandResponse::Error(error) => assert_eq!(error.code, "claim_required"),
        other => panic!("unexpected released mutation response: {other:?}"),
    }
}

#[tokio::test]
async fn sqlite_host_service_keeps_catalog_pointers_consistent_with_archives_after_mutations() {
    let sqlite_path = sqlite_test_path("catalog-consistency");
    let host_session_id: HostSessionId = "session-sqlite-catalog".into();
    let run_id = RunId("run-1".to_owned());
    let mission = sample_mission();
    let checkpoint = evidence_wait_checkpoint();

    seed_sqlite_mission_checkpoint(&sqlite_path, run_id.clone(), mission.clone(), checkpoint);
    let mut session_store = InMemoryHostSessionStore::default();
    session_store.link_run(HostRunLink::new(
        host_session_id.clone(),
        run_id.clone(),
        mission.goal.clone(),
        mission.allowed_chains.clone(),
    ));

    let mut service = sqlite_host_service(
        &sqlite_path,
        InMemoryRunRepository::default(),
        session_store,
    );

    let submit = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-sqlite-catalog-evidence".into()),
            command: RunCommand::SubmitEvidence(SubmitEvidenceCommand {
                command_id: CommandId("cmd-sqlite-catalog-evidence".to_owned()),
                run_id: run_id.clone(),
                evidence: EvidenceSubmission {
                    evidence_id: "quote".to_owned(),
                    kind: EvidenceKind::RouteOrQuote,
                    source: "quote-api".to_owned(),
                    observed_at_ms: Some(42),
                    chain_scope: Some("eip155:1".to_owned()),
                    payload: json!({"amount_out":"1000"}),
                    confidence: Some(0.95),
                },
                expected_version: None,
            }),
        })
        .await;
    assert!(matches!(submit.response, HostCommandResponse::Inspect(_)));

    let stepped = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: Some("request-sqlite-catalog-step".into()),
            command: RunCommand::StepRun(StepRunCommand {
                command_id: CommandId("cmd-sqlite-catalog-step".to_owned()),
                run_id: run_id.clone(),
                until: StepUntil::CompleteOrBoundary,
                budget: Some(StepBudget {
                    max_nodes: Some(8),
                    max_wall_clock_ms: None,
                }),
                expected_version: None,
            }),
        })
        .await;
    assert!(matches!(stepped.response, HostCommandResponse::Inspect(_)));

    let checkpoint_store = SqliteStore::open_path(&sqlite_path).expect("checkpoint store");
    let checkpoint = CheckpointArchive::latest(&checkpoint_store, &run_id.0).expect("checkpoint");
    let catalog_store = SqliteStore::open_path(&sqlite_path).expect("catalog store");
    let catalog = RunCatalogRepository::load(&catalog_store, &run_id).expect("catalog");
    let event_store = SqliteStore::open_path(&sqlite_path).expect("event store");
    let events = EventArchive::read(
        &event_store,
        EventArchiveQuery {
            run_id,
            after_event_seq: Some(0),
            limit: Some(32),
        },
    )
    .expect("events");

    assert_eq!(catalog.latest_checkpoint_seq, checkpoint.checkpoint_seq);
    assert_eq!(catalog.latest_revision, checkpoint.checkpoint_seq);
    assert_eq!(catalog.latest_event_seq, events.latest_event_seq);
}

#[test]
fn sqlite_store_round_trips_side_effect_checkpoint_with_verify_resume_truth() {
    let sqlite_path = sqlite_test_path("verify-resume");
    let run_id = RunId("run-1".to_owned());
    let mission = sample_mission();
    let checkpoint = verifying_after_broadcast_checkpoint();

    let mut mission_store = SqliteStore::open_path(&sqlite_path).expect("mission store");
    mission_store
        .insert(run_id.clone(), mission.clone())
        .expect("insert mission");

    let mut checkpoint_store = SqliteStore::open_path(&sqlite_path).expect("checkpoint store");
    CheckpointArchive::append(
        &mut checkpoint_store,
        CheckpointArchiveEntry {
            snapshot: checkpoint.clone(),
            kind: CheckpointArchiveKind::SideEffect,
        },
    )
    .expect("append side-effect checkpoint");

    let history = CheckpointArchive::history(&checkpoint_store, &run_id.0).expect("history");
    let latest = history.last().expect("latest checkpoint");
    assert_eq!(latest.kind, CheckpointArchiveKind::SideEffect);
    assert_eq!(
        latest
            .snapshot
            .pending_requests
            .pending_confirmation_id
            .as_deref(),
        Some("0xabc")
    );
    assert!(latest.snapshot.effect_contracts.contains_key("effect.swap"));
    assert!(latest
        .snapshot
        .evidence_graph
        .records
        .contains_key("state.pre.out"));

    let restored = restore_active_run(
        &run_id,
        &SqliteStore::open_path(&sqlite_path).expect("mission store"),
        &SqliteStore::open_path(&sqlite_path).expect("checkpoint store"),
        &SqliteStore::open_path(&sqlite_path).expect("signer state store"),
    )
    .expect("restore runtime");
    assert_eq!(
        restored
            .checkpoint
            .pending_requests
            .pending_confirmation_id
            .as_deref(),
        Some("0xabc")
    );
    assert!(restored
        .checkpoint
        .effect_contracts
        .contains_key("effect.swap"));
    assert!(restored
        .checkpoint
        .evidence_graph
        .records
        .contains_key("state.pre.out"));
}

fn sqlite_host_service(
    sqlite_path: &Path,
    run_repo: InMemoryRunRepository,
    session_store: InMemoryHostSessionStore,
) -> SqliteHostService {
    RuntimeHostService::<
        InMemoryRunRepository,
        SqliteStore,
        SqliteStore,
        SqliteStore,
        SqliteStore,
        InMemoryHostSessionStore,
        SqliteStore,
        SqliteStore,
        SqliteStore,
    >::new_with_archives_and_claim_repo(
        run_repo,
        SqliteStore::open_path(sqlite_path).expect("checkpoint store"),
        SqliteStore::open_path(sqlite_path).expect("mission store"),
        SqliteStore::open_path(sqlite_path).expect("catalog store"),
        SqliteStore::open_path(sqlite_path).expect("event store"),
        session_store,
        SqliteStore::open_path(sqlite_path).expect("signer state store"),
        SqliteStore::open_path(sqlite_path).expect("audit archive store"),
        SqliteStore::open_path(sqlite_path).expect("claim store"),
    )
}

fn seed_sqlite_mission_checkpoint(
    sqlite_path: &Path,
    run_id: RunId,
    mission: Mission,
    checkpoint: CheckpointSnapshot,
) {
    let mut mission_store = SqliteStore::open_path(sqlite_path).expect("mission store");
    mission_store
        .insert(run_id, mission)
        .expect("insert mission");

    let mut checkpoint_store = SqliteStore::open_path(sqlite_path).expect("checkpoint store");
    CheckpointArchive::append(
        &mut checkpoint_store,
        CheckpointArchiveEntry {
            snapshot: checkpoint,
            kind: CheckpointArchiveKind::Boundary,
        },
    )
    .expect("append checkpoint");
}

fn sqlite_test_path(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "ais-agent-store-sqlite-{label}-{}-{nanos}.sqlite",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    path
}

fn explain_query_plan(
    conn: &rusqlite::Connection,
    sql: &str,
    params: impl rusqlite::Params,
) -> Vec<String> {
    let explain_sql = format!("EXPLAIN QUERY PLAN {sql}");
    let mut stmt = conn.prepare(&explain_sql).expect("prepare explain");
    let rows = stmt
        .query_map(params, |row| row.get::<_, String>(3))
        .expect("run explain");
    rows.collect::<Result<Vec<_>, _>>()
        .expect("collect explain")
}

fn sample_mission() -> Mission {
    Mission {
        mission_id: "mission-1".to_owned(),
        goal: "swap usdc to eth".to_owned(),
        allowed_chains: vec!["eip155:1".to_owned()],
        budget: MissionBudget {
            max_steps: Some(10),
            max_signer_requests: Some(2),
            max_wall_clock_ms: Some(60_000),
        },
        policy: MissionPolicy {
            policy_mode: Some("guarded".to_owned()),
            allow_raw_envelopes: true,
            require_effect_contract_for_writes: true,
        },
        constraints: BTreeMap::new(),
        metadata: BTreeMap::new(),
    }
}

fn sample_run_catalog_entry(run_id: RunId) -> RunCatalogEntry {
    RunCatalogEntry {
        run_id,
        mission_id: "mission-1".to_owned(),
        status: RunStatus::AwaitingEvidence,
        phase: RunPhase::AwaitingHost,
        active_boundary_kind: Some(BoundaryKind::Evidence),
        latest_checkpoint_seq: 1,
        latest_event_seq: Some(1),
        latest_revision: 1,
        created_at_ms: Some(1000),
        updated_at_ms: Some(2000),
        terminal_at_ms: None,
    }
}

fn sample_checkpoint() -> CheckpointSnapshot {
    let mut lifecycle = RunLifecycleState::new(RunId("run-1".to_owned()), "mission-1");
    lifecycle.mark_running(RunPhase::Planning);
    lifecycle.await_evidence("need quote", vec!["evidence.quote".to_owned()]);
    lifecycle.bump_checkpoint();

    CheckpointSnapshot {
        run_id: "run-1".to_owned(),
        mission_id: "mission-1".to_owned(),
        checkpoint_seq: lifecycle.checkpoint_seq,
        plan_epoch: lifecycle.plan_epoch,
        lifecycle,
        action_graph: ActionGraph {
            graph_id: Some("graph-1".to_owned()),
            roots: Vec::new(),
            terminals: Vec::new(),
            nodes: BTreeMap::new(),
        },
        evidence_graph: EvidenceGraph::default(),
        effect_contracts: Default::default(),
        pending_requests: PendingRequestsSnapshot {
            pending_evidence_refs: vec!["evidence.quote".to_owned()],
            pending_envelope_refs: Vec::new(),
            pending_signer_request_id: None,
            pending_confirmation_id: None,
        },
        last_completed_node_id: None,
        actuation_records: Vec::new(),
    }
}

fn evidence_wait_checkpoint() -> CheckpointSnapshot {
    let mut lifecycle = RunLifecycleState::new(RunId("run-1".to_owned()), "mission-1");
    lifecycle.mark_running(RunPhase::Planning);
    lifecycle.await_evidence("need quote", vec!["evidence.quote".to_owned()]);

    let mut checkpoint = base_checkpoint(lifecycle, vec![derive_terminal_node("derive-quote")]);
    checkpoint
        .evidence_graph
        .requirements
        .push(EvidenceRequirement {
            requirement_id: "req-1".to_owned(),
            reference: "evidence.quote".to_owned(),
            reason: "quote required".to_owned(),
            required_by_node_id: Some("derive-quote".to_owned()),
            satisfied_by_evidence_id: None,
        });
    checkpoint.pending_requests.pending_evidence_refs = vec!["evidence.quote".to_owned()];
    checkpoint
}

fn awaiting_signer_checkpoint(signer_state: &SignerRequestState) -> CheckpointSnapshot {
    let mut lifecycle = RunLifecycleState::new(RunId("run-1".to_owned()), "mission-1");
    lifecycle.mark_running(RunPhase::Broadcasting);
    lifecycle.await_signer_request(signer_state);
    lifecycle.bump_checkpoint();
    lifecycle.bump_plan_epoch();

    let mut checkpoint = base_checkpoint(
        lifecycle,
        vec![
            succeeded_actuate_node("swap"),
            verify_terminal_node("verify-swap", vec!["swap"]),
        ],
    );
    checkpoint.pending_requests.pending_signer_request_id = Some(signer_state.request_id.0.clone());
    checkpoint.last_completed_node_id = Some("simulate-swap".to_owned());
    checkpoint
}

fn completed_checkpoint() -> CheckpointSnapshot {
    let mut lifecycle = RunLifecycleState::new(RunId("run-1".to_owned()), "mission-1");
    lifecycle.mark_running(RunPhase::Verifying);
    lifecycle.bump_checkpoint();
    lifecycle.bump_plan_epoch();
    lifecycle.complete("swap completed");

    let mut checkpoint = base_checkpoint(
        lifecycle,
        vec![
            succeeded_actuate_node("swap"),
            verify_terminal_node("verify-swap", vec!["swap"]),
        ],
    );
    checkpoint.last_completed_node_id = Some("verify-swap".to_owned());
    checkpoint
}

fn verifying_after_broadcast_checkpoint() -> CheckpointSnapshot {
    let mut lifecycle = RunLifecycleState::new(RunId("run-1".to_owned()), "mission-1");
    lifecycle.mark_running(RunPhase::Verifying);
    lifecycle.await_confirmation("waiting for chain receipt 0xabc");
    lifecycle.bump_checkpoint();
    lifecycle.bump_plan_epoch();

    let mut checkpoint = base_checkpoint(
        lifecycle,
        vec![
            succeeded_actuate_node("swap"),
            verify_effect_node("verify-swap", vec!["swap"]),
        ],
    );
    checkpoint.last_completed_node_id = Some("swap".to_owned());
    checkpoint.pending_requests.pending_confirmation_id = Some("0xabc".to_owned());
    checkpoint
        .effect_contracts
        .insert("effect.swap".to_owned(), sample_effect_contract());
    checkpoint.evidence_graph.records.insert(
        "state.pre.out".to_owned(),
        sample_pre_observation("state.pre.out"),
    );
    checkpoint
}

fn sample_signer_state() -> SignerRequestState {
    SignerRequestState::new_pending(
        SignerRequestId("signer-1".to_owned()),
        RunId("run-1".to_owned()),
        "eip155:1",
        "sign swap",
    )
    .with_node_id("swap")
}

fn sample_runtime_audit(audit_seq: u64) -> RuntimeAuditRecord {
    RuntimeAuditRecord {
        audit_id: AuditId(format!("audit-{audit_seq}")),
        run_id: RunId("run-1".to_owned()),
        audit_seq,
        checkpoint_seq: 1,
        plan_epoch: 0,
        audit: RuntimeAudit::GovernorDecision(GovernorDecisionAuditRecord {
            node_id: Some("swap".to_owned()),
            decision: GovernorDecisionAuditKind::Allow,
            reason: "looks good".to_owned(),
            evidence_refs: vec!["evidence.quote".to_owned()],
            signer_request_id: None,
            rejection_code: None,
        }),
    }
}

fn sample_grouped_commit_unit() -> DurableMutationUnit {
    let run_id = RunId("run-1".to_owned());
    let mission = sample_mission();
    let event = sample_event(run_id.clone(), 1);

    DurableMutationUnit {
        run_id: run_id.clone(),
        kind: DurableMutationKind::Progress,
        mission_write: Some(MissionWrite {
            run_id: run_id.clone(),
            mode: MissionWriteMode::Insert,
            mission,
        }),
        checkpoint_write: ais_agent_runtime::persistence::CheckpointWrite {
            entry: CheckpointArchiveEntry {
                snapshot: sample_checkpoint(),
                kind: CheckpointArchiveKind::Progress,
            },
        },
        event_write: ais_agent_runtime::persistence::EventWriteBatch {
            events: vec![event],
        },
        catalog_write: ais_agent_runtime::persistence::CatalogWrite {
            entry: sample_run_catalog_entry(run_id.clone()),
        },
        signer_write: Some(SignerStateWrite::Upsert {
            signer_state: sample_signer_state(),
        }),
        audit_write: ais_agent_runtime::persistence::AuditWriteBatch {
            records: vec![sample_runtime_audit(1)],
        },
    }
}

fn base_checkpoint(lifecycle: RunLifecycleState, nodes: Vec<ActionNode>) -> CheckpointSnapshot {
    let terminals = nodes
        .iter()
        .filter(|node| node.kind == ActionNodeKind::Verify || node.kind == ActionNodeKind::Derive)
        .map(|node| node.node_id.clone())
        .collect();
    CheckpointSnapshot {
        run_id: "run-1".to_owned(),
        mission_id: "mission-1".to_owned(),
        checkpoint_seq: lifecycle.checkpoint_seq,
        plan_epoch: lifecycle.plan_epoch,
        lifecycle,
        action_graph: ActionGraph {
            graph_id: Some("graph-1".to_owned()),
            roots: Vec::new(),
            terminals,
            nodes: nodes
                .into_iter()
                .map(|node| (node.node_id.clone(), node))
                .collect(),
        },
        evidence_graph: EvidenceGraph::default(),
        effect_contracts: Default::default(),
        pending_requests: PendingRequestsSnapshot::default(),
        last_completed_node_id: None,
        actuation_records: Vec::new(),
    }
}

fn derive_terminal_node(node_id: &str) -> ActionNode {
    ActionNode {
        node_id: node_id.to_owned(),
        kind: ActionNodeKind::Derive,
        origin: ActionOrigin::DriverFragment,
        status: ActionNodeStatus::Pending,
        depends_on: Vec::new(),
        inputs: Vec::new(),
        evidence_refs: vec!["evidence.quote".to_owned()],
        payload: ActionPayload::Derive(DeriveAction {
            derive_kind: DeriveKind::Parameter,
            derivation_hint: "derive quote".to_owned(),
            output_key: Some("quote".to_owned()),
        }),
        implementation_hint: None,
        expected_effect_ref: None,
    }
}

fn succeeded_actuate_node(node_id: &str) -> ActionNode {
    ActionNode {
        node_id: node_id.to_owned(),
        kind: ActionNodeKind::Actuate,
        origin: ActionOrigin::DriverFragment,
        status: ActionNodeStatus::Succeeded,
        depends_on: Vec::new(),
        inputs: Vec::new(),
        evidence_refs: Vec::new(),
        payload: ActionPayload::Actuate(ActuateAction {
            mode: ActuateMode::DriverCall,
            actuator_hint: "swap".to_owned(),
            chain: Some("eip155:1".to_owned()),
            envelope_ref: Some("env.swap".to_owned()),
            requires_effect_contract: true,
            live: None,
        }),
        implementation_hint: None,
        expected_effect_ref: Some("effect.swap".to_owned()),
    }
}

fn verify_terminal_node(node_id: &str, depends_on: Vec<&str>) -> ActionNode {
    ActionNode {
        node_id: node_id.to_owned(),
        kind: ActionNodeKind::Verify,
        origin: ActionOrigin::DriverFragment,
        status: ActionNodeStatus::Pending,
        depends_on: depends_on.into_iter().map(str::to_owned).collect(),
        inputs: Vec::new(),
        evidence_refs: Vec::new(),
        payload: ActionPayload::Verify(VerifyAction {
            verify_kind: VerifyKind::EffectContract,
            verifier_hint: "verify final effect".to_owned(),
            pre_observation_ref: None,
            post_observation_ref: None,
            live: None,
        }),
        implementation_hint: None,
        expected_effect_ref: Some("effect.swap".to_owned()),
    }
}

fn verify_effect_node(node_id: &str, depends_on: Vec<&str>) -> ActionNode {
    ActionNode {
        node_id: node_id.to_owned(),
        kind: ActionNodeKind::Verify,
        origin: ActionOrigin::DriverFragment,
        status: ActionNodeStatus::Pending,
        depends_on: depends_on.into_iter().map(str::to_owned).collect(),
        inputs: Vec::new(),
        evidence_refs: Vec::new(),
        payload: ActionPayload::Verify(VerifyAction {
            verify_kind: VerifyKind::EffectContract,
            verifier_hint: "verify final effect".to_owned(),
            pre_observation_ref: Some("state.pre.out".to_owned()),
            post_observation_ref: Some("state.post.out".to_owned()),
            live: None,
        }),
        implementation_hint: None,
        expected_effect_ref: Some("effect.swap".to_owned()),
    }
}

fn sample_effect_contract() -> EffectContract {
    EffectContract {
        effect_id: "effect.swap".to_owned(),
        kind: EffectContractKind::StateTransition,
        assertions: vec![EffectAssertion {
            expression: "post.decoded_u256 == \"120\"".to_owned(),
            description: "post-state output should be 120".to_owned(),
        }],
        tolerance_hint: Some("exact_output".to_owned()),
    }
}

fn sample_pre_observation(evidence_id: &str) -> EvidenceRecord {
    EvidenceRecord {
        evidence_id: evidence_id.to_owned(),
        kind: CoreEvidenceKind::ExternalObservation,
        provenance: EvidenceProvenance {
            source: "test.pre_state".to_owned(),
            chain_scope: Some("eip155:1".to_owned()),
            trace_hint: Some("verify-swap".to_owned()),
        },
        freshness: EvidenceFreshness {
            observed_at_ms: Some(1),
            expires_at_ms: None,
            max_age_ms: None,
        },
        confidence_ppm: Some(1_000_000),
        payload: json!({"decoded_u256":"90"}),
    }
}

fn sample_event(run_id: RunId, event_seq: u64) -> RunEventEnvelope {
    RunEventEnvelope {
        run_id: run_id.clone(),
        event_seq,
        checkpoint_seq: 1,
        plan_epoch: 0,
        event: RunEvent::Progress(RunProgress {
            event_id: EventId(format!("event-{event_seq}")),
            run_id,
            phase: "planning".to_owned(),
            summary: "waiting for quote".to_owned(),
        }),
    }
}

fn sample_claim(
    claim_id: &str,
    run_id: RunId,
    claim_epoch: u64,
    lease_expires_at_ms: Option<u64>,
) -> RunClaim {
    RunClaim {
        claim_id: ClaimId(claim_id.to_owned()),
        run_id,
        host_session_id: "session-1".to_owned(),
        owner_kind: RunClaimOwnerKind::InteractiveHost,
        owner_instance_id: "host-a".to_owned(),
        lease_started_at_ms: 10,
        lease_expires_at_ms,
        last_renewed_at_ms: Some(10),
        claim_epoch,
        mode: RunClaimMode::ExclusiveMutation,
        status: RunClaimStatus::Active,
    }
}
