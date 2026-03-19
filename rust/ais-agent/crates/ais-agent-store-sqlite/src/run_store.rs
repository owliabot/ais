use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::SqliteStore;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredRunHead {
    pub run_id: String,
    pub mission_id: String,
    pub status: String,
    pub phase: Option<String>,
    pub active_boundary_kind: Option<String>,
    pub active_wait_kind: Option<String>,
    pub latest_checkpoint_seq: Option<i64>,
    pub latest_event_seq: Option<i64>,
    pub latest_audit_seq: Option<i64>,
    pub latest_claim_epoch: Option<i64>,
    pub retention_mode: Option<String>,
    pub created_at_ms: Option<i64>,
    pub updated_at_ms: Option<i64>,
    pub terminal_at_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredRunInput {
    pub run_id: String,
    pub mission: Value,
    pub launch_input: Option<Value>,
    pub created_at_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredRunEvent {
    pub run_id: String,
    pub event_seq: i64,
    pub event_kind: String,
    pub phase: Option<String>,
    pub boundary_kind: Option<String>,
    pub emitted_at_ms: i64,
    pub checkpoint_seq: Option<i64>,
    pub revision: Option<i64>,
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredRunEventQuery {
    pub run_id: String,
    pub after_event_seq: Option<i64>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredRunEventSlice {
    pub latest_event_seq: Option<i64>,
    pub next_after_event_seq: Option<i64>,
    pub truncated: bool,
    pub records: Vec<StoredRunEvent>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredRunAudit {
    pub run_id: String,
    pub audit_seq: i64,
    pub audit_kind: String,
    pub decision_class: Option<String>,
    pub emitted_at_ms: i64,
    pub checkpoint_seq: Option<i64>,
    pub revision: Option<i64>,
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredRunAuditQuery {
    pub run_id: String,
    pub after_audit_seq: Option<i64>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredRunAuditSlice {
    pub latest_audit_seq: Option<i64>,
    pub next_after_audit_seq: Option<i64>,
    pub truncated: bool,
    pub records: Vec<StoredRunAudit>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredRunCheckpoint {
    pub checkpoint_id: Option<i64>,
    pub run_id: String,
    pub checkpoint_seq: i64,
    pub plan_epoch: i64,
    pub checkpoint_kind: String,
    pub retention_tier: String,
    pub created_at_ms: i64,
    pub is_terminal: bool,
    pub is_side_effect_boundary: bool,
    pub is_recovery_boundary: bool,
    pub is_first_wait_checkpoint: bool,
    pub snapshot: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredRunWaitState {
    pub run_id: String,
    pub wait_kind: String,
    pub request_id: String,
    pub entered_at_ms: i64,
    pub expires_at_ms: Option<i64>,
    pub state: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredRunClaim {
    pub claim_id: String,
    pub run_id: String,
    pub host_session_id: String,
    pub owner_kind: String,
    pub owner_instance_id: String,
    pub lease_started_at_ms: i64,
    pub lease_expires_at_ms: Option<i64>,
    pub last_renewed_at_ms: Option<i64>,
    pub claim_epoch: i64,
    pub mode: String,
    pub status: String,
}

#[derive(Debug, Error)]
pub enum RunStoreError {
    #[error("sqlite run storage error: {message}")]
    Storage { message: String },
    #[error("sqlite run serialization error: {message}")]
    Serialization { message: String },
    #[error("sqlite run not found: {entity} {key}")]
    NotFound { entity: &'static str, key: String },
}

impl SqliteStore {
    pub fn upsert_run_head(&mut self, head: &StoredRunHead) -> Result<(), RunStoreError> {
        self.connection()
            .execute(
                r#"
                INSERT INTO runs (
                    run_id, mission_id, status, phase, active_boundary_kind, active_wait_kind,
                    latest_checkpoint_seq, latest_event_seq, latest_audit_seq, latest_claim_epoch,
                    retention_mode, created_at_ms, updated_at_ms, terminal_at_ms
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
                ON CONFLICT(run_id) DO UPDATE SET
                    mission_id = excluded.mission_id,
                    status = excluded.status,
                    phase = excluded.phase,
                    active_boundary_kind = excluded.active_boundary_kind,
                    active_wait_kind = excluded.active_wait_kind,
                    latest_checkpoint_seq = excluded.latest_checkpoint_seq,
                    latest_event_seq = excluded.latest_event_seq,
                    latest_audit_seq = excluded.latest_audit_seq,
                    latest_claim_epoch = excluded.latest_claim_epoch,
                    retention_mode = excluded.retention_mode,
                    created_at_ms = excluded.created_at_ms,
                    updated_at_ms = excluded.updated_at_ms,
                    terminal_at_ms = excluded.terminal_at_ms
                "#,
                rusqlite::params![
                    head.run_id,
                    head.mission_id,
                    head.status,
                    head.phase,
                    head.active_boundary_kind,
                    head.active_wait_kind,
                    head.latest_checkpoint_seq,
                    head.latest_event_seq,
                    head.latest_audit_seq,
                    head.latest_claim_epoch,
                    head.retention_mode,
                    head.created_at_ms,
                    head.updated_at_ms,
                    head.terminal_at_ms,
                ],
            )
            .map_err(storage_error)?;
        Ok(())
    }

    pub fn load_run_head(&self, run_id: &str) -> Result<StoredRunHead, RunStoreError> {
        self.connection()
            .query_row(
                r#"
                SELECT
                    mission_id, status, phase, active_boundary_kind, active_wait_kind,
                    latest_checkpoint_seq, latest_event_seq, latest_audit_seq, latest_claim_epoch,
                    retention_mode, created_at_ms, updated_at_ms, terminal_at_ms
                FROM runs
                WHERE run_id = ?1
                "#,
                [run_id],
                |row| {
                    Ok(StoredRunHead {
                        run_id: run_id.to_owned(),
                        mission_id: row.get(0)?,
                        status: row.get(1)?,
                        phase: row.get(2)?,
                        active_boundary_kind: row.get(3)?,
                        active_wait_kind: row.get(4)?,
                        latest_checkpoint_seq: row.get(5)?,
                        latest_event_seq: row.get(6)?,
                        latest_audit_seq: row.get(7)?,
                        latest_claim_epoch: row.get(8)?,
                        retention_mode: row.get(9)?,
                        created_at_ms: row.get(10)?,
                        updated_at_ms: row.get(11)?,
                        terminal_at_ms: row.get(12)?,
                    })
                },
            )
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => RunStoreError::NotFound {
                    entity: "runs",
                    key: run_id.to_owned(),
                },
                other => storage_error(other),
            })
    }

    pub fn upsert_run_input(&mut self, input: &StoredRunInput) -> Result<(), RunStoreError> {
        let mission_json = serde_json::to_string(&input.mission).map_err(serialization_error)?;
        let launch_input_json = input
            .launch_input
            .as_ref()
            .map(|value| serde_json::to_string(value).map_err(serialization_error))
            .transpose()?;
        self.connection()
            .execute(
                r#"
                INSERT INTO run_inputs (run_id, mission_json, launch_input_json, created_at_ms)
                VALUES (?1, ?2, ?3, ?4)
                ON CONFLICT(run_id) DO UPDATE SET
                    mission_json = excluded.mission_json,
                    launch_input_json = excluded.launch_input_json,
                    created_at_ms = excluded.created_at_ms
                "#,
                rusqlite::params![
                    input.run_id,
                    mission_json,
                    launch_input_json,
                    input.created_at_ms,
                ],
            )
            .map_err(storage_error)?;
        Ok(())
    }

    pub fn load_run_input(&self, run_id: &str) -> Result<StoredRunInput, RunStoreError> {
        self.connection()
            .query_row(
                r#"
                SELECT mission_json, launch_input_json, created_at_ms
                FROM run_inputs
                WHERE run_id = ?1
                "#,
                [run_id],
                |row| {
                    Ok(StoredRunInput {
                        run_id: run_id.to_owned(),
                        mission: parse_json_text(row.get::<_, String>(0)?)?,
                        launch_input: row
                            .get::<_, Option<String>>(1)?
                            .map(parse_json_text)
                            .transpose()?,
                        created_at_ms: row.get(2)?,
                    })
                },
            )
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => RunStoreError::NotFound {
                    entity: "run_inputs",
                    key: run_id.to_owned(),
                },
                other => storage_error(other),
            })
    }

    pub fn append_run_event_record(&mut self, event: &StoredRunEvent) -> Result<(), RunStoreError> {
        let payload_json = serde_json::to_string(&event.payload).map_err(serialization_error)?;
        self.connection()
            .execute(
                r#"
                INSERT INTO run_events (
                    run_id, event_seq, event_kind, phase, boundary_kind, emitted_at_ms,
                    checkpoint_seq, revision, payload_json
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                "#,
                rusqlite::params![
                    event.run_id,
                    event.event_seq,
                    event.event_kind,
                    event.phase,
                    event.boundary_kind,
                    event.emitted_at_ms,
                    event.checkpoint_seq,
                    event.revision,
                    payload_json,
                ],
            )
            .map_err(storage_error)?;
        Ok(())
    }

    pub fn read_run_events(
        &self,
        query: StoredRunEventQuery,
    ) -> Result<StoredRunEventSlice, RunStoreError> {
        let latest_event_seq = self
            .connection()
            .query_row(
                "SELECT MAX(event_seq) FROM run_events WHERE run_id = ?1",
                [&query.run_id],
                |row| row.get::<_, Option<i64>>(0),
            )
            .map_err(storage_error)?;
        if latest_event_seq.is_none() {
            return Err(RunStoreError::NotFound {
                entity: "run_events",
                key: query.run_id,
            });
        }

        let mut records = if let Some(fetch_limit) = fetch_limit(query.limit) {
            let mut stmt = self
                .connection()
                .prepare(
                    r#"
                    SELECT
                        event_seq, event_kind, phase, boundary_kind, emitted_at_ms,
                        checkpoint_seq, revision, payload_json
                    FROM run_events
                    WHERE run_id = ?1
                      AND (?2 IS NULL OR event_seq > ?2)
                    ORDER BY event_seq ASC
                    LIMIT ?3
                    "#,
                )
                .map_err(storage_error)?;
            let rows = stmt
                .query_map(
                    rusqlite::params![query.run_id.clone(), query.after_event_seq, fetch_limit],
                    |row| {
                        Ok(StoredRunEvent {
                            run_id: query.run_id.clone(),
                            event_seq: row.get(0)?,
                            event_kind: row.get(1)?,
                            phase: row.get(2)?,
                            boundary_kind: row.get(3)?,
                            emitted_at_ms: row.get(4)?,
                            checkpoint_seq: row.get(5)?,
                            revision: row.get(6)?,
                            payload: parse_json_text(row.get::<_, String>(7)?)?,
                        })
                    },
                )
                .map_err(storage_error)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(storage_error)?
        } else {
            let mut stmt = self
                .connection()
                .prepare(
                    r#"
                    SELECT
                        event_seq, event_kind, phase, boundary_kind, emitted_at_ms,
                        checkpoint_seq, revision, payload_json
                    FROM run_events
                    WHERE run_id = ?1
                      AND (?2 IS NULL OR event_seq > ?2)
                    ORDER BY event_seq ASC
                    "#,
                )
                .map_err(storage_error)?;
            let rows = stmt
                .query_map(
                    rusqlite::params![query.run_id.clone(), query.after_event_seq],
                    |row| {
                        Ok(StoredRunEvent {
                            run_id: query.run_id.clone(),
                            event_seq: row.get(0)?,
                            event_kind: row.get(1)?,
                            phase: row.get(2)?,
                            boundary_kind: row.get(3)?,
                            emitted_at_ms: row.get(4)?,
                            checkpoint_seq: row.get(5)?,
                            revision: row.get(6)?,
                            payload: parse_json_text(row.get::<_, String>(7)?)?,
                        })
                    },
                )
                .map_err(storage_error)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(storage_error)?
        };

        let truncated = query
            .limit
            .map(|limit| records.len() > limit)
            .unwrap_or(false);
        if let Some(limit) = query.limit.filter(|_| truncated) {
            records.truncate(limit);
        }
        let next_after_event_seq = records.last().map(|record| record.event_seq);

        Ok(StoredRunEventSlice {
            latest_event_seq,
            next_after_event_seq,
            truncated,
            records,
        })
    }

    pub fn append_run_audit_record(&mut self, audit: &StoredRunAudit) -> Result<(), RunStoreError> {
        let payload_json = serde_json::to_string(&audit.payload).map_err(serialization_error)?;
        self.connection()
            .execute(
                r#"
                INSERT INTO run_audits (
                    run_id, audit_seq, audit_kind, decision_class, emitted_at_ms,
                    checkpoint_seq, revision, payload_json
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                "#,
                rusqlite::params![
                    audit.run_id,
                    audit.audit_seq,
                    audit.audit_kind,
                    audit.decision_class,
                    audit.emitted_at_ms,
                    audit.checkpoint_seq,
                    audit.revision,
                    payload_json,
                ],
            )
            .map_err(storage_error)?;
        Ok(())
    }

    pub fn read_run_audits(
        &self,
        query: StoredRunAuditQuery,
    ) -> Result<StoredRunAuditSlice, RunStoreError> {
        let latest_audit_seq = self
            .connection()
            .query_row(
                "SELECT MAX(audit_seq) FROM run_audits WHERE run_id = ?1",
                [&query.run_id],
                |row| row.get::<_, Option<i64>>(0),
            )
            .map_err(storage_error)?;
        if latest_audit_seq.is_none() {
            return Err(RunStoreError::NotFound {
                entity: "run_audits",
                key: query.run_id,
            });
        }

        let mut records = if let Some(fetch_limit) = fetch_limit(query.limit) {
            let mut stmt = self
                .connection()
                .prepare(
                    r#"
                    SELECT
                        audit_seq, audit_kind, decision_class, emitted_at_ms,
                        checkpoint_seq, revision, payload_json
                    FROM run_audits
                    WHERE run_id = ?1
                      AND (?2 IS NULL OR audit_seq > ?2)
                    ORDER BY audit_seq ASC
                    LIMIT ?3
                    "#,
                )
                .map_err(storage_error)?;
            let rows = stmt
                .query_map(
                    rusqlite::params![query.run_id.clone(), query.after_audit_seq, fetch_limit],
                    |row| {
                        Ok(StoredRunAudit {
                            run_id: query.run_id.clone(),
                            audit_seq: row.get(0)?,
                            audit_kind: row.get(1)?,
                            decision_class: row.get(2)?,
                            emitted_at_ms: row.get(3)?,
                            checkpoint_seq: row.get(4)?,
                            revision: row.get(5)?,
                            payload: parse_json_text(row.get::<_, String>(6)?)?,
                        })
                    },
                )
                .map_err(storage_error)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(storage_error)?
        } else {
            let mut stmt = self
                .connection()
                .prepare(
                    r#"
                    SELECT
                        audit_seq, audit_kind, decision_class, emitted_at_ms,
                        checkpoint_seq, revision, payload_json
                    FROM run_audits
                    WHERE run_id = ?1
                      AND (?2 IS NULL OR audit_seq > ?2)
                    ORDER BY audit_seq ASC
                    "#,
                )
                .map_err(storage_error)?;
            let rows = stmt
                .query_map(
                    rusqlite::params![query.run_id.clone(), query.after_audit_seq],
                    |row| {
                        Ok(StoredRunAudit {
                            run_id: query.run_id.clone(),
                            audit_seq: row.get(0)?,
                            audit_kind: row.get(1)?,
                            decision_class: row.get(2)?,
                            emitted_at_ms: row.get(3)?,
                            checkpoint_seq: row.get(4)?,
                            revision: row.get(5)?,
                            payload: parse_json_text(row.get::<_, String>(6)?)?,
                        })
                    },
                )
                .map_err(storage_error)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(storage_error)?
        };

        let truncated = query
            .limit
            .map(|limit| records.len() > limit)
            .unwrap_or(false);
        if let Some(limit) = query.limit.filter(|_| truncated) {
            records.truncate(limit);
        }
        let next_after_audit_seq = records.last().map(|record| record.audit_seq);

        Ok(StoredRunAuditSlice {
            latest_audit_seq,
            next_after_audit_seq,
            truncated,
            records,
        })
    }

    pub fn append_run_checkpoint_record(
        &mut self,
        checkpoint: &StoredRunCheckpoint,
    ) -> Result<StoredRunCheckpoint, RunStoreError> {
        let snapshot_json =
            serde_json::to_string(&checkpoint.snapshot).map_err(serialization_error)?;
        self.connection()
            .execute(
                r#"
                INSERT INTO run_checkpoints (
                    run_id, checkpoint_seq, plan_epoch, checkpoint_kind, retention_tier,
                    created_at_ms, is_terminal, is_side_effect_boundary,
                    is_recovery_boundary, is_first_wait_checkpoint, snapshot_json
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                ON CONFLICT(run_id, checkpoint_seq, plan_epoch) DO UPDATE SET
                    checkpoint_kind = excluded.checkpoint_kind,
                    retention_tier = excluded.retention_tier,
                    created_at_ms = excluded.created_at_ms,
                    is_terminal = excluded.is_terminal,
                    is_side_effect_boundary = excluded.is_side_effect_boundary,
                    is_recovery_boundary = excluded.is_recovery_boundary,
                    is_first_wait_checkpoint = excluded.is_first_wait_checkpoint,
                    snapshot_json = excluded.snapshot_json
                "#,
                rusqlite::params![
                    checkpoint.run_id,
                    checkpoint.checkpoint_seq,
                    checkpoint.plan_epoch,
                    checkpoint.checkpoint_kind,
                    checkpoint.retention_tier,
                    checkpoint.created_at_ms,
                    bool_to_i64(checkpoint.is_terminal),
                    bool_to_i64(checkpoint.is_side_effect_boundary),
                    bool_to_i64(checkpoint.is_recovery_boundary),
                    bool_to_i64(checkpoint.is_first_wait_checkpoint),
                    snapshot_json,
                ],
            )
            .map_err(storage_error)?;

        let checkpoint_id = self
            .connection()
            .query_row(
                r#"
                SELECT checkpoint_id
                FROM run_checkpoints
                WHERE run_id = ?1 AND checkpoint_seq = ?2 AND plan_epoch = ?3
                "#,
                rusqlite::params![
                    checkpoint.run_id,
                    checkpoint.checkpoint_seq,
                    checkpoint.plan_epoch
                ],
                |row| row.get::<_, i64>(0),
            )
            .map_err(storage_error)?;
        let mut inserted = checkpoint.clone();
        inserted.checkpoint_id = Some(checkpoint_id);
        Ok(inserted)
    }

    pub fn load_latest_run_checkpoint(
        &self,
        run_id: &str,
    ) -> Result<StoredRunCheckpoint, RunStoreError> {
        self.connection()
            .query_row(
                r#"
                SELECT
                    checkpoint_id, checkpoint_seq, plan_epoch, checkpoint_kind, retention_tier,
                    created_at_ms, is_terminal, is_side_effect_boundary,
                    is_recovery_boundary, is_first_wait_checkpoint, snapshot_json
                FROM run_checkpoints
                WHERE run_id = ?1
                ORDER BY checkpoint_seq DESC, plan_epoch DESC, checkpoint_id DESC
                LIMIT 1
                "#,
                [run_id],
                |row| checkpoint_from_row(run_id, row),
            )
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => RunStoreError::NotFound {
                    entity: "run_checkpoints",
                    key: run_id.to_owned(),
                },
                other => storage_error(other),
            })
    }

    pub fn upsert_run_wait_state(
        &mut self,
        wait_state: &StoredRunWaitState,
    ) -> Result<(), RunStoreError> {
        let state_json = serde_json::to_string(&wait_state.state).map_err(serialization_error)?;
        self.connection()
            .execute(
                r#"
                INSERT INTO run_wait_states (
                    run_id, wait_kind, request_id, entered_at_ms, expires_at_ms, state_json
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                ON CONFLICT(run_id) DO UPDATE SET
                    wait_kind = excluded.wait_kind,
                    request_id = excluded.request_id,
                    entered_at_ms = excluded.entered_at_ms,
                    expires_at_ms = excluded.expires_at_ms,
                    state_json = excluded.state_json
                "#,
                rusqlite::params![
                    wait_state.run_id,
                    wait_state.wait_kind,
                    wait_state.request_id,
                    wait_state.entered_at_ms,
                    wait_state.expires_at_ms,
                    state_json,
                ],
            )
            .map_err(storage_error)?;
        Ok(())
    }

    pub fn load_run_wait_state(&self, run_id: &str) -> Result<StoredRunWaitState, RunStoreError> {
        self.connection()
            .query_row(
                r#"
                SELECT wait_kind, request_id, entered_at_ms, expires_at_ms, state_json
                FROM run_wait_states
                WHERE run_id = ?1
                "#,
                [run_id],
                |row| {
                    Ok(StoredRunWaitState {
                        run_id: run_id.to_owned(),
                        wait_kind: row.get(0)?,
                        request_id: row.get(1)?,
                        entered_at_ms: row.get(2)?,
                        expires_at_ms: row.get(3)?,
                        state: parse_json_text(row.get::<_, String>(4)?)?,
                    })
                },
            )
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => RunStoreError::NotFound {
                    entity: "run_wait_states",
                    key: run_id.to_owned(),
                },
                other => storage_error(other),
            })
    }

    pub fn clear_run_wait_state(&mut self, run_id: &str) -> Result<(), RunStoreError> {
        self.connection()
            .execute("DELETE FROM run_wait_states WHERE run_id = ?1", [run_id])
            .map_err(storage_error)?;
        Ok(())
    }

    pub fn upsert_run_claim_record(&mut self, claim: &StoredRunClaim) -> Result<(), RunStoreError> {
        self.connection()
            .execute(
                r#"
                INSERT INTO run_claim_history (
                    claim_id, run_id, host_session_id, owner_kind, owner_instance_id,
                    lease_started_at_ms, lease_expires_at_ms, last_renewed_at_ms,
                    claim_epoch, mode, status
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                ON CONFLICT(claim_id) DO UPDATE SET
                    run_id = excluded.run_id,
                    host_session_id = excluded.host_session_id,
                    owner_kind = excluded.owner_kind,
                    owner_instance_id = excluded.owner_instance_id,
                    lease_started_at_ms = excluded.lease_started_at_ms,
                    lease_expires_at_ms = excluded.lease_expires_at_ms,
                    last_renewed_at_ms = excluded.last_renewed_at_ms,
                    claim_epoch = excluded.claim_epoch,
                    mode = excluded.mode,
                    status = excluded.status
                "#,
                rusqlite::params![
                    claim.claim_id,
                    claim.run_id,
                    claim.host_session_id,
                    claim.owner_kind,
                    claim.owner_instance_id,
                    claim.lease_started_at_ms,
                    claim.lease_expires_at_ms,
                    claim.last_renewed_at_ms,
                    claim.claim_epoch,
                    claim.mode,
                    claim.status,
                ],
            )
            .map_err(storage_error)?;
        Ok(())
    }

    pub fn load_latest_run_claim_for_run(
        &self,
        run_id: &str,
    ) -> Result<StoredRunClaim, RunStoreError> {
        self.connection()
            .query_row(
                r#"
                SELECT
                    claim_id, host_session_id, owner_kind, owner_instance_id,
                    lease_started_at_ms, lease_expires_at_ms, last_renewed_at_ms,
                    claim_epoch, mode, status
                FROM run_claim_history
                WHERE run_id = ?1
                ORDER BY claim_epoch DESC
                LIMIT 1
                "#,
                [run_id],
                |row| claim_from_row(run_id, row),
            )
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => RunStoreError::NotFound {
                    entity: "run_claim_history",
                    key: run_id.to_owned(),
                },
                other => storage_error(other),
            })
    }

    pub fn load_active_run_claim_for_run(
        &self,
        run_id: &str,
    ) -> Result<Option<StoredRunClaim>, RunStoreError> {
        self.connection()
            .query_row(
                r#"
                SELECT
                    claim_id, host_session_id, owner_kind, owner_instance_id,
                    lease_started_at_ms, lease_expires_at_ms, last_renewed_at_ms,
                    claim_epoch, mode, status
                FROM run_claim_history
                WHERE run_id = ?1 AND status = 'active'
                ORDER BY claim_epoch DESC
                LIMIT 1
                "#,
                [run_id],
                |row| claim_from_row(run_id, row),
            )
            .optional()
            .map_err(storage_error)
    }
}

fn fetch_limit(limit: Option<usize>) -> Option<i64> {
    let limit = limit?;
    let fetch_limit = limit.checked_add(1)?;
    i64::try_from(fetch_limit).ok()
}

fn checkpoint_from_row(
    run_id: &str,
    row: &rusqlite::Row<'_>,
) -> Result<StoredRunCheckpoint, rusqlite::Error> {
    Ok(StoredRunCheckpoint {
        checkpoint_id: row.get(0)?,
        run_id: run_id.to_owned(),
        checkpoint_seq: row.get(1)?,
        plan_epoch: row.get(2)?,
        checkpoint_kind: row.get(3)?,
        retention_tier: row.get(4)?,
        created_at_ms: row.get(5)?,
        is_terminal: row.get::<_, i64>(6)? != 0,
        is_side_effect_boundary: row.get::<_, i64>(7)? != 0,
        is_recovery_boundary: row.get::<_, i64>(8)? != 0,
        is_first_wait_checkpoint: row.get::<_, i64>(9)? != 0,
        snapshot: parse_json_text(row.get::<_, String>(10)?)?,
    })
}

fn claim_from_row(
    run_id: &str,
    row: &rusqlite::Row<'_>,
) -> Result<StoredRunClaim, rusqlite::Error> {
    Ok(StoredRunClaim {
        claim_id: row.get(0)?,
        run_id: run_id.to_owned(),
        host_session_id: row.get(1)?,
        owner_kind: row.get(2)?,
        owner_instance_id: row.get(3)?,
        lease_started_at_ms: row.get(4)?,
        lease_expires_at_ms: row.get(5)?,
        last_renewed_at_ms: row.get(6)?,
        claim_epoch: row.get(7)?,
        mode: row.get(8)?,
        status: row.get(9)?,
    })
}

fn bool_to_i64(value: bool) -> i64 {
    if value {
        1
    } else {
        0
    }
}

fn parse_json_text(json: String) -> Result<Value, rusqlite::Error> {
    serde_json::from_str::<Value>(&json).map_err(deser_error)
}

fn storage_error(error: impl ToString) -> RunStoreError {
    RunStoreError::Storage {
        message: error.to_string(),
    }
}

fn serialization_error(error: impl ToString) -> RunStoreError {
    RunStoreError::Serialization {
        message: error.to_string(),
    }
}

fn deser_error(error: impl ToString) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        0,
        rusqlite::types::Type::Text,
        Box::<dyn std::error::Error + Send + Sync>::from(error.to_string()),
    )
}
