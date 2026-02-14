use crate::embedded::EmbeddedSchema;
use crate::versions::{
    SCHEMA_ENGINE_EVENT_0_0_3, SCHEMA_PACK_0_0_2, SCHEMA_PLAN_0_0_3, SCHEMA_PROTOCOL_0_0_2,
    SCHEMA_WORKFLOW_0_0_3,
};

const PROTOCOL_SCHEMA: &str = include_str!("../../../../../schemas/0.0.2/protocol.schema.json");
const PACK_SCHEMA: &str = include_str!("../../../../../schemas/0.0.2/pack.schema.json");
const WORKFLOW_SCHEMA: &str = include_str!("../../../../../schemas/0.0.2/workflow.schema.json");
const PLAN_SCHEMA: &str = include_str!("../../../../../schemas/0.0.2/plan.schema.json");
const ENGINE_EVENT_SCHEMA: &str = include_str!("../../../../../schemas/0.0.2/engine-event.schema.json");

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
        _ => None,
    }
}

#[cfg(test)]
#[path = "registry_test.rs"]
mod tests;
