pub const CREATE_MAINTENANCE_JOURNAL_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS maintenance_journal (
    journal_id INTEGER PRIMARY KEY AUTOINCREMENT,
    operation_kind TEXT NOT NULL,
    started_at_ms INTEGER NOT NULL,
    finished_at_ms INTEGER,
    status TEXT NOT NULL,
    summary_json TEXT NOT NULL
)
"#;

pub const CREATE_MAINTENANCE_JOURNAL_LATEST_INDEX: &str = r#"
CREATE INDEX IF NOT EXISTS idx_maintenance_journal_latest_lookup
ON maintenance_journal(started_at_ms DESC, journal_id DESC)
"#;

pub const CREATE_STORE_MAINTENANCE_STATE_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS store_maintenance_state (
    singleton_key TEXT PRIMARY KEY NOT NULL CHECK(singleton_key = 'default'),
    last_operation_kind TEXT,
    last_operation_status TEXT,
    last_store_opened_at_ms INTEGER,
    last_prune_started_at_ms INTEGER,
    last_prune_finished_at_ms INTEGER,
    last_pruned_terminal_before_ms INTEGER,
    last_prune_deleted_rows INTEGER,
    last_purge_deleted_rows INTEGER,
    last_vacuum_started_at_ms INTEGER,
    last_vacuum_finished_at_ms INTEGER,
    last_vacuum_at_ms INTEGER,
    last_wal_checkpoint_at_ms INTEGER,
    last_known_page_count INTEGER,
    last_known_freelist_count INTEGER,
    last_known_db_bytes INTEGER,
    last_growth_sampled_at_ms INTEGER,
    schema_retention_version INTEGER NOT NULL,
    metadata_schema_version INTEGER NOT NULL DEFAULT 1
)
"#;

pub const CREATE_RUNS_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS runs (
    run_id TEXT PRIMARY KEY NOT NULL,
    mission_id TEXT NOT NULL,
    status TEXT NOT NULL,
    phase TEXT,
    active_boundary_kind TEXT,
    active_wait_kind TEXT,
    latest_checkpoint_seq INTEGER,
    latest_event_seq INTEGER,
    latest_audit_seq INTEGER,
    latest_claim_epoch INTEGER,
    retention_mode TEXT,
    created_at_ms INTEGER,
    updated_at_ms INTEGER,
    terminal_at_ms INTEGER
)
"#;

pub const CREATE_RUNS_UPDATED_AT_INDEX: &str = r#"
CREATE INDEX IF NOT EXISTS idx_runs_updated_lookup
ON runs(COALESCE(updated_at_ms, created_at_ms, 0) DESC, run_id DESC)
"#;

pub const CREATE_RUN_INPUTS_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS run_inputs (
    run_id TEXT PRIMARY KEY NOT NULL,
    mission_json TEXT NOT NULL,
    launch_input_json TEXT,
    created_at_ms INTEGER
)
"#;

pub const CREATE_RUN_EVENTS_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS run_events (
    run_id TEXT NOT NULL,
    event_seq INTEGER NOT NULL,
    event_kind TEXT NOT NULL,
    phase TEXT,
    boundary_kind TEXT,
    emitted_at_ms INTEGER NOT NULL,
    checkpoint_seq INTEGER,
    revision INTEGER,
    payload_json TEXT NOT NULL,
    PRIMARY KEY (run_id, event_seq)
)
"#;

pub const CREATE_RUN_EVENTS_LATEST_INDEX: &str = r#"
CREATE INDEX IF NOT EXISTS idx_run_events_latest_lookup
ON run_events(run_id, event_seq DESC)
"#;

pub const CREATE_RUN_EVENTS_KIND_TIME_INDEX: &str = r#"
CREATE INDEX IF NOT EXISTS idx_run_events_kind_time_lookup
ON run_events(event_kind, emitted_at_ms DESC)
"#;

pub const CREATE_RUN_AUDITS_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS run_audits (
    run_id TEXT NOT NULL,
    audit_seq INTEGER NOT NULL,
    audit_kind TEXT NOT NULL,
    decision_class TEXT,
    emitted_at_ms INTEGER NOT NULL,
    checkpoint_seq INTEGER,
    revision INTEGER,
    payload_json TEXT NOT NULL,
    PRIMARY KEY (run_id, audit_seq)
)
"#;

pub const CREATE_RUN_AUDITS_LATEST_INDEX: &str = r#"
CREATE INDEX IF NOT EXISTS idx_run_audits_latest_lookup
ON run_audits(run_id, audit_seq DESC)
"#;

pub const CREATE_RUN_AUDITS_KIND_TIME_INDEX: &str = r#"
CREATE INDEX IF NOT EXISTS idx_run_audits_kind_time_lookup
ON run_audits(audit_kind, emitted_at_ms DESC)
"#;

pub const CREATE_RUN_CHECKPOINTS_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS run_checkpoints (
    checkpoint_id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id TEXT NOT NULL,
    checkpoint_seq INTEGER NOT NULL,
    plan_epoch INTEGER NOT NULL,
    checkpoint_kind TEXT NOT NULL,
    retention_tier TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL,
    is_terminal INTEGER NOT NULL DEFAULT 0,
    is_side_effect_boundary INTEGER NOT NULL DEFAULT 0,
    is_recovery_boundary INTEGER NOT NULL DEFAULT 0,
    is_first_wait_checkpoint INTEGER NOT NULL DEFAULT 0,
    snapshot_json TEXT NOT NULL
)
"#;

pub const CREATE_RUN_CHECKPOINTS_UNIQUE_INDEX: &str = r#"
CREATE UNIQUE INDEX IF NOT EXISTS idx_run_checkpoints_run_seq_epoch
ON run_checkpoints(run_id, checkpoint_seq, plan_epoch)
"#;

pub const CREATE_RUN_CHECKPOINTS_LATEST_INDEX: &str = r#"
CREATE INDEX IF NOT EXISTS idx_run_checkpoints_latest_lookup
ON run_checkpoints(run_id, checkpoint_seq DESC, plan_epoch DESC, checkpoint_id DESC)
"#;

pub const CREATE_RUN_CHECKPOINTS_RETENTION_INDEX: &str = r#"
CREATE INDEX IF NOT EXISTS idx_run_checkpoints_retention_lookup
ON run_checkpoints(run_id, retention_tier, created_at_ms DESC)
"#;

pub const CREATE_RUN_WAIT_STATES_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS run_wait_states (
    run_id TEXT PRIMARY KEY NOT NULL,
    wait_kind TEXT NOT NULL,
    request_id TEXT NOT NULL,
    entered_at_ms INTEGER NOT NULL,
    expires_at_ms INTEGER,
    state_json TEXT NOT NULL
)
"#;

pub const CREATE_RUN_WAIT_STATES_KIND_INDEX: &str = r#"
CREATE INDEX IF NOT EXISTS idx_run_wait_states_kind_lookup
ON run_wait_states(wait_kind, entered_at_ms DESC)
"#;

pub const CREATE_RUN_CLAIM_HISTORY_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS run_claim_history (
    claim_id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL,
    host_session_id TEXT NOT NULL,
    owner_kind TEXT NOT NULL,
    owner_instance_id TEXT NOT NULL,
    lease_started_at_ms INTEGER NOT NULL,
    lease_expires_at_ms INTEGER,
    last_renewed_at_ms INTEGER,
    claim_epoch INTEGER NOT NULL,
    mode TEXT NOT NULL,
    status TEXT NOT NULL
)
"#;

pub const CREATE_RUN_CLAIM_HISTORY_ACTIVE_INDEX: &str = r#"
CREATE UNIQUE INDEX IF NOT EXISTS idx_run_claim_history_active_by_run
ON run_claim_history(run_id)
WHERE status = 'active'
"#;

pub const CREATE_RUN_CLAIM_HISTORY_RUN_LOOKUP_INDEX: &str = r#"
CREATE INDEX IF NOT EXISTS idx_run_claim_history_run_epoch_lookup
ON run_claim_history(run_id, claim_epoch DESC)
"#;
