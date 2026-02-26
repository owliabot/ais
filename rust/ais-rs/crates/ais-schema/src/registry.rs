use crate::embedded::EmbeddedSchema;
use crate::versions::{
    SCHEMA_AGENT_INTENT_0_0_1, SCHEMA_AGENT_PLANNING_TOOLS_0_1_0, SCHEMA_ENGINE_EVENT_0_0_3,
    SCHEMA_PACK_0_0_2, SCHEMA_PLAN_0_0_3, SCHEMA_PLAN_SKETCH_0_1_0, SCHEMA_PROTOCOL_0_0_2,
    SCHEMA_SIDE_EFFECT_RECORD_0_1_0, SCHEMA_WORKFLOW_0_0_3,
};

const PROTOCOL_SCHEMA: &str = include_str!("../../../../../schemas/0.0.2/protocol.schema.json");
const PACK_SCHEMA: &str = include_str!("../../../../../schemas/0.0.2/pack.schema.json");
const WORKFLOW_SCHEMA: &str = include_str!("../../../../../schemas/0.0.2/workflow.schema.json");
const PLAN_SCHEMA: &str = include_str!("../../../../../schemas/0.0.2/plan.schema.json");
const ENGINE_EVENT_SCHEMA: &str =
    include_str!("../../../../../schemas/0.0.2/engine-event.schema.json");
const SIDE_EFFECT_RECORD_SCHEMA: &str =
    include_str!("../../../../../schemas/0.0.2/side-effect-record.schema.json");
const AGENT_INTENT_SCHEMA: &str =
    include_str!("../../../../../schemas/0.0.2/agent-intent.schema.json");
const AGENT_PLANNING_TOOLS_SCHEMA: &str =
    include_str!("../../../../../schemas/0.0.2/agent-planning-tools.schema.json");
const PLAN_SKETCH_SCHEMA: &str =
    include_str!("../../../../../schemas/0.0.2/plan-sketch.schema.json");

pub fn get_json_schema(schema_id: &str) -> Option<EmbeddedSchema> {
    match schema_id {
        SCHEMA_PROTOCOL_0_0_2 => Some(EmbeddedSchema {
            id: SCHEMA_PROTOCOL_0_0_2,
            json: PROTOCOL_SCHEMA,
        }),
        SCHEMA_PACK_0_0_2 => Some(EmbeddedSchema {
            id: SCHEMA_PACK_0_0_2,
            json: PACK_SCHEMA,
        }),
        SCHEMA_WORKFLOW_0_0_3 => Some(EmbeddedSchema {
            id: SCHEMA_WORKFLOW_0_0_3,
            json: WORKFLOW_SCHEMA,
        }),
        SCHEMA_PLAN_0_0_3 => Some(EmbeddedSchema {
            id: SCHEMA_PLAN_0_0_3,
            json: PLAN_SCHEMA,
        }),
        SCHEMA_ENGINE_EVENT_0_0_3 => Some(EmbeddedSchema {
            id: SCHEMA_ENGINE_EVENT_0_0_3,
            json: ENGINE_EVENT_SCHEMA,
        }),
        SCHEMA_SIDE_EFFECT_RECORD_0_1_0 => Some(EmbeddedSchema {
            id: SCHEMA_SIDE_EFFECT_RECORD_0_1_0,
            json: SIDE_EFFECT_RECORD_SCHEMA,
        }),
        SCHEMA_AGENT_INTENT_0_0_1 => Some(EmbeddedSchema {
            id: SCHEMA_AGENT_INTENT_0_0_1,
            json: AGENT_INTENT_SCHEMA,
        }),
        SCHEMA_AGENT_PLANNING_TOOLS_0_1_0 => Some(EmbeddedSchema {
            id: SCHEMA_AGENT_PLANNING_TOOLS_0_1_0,
            json: AGENT_PLANNING_TOOLS_SCHEMA,
        }),
        SCHEMA_PLAN_SKETCH_0_1_0 => Some(EmbeddedSchema {
            id: SCHEMA_PLAN_SKETCH_0_1_0,
            json: PLAN_SKETCH_SCHEMA,
        }),
        _ => None,
    }
}

#[cfg(test)]
#[path = "registry_test.rs"]
mod tests;
