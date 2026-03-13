pub const CREATE_MISSIONS_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS missions (
    run_id TEXT PRIMARY KEY NOT NULL,
    mission_json TEXT NOT NULL
)
"#;

pub const CREATE_RUN_CATALOG_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS run_catalog (
    run_id TEXT PRIMARY KEY NOT NULL,
    mission_id TEXT NOT NULL,
    status_json TEXT NOT NULL,
    phase_json TEXT NOT NULL,
    active_boundary_kind_json TEXT,
    latest_checkpoint_seq INTEGER NOT NULL,
    latest_event_seq INTEGER,
    latest_revision INTEGER NOT NULL,
    created_at_ms INTEGER,
    updated_at_ms INTEGER,
    terminal_at_ms INTEGER
)
"#;

pub const CREATE_CHECKPOINT_ARCHIVE_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS checkpoint_archive (
    archive_id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id TEXT NOT NULL,
    checkpoint_seq INTEGER NOT NULL,
    plan_epoch INTEGER NOT NULL,
    archive_kind_json TEXT NOT NULL,
    snapshot_json TEXT NOT NULL
)
"#;

pub const CREATE_CHECKPOINT_ARCHIVE_UNIQUE_INDEX: &str = r#"
CREATE UNIQUE INDEX IF NOT EXISTS idx_checkpoint_archive_run_seq_epoch
ON checkpoint_archive(run_id, checkpoint_seq, plan_epoch)
"#;

pub const CREATE_CHECKPOINT_ARCHIVE_LATEST_INDEX: &str = r#"
CREATE INDEX IF NOT EXISTS idx_checkpoint_archive_latest_lookup
ON checkpoint_archive(run_id, checkpoint_seq DESC, plan_epoch DESC, archive_id DESC)
"#;

pub const CREATE_EVENT_ARCHIVE_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS event_archive (
    run_id TEXT NOT NULL,
    event_seq INTEGER NOT NULL,
    checkpoint_seq INTEGER NOT NULL,
    plan_epoch INTEGER NOT NULL,
    event_json TEXT NOT NULL,
    PRIMARY KEY (run_id, event_seq)
)
"#;

pub const CREATE_RUNTIME_AUDIT_ARCHIVE_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS runtime_audit_archive (
    run_id TEXT NOT NULL,
    audit_seq INTEGER NOT NULL,
    checkpoint_seq INTEGER NOT NULL,
    plan_epoch INTEGER NOT NULL,
    audit_id TEXT NOT NULL,
    audit_json TEXT NOT NULL,
    PRIMARY KEY (run_id, audit_seq)
)
"#;

pub const CREATE_RUNTIME_AUDIT_ARCHIVE_LATEST_INDEX: &str = r#"
CREATE INDEX IF NOT EXISTS idx_runtime_audit_archive_latest_lookup
ON runtime_audit_archive(run_id, audit_seq DESC)
"#;

pub const CREATE_RUN_CLAIMS_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS run_claims (
    claim_id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL,
    host_session_id TEXT NOT NULL,
    owner_kind_json TEXT NOT NULL,
    owner_instance_id TEXT NOT NULL,
    lease_started_at_ms INTEGER NOT NULL,
    lease_expires_at_ms INTEGER,
    last_renewed_at_ms INTEGER,
    claim_epoch INTEGER NOT NULL,
    mode_json TEXT NOT NULL,
    status_json TEXT NOT NULL
)
"#;

pub const CREATE_RUN_CLAIMS_ACTIVE_INDEX: &str = r#"
CREATE UNIQUE INDEX IF NOT EXISTS idx_run_claims_active_by_run
ON run_claims(run_id)
WHERE status_json = '"active"'
"#;

pub const CREATE_RUN_CLAIMS_RUN_LOOKUP_INDEX: &str = r#"
CREATE INDEX IF NOT EXISTS idx_run_claims_run_epoch_lookup
ON run_claims(run_id, claim_epoch DESC)
"#;

pub const CREATE_SIGNER_STATE_ARCHIVE_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS signer_state_archive (
    run_id TEXT PRIMARY KEY NOT NULL,
    request_id TEXT NOT NULL,
    signer_state_json TEXT NOT NULL
)
"#;
