use ais_agent_control::audit::RuntimeAuditRecord;
use ais_agent_core::{
    checkpoint::CheckpointSnapshot, mission::Mission, runtime::SignerRequestState,
};
use ais_agent_runtime::persistence::{
    DurableCommitError, DurableCommitReceipt, DurableMutationExecutor, DurableMutationMember,
    DurableMutationUnit, MissionWriteMode, SignerStateWrite,
};

use crate::SqliteStore;

impl DurableMutationExecutor for SqliteStore {
    fn commit(
        &mut self,
        unit: DurableMutationUnit,
    ) -> Result<DurableCommitReceipt, DurableCommitError> {
        ais_agent_runtime::persistence::validate_durable_mutation_unit(&unit)?;

        let tx = self.connection_mut().transaction().map_err(|error| {
            DurableCommitError::Transaction {
                run_id: unit.run_id.0.clone(),
                phase: "begin".to_owned(),
                message: error.to_string(),
            }
        })?;

        if let Some(write) = unit.mission_write.as_ref() {
            let result = match write.mode {
                MissionWriteMode::Insert => insert_mission(&tx, &write.run_id.0, &write.mission),
                MissionWriteMode::Upsert => upsert_mission(&tx, &write.run_id.0, &write.mission),
            };
            result.map_err(|error| {
                member_write_error(&unit, DurableMutationMember::Mission, error)
            })?;
        }

        append_checkpoint(
            &tx,
            &unit.checkpoint_write.entry.snapshot,
            unit.checkpoint_write.entry.kind,
        )
        .map_err(|error| member_write_error(&unit, DurableMutationMember::Checkpoint, error))?;

        for event in &unit.event_write.events {
            append_event(&tx, event)
                .map_err(|error| member_write_error(&unit, DurableMutationMember::Event, error))?;
        }

        upsert_run_catalog(&tx, &unit.catalog_write.entry)
            .map_err(|error| member_write_error(&unit, DurableMutationMember::Catalog, error))?;

        if let Some(write) = unit.signer_write.as_ref() {
            let result = match write {
                SignerStateWrite::Upsert { signer_state } => upsert_signer_state(&tx, signer_state),
                SignerStateWrite::Clear { run_id } => clear_signer_state(&tx, &run_id.0),
            };
            result
                .map_err(|error| member_write_error(&unit, DurableMutationMember::Signer, error))?;
        }

        for record in &unit.audit_write.records {
            append_runtime_audit(&tx, record)
                .map_err(|error| member_write_error(&unit, DurableMutationMember::Audit, error))?;
        }

        tx.commit()
            .map_err(|error| DurableCommitError::Transaction {
                run_id: unit.run_id.0.clone(),
                phase: "commit".to_owned(),
                message: error.to_string(),
            })?;

        Ok(DurableCommitReceipt {
            run_id: unit.run_id,
            kind: unit.kind,
            checkpoint_seq: unit.checkpoint_write.entry.snapshot.checkpoint_seq,
            plan_epoch: unit.checkpoint_write.entry.snapshot.plan_epoch,
            latest_event_seq: unit.event_write.events.last().map(|event| event.event_seq),
            latest_audit_seq: unit
                .audit_write
                .records
                .last()
                .map(|record| record.audit_seq),
        })
    }
}

fn member_write_error(
    unit: &DurableMutationUnit,
    member: DurableMutationMember,
    error: impl ToString,
) -> DurableCommitError {
    DurableCommitError::MemberWrite {
        run_id: unit.run_id.0.clone(),
        member,
        message: error.to_string(),
    }
}

fn insert_mission(
    tx: &rusqlite::Transaction<'_>,
    run_id: &str,
    mission: &Mission,
) -> Result<(), rusqlite::Error> {
    let mission_json = serde_json::to_string(mission).map_err(to_sqlite_error)?;
    let changed = tx.execute(
        "INSERT OR IGNORE INTO missions (run_id, mission_json) VALUES (?1, ?2)",
        (run_id, &mission_json),
    )?;
    if changed == 0 {
        return Err(rusqlite::Error::ExecuteReturnedResults);
    }
    Ok(())
}

fn upsert_mission(
    tx: &rusqlite::Transaction<'_>,
    run_id: &str,
    mission: &Mission,
) -> Result<(), rusqlite::Error> {
    let mission_json = serde_json::to_string(mission).map_err(to_sqlite_error)?;
    tx.execute(
        "INSERT INTO missions (run_id, mission_json) VALUES (?1, ?2)
         ON CONFLICT(run_id) DO UPDATE SET mission_json = excluded.mission_json",
        (run_id, &mission_json),
    )?;
    Ok(())
}

fn append_checkpoint(
    tx: &rusqlite::Transaction<'_>,
    snapshot: &CheckpointSnapshot,
    kind: ais_agent_runtime::persistence::CheckpointArchiveKind,
) -> Result<(), rusqlite::Error> {
    let snapshot_json = serde_json::to_string(snapshot).map_err(to_sqlite_error)?;
    let kind_json = serde_json::to_string(&kind).map_err(to_sqlite_error)?;
    tx.execute(
        r#"
        INSERT INTO checkpoint_archive (
            run_id,
            checkpoint_seq,
            plan_epoch,
            archive_kind_json,
            snapshot_json
        ) VALUES (?1, ?2, ?3, ?4, ?5)
        "#,
        rusqlite::params![
            snapshot.run_id,
            snapshot.checkpoint_seq,
            snapshot.plan_epoch,
            kind_json,
            snapshot_json,
        ],
    )?;
    Ok(())
}

fn append_event(
    tx: &rusqlite::Transaction<'_>,
    event: &ais_agent_control::events::RunEventEnvelope,
) -> Result<(), rusqlite::Error> {
    let event_json = serde_json::to_string(event).map_err(to_sqlite_error)?;
    tx.execute(
        r#"
        INSERT INTO event_archive (
            run_id,
            event_seq,
            checkpoint_seq,
            plan_epoch,
            event_json
        ) VALUES (?1, ?2, ?3, ?4, ?5)
        "#,
        rusqlite::params![
            event.run_id.0,
            event.event_seq,
            event.checkpoint_seq,
            event.plan_epoch,
            event_json,
        ],
    )?;
    Ok(())
}

fn upsert_run_catalog(
    tx: &rusqlite::Transaction<'_>,
    entry: &ais_agent_runtime::persistence::RunCatalogEntry,
) -> Result<(), rusqlite::Error> {
    let status_json = serde_json::to_string(&entry.status).map_err(to_sqlite_error)?;
    let phase_json = serde_json::to_string(&entry.phase).map_err(to_sqlite_error)?;
    let boundary_json = entry
        .active_boundary_kind
        .as_ref()
        .map(|kind| serde_json::to_string(kind).map_err(to_sqlite_error))
        .transpose()?;
    tx.execute(
        r#"
        INSERT INTO run_catalog (
            run_id,
            mission_id,
            status_json,
            phase_json,
            active_boundary_kind_json,
            latest_checkpoint_seq,
            latest_event_seq,
            latest_revision,
            created_at_ms,
            updated_at_ms,
            terminal_at_ms
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
        ON CONFLICT(run_id) DO UPDATE SET
            mission_id = excluded.mission_id,
            status_json = excluded.status_json,
            phase_json = excluded.phase_json,
            active_boundary_kind_json = excluded.active_boundary_kind_json,
            latest_checkpoint_seq = excluded.latest_checkpoint_seq,
            latest_event_seq = excluded.latest_event_seq,
            latest_revision = excluded.latest_revision,
            created_at_ms = excluded.created_at_ms,
            updated_at_ms = excluded.updated_at_ms,
            terminal_at_ms = excluded.terminal_at_ms
        "#,
        rusqlite::params![
            entry.run_id.0,
            entry.mission_id,
            status_json,
            phase_json,
            boundary_json,
            entry.latest_checkpoint_seq,
            entry.latest_event_seq,
            entry.latest_revision,
            entry.created_at_ms,
            entry.updated_at_ms,
            entry.terminal_at_ms,
        ],
    )?;
    Ok(())
}

fn upsert_signer_state(
    tx: &rusqlite::Transaction<'_>,
    signer_state: &SignerRequestState,
) -> Result<(), rusqlite::Error> {
    let signer_state_json = serde_json::to_string(signer_state).map_err(to_sqlite_error)?;
    tx.execute(
        r#"
        INSERT INTO signer_state_archive (
            run_id,
            request_id,
            signer_state_json
        ) VALUES (?1, ?2, ?3)
        ON CONFLICT(run_id) DO UPDATE SET
            request_id = excluded.request_id,
            signer_state_json = excluded.signer_state_json
        "#,
        rusqlite::params![
            signer_state.run_id.0,
            signer_state.request_id.0,
            signer_state_json,
        ],
    )?;
    Ok(())
}

fn clear_signer_state(tx: &rusqlite::Transaction<'_>, run_id: &str) -> Result<(), rusqlite::Error> {
    tx.execute(
        "DELETE FROM signer_state_archive WHERE run_id = ?1",
        [run_id],
    )?;
    Ok(())
}

fn append_runtime_audit(
    tx: &rusqlite::Transaction<'_>,
    record: &RuntimeAuditRecord,
) -> Result<(), rusqlite::Error> {
    let audit_json = serde_json::to_string(record).map_err(to_sqlite_error)?;
    tx.execute(
        r#"
        INSERT INTO runtime_audit_archive (
            run_id,
            audit_seq,
            checkpoint_seq,
            plan_epoch,
            audit_id,
            audit_json
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        "#,
        rusqlite::params![
            record.run_id.0,
            record.audit_seq,
            record.checkpoint_seq,
            record.plan_epoch,
            record.audit_id.0,
            audit_json,
        ],
    )?;
    Ok(())
}

fn to_sqlite_error(error: impl ToString) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::<dyn std::error::Error + Send + Sync>::from(
        error.to_string(),
    ))
}
