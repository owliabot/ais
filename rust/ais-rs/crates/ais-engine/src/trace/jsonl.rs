use crate::events::EngineEventRecord;

use super::{redact_engine_event_record, TraceRedactOptions};

#[derive(Debug, thiserror::Error)]
pub enum TraceEncodeError {
    #[error("failed to encode trace JSONL line: {0}")]
    Json(#[from] serde_json::Error),
}

pub fn encode_trace_jsonl_line(
    record: &EngineEventRecord,
    options: &TraceRedactOptions,
) -> Result<String, TraceEncodeError> {
    let redacted = redact_engine_event_record(record, options);
    let mut line = serde_json::to_string(&redacted)?;
    line.push('\n');
    Ok(line)
}

#[cfg(test)]
#[path = "jsonl_test.rs"]
mod tests;
