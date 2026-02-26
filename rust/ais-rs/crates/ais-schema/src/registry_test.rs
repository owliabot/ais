use crate::get_json_schema;
use crate::versions::{
    SCHEMA_AGENT_PLANNING_TOOLS_0_1_0, SCHEMA_ENGINE_EVENT_0_0_3, SCHEMA_PACK_0_0_2,
    SCHEMA_PLAN_SKETCH_0_1_0, SCHEMA_PROTOCOL_0_0_2, SCHEMA_SIDE_EFFECT_RECORD_0_1_0,
};

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
fn registry_covers_plan_sketch_schema() {
    let schema = get_json_schema(SCHEMA_PLAN_SKETCH_0_1_0).expect("schema must exist");
    assert!(schema.json.contains("ais-plan-sketch/0.1.0"));
}

#[test]
fn registry_covers_side_effect_record_schema() {
    let schema = get_json_schema(SCHEMA_SIDE_EFFECT_RECORD_0_1_0).expect("schema must exist");
    assert!(schema.json.contains("ais-side-effect-record/0.1.0"));
}

#[test]
fn registry_covers_agent_planning_tools_schema() {
    let schema = get_json_schema(SCHEMA_AGENT_PLANNING_TOOLS_0_1_0).expect("schema must exist");
    assert!(schema.json.contains("ais-agent-planning-tools/0.1.0"));
}

#[test]
fn unknown_schema_returns_none() {
    assert!(get_json_schema(SCHEMA_PACK_0_0_2).is_some());
    assert!(get_json_schema("ais-unknown/1").is_none());
}
