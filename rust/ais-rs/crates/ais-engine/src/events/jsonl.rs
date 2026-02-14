use super::types::EngineEventRecord;

pub fn encode_event_jsonl_line(record: &EngineEventRecord) -> serde_json::Result<String> {
    let mut line = serde_json::to_string(record)?;
    line.push('\n');
    Ok(line)
}

pub fn parse_event_jsonl_line(line: &str) -> serde_json::Result<EngineEventRecord> {
    serde_json::from_str::<EngineEventRecord>(line.trim_end())
}
