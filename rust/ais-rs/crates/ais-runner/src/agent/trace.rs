use crate::audit_contract::AuditStreamAttempt;
use crate::error::RunnerError;
use ais_engine::events::wall_clock_timestamp_rfc3339;
use serde_json::{Map, Value};
use std::cell::RefCell;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;

const AGENT_TRACE_SCHEMA: &str = "ais-runner-agent-trace/0.0.1";

thread_local! {
    static AGENT_TRACE_SINK: RefCell<Option<AgentTraceSink>> = const { RefCell::new(None) };
    static AGENT_TRACE_ERROR: RefCell<Option<String>> = const { RefCell::new(None) };
}

struct AgentTraceSink {
    file: File,
    run_id: String,
    attempt_id: String,
    attempt_index: u64,
    seq_scope: String,
    next_seq: u64,
}

pub(super) struct AgentTraceGuard {
    previous: Option<AgentTraceSink>,
    previous_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PendingTraceRecord {
    pub phase: String,
    pub event: String,
    pub fields: Vec<(String, String)>,
}

impl PendingTraceRecord {
    pub fn new(
        phase: impl Into<String>,
        event: impl Into<String>,
        fields: Vec<(String, String)>,
    ) -> Self {
        Self {
            phase: phase.into(),
            event: event.into(),
            fields,
        }
    }
}

impl Drop for AgentTraceGuard {
    fn drop(&mut self) {
        AGENT_TRACE_SINK.with(|slot| {
            let restored = self.previous.take();
            *slot.borrow_mut() = restored;
        });
        AGENT_TRACE_ERROR.with(|slot| {
            *slot.borrow_mut() = self.previous_error.take();
        });
    }
}

pub(super) fn install_jsonl_sink(
    path: Option<&Path>,
    run_id: &str,
    attempt: &AuditStreamAttempt,
) -> Result<AgentTraceGuard, RunnerError> {
    let next = match path {
        Some(path) => Some(AgentTraceSink {
            file: OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .map_err(|error| RunnerError::AgentTraceIo(error.to_string()))?,
            run_id: run_id.to_string(),
            attempt_id: attempt.attempt_id.clone(),
            attempt_index: attempt.attempt_index,
            seq_scope: attempt.seq_scope.clone(),
            next_seq: 0,
        }),
        None => None,
    };
    let previous = AGENT_TRACE_SINK.with(|slot| slot.replace(next));
    let previous_error = AGENT_TRACE_ERROR.with(|slot| slot.replace(None));
    Ok(AgentTraceGuard {
        previous,
        previous_error,
    })
}

pub(super) fn flush_pending(enabled: bool, records: &[PendingTraceRecord]) {
    for record in records {
        let fields = record
            .fields
            .iter()
            .map(|(key, value)| (key.as_str(), value.clone()))
            .collect::<Vec<_>>();
        emit(
            enabled,
            record.phase.as_str(),
            record.event.as_str(),
            fields.as_slice(),
        );
    }
}

pub(super) fn ensure_sink_healthy() -> Result<(), RunnerError> {
    let error = AGENT_TRACE_ERROR.with(|slot| slot.borrow().clone());
    if let Some(reason) = error {
        return Err(RunnerError::AgentTraceIo(reason));
    }
    Ok(())
}

pub(super) fn emit(enabled: bool, phase: &str, event: &str, fields: &[(&str, String)]) {
    if enabled {
        let mut line = format!("[agent.trace] phase={phase} event={event}");
        for (key, value) in fields {
            if value.trim().is_empty() {
                continue;
            }
            line.push(' ');
            line.push_str(key);
            line.push('=');
            line.push_str(compact_value(value).as_str());
        }
        eprintln!("{line}");
    }

    AGENT_TRACE_SINK.with(|slot| {
        let mut borrow = slot.borrow_mut();
        let Some(sink) = borrow.as_mut() else {
            return;
        };
        let mut field_map = Map::new();
        for (key, value) in fields {
            let compact = compact_value(value);
            if compact.is_empty() {
                continue;
            }
            field_map.insert(key.to_string(), Value::String(compact));
        }
        let record = serde_json::json!({
            "schema": AGENT_TRACE_SCHEMA,
            "run_id": sink.run_id,
            "attempt_id": sink.attempt_id,
            "attempt_index": sink.attempt_index,
            "seq_scope": sink.seq_scope,
            "seq": sink.next_seq,
            "ts": wall_clock_timestamp_rfc3339(),
            "phase": phase,
            "event": event,
            "fields": field_map,
        });
        sink.next_seq = sink.next_seq.saturating_add(1);
        if let Err(error) = write_json_line(&mut sink.file, &record) {
            let message = error.to_string();
            AGENT_TRACE_ERROR.with(|slot| {
                let mut stored = slot.borrow_mut();
                if stored.is_none() {
                    *stored = Some(message.clone());
                }
            });
            eprintln!("[agent.trace sink error] {message}");
        }
    });
}

fn write_json_line(file: &mut File, value: &Value) -> Result<(), std::io::Error> {
    serde_json::to_writer(&mut *file, value)?;
    file.write_all(b"\n")
}

fn compact_value(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
#[path = "tests/trace.rs"]
mod tests;
