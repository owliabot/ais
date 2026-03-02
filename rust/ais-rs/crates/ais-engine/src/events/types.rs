use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::time::{SystemTime, UNIX_EPOCH};

pub const ENGINE_EVENT_SCHEMA_0_0_3: &str = "ais-engine-event/0.0.3";
pub const ENGINE_EVENT_CHECKS_SCHEMA_0_0_1: &str = "ais-engine-checks/0.0.1";

pub fn wall_clock_timestamp_rfc3339() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format_unix_seconds_rfc3339(seconds)
}

pub(crate) fn format_unix_seconds_rfc3339(seconds: u64) -> String {
    let days = (seconds / 86_400) as i64;
    let seconds_of_day = (seconds % 86_400) as u32;
    let (year, month, day) = civil_from_days_since_unix_epoch(days);
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

fn civil_from_days_since_unix_epoch(days: i64) -> (i32, u32, u32) {
    // Howard Hinnant's civil-from-days algorithm for Gregorian calendar conversion.
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    let year = y + if month <= 2 { 1 } else { 0 };
    (year as i32, month as u32, day as u32)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineEventType {
    PlanReady,
    NodeReady,
    NodeBlocked,
    NeedUserConfirm,
    NeedUserInput,
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
    PlanReplaced,
    CommandAccepted,
    CommandRejected,
    PatchApplied,
    PatchRejected,
    SideEffectObserved,
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
    pub fn new(
        run_id: impl Into<String>,
        seq: u64,
        ts: impl Into<String>,
        event: EngineEvent,
    ) -> Self {
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

pub fn ensure_monotonic_sequence(
    records: &[EngineEventRecord],
) -> Result<(), EngineEventSequenceError> {
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
