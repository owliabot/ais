use super::{load_checkpoint_from_path, save_checkpoint_to_path};
use crate::checkpoint::{create_checkpoint_document, CheckpointEngineState};
use serde_json::json;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn checkpoint_store_roundtrip_path() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock ok")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("ais-engine-checkpoint-{nanos}.json"));

    let document = create_checkpoint_document(
        "run-store-1",
        "plan-store-hash",
        CheckpointEngineState {
            completed_node_ids: vec!["node-1".to_string()],
            paused_reason: Some("paused".to_string()),
            seen_command_ids: vec!["cmd-1".to_string()],
            pending_retries: serde_json::Map::from_iter([("node-2".to_string(), json!({"attempt": 1}))]),
        },
        Some(json!({"ctx": {"chain_id": "eip155:1"}})),
        None,
    );

    save_checkpoint_to_path(&path, &document).expect("must save");
    let loaded = load_checkpoint_from_path(&path).expect("must load");
    assert_eq!(loaded, document);

    let _ = std::fs::remove_file(path);
}
