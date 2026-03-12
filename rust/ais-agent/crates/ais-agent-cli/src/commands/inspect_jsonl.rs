use ais_agent_transport::jsonl::{JsonlInboundFrame, JsonlOutboundFrame};

use crate::cli::args::JsonlDirection;

pub fn inspect_jsonl(
    direction: JsonlDirection,
    line: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let pretty = match direction {
        JsonlDirection::Inbound => {
            serde_json::to_string_pretty(&serde_json::from_str::<JsonlInboundFrame>(line)?)?
        }
        JsonlDirection::Outbound => {
            serde_json::to_string_pretty(&serde_json::from_str::<JsonlOutboundFrame>(line)?)?
        }
    };

    println!("{pretty}");
    Ok(())
}
