use ais_agent_control::{audit::RuntimeAuditRecord, ids::RunId};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeAuditQuery {
    pub run_id: RunId,
    pub after_audit_seq: Option<u64>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeAuditSlice {
    pub run_id: RunId,
    pub after_audit_seq: Option<u64>,
    pub latest_audit_seq: Option<u64>,
    pub next_after_audit_seq: Option<u64>,
    pub truncated: bool,
    pub records: Vec<RuntimeAuditRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RuntimeAuditArchiveError {
    #[error("audit archive not found for run `{run_id}`")]
    NotFound { run_id: String },
    #[error("audit archive storage error: {message}")]
    Storage { message: String },
}

pub trait RuntimeAuditArchive {
    fn append(&mut self, record: RuntimeAuditRecord) -> Result<(), RuntimeAuditArchiveError>;

    fn read(&self, query: RuntimeAuditQuery)
        -> Result<RuntimeAuditSlice, RuntimeAuditArchiveError>;
}

impl<T> RuntimeAuditArchive for &mut T
where
    T: RuntimeAuditArchive + ?Sized,
{
    fn append(&mut self, record: RuntimeAuditRecord) -> Result<(), RuntimeAuditArchiveError> {
        (**self).append(record)
    }

    fn read(
        &self,
        query: RuntimeAuditQuery,
    ) -> Result<RuntimeAuditSlice, RuntimeAuditArchiveError> {
        (**self).read(query)
    }
}
