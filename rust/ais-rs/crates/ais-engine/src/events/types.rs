use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub const ENGINE_EVENT_SCHEMA_0_0_3: &str = "ais-engine-event/0.0.3";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineEventType {
    PlanReady,
    NodeReady,
    NodeBlocked,
    NeedUserConfirm,
    QueryResult,
    TxPrepared,
    TxSent,
    TxConfirmed,
    NodeWaiting,
    CheckpointSaved,
    EnginePaused,
    Error,
    SolverApplied,
    NodePaused,
    Skipped,
    CommandAccepted,
    CommandRejected,
    PatchApplied,
    PatchRejected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EngineEvent {
    #[serde(rename = "type")]
    pub event_type: EngineEventType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    #[serde(default)]
    pub data: Map<String, Value>,
    #[serde(default)]
    pub extensions: Map<String, Value>,
}

impl EngineEvent {
    pub fn new(event_type: EngineEventType) -> Self {
        Self {
            event_type,
            node_id: None,
            data: Map::new(),
            extensions: Map::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EngineEventRecord {
    pub schema: String,
    pub run_id: String,
    pub seq: u64,
    pub ts: String,
    pub event: EngineEvent,
}

impl EngineEventRecord {
    pub fn new(run_id: impl Into<String>, seq: u64, ts: impl Into<String>, event: EngineEvent) -> Self {
        Self {
            schema: ENGINE_EVENT_SCHEMA_0_0_3.to_string(),
            run_id: run_id.into(),
            seq,
            ts: ts.into(),
            event,
        }
    }
}

#[derive(Debug, Clone)]
pub struct EngineEventStream {
    run_id: String,
    next_seq: u64,
}

impl EngineEventStream {
    pub fn new(run_id: impl Into<String>) -> Self {
        Self {
            run_id: run_id.into(),
            next_seq: 0,
        }
    }

    pub fn with_start_seq(run_id: impl Into<String>, start_seq: u64) -> Self {
        Self {
            run_id: run_id.into(),
            next_seq: start_seq,
        }
    }

    pub fn next_record(&mut self, ts: impl Into<String>, event: EngineEvent) -> EngineEventRecord {
        let seq = self.next_seq;
        self.next_seq = self.next_seq.saturating_add(1);
        EngineEventRecord::new(self.run_id.clone(), seq, ts, event)
    }

    pub fn next_seq(&self) -> u64 {
        self.next_seq
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EngineEventSequenceError {
    #[error("sequence is empty")]
    Empty,
    #[error("sequence must start at 0, got {actual}")]
    InvalidStart { actual: u64 },
    #[error("sequence is not monotonic at index {index}: expected {expected}, got {actual}")]
    NonMonotonic {
        index: usize,
        expected: u64,
        actual: u64,
    },
}

pub fn ensure_monotonic_sequence(records: &[EngineEventRecord]) -> Result<(), EngineEventSequenceError> {
    let Some(first) = records.first() else {
        return Err(EngineEventSequenceError::Empty);
    };
    if first.seq != 0 {
        return Err(EngineEventSequenceError::InvalidStart { actual: first.seq });
    }
    for index in 1..records.len() {
        let expected = records[index - 1].seq + 1;
        let actual = records[index].seq;
        if actual != expected {
            return Err(EngineEventSequenceError::NonMonotonic {
                index,
                expected,
                actual,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "types_test.rs"]
mod tests;
