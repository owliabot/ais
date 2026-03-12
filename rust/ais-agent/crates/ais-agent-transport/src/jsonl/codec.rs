use crate::jsonl::{JsonlInboundFrame, JsonlOutboundFrame};

pub fn decode_inbound_line(line: &str) -> Result<JsonlInboundFrame, serde_json::Error> {
    serde_json::from_str(line)
}

pub fn encode_outbound_frame(frame: &JsonlOutboundFrame) -> Result<String, serde_json::Error> {
    serde_json::to_string(frame)
}
