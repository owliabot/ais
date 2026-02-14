use super::{decode_command_jsonl_line, encode_command_jsonl_line};
use crate::commands::{EngineCommand, EngineCommandEnvelope, EngineCommandType};
use serde_json::json;

#[test]
fn command_jsonl_roundtrip() {
    let envelope = EngineCommandEnvelope::new(EngineCommand {
        id: "cmd-1".to_string(),
        command_type: EngineCommandType::ApplyPatches,
        data: serde_json::Map::from_iter([
            ("patch_count".to_string(), json!(2)),
            ("scope".to_string(), json!("runtime")),
        ]),
    });

    let line = encode_command_jsonl_line(&envelope).expect("must encode");
    assert!(line.ends_with('\n'));
    let decoded = decode_command_jsonl_line(&line).expect("must decode");
    assert_eq!(decoded, envelope);
}
