use ais_agent_control::ids::RunId;
use ais_agent_runtime::persistence::{
    RunWaitStateRecord, RunWaitStateStore, RunWaitStateStoreError,
};

use crate::{run_projection, SqliteStore};

impl RunWaitStateStore for SqliteStore {
    fn upsert_wait_state(
        &mut self,
        wait_state: RunWaitStateRecord,
    ) -> Result<(), RunWaitStateStoreError> {
        run_projection::upsert_wait_state_record(self.connection(), &wait_state)
            .map_err(storage_error)?;
        Ok(())
    }

    fn load_wait_state(
        &self,
        run_id: &RunId,
    ) -> Result<RunWaitStateRecord, RunWaitStateStoreError> {
        let wait_state = self
            .load_run_wait_state(&run_id.0)
            .map_err(|error| match error {
                crate::RunStoreError::NotFound { .. } => RunWaitStateStoreError::NotFound {
                    run_id: run_id.0.clone(),
                },
                other => RunWaitStateStoreError::Storage {
                    message: other.to_string(),
                },
            })?;
        Ok(RunWaitStateRecord {
            run_id: RunId(wait_state.run_id),
            wait_kind: wait_state.wait_kind,
            request_id: wait_state.request_id,
            entered_at_ms: wait_state.entered_at_ms as u64,
            expires_at_ms: wait_state.expires_at_ms.map(|value| value as u64),
            state: wait_state.state,
        })
    }

    fn clear_wait_state(&mut self, run_id: &RunId) -> Result<(), RunWaitStateStoreError> {
        run_projection::clear_wait_state(self.connection(), &run_id.0).map_err(storage_error)?;
        Ok(())
    }
}

fn storage_error(error: impl ToString) -> RunWaitStateStoreError {
    RunWaitStateStoreError::Storage {
        message: error.to_string(),
    }
}
