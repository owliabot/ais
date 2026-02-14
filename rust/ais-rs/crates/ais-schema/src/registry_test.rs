use crate::get_json_schema;
use crate::versions::{SCHEMA_ENGINE_EVENT_0_0_3, SCHEMA_PACK_0_0_2, SCHEMA_PROTOCOL_0_0_2};

#[test]
fn registry_returns_known_schema() {
    let schema = get_json_schema(SCHEMA_PROTOCOL_0_0_2).expect("schema must exist");
    assert!(schema.json.contains("$schema"));
}

#[test]
fn registry_covers_engine_event_schema() {
    let schema = get_json_schema(SCHEMA_ENGINE_EVENT_0_0_3).expect("schema must exist");
    assert!(schema.json.contains("ais-engine-event"));
}

#[test]
fn unknown_schema_returns_none() {
    assert!(get_json_schema(SCHEMA_PACK_0_0_2).is_some());
    assert!(get_json_schema("ais-unknown/1").is_none());
}
