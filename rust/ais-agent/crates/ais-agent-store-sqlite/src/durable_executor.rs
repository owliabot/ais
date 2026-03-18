use ais_agent_core::mission::Mission;
use ais_agent_runtime::persistence::{
    DurableCommitError, DurableCommitReceipt, DurableMutationExecutor, DurableMutationMember,
    DurableMutationUnit, MissionWriteMode, RunWaitStateWrite,
};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{run_projection, SqliteStore};

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
        let mutation_emitted_at_ms = current_time_ms();
        let mutation_revision =
            i64::try_from(unit.catalog_write.entry.latest_revision).map_err(|error| {
                DurableCommitError::Transaction {
                    run_id: unit.run_id.0.clone(),
                    phase: "metadata".to_owned(),
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

        run_projection::append_checkpoint(
            &tx,
            &unit.checkpoint_write.entry.snapshot,
            unit.checkpoint_write.entry.kind,
        )
        .map_err(|error| member_write_error(&unit, DurableMutationMember::Checkpoint, error))?;

        for event in &unit.event_write.events {
            run_projection::append_event_with_metadata(
                &tx,
                event,
                mutation_emitted_at_ms,
                Some(mutation_revision),
            )
            .map_err(|error| member_write_error(&unit, DurableMutationMember::Event, error))?;
        }

        run_projection::upsert_run_head(
            &tx,
            &unit.catalog_write.entry,
            unit.audit_write
                .records
                .last()
                .map(|record| record.audit_seq as i64),
            None,
        )
        .map_err(|error| member_write_error(&unit, DurableMutationMember::Catalog, error))?;

        if let Some(write) = unit.wait_state_write.as_ref() {
            let result = match write {
                RunWaitStateWrite::Upsert { wait_state } => {
                    run_projection::upsert_wait_state_record(&tx, wait_state)
                }
                RunWaitStateWrite::Clear { run_id } => {
                    run_projection::clear_wait_state(&tx, &run_id.0)
                }
            };
            result.map_err(|error| {
                member_write_error(&unit, DurableMutationMember::WaitState, error)
            })?;
        }

        for record in &unit.audit_write.records {
            run_projection::append_audit_with_metadata(
                &tx,
                record,
                mutation_emitted_at_ms,
                Some(mutation_revision),
            )
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
        "INSERT OR IGNORE INTO run_inputs (run_id, mission_json, launch_input_json, created_at_ms) VALUES (?1, ?2, NULL, NULL)",
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
        "INSERT INTO run_inputs (run_id, mission_json, launch_input_json, created_at_ms) VALUES (?1, ?2, NULL, NULL)
         ON CONFLICT(run_id) DO UPDATE SET mission_json = excluded.mission_json",
        (run_id, &mission_json),
    )?;
    Ok(())
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
