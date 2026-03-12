use ais_agent_control::ids::RunId;
use ais_agent_core::runtime::SignerRequestState;
use ais_agent_runtime::persistence::{SignerStateArchive, SignerStateArchiveError};

use crate::SqliteStore;

impl SignerStateArchive for SqliteStore {
    fn upsert(&mut self, signer_state: SignerRequestState) -> Result<(), SignerStateArchiveError> {
        let signer_state_json = serde_json::to_string(&signer_state).map_err(storage_error)?;
        self.connection()
            .execute(
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
            )
            .map_err(storage_error)?;
        Ok(())
    }

    fn load(&self, run_id: &RunId) -> Result<SignerRequestState, SignerStateArchiveError> {
        self.connection()
            .query_row(
                "SELECT signer_state_json FROM signer_state_archive WHERE run_id = ?1",
                [&run_id.0],
                |row| {
                    let json = row.get::<_, String>(0)?;
                    serde_json::from_str::<SignerRequestState>(&json).map_err(deser_error)
                },
            )
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => SignerStateArchiveError::NotFound {
                    run_id: run_id.0.clone(),
                },
                other => storage_error(other),
            })
    }

    fn clear(&mut self, run_id: &RunId) -> Result<(), SignerStateArchiveError> {
        self.connection()
            .execute(
                "DELETE FROM signer_state_archive WHERE run_id = ?1",
                [&run_id.0],
            )
            .map_err(storage_error)?;
        Ok(())
    }
}

fn storage_error(error: impl ToString) -> SignerStateArchiveError {
    SignerStateArchiveError::Storage {
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
