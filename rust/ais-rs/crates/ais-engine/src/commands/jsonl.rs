use super::types::EngineCommandEnvelope;

pub fn encode_command_jsonl_line(envelope: &EngineCommandEnvelope) -> serde_json::Result<String> {
    let mut line = serde_json::to_string(envelope)?;
    line.push('\n');
    Ok(line)
}

pub fn decode_command_jsonl_line(line: &str) -> serde_json::Result<EngineCommandEnvelope> {
    serde_json::from_str::<EngineCommandEnvelope>(line.trim_end())
}

#[cfg(test)]
#[path = "jsonl_test.rs"]
mod tests;
