//! Repository contract for durable run event archive state.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use ais_agent_control::{events::RunEventEnvelope, ids::RunId};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventArchiveQuery {
    pub run_id: RunId,
    pub after_event_seq: Option<u64>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventArchiveSlice {
    pub run_id: RunId,
    pub after_event_seq: Option<u64>,
    pub latest_event_seq: Option<u64>,
    pub next_after_event_seq: Option<u64>,
    pub truncated: bool,
    #[serde(default)]
    pub events: Vec<RunEventEnvelope>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EventArchiveError {
    #[error("event archive not found for run `{run_id}`")]
    NotFound { run_id: String },
    #[error("event archive storage error: {message}")]
    Storage { message: String },
}

pub trait EventArchive {
    fn append(&mut self, event: RunEventEnvelope) -> Result<(), EventArchiveError>;

    fn read(&self, query: EventArchiveQuery) -> Result<EventArchiveSlice, EventArchiveError>;
}

impl<T> EventArchive for &mut T
where
    T: EventArchive + ?Sized,
{
    fn append(&mut self, event: RunEventEnvelope) -> Result<(), EventArchiveError> {
        (**self).append(event)
    }

    fn read(&self, query: EventArchiveQuery) -> Result<EventArchiveSlice, EventArchiveError> {
        (**self).read(query)
    }
}
