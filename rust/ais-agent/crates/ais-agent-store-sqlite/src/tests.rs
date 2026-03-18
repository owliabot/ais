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
        SignerResolutionKind, SignerResolutionSubmission, SubmitEvidenceCommand,
        SubmitSignerResolutionCommand,
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
    runtime::{BoundaryKind, RunLifecycleState, RunPhase, RunStatus, SignerRequestState},
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
    RuntimeAuditQuery, SignerStateStore, SignerStateStoreError, SignerStateWrite,
};
use ais_agent_runtime::{
    runtime::{InMemoryRunRepository, RunRepository},
    service::RuntimeHostService,
};
use serde_json::json;

use crate::{
    migrate_connection, MaintenanceJournalAppend, MaintenanceOperationKind,
    MaintenanceOperationStatus, RunStoreError, SqliteStore, StoreMaintenanceState,
    StorePruneRequest, StorePurgeRequest, StorePurgeTable, StorePurgeTarget, StoreVacuumRequest,
    StoredRunAudit, StoredRunAuditQuery, StoredRunCheckpoint, StoredRunClaim, StoredRunEvent,
    StoredRunEventQuery, StoredRunHead, StoredRunInput, StoredRunWaitState, SCHEMA_VERSION,
    STORE_METADATA_SCHEMA_VERSION, STORE_RETENTION_SCHEMA_VERSION,
};

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
fn sqlite_store_bootstraps_maintenance_metadata_tables() {
    let store = SqliteStore::open_in_memory().expect("open sqlite store");

    let maintenance_journal_exists: i64 = store
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'maintenance_journal'",
            [],
            |row| row.get(0),
        )
        .expect("maintenance_journal exists");
    let maintenance_state_exists: i64 = store
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'store_maintenance_state'",
            [],
            |row| row.get(0),
        )
        .expect("store_maintenance_state exists");

    assert_eq!(maintenance_journal_exists, 1);
    assert_eq!(maintenance_state_exists, 1);
}

#[test]
fn sqlite_store_bootstraps_final_tables() {
    let store = SqliteStore::open_in_memory().expect("open sqlite store");

    for table_name in [
        "runs",
        "run_inputs",
        "run_events",
        "run_audits",
        "run_checkpoints",
        "run_wait_states",
        "run_claim_history",
    ] {
        let exists: i64 = store
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                [table_name],
                |row| row.get(0),
            )
            .unwrap_or_else(|error| panic!("query sqlite_master for {table_name}: {error}"));
        assert_eq!(exists, 1, "expected bootstrap for table {table_name}");
    }
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
fn sqlite_store_open_stamps_global_metadata_snapshot() {
    let sqlite_path = sqlite_test_path("open-metadata");
    let store = SqliteStore::open_path(&sqlite_path).expect("open sqlite store");

    let state = store
        .load_store_maintenance_state()
        .expect("load maintenance state")
        .expect("state row");
    assert!(state.last_store_opened_at_ms.is_some());
    assert!(state.last_known_page_count.is_some());
    assert!(state.last_known_freelist_count.is_some());
    assert!(state.last_known_db_bytes.is_some());
    assert!(state.last_growth_sampled_at_ms.is_some());
    assert_eq!(state.metadata_schema_version, STORE_METADATA_SCHEMA_VERSION);
    assert_eq!(
        state.schema_retention_version,
        STORE_RETENTION_SCHEMA_VERSION
    );

    let _ = std::fs::remove_file(sqlite_path);
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

    let run_input = store
        .load_run_input("run-1")
        .expect("load stored run input");
    assert_eq!(run_input.run_id, "run-1");
    assert_eq!(run_input.mission["mission_id"], "mission-1");

    let run_head = store.load_run_head("run-1").expect("load stored run head");
    assert_eq!(run_head.status, "awaiting_evidence");
    assert_eq!(run_head.phase.as_deref(), Some("awaiting_host"));
    assert_eq!(run_head.active_boundary_kind.as_deref(), Some("evidence"));
    assert_eq!(run_head.active_wait_kind.as_deref(), Some("evidence"));
    assert_eq!(run_head.latest_checkpoint_seq, Some(1));
    assert_eq!(run_head.latest_event_seq, Some(1));

    let stored_checkpoint = store
        .load_latest_run_checkpoint("run-1")
        .expect("load stored checkpoint");
    assert_eq!(stored_checkpoint.checkpoint_seq, 1);
    assert_eq!(stored_checkpoint.checkpoint_kind, "boundary");

    let stored_events = store
        .read_run_events(StoredRunEventQuery {
            run_id: "run-1".to_owned(),
            after_event_seq: None,
            limit: Some(10),
        })
        .expect("read stored events");
    assert_eq!(stored_events.latest_event_seq, Some(1));
    assert_eq!(stored_events.records.len(), 1);
    assert!(stored_events.records[0].emitted_at_ms > 0);
    assert_eq!(stored_events.records[0].revision, Some(1));
    assert_eq!(
        stored_events.records[0].event_kind,
        "run.transition.progress"
    );
}

#[test]
fn sqlite_run_catalog_loads_from_runs_table() {
    let mut store = SqliteStore::open_in_memory().expect("open sqlite store");
    let run_id = RunId("run-catalog-store".to_owned());
    let entry = RunCatalogEntry {
        run_id: run_id.clone(),
        mission_id: "mission-1".to_owned(),
        status: RunStatus::AwaitingConfirmation,
        phase: RunPhase::Broadcasting,
        active_boundary_kind: Some(BoundaryKind::Confirmation),
        latest_checkpoint_seq: 7,
        latest_event_seq: Some(6),
        latest_revision: 7,
        created_at_ms: Some(100),
        updated_at_ms: Some(200),
        terminal_at_ms: None,
    };

    RunCatalogRepository::upsert(&mut store, entry.clone()).expect("upsert catalog");

    let loaded = RunCatalogRepository::load(&store, &run_id).expect("load catalog from runs");
    assert_eq!(loaded.run_id, entry.run_id);
    assert_eq!(loaded.mission_id, entry.mission_id);
    assert_eq!(loaded.status, entry.status);
    assert_eq!(loaded.phase, entry.phase);
    assert_eq!(loaded.active_boundary_kind, entry.active_boundary_kind);
    assert_eq!(loaded.latest_checkpoint_seq, entry.latest_checkpoint_seq);
    assert_eq!(loaded.latest_event_seq, entry.latest_event_seq);
    assert_eq!(loaded.latest_revision, entry.latest_checkpoint_seq);
}

#[test]
fn sqlite_run_catalog_updates_retention_mode_when_run_becomes_terminal() {
    let mut store = SqliteStore::open_in_memory().expect("open sqlite store");
    let run_id = RunId("run-catalog-terminal".to_owned());

    RunCatalogRepository::upsert(
        &mut store,
        RunCatalogEntry {
            run_id: run_id.clone(),
            mission_id: "mission-1".to_owned(),
            status: RunStatus::Running,
            phase: RunPhase::Planning,
            active_boundary_kind: None,
            latest_checkpoint_seq: 1,
            latest_event_seq: Some(1),
            latest_revision: 1,
            created_at_ms: Some(100),
            updated_at_ms: Some(100),
            terminal_at_ms: None,
        },
    )
    .expect("upsert active catalog");

    let active = store
        .load_run_head(&run_id.0)
        .expect("load active run head");
    assert_eq!(active.retention_mode.as_deref(), Some("active_full"));
    assert_eq!(active.terminal_at_ms, None);

    RunCatalogRepository::upsert(
        &mut store,
        RunCatalogEntry {
            run_id: run_id.clone(),
            mission_id: "mission-1".to_owned(),
            status: RunStatus::Completed,
            phase: RunPhase::Finalized,
            active_boundary_kind: Some(BoundaryKind::Completion),
            latest_checkpoint_seq: 2,
            latest_event_seq: Some(2),
            latest_revision: 2,
            created_at_ms: Some(100),
            updated_at_ms: Some(200),
            terminal_at_ms: Some(200),
        },
    )
    .expect("upsert terminal catalog");

    let terminal = store
        .load_run_head(&run_id.0)
        .expect("load terminal run head");
    assert_eq!(terminal.retention_mode.as_deref(), Some("terminal_tiered"));
    assert_eq!(terminal.terminal_at_ms, Some(200));
}

#[test]
fn sqlite_store_round_trips_pending_signer_state() {
    let mut store = SqliteStore::open_in_memory().expect("open sqlite store");
    let signer_state = sample_signer_state();

    SignerStateStore::upsert(&mut store, signer_state.clone()).expect("upsert signer state");

    let loaded = SignerStateStore::load(&store, &signer_state.run_id).expect("load signer state");
    assert_eq!(loaded, signer_state);

    let stored_request_id: String = store
        .connection()
        .query_row(
            "SELECT request_id FROM run_wait_states WHERE run_id = ?1",
            [&loaded.run_id.0],
            |row| row.get(0),
        )
        .expect("load signer request id");
    assert_eq!(stored_request_id, loaded.request_id.0);

    let wait_state = store
        .load_run_wait_state(&loaded.run_id.0)
        .expect("load run wait state");
    assert_eq!(wait_state.wait_kind, "signer");
    assert_eq!(wait_state.request_id, loaded.request_id.0);
}

#[test]
fn sqlite_store_round_trips_maintenance_journal_and_state() {
    let mut store = SqliteStore::open_in_memory().expect("open sqlite store");

    let entry = store
        .append_maintenance_journal(MaintenanceJournalAppend {
            operation_kind: MaintenanceOperationKind::Prune,
            started_at_ms: 1_000,
            finished_at_ms: Some(1_200),
            status: MaintenanceOperationStatus::Succeeded,
            summary: json!({
                "deleted_checkpoints": 12,
                "affected_runs": 3,
            }),
        })
        .expect("append maintenance journal");

    let state = StoreMaintenanceState {
        last_operation_kind: Some(MaintenanceOperationKind::Prune),
        last_operation_status: Some(MaintenanceOperationStatus::Succeeded),
        last_store_opened_at_ms: Some(900),
        last_prune_started_at_ms: Some(1_000),
        last_prune_finished_at_ms: Some(1_200),
        last_pruned_terminal_before_ms: Some(900),
        last_prune_deleted_rows: Some(12),
        last_purge_deleted_rows: Some(0),
        last_vacuum_started_at_ms: Some(1_250),
        last_vacuum_finished_at_ms: Some(1_300),
        last_vacuum_at_ms: Some(1_300),
        last_wal_checkpoint_at_ms: Some(1_250),
        last_known_page_count: Some(64),
        last_known_freelist_count: Some(2),
        last_known_db_bytes: Some(262_144),
        last_growth_sampled_at_ms: Some(1_300),
        schema_retention_version: 1,
        metadata_schema_version: STORE_METADATA_SCHEMA_VERSION,
    };
    store
        .upsert_store_maintenance_state(&state)
        .expect("upsert maintenance state");

    let journal = store
        .list_maintenance_journal(10)
        .expect("list maintenance journal");
    let loaded_state = store
        .load_store_maintenance_state()
        .expect("load maintenance state")
        .expect("maintenance state row");

    assert_eq!(journal.len(), 1);
    assert_eq!(journal[0], entry);
    assert_eq!(journal[0].summary["deleted_checkpoints"], 12);
    assert_eq!(loaded_state, state);
}

#[test]
fn sqlite_run_catalog_retiers_terminal_checkpoints() {
    let mut store = SqliteStore::open_in_memory().expect("open sqlite store");
    let run_id = RunId("run-retier".to_owned());

    for checkpoint in [
        StoredRunCheckpoint {
            checkpoint_id: None,
            run_id: run_id.0.clone(),
            checkpoint_seq: 1,
            plan_epoch: 0,
            checkpoint_kind: "boundary".to_owned(),
            retention_tier: "active_full".to_owned(),
            created_at_ms: 100,
            is_terminal: false,
            is_side_effect_boundary: false,
            is_recovery_boundary: false,
            is_first_wait_checkpoint: false,
            snapshot: json!({"checkpoint_seq": 1}),
        },
        StoredRunCheckpoint {
            checkpoint_id: None,
            run_id: run_id.0.clone(),
            checkpoint_seq: 2,
            plan_epoch: 0,
            checkpoint_kind: "side_effect".to_owned(),
            retention_tier: "terminal_boundary".to_owned(),
            created_at_ms: 200,
            is_terminal: false,
            is_side_effect_boundary: true,
            is_recovery_boundary: false,
            is_first_wait_checkpoint: false,
            snapshot: json!({"checkpoint_seq": 2}),
        },
        StoredRunCheckpoint {
            checkpoint_id: None,
            run_id: run_id.0.clone(),
            checkpoint_seq: 3,
            plan_epoch: 0,
            checkpoint_kind: "boundary".to_owned(),
            retention_tier: "terminal_intermediate".to_owned(),
            created_at_ms: 300,
            is_terminal: true,
            is_side_effect_boundary: false,
            is_recovery_boundary: false,
            is_first_wait_checkpoint: false,
            snapshot: json!({"checkpoint_seq": 3}),
        },
    ] {
        store
            .append_run_checkpoint_record(&checkpoint)
            .expect("append checkpoint");
    }

    RunCatalogRepository::upsert(
        &mut store,
        RunCatalogEntry {
            run_id: run_id.clone(),
            mission_id: "mission-1".to_owned(),
            status: RunStatus::Completed,
            phase: RunPhase::Finalized,
            active_boundary_kind: Some(BoundaryKind::Completion),
            latest_checkpoint_seq: 3,
            latest_event_seq: Some(3),
            latest_revision: 3,
            created_at_ms: Some(100),
            updated_at_ms: Some(300),
            terminal_at_ms: Some(300),
        },
    )
    .expect("upsert terminal catalog");

    let tiers: Vec<(i64, String)> = {
        let mut stmt = store
            .connection()
            .prepare(
                "SELECT checkpoint_seq, retention_tier FROM run_checkpoints WHERE run_id = ?1 ORDER BY checkpoint_seq ASC",
            )
            .expect("prepare");
        let rows = stmt
            .query_map([run_id.0.as_str()], |row| Ok((row.get(0)?, row.get(1)?)))
            .expect("query");
        rows.collect::<Result<Vec<_>, _>>().expect("collect")
    };

    assert_eq!(
        tiers,
        vec![
            (1, "terminal_intermediate".to_owned()),
            (2, "terminal_boundary".to_owned()),
            (3, "terminal_final".to_owned()),
        ]
    );
}

#[test]
fn sqlite_store_prune_is_idempotent_and_keeps_active_waits() {
    let mut store = SqliteStore::open_in_memory().expect("open sqlite store");

    for head in [
        StoredRunHead {
            run_id: "run-terminal".to_owned(),
            mission_id: "mission-terminal".to_owned(),
            status: "completed".to_owned(),
            phase: Some("completed".to_owned()),
            active_boundary_kind: None,
            active_wait_kind: None,
            latest_checkpoint_seq: Some(3),
            latest_event_seq: None,
            latest_audit_seq: None,
            latest_claim_epoch: None,
            retention_mode: Some("terminal_tiered".to_owned()),
            created_at_ms: Some(100),
            updated_at_ms: Some(900),
            terminal_at_ms: Some(900),
        },
        StoredRunHead {
            run_id: "run-active".to_owned(),
            mission_id: "mission-active".to_owned(),
            status: "awaiting_signer".to_owned(),
            phase: Some("awaiting_host".to_owned()),
            active_boundary_kind: Some("signer".to_owned()),
            active_wait_kind: Some("signer".to_owned()),
            latest_checkpoint_seq: Some(1),
            latest_event_seq: None,
            latest_audit_seq: None,
            latest_claim_epoch: None,
            retention_mode: Some("active_full".to_owned()),
            created_at_ms: Some(1_500),
            updated_at_ms: Some(1_500),
            terminal_at_ms: None,
        },
        StoredRunHead {
            run_id: "run-stale".to_owned(),
            mission_id: "mission-stale".to_owned(),
            status: "awaiting_host".to_owned(),
            phase: Some("awaiting_host".to_owned()),
            active_boundary_kind: None,
            active_wait_kind: None,
            latest_checkpoint_seq: None,
            latest_event_seq: None,
            latest_audit_seq: None,
            latest_claim_epoch: None,
            retention_mode: Some("active_full".to_owned()),
            created_at_ms: Some(400),
            updated_at_ms: Some(800),
            terminal_at_ms: None,
        },
    ] {
        store.upsert_run_head(&head).expect("upsert head");
    }

    for checkpoint in [
        StoredRunCheckpoint {
            checkpoint_id: None,
            run_id: "run-terminal".to_owned(),
            checkpoint_seq: 1,
            plan_epoch: 0,
            checkpoint_kind: "boundary".to_owned(),
            retention_tier: "terminal_intermediate".to_owned(),
            created_at_ms: 500,
            is_terminal: false,
            is_side_effect_boundary: false,
            is_recovery_boundary: false,
            is_first_wait_checkpoint: false,
            snapshot: json!({"checkpoint_seq": 1}),
        },
        StoredRunCheckpoint {
            checkpoint_id: None,
            run_id: "run-terminal".to_owned(),
            checkpoint_seq: 2,
            plan_epoch: 0,
            checkpoint_kind: "boundary".to_owned(),
            retention_tier: "terminal_boundary".to_owned(),
            created_at_ms: 700,
            is_terminal: false,
            is_side_effect_boundary: true,
            is_recovery_boundary: false,
            is_first_wait_checkpoint: false,
            snapshot: json!({"checkpoint_seq": 2}),
        },
        StoredRunCheckpoint {
            checkpoint_id: None,
            run_id: "run-terminal".to_owned(),
            checkpoint_seq: 3,
            plan_epoch: 0,
            checkpoint_kind: "boundary".to_owned(),
            retention_tier: "terminal_final".to_owned(),
            created_at_ms: 900,
            is_terminal: true,
            is_side_effect_boundary: false,
            is_recovery_boundary: false,
            is_first_wait_checkpoint: false,
            snapshot: json!({"checkpoint_seq": 3}),
        },
    ] {
        store
            .append_run_checkpoint_record(&checkpoint)
            .expect("append checkpoint");
    }

    for wait_state in [
        StoredRunWaitState {
            run_id: "run-terminal".to_owned(),
            wait_kind: "signer".to_owned(),
            request_id: "req-terminal".to_owned(),
            entered_at_ms: 800,
            expires_at_ms: None,
            state: json!({"status":"stale-terminal"}),
        },
        StoredRunWaitState {
            run_id: "run-active".to_owned(),
            wait_kind: "signer".to_owned(),
            request_id: "req-active".to_owned(),
            entered_at_ms: 1_500,
            expires_at_ms: None,
            state: json!({"status":"active"}),
        },
        StoredRunWaitState {
            run_id: "run-stale".to_owned(),
            wait_kind: "signer".to_owned(),
            request_id: "req-stale".to_owned(),
            entered_at_ms: 700,
            expires_at_ms: None,
            state: json!({"status":"orphan"}),
        },
        StoredRunWaitState {
            run_id: "run-missing".to_owned(),
            wait_kind: "signer".to_owned(),
            request_id: "req-missing".to_owned(),
            entered_at_ms: 600,
            expires_at_ms: None,
            state: json!({"status":"missing"}),
        },
    ] {
        store
            .upsert_run_wait_state(&wait_state)
            .expect("upsert wait");
    }

    let request = StorePruneRequest {
        started_at_ms: 2_000,
        finished_at_ms: 2_001,
        terminal_before_ms: 1_000,
        wait_state_orphan_before_ms: 1_000,
        vacuum_freelist_threshold_pages: 10_000,
        schema_retention_version: STORE_RETENTION_SCHEMA_VERSION,
    };
    let first = store.prune_retention(&request).expect("first prune");
    let second = store.prune_retention(&request).expect("second prune");

    assert_eq!(first.deleted_checkpoints, 1);
    assert_eq!(first.deleted_wait_states, 3);
    assert_eq!(second.deleted_checkpoints, 0);
    assert_eq!(second.deleted_wait_states, 0);
    assert!(!first.vacuum.as_ref().expect("vacuum result").executed);
    assert!(!second.vacuum.as_ref().expect("vacuum result").executed);

    let remaining_tiers: Vec<String> = {
        let mut stmt = store
            .connection()
            .prepare(
                "SELECT retention_tier FROM run_checkpoints WHERE run_id = 'run-terminal' ORDER BY checkpoint_seq ASC",
            )
            .expect("prepare");
        let rows = stmt.query_map([], |row| row.get(0)).expect("query");
        rows.collect::<Result<Vec<_>, _>>().expect("collect")
    };
    assert_eq!(
        remaining_tiers,
        vec!["terminal_boundary".to_owned(), "terminal_final".to_owned()]
    );
    assert_eq!(
        store
            .load_run_wait_state("run-active")
            .expect("active wait should remain")
            .request_id,
        "req-active"
    );
    assert!(matches!(
        store.load_run_wait_state("run-terminal"),
        Err(RunStoreError::NotFound { .. })
    ));

    let latest_journal = store.list_maintenance_journal(1).expect("journal");
    assert_eq!(
        latest_journal[0].operation_kind,
        MaintenanceOperationKind::Prune
    );
    assert_eq!(latest_journal[0].summary["deleted_checkpoints"], 0);
    assert!(latest_journal[0].summary["storage_before"]["page_count"].is_i64());
    assert!(latest_journal[0].summary["storage_after"]["freelist_count"].is_i64());
    assert!(latest_journal[0].summary["storage_delta"]["freelist_count"].is_i64());
    let state = store
        .load_store_maintenance_state()
        .expect("load maintenance state")
        .expect("state row");
    assert_eq!(
        state.last_operation_kind,
        Some(MaintenanceOperationKind::Prune)
    );
    assert_eq!(state.last_pruned_terminal_before_ms, Some(1_000));
    assert_eq!(state.last_prune_deleted_rows, Some(0));
    assert!(state.last_known_page_count.is_some());
    assert!(state.last_known_db_bytes.is_some());
    assert_eq!(state.metadata_schema_version, STORE_METADATA_SCHEMA_VERSION);
}

#[test]
fn sqlite_store_purge_deletes_target_scope_and_records_maintenance() {
    let mut store = SqliteStore::open_in_memory().expect("open sqlite store");

    for head in [
        StoredRunHead {
            run_id: "run-purge-target".to_owned(),
            mission_id: "mission-target".to_owned(),
            status: "completed".to_owned(),
            phase: Some("completed".to_owned()),
            active_boundary_kind: None,
            active_wait_kind: None,
            latest_checkpoint_seq: Some(1),
            latest_event_seq: Some(1),
            latest_audit_seq: Some(1),
            latest_claim_epoch: Some(1),
            retention_mode: Some("terminal_tiered".to_owned()),
            created_at_ms: Some(100),
            updated_at_ms: Some(100),
            terminal_at_ms: Some(100),
        },
        StoredRunHead {
            run_id: "run-purge-keep".to_owned(),
            mission_id: "mission-keep".to_owned(),
            status: "completed".to_owned(),
            phase: Some("completed".to_owned()),
            active_boundary_kind: None,
            active_wait_kind: None,
            latest_checkpoint_seq: Some(1),
            latest_event_seq: Some(1),
            latest_audit_seq: Some(1),
            latest_claim_epoch: Some(1),
            retention_mode: Some("terminal_tiered".to_owned()),
            created_at_ms: Some(200),
            updated_at_ms: Some(200),
            terminal_at_ms: Some(200),
        },
    ] {
        store.upsert_run_head(&head).expect("upsert head");
        store
            .upsert_run_input(&StoredRunInput {
                run_id: head.run_id.clone(),
                mission: json!({"mission_id": head.mission_id}),
                launch_input: None,
                created_at_ms: head.created_at_ms,
            })
            .expect("upsert input");
        store
            .append_run_event_record(&StoredRunEvent {
                run_id: head.run_id.clone(),
                event_seq: 1,
                event_kind: "run.completed".to_owned(),
                phase: Some("completed".to_owned()),
                boundary_kind: Some("completion".to_owned()),
                emitted_at_ms: 100,
                checkpoint_seq: Some(1),
                revision: Some(1),
                payload: json!({"run_id": head.run_id}),
            })
            .expect("append event");
        store
            .append_run_audit_record(&StoredRunAudit {
                run_id: head.run_id.clone(),
                audit_seq: 1,
                audit_kind: "durable_commit".to_owned(),
                decision_class: None,
                emitted_at_ms: 100,
                checkpoint_seq: Some(1),
                revision: Some(1),
                payload: json!({"run_id": head.run_id}),
            })
            .expect("append audit");
        store
            .append_run_checkpoint_record(&StoredRunCheckpoint {
                checkpoint_id: None,
                run_id: head.run_id.clone(),
                checkpoint_seq: 1,
                plan_epoch: 0,
                checkpoint_kind: "boundary".to_owned(),
                retention_tier: "terminal_final".to_owned(),
                created_at_ms: 100,
                is_terminal: true,
                is_side_effect_boundary: false,
                is_recovery_boundary: false,
                is_first_wait_checkpoint: false,
                snapshot: json!({"run_id": head.run_id}),
            })
            .expect("append checkpoint");
        store
            .upsert_run_wait_state(&StoredRunWaitState {
                run_id: head.run_id.clone(),
                wait_kind: "signer".to_owned(),
                request_id: format!("req-{}", head.run_id),
                entered_at_ms: 100,
                expires_at_ms: None,
                state: json!({"run_id": head.run_id}),
            })
            .expect("upsert wait");
        store
            .upsert_run_claim_record(&StoredRunClaim {
                claim_id: format!("claim-{}", head.run_id),
                run_id: head.run_id.clone(),
                host_session_id: "session-1".to_owned(),
                owner_kind: "host_session".to_owned(),
                owner_instance_id: "ais-agent".to_owned(),
                lease_started_at_ms: 100,
                lease_expires_at_ms: Some(200),
                last_renewed_at_ms: Some(150),
                claim_epoch: 1,
                mode: "exclusive".to_owned(),
                status: "released".to_owned(),
            })
            .expect("upsert claim");
    }

    let purge = store
        .purge_retention(&StorePurgeRequest {
            started_at_ms: 1_000,
            finished_at_ms: 1_001,
            target: StorePurgeTarget::RunId {
                run_id: "run-purge-target".to_owned(),
            },
            vacuum_freelist_threshold_pages: 10_000,
            schema_retention_version: STORE_RETENTION_SCHEMA_VERSION,
        })
        .expect("purge run");
    assert_eq!(purge.deleted_runs, 1);
    assert_eq!(purge.deleted_events, 1);
    assert_eq!(purge.deleted_checkpoints, 1);

    assert!(matches!(
        store.load_run_head("run-purge-target"),
        Err(RunStoreError::NotFound { .. })
    ));
    assert_eq!(
        store
            .load_run_head("run-purge-keep")
            .expect("keep run should remain")
            .run_id,
        "run-purge-keep"
    );

    let table_purge = store
        .purge_retention(&StorePurgeRequest {
            started_at_ms: 2_000,
            finished_at_ms: 2_001,
            target: StorePurgeTarget::Table {
                table: StorePurgeTable::RunEvents,
            },
            vacuum_freelist_threshold_pages: 10_000,
            schema_retention_version: STORE_RETENTION_SCHEMA_VERSION,
        })
        .expect("purge table");
    assert_eq!(table_purge.deleted_table_rows, 1);
    assert!(!table_purge.vacuum.as_ref().expect("vacuum result").executed);
    let remaining_events: i64 = store
        .connection()
        .query_row("SELECT COUNT(*) FROM run_events", [], |row| row.get(0))
        .expect("count events");
    assert_eq!(remaining_events, 0);

    let latest_journal = store.list_maintenance_journal(1).expect("journal");
    assert_eq!(
        latest_journal[0].operation_kind,
        MaintenanceOperationKind::Purge
    );
    assert_eq!(latest_journal[0].summary["deleted_table_rows"], 1);
    assert!(latest_journal[0].summary["storage_before"]["page_count"].is_i64());
    assert!(latest_journal[0].summary["storage_after"]["freelist_count"].is_i64());
    let state = store
        .load_store_maintenance_state()
        .expect("load maintenance state")
        .expect("state row");
    assert_eq!(
        state.last_operation_kind,
        Some(MaintenanceOperationKind::Purge)
    );
    assert_eq!(state.last_purge_deleted_rows, Some(1));
    assert!(state.last_known_page_count.is_some());
    assert_eq!(state.metadata_schema_version, STORE_METADATA_SCHEMA_VERSION);

    let purge_journal = store
        .purge_retention(&StorePurgeRequest {
            started_at_ms: 3_000,
            finished_at_ms: 3_001,
            target: StorePurgeTarget::Table {
                table: StorePurgeTable::MaintenanceJournal,
            },
            vacuum_freelist_threshold_pages: 10_000,
            schema_retention_version: STORE_RETENTION_SCHEMA_VERSION,
        })
        .expect("purge maintenance journal");
    assert!(purge_journal.deleted_table_rows >= 1);
    assert!(purge_journal.vacuum.is_none());
    let remaining_maintenance_journal: i64 = store
        .connection()
        .query_row("SELECT COUNT(*) FROM maintenance_journal", [], |row| {
            row.get(0)
        })
        .expect("count maintenance journal");
    assert_eq!(remaining_maintenance_journal, 0);
    let state_after_journal_purge = store
        .load_store_maintenance_state()
        .expect("load maintenance state after journal purge")
        .expect("state row after journal purge");
    assert_eq!(
        state_after_journal_purge.last_operation_kind,
        Some(MaintenanceOperationKind::Purge)
    );

    let purge_state = store
        .purge_retention(&StorePurgeRequest {
            started_at_ms: 4_000,
            finished_at_ms: 4_001,
            target: StorePurgeTarget::Table {
                table: StorePurgeTable::StoreMaintenanceState,
            },
            vacuum_freelist_threshold_pages: 10_000,
            schema_retention_version: STORE_RETENTION_SCHEMA_VERSION,
        })
        .expect("purge maintenance state");
    assert_eq!(purge_state.deleted_table_rows, 1);
    assert!(purge_state.vacuum.is_none());
    assert!(
        store
            .load_store_maintenance_state()
            .expect("load state")
            .is_none(),
        "store maintenance state should stay empty after explicit purge"
    );
}

#[test]
fn sqlite_store_vacuum_executes_when_freelist_threshold_is_met() {
    let sqlite_path = sqlite_test_path("vacuum-threshold");
    let mut store = SqliteStore::open_path(&sqlite_path).expect("open sqlite store");

    for seq in 0..32 {
        store
            .append_run_event_record(&StoredRunEvent {
                run_id: "run-vacuum".to_owned(),
                event_seq: seq + 1,
                event_kind: "run.progress".to_owned(),
                phase: Some("awaiting_host".to_owned()),
                boundary_kind: Some("evidence".to_owned()),
                emitted_at_ms: 100 + seq,
                checkpoint_seq: Some(1),
                revision: Some(1),
                payload: json!({
                    "blob": "x".repeat(8_192),
                    "seq": seq,
                }),
            })
            .expect("append event");
    }
    let deleted = store
        .connection()
        .execute("DELETE FROM run_events WHERE run_id = 'run-vacuum'", [])
        .expect("delete events");
    assert!(deleted > 0);

    let result = store
        .maybe_vacuum_retention(&StoreVacuumRequest {
            started_at_ms: 3_000,
            finished_at_ms: 3_001,
            freelist_threshold_pages: 1,
            force: false,
            schema_retention_version: STORE_RETENTION_SCHEMA_VERSION,
        })
        .expect("vacuum");

    assert!(result.executed);
    assert!(result.freelist_pages_before >= 1);
    assert_eq!(result.freelist_pages_after, 0);
    let state = store
        .load_store_maintenance_state()
        .expect("load maintenance state")
        .expect("state row");
    assert_eq!(
        state.last_operation_kind,
        Some(MaintenanceOperationKind::Vacuum)
    );
    assert_eq!(state.last_vacuum_started_at_ms, Some(3_000));
    assert_eq!(state.last_vacuum_finished_at_ms, Some(3_001));
    assert_eq!(state.last_vacuum_at_ms, Some(3_001));
    assert_eq!(state.last_wal_checkpoint_at_ms, Some(3_000));
    assert_eq!(state.last_known_freelist_count, Some(0));
    assert!(state.last_known_db_bytes.is_some());

    let journal = store.list_maintenance_journal(1).expect("load journal");
    assert_eq!(journal[0].operation_kind, MaintenanceOperationKind::Vacuum);
    assert_eq!(journal[0].summary["executed"], true);
    assert_eq!(
        journal[0].summary["storage_before"]["page_count"],
        result.page_count_before
    );
    assert_eq!(
        journal[0].summary["storage_after"]["page_count"],
        result.page_count_after
    );
    assert!(journal[0].summary["storage_delta"]["db_bytes"].is_i64());

    let _ = std::fs::remove_file(sqlite_path);
}

#[test]
fn sqlite_store_accepts_inserts_into_final_tables() {
    let store = SqliteStore::open_in_memory().expect("open sqlite store");
    let conn = store.connection();

    conn.execute(
        r#"
        INSERT INTO runs (
            run_id, mission_id, status, phase, active_boundary_kind, active_wait_kind,
            latest_checkpoint_seq, latest_event_seq, latest_audit_seq, latest_claim_epoch,
            retention_mode, created_at_ms, updated_at_ms, terminal_at_ms
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
        "#,
        rusqlite::params![
            "run-green-1",
            "mission-green-1",
            "completed",
            "completed",
            "completed",
            rusqlite::types::Null,
            7i64,
            11i64,
            13i64,
            17i64,
            "terminal_tiered",
            1_000i64,
            1_100i64,
            1_200i64,
        ],
    )
    .expect("insert runs row");
    conn.execute(
        r#"
        INSERT INTO run_inputs (run_id, mission_json, launch_input_json, created_at_ms)
        VALUES (?1, ?2, ?3, ?4)
        "#,
        rusqlite::params![
            "run-green-1",
            r#"{"goal":"transfer"}"#,
            r#"{"launch":"artifact"}"#,
            1_000i64,
        ],
    )
    .expect("insert run_inputs row");
    conn.execute(
        r#"
        INSERT INTO run_events (
            run_id, event_seq, event_kind, phase, boundary_kind, emitted_at_ms,
            checkpoint_seq, revision, payload_json
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
        "#,
        rusqlite::params![
            "run-green-1",
            1i64,
            "run_completed",
            "completed",
            "completed",
            1_200i64,
            7i64,
            11i64,
            r#"{"summary":"done"}"#,
        ],
    )
    .expect("insert run_events row");
    conn.execute(
        r#"
        INSERT INTO run_audits (
            run_id, audit_seq, audit_kind, decision_class, emitted_at_ms,
            checkpoint_seq, revision, payload_json
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
        rusqlite::params![
            "run-green-1",
            1i64,
            "maintenance_summary",
            "allow",
            1_210i64,
            7i64,
            11i64,
            r#"{"actor":"system"}"#,
        ],
    )
    .expect("insert run_audits row");
    conn.execute(
        r#"
        INSERT INTO run_checkpoints (
            run_id, checkpoint_seq, plan_epoch, checkpoint_kind, retention_tier,
            created_at_ms, is_terminal, is_side_effect_boundary, is_recovery_boundary,
            is_first_wait_checkpoint, snapshot_json
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
        "#,
        rusqlite::params![
            "run-green-1",
            7i64,
            0i64,
            "boundary",
            "terminal_final",
            1_200i64,
            1i64,
            0i64,
            0i64,
            0i64,
            r#"{"checkpoint_seq":7}"#,
        ],
    )
    .expect("insert run_checkpoints row");
    conn.execute(
        r#"
        INSERT INTO run_wait_states (
            run_id, wait_kind, request_id, entered_at_ms, expires_at_ms, state_json
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        "#,
        rusqlite::params![
            "run-green-wait",
            "signer",
            "request-1",
            2_000i64,
            2_500i64,
            r#"{"status":"awaiting"}"#,
        ],
    )
    .expect("insert run_wait_states row");
    conn.execute(
        r#"
        INSERT INTO run_claim_history (
            claim_id, run_id, host_session_id, owner_kind, owner_instance_id,
            lease_started_at_ms, lease_expires_at_ms, last_renewed_at_ms,
            claim_epoch, mode, status
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
        "#,
        rusqlite::params![
            "claim-green-1",
            "run-green-1",
            "session-1",
            "host_session",
            "ais-agent-dev",
            1_000i64,
            1_500i64,
            1_200i64,
            1i64,
            "exclusive",
            "active",
        ],
    )
    .expect("insert run_claim_history row");

    let run_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM runs", [], |row| row.get(0))
        .expect("count runs");
    let event_kind: String = conn
        .query_row(
            "SELECT event_kind FROM run_events WHERE run_id = ?1 AND event_seq = 1",
            ["run-green-1"],
            |row| row.get(0),
        )
        .expect("query run_events");
    let retention_tier: String = conn
        .query_row(
            "SELECT retention_tier FROM run_checkpoints WHERE run_id = ?1 AND checkpoint_seq = 7",
            ["run-green-1"],
            |row| row.get(0),
        )
        .expect("query run_checkpoints");

    assert_eq!(run_count, 1);
    assert_eq!(event_kind, "run_completed");
    assert_eq!(retention_tier, "terminal_final");
}

#[test]
fn sqlite_store_round_trips_run_store_api() {
    let mut store = SqliteStore::open_in_memory().expect("open sqlite store");

    let run_head = StoredRunHead {
        run_id: "run-green-api-1".to_owned(),
        mission_id: "mission-green-api-1".to_owned(),
        status: "awaiting_signer".to_owned(),
        phase: Some("blocked".to_owned()),
        active_boundary_kind: Some("awaiting_signer".to_owned()),
        active_wait_kind: Some("signer".to_owned()),
        latest_checkpoint_seq: Some(4),
        latest_event_seq: Some(2),
        latest_audit_seq: Some(1),
        latest_claim_epoch: Some(3),
        retention_mode: Some("active_full".to_owned()),
        created_at_ms: Some(1_000),
        updated_at_ms: Some(1_100),
        terminal_at_ms: None,
    };
    store.upsert_run_head(&run_head).expect("upsert run head");
    assert_eq!(
        store
            .load_run_head(&run_head.run_id)
            .expect("load run head"),
        run_head
    );

    let run_input = StoredRunInput {
        run_id: run_head.run_id.clone(),
        mission: json!({"goal":"swap"}),
        launch_input: Some(json!({"artifact":"uniswap_exact_in"})),
        created_at_ms: Some(1_000),
    };
    store
        .upsert_run_input(&run_input)
        .expect("upsert run input");
    assert_eq!(
        store
            .load_run_input(&run_head.run_id)
            .expect("load run input"),
        run_input
    );

    store
        .append_run_event_record(&StoredRunEvent {
            run_id: run_head.run_id.clone(),
            event_seq: 1,
            event_kind: "run_started".to_owned(),
            phase: Some("running".to_owned()),
            boundary_kind: None,
            emitted_at_ms: 1_010,
            checkpoint_seq: Some(1),
            revision: Some(1),
            payload: json!({"message":"started"}),
        })
        .expect("append event 1");
    store
        .append_run_event_record(&StoredRunEvent {
            run_id: run_head.run_id.clone(),
            event_seq: 2,
            event_kind: "awaiting_signer".to_owned(),
            phase: Some("blocked".to_owned()),
            boundary_kind: Some("awaiting_signer".to_owned()),
            emitted_at_ms: 1_020,
            checkpoint_seq: Some(4),
            revision: Some(2),
            payload: json!({"request_id":"signer-1"}),
        })
        .expect("append event 2");
    let event_slice = store
        .read_run_events(StoredRunEventQuery {
            run_id: run_head.run_id.clone(),
            after_event_seq: None,
            limit: Some(1),
        })
        .expect("read events");
    assert_eq!(event_slice.latest_event_seq, Some(2));
    assert!(event_slice.truncated);
    assert_eq!(event_slice.records.len(), 1);
    assert_eq!(event_slice.records[0].event_kind, "run_started");

    store
        .append_run_audit_record(&StoredRunAudit {
            run_id: run_head.run_id.clone(),
            audit_seq: 1,
            audit_kind: "governor_decision".to_owned(),
            decision_class: Some("allow".to_owned()),
            emitted_at_ms: 1_015,
            checkpoint_seq: Some(2),
            revision: Some(1),
            payload: json!({"policy":"default"}),
        })
        .expect("append audit");
    let audit_slice = store
        .read_run_audits(StoredRunAuditQuery {
            run_id: run_head.run_id.clone(),
            after_audit_seq: None,
            limit: Some(10),
        })
        .expect("read audits");
    assert_eq!(audit_slice.latest_audit_seq, Some(1));
    assert_eq!(audit_slice.records[0].audit_kind, "governor_decision");

    let checkpoint = store
        .append_run_checkpoint_record(&StoredRunCheckpoint {
            checkpoint_id: None,
            run_id: run_head.run_id.clone(),
            checkpoint_seq: 4,
            plan_epoch: 0,
            checkpoint_kind: "boundary".to_owned(),
            retention_tier: "active_full".to_owned(),
            created_at_ms: 1_020,
            is_terminal: false,
            is_side_effect_boundary: false,
            is_recovery_boundary: false,
            is_first_wait_checkpoint: true,
            snapshot: json!({"checkpoint_seq":4}),
        })
        .expect("append checkpoint");
    assert!(checkpoint.checkpoint_id.is_some());
    assert_eq!(
        store
            .load_latest_run_checkpoint(&run_head.run_id)
            .expect("load latest checkpoint")
            .checkpoint_seq,
        4
    );

    let wait_state = StoredRunWaitState {
        run_id: run_head.run_id.clone(),
        wait_kind: "signer".to_owned(),
        request_id: "signer-1".to_owned(),
        entered_at_ms: 1_020,
        expires_at_ms: Some(1_320),
        state: json!({"status":"awaiting"}),
    };
    store
        .upsert_run_wait_state(&wait_state)
        .expect("upsert wait state");
    assert_eq!(
        store
            .load_run_wait_state(&run_head.run_id)
            .expect("load wait state"),
        wait_state
    );
    store
        .clear_run_wait_state(&run_head.run_id)
        .expect("clear wait state");

    let claim = StoredRunClaim {
        claim_id: "claim-green-api-1".to_owned(),
        run_id: run_head.run_id.clone(),
        host_session_id: "session-1".to_owned(),
        owner_kind: "host_session".to_owned(),
        owner_instance_id: "ais-agent-dev".to_owned(),
        lease_started_at_ms: 1_000,
        lease_expires_at_ms: Some(1_500),
        last_renewed_at_ms: Some(1_250),
        claim_epoch: 3,
        mode: "exclusive".to_owned(),
        status: "active".to_owned(),
    };
    store
        .upsert_run_claim_record(&claim)
        .expect("upsert claim history");
    assert_eq!(
        store
            .load_latest_run_claim_for_run(&run_head.run_id)
            .expect("load latest claim"),
        claim
    );
    assert_eq!(
        store
            .load_active_run_claim_for_run(&run_head.run_id)
            .expect("load active claim"),
        Some(claim)
    );
}

#[test]
fn sqlite_run_claim_repository_round_trips_active_claim_and_history() {
    let mut store = SqliteStore::open_in_memory().expect("open sqlite store");
    let run_id = RunId("run-claim-1".to_owned());
    let claim = sample_claim("claim-1", run_id.clone(), 1, Some(20));
    RunCatalogRepository::upsert(&mut store, sample_run_catalog_entry(run_id.clone()))
        .expect("seed catalog");

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
    let stored_claim = store
        .load_latest_run_claim_for_run(&run_id.0)
        .expect("load stored latest claim");
    assert_eq!(stored_claim.claim_id, "claim-1");
    assert_eq!(stored_claim.status, "active");
    let run_head = store
        .load_run_head(&run_id.0)
        .expect("load run head after acquire");
    assert_eq!(run_head.latest_claim_epoch, Some(1));
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
    RunCatalogRepository::upsert(&mut store, sample_run_catalog_entry(run_id.clone()))
        .expect("seed catalog");
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
    let expired_head = store
        .load_run_head(&run_id.0)
        .expect("load run head after expire");
    assert_eq!(expired_head.latest_claim_epoch, Some(2));

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
    let active_claim = store
        .load_active_run_claim_for_run(&run_id.0)
        .expect("load active stored claim")
        .expect("active stored claim");
    assert_eq!(active_claim.claim_id, "claim-3");
    assert_eq!(active_claim.status, "active");
    let run_head = store
        .load_run_head(&run_id.0)
        .expect("load run head after supersede");
    assert_eq!(run_head.latest_claim_epoch, Some(1));
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
    RunCatalogRepository::upsert(&mut store, sample_run_catalog_entry(first.run_id.clone()))
        .expect("seed catalog");

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

    let stored_audits = store
        .read_run_audits(StoredRunAuditQuery {
            run_id: first.run_id.0.clone(),
            after_audit_seq: Some(1),
            limit: Some(10),
        })
        .expect("read stored audit slice");
    assert_eq!(stored_audits.latest_audit_seq, Some(2));
    assert_eq!(stored_audits.records.len(), 1);
    assert_eq!(
        stored_audits.records[0].decision_class.as_deref(),
        Some("allow")
    );
    assert!(stored_audits.records[0].emitted_at_ms > 0);
    assert_eq!(stored_audits.records[0].revision, Some(1));
    let run_head = store
        .load_run_head(&first.run_id.0)
        .expect("load run head after audits");
    assert_eq!(run_head.latest_audit_seq, Some(2));
}

#[test]
fn sqlite_store_reads_runtime_traits_from_final_tables() {
    let mut store = SqliteStore::open_in_memory().expect("open sqlite store");
    let run_id = RunId("run-1".to_owned());
    let mission = sample_mission();
    let checkpoint = sample_checkpoint();
    let event = sample_event(run_id.clone(), 1);
    let audit = sample_runtime_audit(1);
    let signer = sample_signer_state();

    MissionRepository::insert(&mut store, run_id.clone(), mission.clone()).expect("insert mission");
    CheckpointArchive::append(
        &mut store,
        CheckpointArchiveEntry {
            snapshot: checkpoint.clone(),
            kind: CheckpointArchiveKind::Boundary,
        },
    )
    .expect("append checkpoint");
    EventArchive::append(&mut store, event.clone()).expect("append event");
    RuntimeAuditArchive::append(&mut store, audit.clone()).expect("append audit");
    SignerStateStore::upsert(&mut store, signer.clone()).expect("append signer");

    let restored_mission = MissionRepository::load(&store, &run_id).expect("stored mission");
    assert_eq!(restored_mission.mission_id, mission.mission_id);
    assert_eq!(restored_mission.goal, mission.goal);
    assert_eq!(restored_mission.allowed_chains, mission.allowed_chains);
    let restored_checkpoint =
        CheckpointArchive::latest(&store, &run_id.0).expect("stored checkpoint");
    assert_eq!(restored_checkpoint.run_id, checkpoint.run_id);
    assert_eq!(restored_checkpoint.mission_id, checkpoint.mission_id);
    assert_eq!(
        restored_checkpoint.checkpoint_seq,
        checkpoint.checkpoint_seq
    );
    assert_eq!(restored_checkpoint.plan_epoch, checkpoint.plan_epoch);
    assert_eq!(
        restored_checkpoint.lifecycle.status,
        checkpoint.lifecycle.status
    );
    let history = CheckpointArchive::history(&store, &run_id.0).expect("stored history");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].snapshot.checkpoint_seq, 1);
    let events = EventArchive::read(
        &store,
        EventArchiveQuery {
            run_id: run_id.clone(),
            after_event_seq: None,
            limit: Some(8),
        },
    )
    .expect("stored events");
    assert_eq!(events.latest_event_seq, Some(1));
    assert_eq!(events.events.len(), 1);
    assert_eq!(events.events[0].event_seq, event.event_seq);
    let audits = RuntimeAuditArchive::read(
        &store,
        RuntimeAuditQuery {
            run_id: run_id.clone(),
            after_audit_seq: None,
            limit: Some(8),
        },
    )
    .expect("stored audits");
    assert_eq!(audits.latest_audit_seq, Some(1));
    assert_eq!(audits.records.len(), 1);
    assert_eq!(audits.records[0].audit_id, audit.audit_id);
    let restored_signer = SignerStateStore::load(&store, &run_id).expect("stored signer state");
    assert_eq!(restored_signer, signer);
}

#[test]
fn sqlite_grouped_commit_round_trips_all_members() {
    let mut store = SqliteStore::open_in_memory().expect("open sqlite store");
    let mut unit = sample_grouped_commit_unit();
    unit.catalog_write.entry.latest_revision = 42;

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

    let signer = SignerStateStore::load(&store, &unit.run_id).expect("load signer state");
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

    let run_input = store
        .load_run_input("run-1")
        .expect("load stored run input");
    assert_eq!(run_input.mission["mission_id"], "mission-1");
    let run_head = store.load_run_head("run-1").expect("load stored run head");
    assert_eq!(run_head.latest_checkpoint_seq, Some(1));
    assert_eq!(run_head.latest_event_seq, Some(1));
    assert_eq!(run_head.latest_audit_seq, Some(1));
    let stored_events = store
        .read_run_events(StoredRunEventQuery {
            run_id: "run-1".to_owned(),
            after_event_seq: None,
            limit: Some(8),
        })
        .expect("read stored events");
    assert_eq!(stored_events.records.len(), 1);
    assert!(stored_events.records[0].emitted_at_ms > 0);
    assert_eq!(stored_events.records[0].revision, Some(42));
    let stored_audits = store
        .read_run_audits(StoredRunAuditQuery {
            run_id: "run-1".to_owned(),
            after_audit_seq: None,
            limit: Some(8),
        })
        .expect("read stored audits");
    assert_eq!(stored_audits.records.len(), 1);
    assert_eq!(
        stored_events.records[0].emitted_at_ms,
        stored_audits.records[0].emitted_at_ms
    );
    assert!(stored_audits.records[0].emitted_at_ms > 0);
    assert_eq!(stored_audits.records[0].revision, Some(42));
    let stored_checkpoint = store
        .load_latest_run_checkpoint("run-1")
        .expect("load stored checkpoint");
    assert_eq!(stored_checkpoint.checkpoint_kind, "progress");
    let stored_wait = store
        .load_run_wait_state("run-1")
        .expect("load stored wait");
    assert_eq!(stored_wait.wait_kind, "signer");
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
    assert!(SignerStateStore::load(&store, &unit.run_id).is_err());
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
    assert!(store.load_run_input("run-1").is_err());
    assert!(store.load_run_head("run-1").is_err());
    assert!(store
        .read_run_events(StoredRunEventQuery {
            run_id: "run-1".to_owned(),
            after_event_seq: None,
            limit: Some(8),
        })
        .is_err());
    let stored_audits = store
        .read_run_audits(StoredRunAuditQuery {
            run_id: "run-1".to_owned(),
            after_audit_seq: None,
            limit: Some(8),
        })
        .expect("seeded stored audit remains");
    assert_eq!(stored_audits.latest_audit_seq, Some(1));
    assert_eq!(stored_audits.records.len(), 1);
    assert!(store.load_latest_run_checkpoint("run-1").is_err());
    assert!(store.load_run_wait_state("run-1").is_err());
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
    updated.apply_resolution(ais_agent_core::runtime::SignerResolution {
        request_id: updated.request_id.clone(),
        kind: ais_agent_core::runtime::SignerResolutionKind::Submitted,
        resolved_at_ms: Some(150),
        tx_hash: Some("0xabc".to_owned()),
        signed_payload: None,
    });

    SignerStateStore::upsert(&mut store, original).expect("upsert original signer state");
    SignerStateStore::upsert(&mut store, updated.clone()).expect("upsert updated signer state");

    let loaded =
        SignerStateStore::load(&store, &updated.run_id).expect("load updated signer state");
    assert_eq!(loaded, updated);

    SignerStateStore::clear(&mut store, &updated.run_id).expect("clear signer state");
    match SignerStateStore::load(&store, &updated.run_id) {
        Err(SignerStateStoreError::NotFound { run_id }) => assert_eq!(run_id, updated.run_id.0),
        other => panic!("unexpected signer state store result after clear: {other:?}"),
    }
    match store.load_run_wait_state(&updated.run_id.0) {
        Err(RunStoreError::NotFound { entity, key }) => {
            assert_eq!(entity, "run_wait_states");
            assert_eq!(key, updated.run_id.0);
        }
        other => panic!("unexpected stored wait-state result after clear: {other:?}"),
    }

    SignerStateStore::clear(&mut store, &updated.run_id)
        .expect("clearing absent signer state stays idempotent");
}

#[test]
fn migrate_connection_is_idempotent() {
    let conn = rusqlite::Connection::open_in_memory().expect("open connection");
    migrate_connection(&conn).expect("migrate once");
    migrate_connection(&conn).expect("migrate twice");

    let version: i32 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .expect("query version");
    assert_eq!(version, SCHEMA_VERSION);

    let maintenance_journal_exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'maintenance_journal'",
            [],
            |row| row.get(0),
        )
        .expect("maintenance_journal exists");
    let maintenance_state_exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'store_maintenance_state'",
            [],
            |row| row.get(0),
        )
        .expect("store_maintenance_state exists");
    let runs_exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'runs'",
            [],
            |row| row.get(0),
        )
        .expect("runs exists");
    let run_events_exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'run_events'",
            [],
            |row| row.get(0),
        )
        .expect("run_events exists");

    assert_eq!(maintenance_journal_exists, 1);
    assert_eq!(maintenance_state_exists, 1);
    assert_eq!(runs_exists, 1);
    assert_eq!(run_events_exists, 1);
}

#[test]
fn migrate_connection_backfills_store_metadata_columns() {
    let conn = rusqlite::Connection::open_in_memory().expect("open connection");
    conn.execute_batch(
        r#"
        CREATE TABLE store_maintenance_state (
            singleton_key TEXT PRIMARY KEY NOT NULL CHECK(singleton_key = 'default'),
            last_operation_kind TEXT,
            last_operation_status TEXT,
            last_prune_started_at_ms INTEGER,
            last_prune_finished_at_ms INTEGER,
            last_pruned_terminal_before_ms INTEGER,
            last_vacuum_at_ms INTEGER,
            schema_retention_version INTEGER NOT NULL
        );
        "#,
    )
    .expect("seed legacy maintenance table");

    migrate_connection(&conn).expect("migrate legacy schema");

    let columns: Vec<String> = {
        let mut stmt = conn
            .prepare("PRAGMA table_info(store_maintenance_state)")
            .expect("prepare pragma");
        let rows = stmt
            .query_map([], |row| row.get(1))
            .expect("query pragma columns");
        rows.collect::<Result<Vec<_>, _>>()
            .expect("collect columns")
    };

    for expected in [
        "last_store_opened_at_ms",
        "last_prune_deleted_rows",
        "last_purge_deleted_rows",
        "last_vacuum_started_at_ms",
        "last_vacuum_finished_at_ms",
        "last_wal_checkpoint_at_ms",
        "last_known_page_count",
        "last_known_freelist_count",
        "last_known_db_bytes",
        "last_growth_sampled_at_ms",
        "metadata_schema_version",
    ] {
        assert!(
            columns.iter().any(|column| column == expected),
            "expected migrated column {expected}"
        );
    }
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
fn sqlite_query_plans_use_supporting_indexes_for_hot_store_reads() {
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
        FROM run_checkpoints
        WHERE run_id = ?1
        ORDER BY checkpoint_seq DESC, plan_epoch DESC, checkpoint_id DESC
        LIMIT 1
        "#,
        rusqlite::params!["run-1"],
    );
    assert!(
        latest_plan.iter().any(|detail| {
            detail.contains("idx_run_checkpoints_latest_lookup")
                || detail.contains("idx_run_checkpoints_run_seq_epoch")
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
        SELECT payload_json
        FROM run_events
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
            .any(|detail| detail.contains("run_events") && detail.contains("INDEX")),
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
    ) = service.into_parts_with_signer_state_store();
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
    ) = service.into_parts_with_signer_state_store();
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
    ) = restarted.into_parts_with_signer_state_store();
    assert!(run_repo.load(&run_id).is_err());
}

#[tokio::test]
async fn sqlite_host_service_replays_begin_run_truth_after_restart_from_store() {
    let sqlite_path = sqlite_test_path("begin-store-restart");
    let host_session_id: HostSessionId = "session-sqlite-begin-store-restart".into();
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
                command_id: CommandId("cmd-sqlite-begin-store-restart".to_owned()),
                idempotency_key: "idem-sqlite-begin-store-restart".into(),
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
    ) = service.into_parts_with_signer_state_store();

    let mut restarted = sqlite_host_service(
        &sqlite_path,
        InMemoryRunRepository::default(),
        session_store,
    );

    let inspect = restarted
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-sqlite-begin-store-restart-inspect".into()),
            command: RunCommand::InspectRun(InspectRunCommand {
                command_id: CommandId("cmd-sqlite-begin-store-restart-inspect".to_owned()),
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
        .expect("started event batch from store");
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
    ) = restarted.into_parts_with_signer_state_store();
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
    match submit.response {
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
    SignerStateStore::upsert(&mut signer_store, signer_state.clone())
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
            command: RunCommand::SubmitSignerResolution(SubmitSignerResolutionCommand {
                command_id: CommandId("cmd-sqlite-signer".to_owned()),
                run_id: run_id.clone(),
                resolution: SignerResolutionSubmission {
                    request_id: SignerRequestId("signer-1".to_owned()),
                    kind: SignerResolutionKind::Submitted,
                    tx_hash: Some("0xabc".to_owned()),
                    signed_payload: None,
                    details: BTreeMap::new(),
                },
                expected_version: None,
            }),
        })
        .await;
    match signer.response {
        HostCommandResponse::Pause(pause) => assert_eq!(
            pause.kind,
            ais_agent_host::inspect::PauseKind::NeedConfirmation
        ),
        other => panic!("unexpected response: {other:?}"),
    }

    let checkpoint_store = SqliteStore::open_path(&sqlite_path).expect("checkpoint store");
    let signer_store = SqliteStore::open_path(&sqlite_path).expect("signer store");
    let catalog_store = SqliteStore::open_path(&sqlite_path).expect("catalog store");
    let run_store = SqliteStore::open_path(&sqlite_path).expect("stored run state");
    let checkpoint = CheckpointArchive::latest(&checkpoint_store, &run_id.0)
        .expect("latest checkpoint after signer decision");
    let catalog =
        RunCatalogRepository::load(&catalog_store, &run_id).expect("catalog after signer decision");
    assert_eq!(checkpoint.checkpoint_seq, catalog.latest_checkpoint_seq);
    assert!(matches!(
        SignerStateStore::load(&signer_store, &run_id),
        Err(SignerStateStoreError::NotFound { .. })
    ));
    assert_eq!(
        checkpoint
            .pending_requests
            .pending_signer_request_id
            .as_deref(),
        None
    );
    assert_eq!(
        checkpoint
            .pending_requests
            .pending_confirmation_id
            .as_deref(),
        Some("0xabc")
    );
    let run_input = run_store
        .load_run_input(&run_id.0)
        .expect("load run input after signer resolution");
    assert_eq!(run_input.mission["mission_id"], "mission-1");
    let run_head = run_store
        .load_run_head(&run_id.0)
        .expect("load run head after signer resolution");
    assert_eq!(
        run_head.active_boundary_kind.as_deref(),
        Some("confirmation")
    );
    let checkpoint_seq = i64::try_from(checkpoint.checkpoint_seq).expect("checkpoint seq fits i64");
    assert_eq!(run_head.latest_checkpoint_seq, Some(checkpoint_seq));
    let latest_checkpoint = run_store
        .load_latest_run_checkpoint(&run_id.0)
        .expect("load checkpoint after signer resolution");
    assert_eq!(latest_checkpoint.checkpoint_seq, checkpoint_seq);
    let stored_events = run_store
        .read_run_events(StoredRunEventQuery {
            run_id: run_id.0.clone(),
            after_event_seq: None,
            limit: Some(16),
        })
        .expect("read events after signer resolution");
    assert_eq!(stored_events.latest_event_seq, run_head.latest_event_seq);
    assert!(
        stored_events.records.iter().any(|record| {
            record.boundary_kind.as_deref() == Some("confirmation")
                || record.event_kind == "awaiting_confirm"
        }),
        "expected confirmation-related event in stored timeline"
    );
    match run_store.load_run_wait_state(&run_id.0) {
        Err(RunStoreError::NotFound { entity, key }) => {
            assert_eq!(entity, "run_wait_states");
            assert_eq!(key, run_id.0);
        }
        other => panic!("unexpected wait-state result: {other:?}"),
    }
}

#[tokio::test]
async fn sqlite_host_service_can_resume_awaiting_signer_from_store_after_restart() {
    let sqlite_path = sqlite_test_path("signer-store");
    let host_session_id: HostSessionId = "session-sqlite-signer-store".into();
    let run_id = RunId("run-1".to_owned());
    let mission = sample_mission();
    let signer_state = sample_signer_state();
    let checkpoint = awaiting_signer_checkpoint(&signer_state);

    seed_sqlite_mission_checkpoint(&sqlite_path, run_id.clone(), mission.clone(), checkpoint);
    let mut signer_store = SqliteStore::open_path(&sqlite_path).expect("signer state store");
    SignerStateStore::upsert(&mut signer_store, signer_state.clone())
        .expect("persist signer state");

    let restored = restore_active_run(
        &run_id,
        &SqliteStore::open_path(&sqlite_path).expect("mission store"),
        &SqliteStore::open_path(&sqlite_path).expect("checkpoint store"),
        &SqliteStore::open_path(&sqlite_path).expect("signer state store"),
    )
    .expect("restore runtime from store");

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
            host_session_id,
            host_request_id: Some("request-sqlite-signer-store".into()),
            command: RunCommand::SubmitSignerResolution(SubmitSignerResolutionCommand {
                command_id: CommandId("cmd-sqlite-signer-store".to_owned()),
                run_id: run_id.clone(),
                resolution: SignerResolutionSubmission {
                    request_id: SignerRequestId("signer-1".to_owned()),
                    kind: SignerResolutionKind::Submitted,
                    tx_hash: Some("0xabc".to_owned()),
                    signed_payload: None,
                    details: BTreeMap::new(),
                },
                expected_version: None,
            }),
        })
        .await;
    match signer.response {
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
    SignerStateStore::upsert(&mut signer_store, signer_state.clone())
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
            command: RunCommand::SubmitSignerResolution(SubmitSignerResolutionCommand {
                command_id: CommandId("cmd-sqlite-cancel-pending-signer".to_owned()),
                run_id: run_id.clone(),
                resolution: SignerResolutionSubmission {
                    request_id: SignerRequestId("signer-1".to_owned()),
                    kind: SignerResolutionKind::Submitted,
                    tx_hash: Some("0xabc".to_owned()),
                    signed_payload: None,
                    details: BTreeMap::new(),
                },
                expected_version: None,
            }),
        })
        .await;
    match signer.response {
        HostCommandResponse::Pause(pause) => assert_eq!(
            pause.kind,
            ais_agent_host::inspect::PauseKind::NeedConfirmation
        ),
        other => panic!("unexpected response: {other:?}"),
    }

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
    ) = service.into_parts_with_signer_state_store();
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
    match submit.response {
        HostCommandResponse::Inspect(snapshot) => assert_eq!(
            snapshot.status,
            ais_agent_host::inspect::RunStatus::Completed
        ),
        other => panic!("unexpected response: {other:?}"),
    }

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
            pending_signer_request: None,
            pending_confirmation_id: None,
        },
        execution_artifact: None,
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
        wait_state_write: Some(SignerStateWrite::Upsert {
            wait_state: ais_agent_runtime::persistence::signer_state_into_wait_state_record(
                sample_signer_state(),
            )
            .expect("encode signer wait state"),
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
        execution_artifact: None,
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
        trace_context: None,
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
