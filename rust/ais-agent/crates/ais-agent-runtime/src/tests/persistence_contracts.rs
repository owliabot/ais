use std::collections::BTreeMap;

use ais_agent_control::{
    events::{RunEvent, RunEventEnvelope, RunProgress},
    ids::{ClaimId, EventId, RunId},
    ownership::{RunClaim, RunClaimMode, RunClaimOwnerKind, RunClaimStatus},
};
use ais_agent_core::{
    mission::{Mission, MissionBudget, MissionPolicy},
    runtime::{BoundaryKind, RunPhase, RunStatus},
};

use crate::persistence::{
    CheckpointArchive, CheckpointArchiveEntry, CheckpointArchiveError, CheckpointArchiveKind,
    ClaimExpireRequest, ClaimReleaseRequest, ClaimRenewRequest, ClaimSupersedeRequest,
    DurableMutationExecutor, EventArchive, EventArchiveError, EventArchiveQuery,
    InMemoryCheckpointRepository, InMemoryEventArchive, InMemoryMissionRepository,
    InMemoryRunCatalogRepository, InMemoryRunClaimRepository, InMemoryRuntimeAuditArchive,
    InMemorySignerStateArchive, LinearDurableMutationExecutor, MissionRepository,
    MissionRepositoryError, RunCatalogEntry, RunCatalogRepository, RunCatalogRepositoryError,
    RunClaimRepository, RunClaimRepositoryError, RuntimeAuditArchive,
};

#[test]
fn mission_repository_inserts_and_loads_by_run_id() {
    let mut repo = InMemoryMissionRepository::default();
    let run_id = RunId("run-1".to_owned());
    let mission = sample_mission();

    repo.insert(run_id.clone(), mission.clone())
        .expect("insert mission");

    let loaded = repo.load(&run_id).expect("load mission");
    assert_eq!(loaded.mission_id, mission.mission_id);
    assert_eq!(loaded.goal, mission.goal);
}

#[test]
fn mission_repository_reports_duplicate_and_not_found() {
    let mut repo = InMemoryMissionRepository::default();
    let run_id = RunId("run-1".to_owned());

    repo.insert(run_id.clone(), sample_mission())
        .expect("insert mission");

    let duplicate = repo
        .insert(run_id.clone(), sample_mission())
        .expect_err("duplicate mission should fail");
    assert_eq!(
        duplicate,
        MissionRepositoryError::AlreadyExists {
            run_id: "run-1".to_owned(),
        }
    );

    let missing = repo
        .load(&RunId("run-2".to_owned()))
        .expect_err("missing mission should fail");
    assert_eq!(
        missing,
        MissionRepositoryError::NotFound {
            run_id: "run-2".to_owned(),
        }
    );
}

#[test]
fn mission_repository_upsert_replaces_existing_mission() {
    let mut repo = InMemoryMissionRepository::default();
    let run_id = RunId("run-1".to_owned());
    let mut mission = sample_mission();

    repo.insert(run_id.clone(), mission.clone())
        .expect("insert mission");
    mission
        .constraints
        .insert("slippage_bps".to_owned(), 50.into());
    repo.upsert(run_id.clone(), mission.clone())
        .expect("upsert mission");

    let loaded = repo.load(&run_id).expect("load mission");
    assert_eq!(loaded.constraints.get("slippage_bps"), Some(&50.into()));
}

#[test]
fn run_catalog_repository_upserts_and_loads_latest_summary() {
    let mut repo = InMemoryRunCatalogRepository::default();
    let run_id = RunId("run-1".to_owned());

    repo.upsert(sample_run_catalog_entry(
        run_id.clone(),
        RunStatus::Running,
        Some(1),
    ))
    .expect("insert catalog");
    repo.upsert(sample_run_catalog_entry(
        run_id.clone(),
        RunStatus::Completed,
        Some(3),
    ))
    .expect("update catalog");

    let loaded = repo.load(&run_id).expect("load catalog");
    assert_eq!(loaded.status, RunStatus::Completed);
    assert_eq!(loaded.latest_event_seq, Some(3));
    assert_eq!(loaded.active_boundary_kind, Some(BoundaryKind::Completion));
}

#[test]
fn run_catalog_repository_reports_structured_not_found() {
    let repo = InMemoryRunCatalogRepository::default();

    let missing = repo
        .load(&RunId("run-missing".to_owned()))
        .expect_err("missing catalog should fail");
    assert_eq!(
        missing,
        RunCatalogRepositoryError::NotFound {
            run_id: "run-missing".to_owned(),
        }
    );
}

#[test]
fn run_claim_repository_acquires_renews_and_releases_active_claims() {
    let mut repo = InMemoryRunClaimRepository::default();
    let run_id = RunId("run-1".to_owned());

    let acquired = repo
        .acquire(sample_claim("claim-1", run_id.clone(), 1, Some(20)))
        .expect("acquire claim");
    assert_eq!(
        repo.load_active(&run_id).expect("load active claim"),
        Some(acquired.clone())
    );

    let renewed = repo
        .renew(ClaimRenewRequest {
            run_id: run_id.clone(),
            claim_id: ClaimId("claim-1".to_owned()),
            claim_epoch: 1,
            renewed_at_ms: 15,
            lease_expires_at_ms: Some(25),
        })
        .expect("renew claim");
    assert_eq!(renewed.claim_epoch, 2);
    assert_eq!(renewed.last_renewed_at_ms, Some(15));
    assert_eq!(renewed.lease_expires_at_ms, Some(25));

    let released = repo
        .release(ClaimReleaseRequest {
            run_id: run_id.clone(),
            claim_id: ClaimId("claim-1".to_owned()),
            claim_epoch: 2,
        })
        .expect("release claim");
    assert_eq!(released.status, RunClaimStatus::Released);
    assert!(repo.load_active(&run_id).expect("load active").is_none());
}

#[test]
fn run_claim_repository_reports_conflict_and_epoch_mismatch() {
    let mut repo = InMemoryRunClaimRepository::default();
    let run_id = RunId("run-1".to_owned());
    repo.acquire(sample_claim("claim-1", run_id.clone(), 1, Some(20)))
        .expect("acquire claim");

    let conflict = repo
        .acquire(sample_claim("claim-2", run_id.clone(), 1, Some(30)))
        .expect_err("second active claim should conflict");
    assert_eq!(
        conflict,
        RunClaimRepositoryError::ActiveClaimConflict {
            run_id: "run-1".to_owned(),
            claim_id: "claim-1".to_owned(),
        }
    );

    let stale_epoch = repo
        .renew(ClaimRenewRequest {
            run_id: run_id.clone(),
            claim_id: ClaimId("claim-1".to_owned()),
            claim_epoch: 9,
            renewed_at_ms: 15,
            lease_expires_at_ms: Some(25),
        })
        .expect_err("renew with stale epoch should fail");
    assert_eq!(
        stale_epoch,
        RunClaimRepositoryError::ClaimEpochConflict {
            claim_id: "claim-1".to_owned(),
            expected_claim_epoch: 9,
            actual_claim_epoch: 1,
        }
    );
}

#[test]
fn run_claim_repository_expires_and_supersedes_claims() {
    let mut repo = InMemoryRunClaimRepository::default();
    let run_id = RunId("run-1".to_owned());
    repo.acquire(sample_claim("claim-1", run_id.clone(), 1, Some(20)))
        .expect("acquire claim");

    let expired = repo
        .expire_stale(ClaimExpireRequest {
            run_id: run_id.clone(),
            now_ms: 25,
        })
        .expect("expire stale claim")
        .expect("claim should expire");
    assert_eq!(expired.status, RunClaimStatus::Expired);
    assert!(repo.load_active(&run_id).expect("load active").is_none());

    repo.acquire(sample_claim("claim-2", run_id.clone(), 1, Some(40)))
        .expect("acquire successor after expiry");

    let supersede = repo
        .supersede(ClaimSupersedeRequest {
            run_id: run_id.clone(),
            predecessor_claim_id: ClaimId("claim-2".to_owned()),
            predecessor_claim_epoch: 1,
            successor_claim: sample_claim("claim-3", run_id.clone(), 1, Some(60)),
        })
        .expect("supersede active claim");
    assert_eq!(supersede.predecessor.status, RunClaimStatus::Superseded);
    assert_eq!(supersede.successor.claim_id.0, "claim-3");
    assert_eq!(
        repo.load_active(&run_id)
            .expect("load active successor")
            .expect("active successor")
            .claim_id
            .0,
        "claim-3"
    );
}

#[test]
fn event_archive_supports_cursor_reads_and_limit_truncation() {
    let mut archive = InMemoryEventArchive::default();
    let run_id = RunId("run-1".to_owned());
    archive
        .append(sample_event(run_id.clone(), 1))
        .expect("append event 1");
    archive
        .append(sample_event(run_id.clone(), 2))
        .expect("append event 2");
    archive
        .append(sample_event(run_id.clone(), 3))
        .expect("append event 3");

    let slice = archive
        .read(EventArchiveQuery {
            run_id: run_id.clone(),
            after_event_seq: Some(1),
            limit: Some(1),
        })
        .expect("read event slice");

    assert_eq!(slice.latest_event_seq, Some(3));
    assert_eq!(slice.next_after_event_seq, Some(2));
    assert!(slice.truncated);
    assert_eq!(slice.events.len(), 1);
    assert_eq!(slice.events[0].event_seq, 2);
}

#[test]
fn event_archive_reports_structured_not_found() {
    let archive = InMemoryEventArchive::default();

    let missing = archive
        .read(EventArchiveQuery {
            run_id: RunId("run-missing".to_owned()),
            after_event_seq: None,
            limit: None,
        })
        .expect_err("missing archive should fail");
    assert_eq!(
        missing,
        EventArchiveError::NotFound {
            run_id: "run-missing".to_owned(),
        }
    );
}

#[test]
fn checkpoint_archive_history_keeps_archive_kinds() {
    let mut archive = InMemoryCheckpointRepository::default();
    let mut first = crate::tests::checkpoint_repository::sample_checkpoint();
    first.checkpoint_seq = 1;
    let mut second = crate::tests::checkpoint_repository::sample_checkpoint();
    second.checkpoint_seq = 2;

    archive
        .append(CheckpointArchiveEntry {
            snapshot: first,
            kind: CheckpointArchiveKind::Boundary,
        })
        .expect("append boundary");
    archive
        .append(CheckpointArchiveEntry {
            snapshot: second,
            kind: CheckpointArchiveKind::SideEffect,
        })
        .expect("append side-effect");

    let history = archive.history("run-1").expect("history");
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].kind, CheckpointArchiveKind::Boundary);
    assert_eq!(history[1].kind, CheckpointArchiveKind::SideEffect);

    let missing = archive
        .history("run-missing")
        .expect_err("missing history should fail");
    assert_eq!(
        missing,
        CheckpointArchiveError::NotFound {
            run_id: "run-missing".to_owned(),
        }
    );
}

#[test]
fn linear_durable_mutation_executor_commits_consistent_grouped_unit() {
    let mut executor = LinearDurableMutationExecutor::new(
        InMemoryMissionRepository::default(),
        InMemoryCheckpointRepository::default(),
        InMemoryEventArchive::default(),
        InMemoryRunCatalogRepository::default(),
        InMemorySignerStateArchive::default(),
        InMemoryRuntimeAuditArchive::default(),
    );

    let unit = crate::tests::durable_mutation::sample_unit();
    let run_id = unit.run_id.clone();

    let receipt = executor.commit(unit).expect("commit grouped mutation unit");
    assert_eq!(receipt.run_id, run_id.clone());
    assert_eq!(receipt.latest_event_seq, Some(1));
    assert_eq!(receipt.latest_audit_seq, Some(1));

    let (
        mission_repo,
        checkpoint_repo,
        event_archive,
        run_catalog_repo,
        _signer_archive,
        audit_archive,
    ) = executor.into_parts();

    assert_eq!(
        mission_repo.load(&run_id).expect("load mission").mission_id,
        "mission-1"
    );
    assert_eq!(
        checkpoint_repo
            .latest(&run_id.0)
            .expect("latest checkpoint")
            .checkpoint_seq,
        1
    );
    assert_eq!(
        event_archive
            .read(EventArchiveQuery {
                run_id: run_id.clone(),
                after_event_seq: None,
                limit: None,
            })
            .expect("read event archive")
            .latest_event_seq,
        Some(1)
    );
    assert_eq!(
        run_catalog_repo
            .load(&run_id)
            .expect("load catalog")
            .latest_event_seq,
        Some(1)
    );
    assert_eq!(
        audit_archive
            .read(crate::persistence::RuntimeAuditQuery {
                run_id: run_id.clone(),
                after_audit_seq: None,
                limit: None,
            })
            .expect("read audit archive")
            .latest_audit_seq,
        Some(1)
    );
}

#[test]
fn linear_durable_mutation_executor_rejects_invalid_unit_before_writing() {
    let mut executor = LinearDurableMutationExecutor::new(
        InMemoryMissionRepository::default(),
        InMemoryCheckpointRepository::default(),
        InMemoryEventArchive::default(),
        InMemoryRunCatalogRepository::default(),
        InMemorySignerStateArchive::default(),
        InMemoryRuntimeAuditArchive::default(),
    );

    let mut unit = crate::tests::durable_mutation::sample_unit();
    let run_id = unit.run_id.clone();
    unit.catalog_write.entry.latest_checkpoint_seq = 99;

    let error = executor
        .commit(unit)
        .expect_err("invalid grouped unit must fail");
    assert!(matches!(
        error,
        crate::persistence::DurableCommitError::InvalidUnit(
            crate::persistence::DurableMutationContractError::CatalogCheckpointMismatch { .. }
        )
    ));

    let (
        mission_repo,
        checkpoint_repo,
        event_archive,
        run_catalog_repo,
        _signer_archive,
        audit_archive,
    ) = executor.into_parts();

    assert!(matches!(
        mission_repo.load(&run_id),
        Err(MissionRepositoryError::NotFound { .. })
    ));
    assert!(matches!(
        checkpoint_repo.latest(&run_id.0),
        Err(CheckpointArchiveError::NotFound { .. })
    ));
    assert!(matches!(
        event_archive.read(EventArchiveQuery {
            run_id: run_id.clone(),
            after_event_seq: None,
            limit: None,
        }),
        Err(EventArchiveError::NotFound { .. })
    ));
    assert!(matches!(
        run_catalog_repo.load(&run_id),
        Err(RunCatalogRepositoryError::NotFound { .. })
    ));
    assert!(matches!(
        audit_archive.read(crate::persistence::RuntimeAuditQuery {
            run_id,
            after_audit_seq: None,
            limit: None,
        }),
        Err(crate::persistence::RuntimeAuditArchiveError::NotFound { .. })
    ));
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

fn sample_run_catalog_entry(
    run_id: RunId,
    status: RunStatus,
    latest_event_seq: Option<u64>,
) -> RunCatalogEntry {
    RunCatalogEntry {
        run_id,
        mission_id: "mission-1".to_owned(),
        status: status.clone(),
        phase: if status == RunStatus::Completed {
            RunPhase::Finalized
        } else {
            RunPhase::Planning
        },
        active_boundary_kind: if status == RunStatus::Completed {
            Some(BoundaryKind::Completion)
        } else {
            None
        },
        latest_checkpoint_seq: latest_event_seq.unwrap_or(0),
        latest_event_seq,
        latest_revision: latest_event_seq.unwrap_or(0),
        created_at_ms: Some(1_000),
        updated_at_ms: Some(2_000),
        terminal_at_ms: if status == RunStatus::Completed {
            Some(3_000)
        } else {
            None
        },
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

fn sample_event(run_id: RunId, event_seq: u64) -> RunEventEnvelope {
    RunEventEnvelope {
        run_id: run_id.clone(),
        event_seq,
        checkpoint_seq: event_seq,
        plan_epoch: 0,
        event: RunEvent::Progress(RunProgress {
            event_id: EventId(format!("event-{event_seq}")),
            run_id,
            phase: "planning".to_owned(),
            summary: format!("step {event_seq}"),
        }),
    }
}
