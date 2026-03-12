use ais_agent_control::ids::RunId;
use ais_agent_core::runtime::SignerRequestState;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SignerStateArchiveError {
    #[error("signer state for run `{run_id}` not found")]
    NotFound { run_id: String },
    #[error("signer state archive failed: {message}")]
    Storage { message: String },
}

pub trait SignerStateArchive {
    fn upsert(&mut self, signer_state: SignerRequestState) -> Result<(), SignerStateArchiveError>;

    fn load(&self, run_id: &RunId) -> Result<SignerRequestState, SignerStateArchiveError>;

    fn clear(&mut self, run_id: &RunId) -> Result<(), SignerStateArchiveError>;
}

impl<T> SignerStateArchive for &mut T
where
    T: SignerStateArchive + ?Sized,
{
    fn upsert(&mut self, signer_state: SignerRequestState) -> Result<(), SignerStateArchiveError> {
        (**self).upsert(signer_state)
    }

    fn load(&self, run_id: &RunId) -> Result<SignerRequestState, SignerStateArchiveError> {
        (**self).load(run_id)
    }

    fn clear(&mut self, run_id: &RunId) -> Result<(), SignerStateArchiveError> {
        (**self).clear(run_id)
    }
}
