use crate::trace::{redact_value, TraceRedactOptions};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeSet;

pub const CHECKPOINT_SCHEMA_0_0_1: &str = "ais-checkpoint/0.0.1";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct CheckpointEngineState {
    #[serde(default)]
    pub completed_node_ids: Vec<String>,
    #[serde(default)]
    pub paused_reason: Option<String>,
    #[serde(default)]
    pub seen_command_ids: Vec<String>,
    #[serde(default)]
    pub pending_retries: Map<String, Value>,
}

impl CheckpointEngineState {
    pub fn normalize(&mut self) {
        self.completed_node_ids = dedup_sort_strings(std::mem::take(&mut self.completed_node_ids));
        self.seen_command_ids = dedup_sort_strings(std::mem::take(&mut self.seen_command_ids));
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckpointDocument {
    pub schema: String,
    pub run_id: String,
    pub plan_hash: String,
    pub engine_state: CheckpointEngineState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_snapshot: Option<Value>,
}

impl CheckpointDocument {
    pub fn new(
        run_id: impl Into<String>,
        plan_hash: impl Into<String>,
        mut engine_state: CheckpointEngineState,
        runtime_snapshot: Option<Value>,
    ) -> Self {
        engine_state.normalize();
        Self {
            schema: CHECKPOINT_SCHEMA_0_0_1.to_string(),
            run_id: run_id.into(),
            plan_hash: plan_hash.into(),
            engine_state,
            runtime_snapshot,
        }
    }
}

pub fn create_checkpoint_document(
    run_id: impl Into<String>,
    plan_hash: impl Into<String>,
    engine_state: CheckpointEngineState,
    runtime_snapshot: Option<Value>,
    redact_options: Option<&TraceRedactOptions>,
) -> CheckpointDocument {
    let runtime_snapshot = runtime_snapshot.map(|mut value| {
        if let Some(options) = redact_options {
            redact_value(&mut value, options);
        }
        value
    });
    CheckpointDocument::new(run_id, plan_hash, engine_state, runtime_snapshot)
}

pub fn encode_checkpoint_json(document: &CheckpointDocument) -> serde_json::Result<String> {
    serde_json::to_string_pretty(document)
}

pub fn decode_checkpoint_json(input: &str) -> serde_json::Result<CheckpointDocument> {
    serde_json::from_str::<CheckpointDocument>(input)
}

fn dedup_sort_strings(values: Vec<String>) -> Vec<String> {
    values.into_iter().collect::<BTreeSet<_>>().into_iter().collect()
}

#[cfg(test)]
#[path = "types_test.rs"]
mod tests;
