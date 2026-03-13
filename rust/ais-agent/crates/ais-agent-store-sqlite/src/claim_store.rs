use ais_agent_control::{
    ids::{ClaimId, RunId},
    ownership::{RunClaim, RunClaimMode, RunClaimOwnerKind, RunClaimStatus},
};
use ais_agent_runtime::persistence::{
    ClaimExpireRequest, ClaimReleaseRequest, ClaimRenewRequest, ClaimSupersedeRequest,
    ClaimSupersedeResult, RunClaimRepository, RunClaimRepositoryError,
};

use crate::SqliteStore;

impl RunClaimRepository for SqliteStore {
    fn acquire(&mut self, claim: RunClaim) -> Result<RunClaim, RunClaimRepositoryError> {
        validate_active_claim(&claim)?;
        let tx = self.connection_mut().transaction().map_err(storage_error)?;

        if let Some(existing) = load_active_from_tx(&tx, &claim.run_id)? {
            return Err(RunClaimRepositoryError::ActiveClaimConflict {
                run_id: claim.run_id.0.clone(),
                claim_id: existing.claim_id.0,
            });
        }

        insert_claim(&tx, &claim)?;
        tx.commit().map_err(storage_error)?;
        Ok(claim)
    }

    fn renew(&mut self, request: ClaimRenewRequest) -> Result<RunClaim, RunClaimRepositoryError> {
        let tx = self.connection_mut().transaction().map_err(storage_error)?;
        let current = load_claim_from_tx(&tx, &request.claim_id)?;
        assert_transition_preconditions(&current, &request.run_id, request.claim_epoch)?;

        let mut renewed = current.clone();
        renewed.last_renewed_at_ms = Some(request.renewed_at_ms);
        renewed.lease_expires_at_ms = request.lease_expires_at_ms;
        renewed.claim_epoch += 1;
        validate_active_claim(&renewed)?;

        update_claim(&tx, &renewed)?;
        tx.commit().map_err(storage_error)?;
        Ok(renewed)
    }

    fn release(
        &mut self,
        request: ClaimReleaseRequest,
    ) -> Result<RunClaim, RunClaimRepositoryError> {
        let tx = self.connection_mut().transaction().map_err(storage_error)?;
        let current = load_claim_from_tx(&tx, &request.claim_id)?;
        assert_transition_preconditions(&current, &request.run_id, request.claim_epoch)?;

        let mut released = current.clone();
        released.status = RunClaimStatus::Released;
        released.claim_epoch += 1;
        released
            .validate()
            .map_err(|message| RunClaimRepositoryError::InvalidClaim { message })?;

        update_claim(&tx, &released)?;
        tx.commit().map_err(storage_error)?;
        Ok(released)
    }

    fn load_active(&self, run_id: &RunId) -> Result<Option<RunClaim>, RunClaimRepositoryError> {
        load_active_from_conn(self.connection(), run_id)
    }

    fn load_latest_for_run(
        &self,
        run_id: &RunId,
    ) -> Result<Option<RunClaim>, RunClaimRepositoryError> {
        load_latest_for_run_from_conn(self.connection(), run_id)
    }

    fn load_claim(&self, claim_id: &ClaimId) -> Result<RunClaim, RunClaimRepositoryError> {
        load_claim_from_conn(self.connection(), claim_id)
    }

    fn expire_stale(
        &mut self,
        request: ClaimExpireRequest,
    ) -> Result<Option<RunClaim>, RunClaimRepositoryError> {
        let tx = self.connection_mut().transaction().map_err(storage_error)?;
        let Some(current) = load_active_from_tx(&tx, &request.run_id)? else {
            return Ok(None);
        };
        let Some(lease_expires_at_ms) = current.lease_expires_at_ms else {
            return Ok(None);
        };
        if lease_expires_at_ms > request.now_ms {
            return Ok(None);
        }

        let mut expired = current;
        expired.status = RunClaimStatus::Expired;
        expired.claim_epoch += 1;
        expired
            .validate()
            .map_err(|message| RunClaimRepositoryError::InvalidClaim { message })?;

        update_claim(&tx, &expired)?;
        tx.commit().map_err(storage_error)?;
        Ok(Some(expired))
    }

    fn supersede(
        &mut self,
        request: ClaimSupersedeRequest,
    ) -> Result<ClaimSupersedeResult, RunClaimRepositoryError> {
        validate_active_claim(&request.successor_claim)?;
        if request.successor_claim.run_id != request.run_id {
            return Err(RunClaimRepositoryError::InvalidClaim {
                message: "successor claim run_id does not match supersede request".to_owned(),
            });
        }

        let tx = self.connection_mut().transaction().map_err(storage_error)?;
        let current_active = load_active_from_tx(&tx, &request.run_id)?.ok_or_else(|| {
            RunClaimRepositoryError::ClaimNotFound {
                claim_id: request.predecessor_claim_id.0.clone(),
            }
        })?;
        if current_active.claim_id != request.predecessor_claim_id {
            return Err(RunClaimRepositoryError::ActiveClaimConflict {
                run_id: request.run_id.0.clone(),
                claim_id: current_active.claim_id.0,
            });
        }
        if current_active.claim_epoch != request.predecessor_claim_epoch {
            return Err(RunClaimRepositoryError::ClaimEpochConflict {
                claim_id: current_active.claim_id.0,
                expected_claim_epoch: request.predecessor_claim_epoch,
                actual_claim_epoch: current_active.claim_epoch,
            });
        }

        let mut predecessor = current_active;
        predecessor.status = RunClaimStatus::Superseded;
        predecessor.claim_epoch += 1;
        predecessor
            .validate()
            .map_err(|message| RunClaimRepositoryError::InvalidClaim { message })?;

        update_claim(&tx, &predecessor)?;
        insert_claim(&tx, &request.successor_claim)?;
        tx.commit().map_err(storage_error)?;

        Ok(ClaimSupersedeResult {
            predecessor,
            successor: request.successor_claim,
        })
    }
}

fn load_active_from_conn(
    conn: &rusqlite::Connection,
    run_id: &RunId,
) -> Result<Option<RunClaim>, RunClaimRepositoryError> {
    let mut stmt = conn
        .prepare(
            r#"
            SELECT
                claim_id,
                run_id,
                host_session_id,
                owner_kind_json,
                owner_instance_id,
                lease_started_at_ms,
                lease_expires_at_ms,
                last_renewed_at_ms,
                claim_epoch,
                mode_json,
                status_json
            FROM run_claims
            WHERE run_id = ?1
              AND status_json = '"active"'
            ORDER BY claim_epoch DESC
            LIMIT 1
            "#,
        )
        .map_err(storage_error)?;

    match stmt.query_row([&run_id.0], claim_from_row) {
        Ok(claim) => Ok(Some(claim)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(other) => Err(storage_error(other)),
    }
}

fn load_active_from_tx(
    tx: &rusqlite::Transaction<'_>,
    run_id: &RunId,
) -> Result<Option<RunClaim>, RunClaimRepositoryError> {
    let mut stmt = tx
        .prepare(
            r#"
            SELECT
                claim_id,
                run_id,
                host_session_id,
                owner_kind_json,
                owner_instance_id,
                lease_started_at_ms,
                lease_expires_at_ms,
                last_renewed_at_ms,
                claim_epoch,
                mode_json,
                status_json
            FROM run_claims
            WHERE run_id = ?1
              AND status_json = '"active"'
            ORDER BY claim_epoch DESC
            LIMIT 1
            "#,
        )
        .map_err(storage_error)?;

    match stmt.query_row([&run_id.0], claim_from_row) {
        Ok(claim) => Ok(Some(claim)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(other) => Err(storage_error(other)),
    }
}

fn load_claim_from_conn(
    conn: &rusqlite::Connection,
    claim_id: &ClaimId,
) -> Result<RunClaim, RunClaimRepositoryError> {
    conn.query_row(
        r#"
        SELECT
            claim_id,
            run_id,
            host_session_id,
            owner_kind_json,
            owner_instance_id,
            lease_started_at_ms,
            lease_expires_at_ms,
            last_renewed_at_ms,
            claim_epoch,
            mode_json,
            status_json
        FROM run_claims
        WHERE claim_id = ?1
        "#,
        [&claim_id.0],
        claim_from_row,
    )
    .map_err(|error| match error {
        rusqlite::Error::QueryReturnedNoRows => RunClaimRepositoryError::ClaimNotFound {
            claim_id: claim_id.0.clone(),
        },
        other => storage_error(other),
    })
}

fn load_latest_for_run_from_conn(
    conn: &rusqlite::Connection,
    run_id: &RunId,
) -> Result<Option<RunClaim>, RunClaimRepositoryError> {
    let mut stmt = conn
        .prepare(
            r#"
            SELECT
                claim_id,
                run_id,
                host_session_id,
                owner_kind_json,
                owner_instance_id,
                lease_started_at_ms,
                lease_expires_at_ms,
                last_renewed_at_ms,
                claim_epoch,
                mode_json,
                status_json
            FROM run_claims
            WHERE run_id = ?1
            ORDER BY lease_started_at_ms DESC, claim_epoch DESC, claim_id DESC
            LIMIT 1
            "#,
        )
        .map_err(storage_error)?;

    match stmt.query_row([&run_id.0], claim_from_row) {
        Ok(claim) => Ok(Some(claim)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(other) => Err(storage_error(other)),
    }
}

fn load_claim_from_tx(
    tx: &rusqlite::Transaction<'_>,
    claim_id: &ClaimId,
) -> Result<RunClaim, RunClaimRepositoryError> {
    tx.query_row(
        r#"
        SELECT
            claim_id,
            run_id,
            host_session_id,
            owner_kind_json,
            owner_instance_id,
            lease_started_at_ms,
            lease_expires_at_ms,
            last_renewed_at_ms,
            claim_epoch,
            mode_json,
            status_json
        FROM run_claims
        WHERE claim_id = ?1
        "#,
        [&claim_id.0],
        claim_from_row,
    )
    .map_err(|error| match error {
        rusqlite::Error::QueryReturnedNoRows => RunClaimRepositoryError::ClaimNotFound {
            claim_id: claim_id.0.clone(),
        },
        other => storage_error(other),
    })
}

fn assert_transition_preconditions(
    current: &RunClaim,
    run_id: &RunId,
    claim_epoch: u64,
) -> Result<(), RunClaimRepositoryError> {
    if &current.run_id != run_id {
        return Err(RunClaimRepositoryError::InvalidClaim {
            message: "claim transition run_id does not match existing claim".to_owned(),
        });
    }
    if current.status != RunClaimStatus::Active {
        return Err(RunClaimRepositoryError::InvalidStatus {
            claim_id: current.claim_id.0.clone(),
            status: current.status.clone(),
        });
    }
    if current.claim_epoch != claim_epoch {
        return Err(RunClaimRepositoryError::ClaimEpochConflict {
            claim_id: current.claim_id.0.clone(),
            expected_claim_epoch: claim_epoch,
            actual_claim_epoch: current.claim_epoch,
        });
    }
    Ok(())
}

fn insert_claim(
    tx: &rusqlite::Transaction<'_>,
    claim: &RunClaim,
) -> Result<(), RunClaimRepositoryError> {
    let owner_kind_json = serde_json::to_string(&claim.owner_kind).map_err(storage_error)?;
    let mode_json = serde_json::to_string(&claim.mode).map_err(storage_error)?;
    let status_json = serde_json::to_string(&claim.status).map_err(storage_error)?;
    tx.execute(
        r#"
        INSERT INTO run_claims (
            claim_id,
            run_id,
            host_session_id,
            owner_kind_json,
            owner_instance_id,
            lease_started_at_ms,
            lease_expires_at_ms,
            last_renewed_at_ms,
            claim_epoch,
            mode_json,
            status_json
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
        "#,
        rusqlite::params![
            claim.claim_id.0,
            claim.run_id.0,
            claim.host_session_id,
            owner_kind_json,
            claim.owner_instance_id,
            claim.lease_started_at_ms,
            claim.lease_expires_at_ms,
            claim.last_renewed_at_ms,
            claim.claim_epoch,
            mode_json,
            status_json,
        ],
    )
    .map_err(map_write_error(
        claim.run_id.0.clone(),
        claim.claim_id.0.clone(),
    ))?;
    Ok(())
}

fn update_claim(
    tx: &rusqlite::Transaction<'_>,
    claim: &RunClaim,
) -> Result<(), RunClaimRepositoryError> {
    let owner_kind_json = serde_json::to_string(&claim.owner_kind).map_err(storage_error)?;
    let mode_json = serde_json::to_string(&claim.mode).map_err(storage_error)?;
    let status_json = serde_json::to_string(&claim.status).map_err(storage_error)?;
    let changed = tx
        .execute(
            r#"
            UPDATE run_claims SET
                run_id = ?2,
                host_session_id = ?3,
                owner_kind_json = ?4,
                owner_instance_id = ?5,
                lease_started_at_ms = ?6,
                lease_expires_at_ms = ?7,
                last_renewed_at_ms = ?8,
                claim_epoch = ?9,
                mode_json = ?10,
                status_json = ?11
            WHERE claim_id = ?1
            "#,
            rusqlite::params![
                claim.claim_id.0,
                claim.run_id.0,
                claim.host_session_id,
                owner_kind_json,
                claim.owner_instance_id,
                claim.lease_started_at_ms,
                claim.lease_expires_at_ms,
                claim.last_renewed_at_ms,
                claim.claim_epoch,
                mode_json,
                status_json,
            ],
        )
        .map_err(storage_error)?;
    if changed == 0 {
        return Err(RunClaimRepositoryError::ClaimNotFound {
            claim_id: claim.claim_id.0.clone(),
        });
    }
    Ok(())
}

fn validate_active_claim(claim: &RunClaim) -> Result<(), RunClaimRepositoryError> {
    claim
        .validate()
        .map_err(|message| RunClaimRepositoryError::InvalidClaim { message })?;
    if claim.status != RunClaimStatus::Active {
        return Err(RunClaimRepositoryError::InvalidStatus {
            claim_id: claim.claim_id.0.clone(),
            status: claim.status.clone(),
        });
    }
    Ok(())
}

fn claim_from_row(row: &rusqlite::Row<'_>) -> Result<RunClaim, rusqlite::Error> {
    let owner_kind_json = row.get::<_, String>(3)?;
    let mode_json = row.get::<_, String>(9)?;
    let status_json = row.get::<_, String>(10)?;
    Ok(RunClaim {
        claim_id: ClaimId(row.get(0)?),
        run_id: RunId(row.get(1)?),
        host_session_id: row.get(2)?,
        owner_kind: serde_json::from_str::<RunClaimOwnerKind>(&owner_kind_json)
            .map_err(deser_error)?,
        owner_instance_id: row.get(4)?,
        lease_started_at_ms: row.get(5)?,
        lease_expires_at_ms: row.get(6)?,
        last_renewed_at_ms: row.get(7)?,
        claim_epoch: row.get(8)?,
        mode: serde_json::from_str::<RunClaimMode>(&mode_json).map_err(deser_error)?,
        status: serde_json::from_str::<RunClaimStatus>(&status_json).map_err(deser_error)?,
    })
}

fn map_write_error(
    run_id: String,
    claim_id: String,
) -> impl Fn(rusqlite::Error) -> RunClaimRepositoryError {
    move |error| match &error {
        rusqlite::Error::SqliteFailure(_, Some(message))
            if message.contains("idx_run_claims_active_by_run") =>
        {
            RunClaimRepositoryError::ActiveClaimConflict {
                run_id: run_id.clone(),
                claim_id: claim_id.clone(),
            }
        }
        _ => storage_error(error),
    }
}

fn storage_error(error: impl ToString) -> RunClaimRepositoryError {
    RunClaimRepositoryError::Storage {
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
