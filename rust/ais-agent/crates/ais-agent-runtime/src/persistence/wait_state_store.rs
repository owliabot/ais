use ais_agent_control::ids::RunId;
use ais_agent_core::runtime::SignerRequestState;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunWaitStateRecord {
    pub run_id: RunId,
    pub wait_kind: String,
    pub request_id: String,
    pub entered_at_ms: u64,
    pub expires_at_ms: Option<u64>,
    pub state: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RunWaitStateStoreError {
    #[error("wait state for run `{run_id}` not found")]
    NotFound { run_id: String },
    #[error(
        "wait state kind mismatch for run `{run_id}`: expected `{expected_wait_kind}`, found `{actual_wait_kind}`"
    )]
    WaitKindMismatch {
        run_id: String,
        expected_wait_kind: String,
        actual_wait_kind: String,
    },
    #[error("wait state store failed: {message}")]
    Storage { message: String },
}

pub trait RunWaitStateStore {
    fn upsert_wait_state(
        &mut self,
        wait_state: RunWaitStateRecord,
    ) -> Result<(), RunWaitStateStoreError>;

    fn load_wait_state(&self, run_id: &RunId)
        -> Result<RunWaitStateRecord, RunWaitStateStoreError>;

    fn clear_wait_state(&mut self, run_id: &RunId) -> Result<(), RunWaitStateStoreError>;
}

impl<T> RunWaitStateStore for &mut T
where
    T: RunWaitStateStore + ?Sized,
{
    fn upsert_wait_state(
        &mut self,
        wait_state: RunWaitStateRecord,
    ) -> Result<(), RunWaitStateStoreError> {
        (**self).upsert_wait_state(wait_state)
    }

    fn load_wait_state(
        &self,
        run_id: &RunId,
    ) -> Result<RunWaitStateRecord, RunWaitStateStoreError> {
        (**self).load_wait_state(run_id)
    }

    fn clear_wait_state(&mut self, run_id: &RunId) -> Result<(), RunWaitStateStoreError> {
        (**self).clear_wait_state(run_id)
    }
}

pub fn signer_state_into_wait_state_record(
    signer_state: SignerRequestState,
) -> Result<RunWaitStateRecord, RunWaitStateStoreError> {
    Ok(RunWaitStateRecord {
        run_id: signer_state.run_id.clone(),
        wait_kind: "signer".to_owned(),
        request_id: signer_state.request_id.0.clone(),
        entered_at_ms: signer_state
            .timeout
            .as_ref()
            .map(|timeout| timeout.requested_at_ms)
            .unwrap_or(0),
        expires_at_ms: signer_state
            .timeout
            .as_ref()
            .and_then(|timeout| timeout.expires_at_ms),
        state: serde_json::to_value(signer_state).map_err(|error| {
            RunWaitStateStoreError::Storage {
                message: error.to_string(),
            }
        })?,
    })
}

pub fn wait_state_record_into_signer_state(
    wait_state: RunWaitStateRecord,
) -> Result<SignerRequestState, RunWaitStateStoreError> {
    if wait_state.wait_kind != "signer" {
        return Err(RunWaitStateStoreError::WaitKindMismatch {
            run_id: wait_state.run_id.0,
            expected_wait_kind: "signer".to_owned(),
            actual_wait_kind: wait_state.wait_kind,
        });
    }
    serde_json::from_value(wait_state.state).map_err(|error| RunWaitStateStoreError::Storage {
        message: error.to_string(),
    })
}

pub trait SignerStateStore: RunWaitStateStore {
    fn upsert(&mut self, signer_state: SignerRequestState) -> Result<(), SignerStateStoreError> {
        self.upsert_wait_state(signer_state_into_wait_state_record(signer_state)?)
    }

    fn load(&self, run_id: &RunId) -> Result<SignerRequestState, SignerStateStoreError> {
        self.load_wait_state(run_id)
            .and_then(wait_state_record_into_signer_state)
    }

    fn clear(&mut self, run_id: &RunId) -> Result<(), SignerStateStoreError> {
        self.clear_wait_state(run_id)
    }
}

impl<T> SignerStateStore for T where T: RunWaitStateStore + ?Sized {}

pub type SignerStateStoreError = RunWaitStateStoreError;
