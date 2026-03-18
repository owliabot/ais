use ais_agent_control::{
    audit::{RuntimeAudit, RuntimeAuditRecord},
    events::{RunEvent, RunEventEnvelope},
};
use ais_agent_core::checkpoint::CheckpointSnapshot;
use ais_agent_runtime::persistence::{CheckpointArchiveKind, RunCatalogEntry, RunWaitStateRecord};
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) fn append_checkpoint(
    conn: &rusqlite::Connection,
    snapshot: &CheckpointSnapshot,
    kind: CheckpointArchiveKind,
) -> Result<(), rusqlite::Error> {
    let snapshot_json = serde_json::to_string(snapshot).map_err(to_sqlite_error)?;
    conn.execute(
        r#"
        INSERT INTO run_checkpoints (
            run_id,
            checkpoint_seq,
            plan_epoch,
            checkpoint_kind,
            retention_tier,
            created_at_ms,
            is_terminal,
            is_side_effect_boundary,
            is_recovery_boundary,
            is_first_wait_checkpoint,
            snapshot_json
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
        "#,
        rusqlite::params![
            snapshot.run_id,
            snapshot.checkpoint_seq,
            snapshot.plan_epoch,
            checkpoint_kind_string(kind),
            checkpoint_retention_tier(snapshot, kind),
            0i64,
            bool_to_i64(is_terminal_status(&snapshot.lifecycle.status)),
            bool_to_i64(matches!(kind, CheckpointArchiveKind::SideEffect)),
            bool_to_i64(matches!(
                snapshot.lifecycle.phase,
                ais_agent_core::runtime::RunPhase::Recovering
            )),
            0i64,
            snapshot_json,
        ],
    )?;
    Ok(())
}

pub(crate) fn append_event(
    conn: &rusqlite::Connection,
    event: &RunEventEnvelope,
) -> Result<(), rusqlite::Error> {
    append_event_with_metadata(
        conn,
        event,
        current_time_ms(),
        Some(i64::try_from(event.checkpoint_seq).map_err(to_sqlite_error)?),
    )
}

pub(crate) fn append_event_with_metadata(
    conn: &rusqlite::Connection,
    event: &RunEventEnvelope,
    emitted_at_ms: i64,
    revision: Option<i64>,
) -> Result<(), rusqlite::Error> {
    let payload_json = serde_json::to_string(event).map_err(to_sqlite_error)?;
    conn.execute(
        r#"
        INSERT INTO run_events (
            run_id,
            event_seq,
            event_kind,
            phase,
            boundary_kind,
            emitted_at_ms,
            checkpoint_seq,
            revision,
            payload_json
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
        "#,
        rusqlite::params![
            event.run_id.0,
            event.event_seq,
            event.descriptor().event_type,
            event_phase(event),
            event_boundary_kind(event),
            emitted_at_ms,
            i64::try_from(event.checkpoint_seq).map_err(to_sqlite_error)?,
            revision,
            payload_json,
        ],
    )?;
    Ok(())
}

pub(crate) fn upsert_run_head(
    conn: &rusqlite::Connection,
    entry: &RunCatalogEntry,
    latest_audit_seq: Option<i64>,
    latest_claim_epoch: Option<i64>,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        r#"
        INSERT INTO runs (
            run_id,
            mission_id,
            status,
            phase,
            active_boundary_kind,
            active_wait_kind,
            latest_checkpoint_seq,
            latest_event_seq,
            latest_audit_seq,
            latest_claim_epoch,
            retention_mode,
            created_at_ms,
            updated_at_ms,
            terminal_at_ms
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
        ON CONFLICT(run_id) DO UPDATE SET
            mission_id = excluded.mission_id,
            status = excluded.status,
            phase = excluded.phase,
            active_boundary_kind = excluded.active_boundary_kind,
            active_wait_kind = excluded.active_wait_kind,
            latest_checkpoint_seq = excluded.latest_checkpoint_seq,
            latest_event_seq = excluded.latest_event_seq,
            latest_audit_seq = COALESCE(excluded.latest_audit_seq, runs.latest_audit_seq),
            latest_claim_epoch = COALESCE(excluded.latest_claim_epoch, runs.latest_claim_epoch),
            retention_mode = excluded.retention_mode,
            created_at_ms = excluded.created_at_ms,
            updated_at_ms = excluded.updated_at_ms,
            terminal_at_ms = excluded.terminal_at_ms
        "#,
        rusqlite::params![
            entry.run_id.0,
            entry.mission_id,
            enum_as_string(&entry.status)?,
            enum_as_string(&entry.phase)?,
            entry
                .active_boundary_kind
                .as_ref()
                .map(enum_as_string)
                .transpose()?,
            active_wait_kind(entry.active_boundary_kind.as_ref()),
            i64::try_from(entry.latest_checkpoint_seq).map_err(to_sqlite_error)?,
            entry
                .latest_event_seq
                .map(i64::try_from)
                .transpose()
                .map_err(to_sqlite_error)?,
            latest_audit_seq,
            latest_claim_epoch,
            run_retention_mode(&entry.status),
            entry.created_at_ms.map(|value| value as i64),
            entry.updated_at_ms.map(|value| value as i64),
            entry.terminal_at_ms.map(|value| value as i64),
        ],
    )?;
    if is_terminal_status(&entry.status) {
        retier_terminal_checkpoints(
            conn,
            &entry.run_id.0,
            i64::try_from(entry.latest_checkpoint_seq).map_err(to_sqlite_error)?,
        )?;
    }
    Ok(())
}

pub(crate) fn upsert_wait_state_record(
    conn: &rusqlite::Connection,
    wait_state: &RunWaitStateRecord,
) -> Result<(), rusqlite::Error> {
    let state_json = serde_json::to_string(&wait_state.state).map_err(to_sqlite_error)?;
    conn.execute(
        r#"
        INSERT INTO run_wait_states (
            run_id,
            wait_kind,
            request_id,
            entered_at_ms,
            expires_at_ms,
            state_json
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        ON CONFLICT(run_id) DO UPDATE SET
            wait_kind = excluded.wait_kind,
            request_id = excluded.request_id,
            entered_at_ms = excluded.entered_at_ms,
            expires_at_ms = excluded.expires_at_ms,
            state_json = excluded.state_json
        "#,
        rusqlite::params![
            wait_state.run_id.0,
            wait_state.wait_kind,
            wait_state.request_id,
            wait_state.entered_at_ms as i64,
            wait_state.expires_at_ms.map(|value| value as i64),
            state_json,
        ],
    )?;
    Ok(())
}

pub(crate) fn clear_wait_state(
    conn: &rusqlite::Connection,
    run_id: &str,
) -> Result<(), rusqlite::Error> {
    conn.execute("DELETE FROM run_wait_states WHERE run_id = ?1", [run_id])?;
    Ok(())
}

pub(crate) fn append_audit(
    conn: &rusqlite::Connection,
    record: &RuntimeAuditRecord,
) -> Result<(), rusqlite::Error> {
    append_audit_with_metadata(
        conn,
        record,
        current_time_ms(),
        Some(i64::try_from(record.checkpoint_seq).map_err(to_sqlite_error)?),
    )
}

pub(crate) fn append_audit_with_metadata(
    conn: &rusqlite::Connection,
    record: &RuntimeAuditRecord,
    emitted_at_ms: i64,
    revision: Option<i64>,
) -> Result<(), rusqlite::Error> {
    let payload_json = serde_json::to_string(record).map_err(to_sqlite_error)?;
    conn.execute(
        r#"
        INSERT INTO run_audits (
            run_id,
            audit_seq,
            audit_kind,
            decision_class,
            emitted_at_ms,
            checkpoint_seq,
            revision,
            payload_json
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
        rusqlite::params![
            record.run_id.0,
            record.audit_seq,
            runtime_audit_kind(record),
            runtime_audit_decision_class(record)?,
            emitted_at_ms,
            i64::try_from(record.checkpoint_seq).map_err(to_sqlite_error)?,
            revision,
            payload_json,
        ],
    )?;
    Ok(())
}

pub(crate) fn update_run_latest_audit_seq(
    conn: &rusqlite::Connection,
    run_id: &str,
    audit_seq: i64,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "UPDATE runs SET latest_audit_seq = ?2 WHERE run_id = ?1",
        rusqlite::params![run_id, audit_seq],
    )?;
    Ok(())
}

pub(crate) fn update_run_latest_claim_epoch(
    conn: &rusqlite::Connection,
    run_id: &str,
    claim_epoch: i64,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "UPDATE runs SET latest_claim_epoch = ?2 WHERE run_id = ?1",
        rusqlite::params![run_id, claim_epoch],
    )?;
    Ok(())
}

fn enum_as_string<T: serde::Serialize>(value: &T) -> Result<String, rusqlite::Error> {
    match serde_json::to_value(value).map_err(to_sqlite_error)? {
        serde_json::Value::String(value) => Ok(value),
        other => Err(to_sqlite_error(format!(
            "expected enum string representation, got {other}"
        ))),
    }
}

fn active_wait_kind(
    boundary_kind: Option<&ais_agent_core::runtime::BoundaryKind>,
) -> Option<&'static str> {
    match boundary_kind {
        Some(ais_agent_core::runtime::BoundaryKind::Pause) => Some("pause"),
        Some(ais_agent_core::runtime::BoundaryKind::Evidence) => Some("evidence"),
        Some(ais_agent_core::runtime::BoundaryKind::Signer) => Some("signer"),
        Some(ais_agent_core::runtime::BoundaryKind::Confirmation) => Some("confirmation"),
        Some(ais_agent_core::runtime::BoundaryKind::ArtifactContinuation) => {
            Some("artifact_continuation")
        }
        Some(ais_agent_core::runtime::BoundaryKind::Completion)
        | Some(ais_agent_core::runtime::BoundaryKind::Failure)
        | Some(ais_agent_core::runtime::BoundaryKind::Cancellation)
        | None => None,
    }
}

fn checkpoint_kind_string(kind: CheckpointArchiveKind) -> &'static str {
    match kind {
        CheckpointArchiveKind::Boundary => "boundary",
        CheckpointArchiveKind::Progress => "progress",
        CheckpointArchiveKind::SideEffect => "side_effect",
    }
}

fn checkpoint_retention_tier(
    snapshot: &CheckpointSnapshot,
    kind: CheckpointArchiveKind,
) -> &'static str {
    if is_terminal_status(&snapshot.lifecycle.status) {
        "terminal_intermediate"
    } else if matches!(kind, CheckpointArchiveKind::SideEffect) {
        "terminal_boundary"
    } else {
        "active_full"
    }
}

fn retier_terminal_checkpoints(
    conn: &rusqlite::Connection,
    run_id: &str,
    latest_checkpoint_seq: i64,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        r#"
        UPDATE run_checkpoints
        SET retention_tier = CASE
            WHEN checkpoint_seq = ?2 THEN 'terminal_final'
            WHEN is_side_effect_boundary != 0
              OR is_recovery_boundary != 0
              OR is_first_wait_checkpoint != 0
            THEN 'terminal_boundary'
            ELSE 'terminal_intermediate'
        END
        WHERE run_id = ?1
          AND checkpoint_seq <= ?2
        "#,
        rusqlite::params![run_id, latest_checkpoint_seq],
    )?;
    Ok(())
}

fn run_retention_mode(status: &ais_agent_core::runtime::RunStatus) -> &'static str {
    if is_terminal_status(status) {
        "terminal_tiered"
    } else {
        "active_full"
    }
}

fn is_terminal_status(status: &ais_agent_core::runtime::RunStatus) -> bool {
    matches!(
        status,
        ais_agent_core::runtime::RunStatus::Completed
            | ais_agent_core::runtime::RunStatus::Failed
            | ais_agent_core::runtime::RunStatus::Cancelled
    )
}

fn event_phase(event: &RunEventEnvelope) -> Option<String> {
    match &event.event {
        RunEvent::Started(payload) => Some(payload.phase.clone()),
        RunEvent::Progress(payload) => Some(payload.phase.clone()),
        _ => None,
    }
}

fn event_boundary_kind(event: &RunEventEnvelope) -> Option<&'static str> {
    match &event.event {
        RunEvent::Paused(_) => Some("pause"),
        RunEvent::AwaitingEvidence(_) => Some("evidence"),
        RunEvent::AwaitingConfirm(_) => Some("confirmation"),
        RunEvent::AwaitingSigner(_) => Some("signer"),
        RunEvent::AwaitingContinuation(_) => Some("artifact_continuation"),
        RunEvent::Completed(_) => Some("completion"),
        RunEvent::Failed(_) => Some("failure"),
        _ => None,
    }
}

fn runtime_audit_kind(record: &RuntimeAuditRecord) -> &'static str {
    match &record.audit {
        RuntimeAudit::Recovery(_) => "recovery",
        RuntimeAudit::GovernorDecision(_) => "governor_decision",
        RuntimeAudit::PlanPatch(_) => "plan_patch",
        RuntimeAudit::Cancellation(_) => "cancellation",
        RuntimeAudit::Interruption(_) => "interruption",
        RuntimeAudit::DurableCommit(_) => "durable_commit",
    }
}

fn runtime_audit_decision_class(
    record: &RuntimeAuditRecord,
) -> Result<Option<String>, rusqlite::Error> {
    match &record.audit {
        RuntimeAudit::GovernorDecision(payload) => Ok(Some(enum_as_string(&payload.decision)?)),
        _ => Ok(None),
    }
}

fn bool_to_i64(value: bool) -> i64 {
    if value {
        1
    } else {
        0
    }
}

fn to_sqlite_error(error: impl ToString) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::<dyn std::error::Error + Send + Sync>::from(
        error.to_string(),
    ))
}

fn current_time_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after UNIX_EPOCH")
        .as_millis() as i64
}
