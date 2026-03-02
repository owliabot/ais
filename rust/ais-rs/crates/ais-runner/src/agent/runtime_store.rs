use super::todos::TodoBoard;
use serde_json::{Map, Value};

pub(super) fn record_runtime_agent_field(runtime: &mut Value, key: &str, value: Value) {
    if !runtime.is_object() {
        *runtime = Value::Object(Map::new());
    }
    let Some(root) = runtime.as_object_mut() else {
        return;
    };
    let agent_entry = root
        .entry("agent".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if !agent_entry.is_object() {
        *agent_entry = Value::Object(Map::new());
    }
    if let Some(agent) = agent_entry.as_object_mut() {
        agent.insert(key.to_string(), value);
    }
}

pub(super) fn record_todo_progress(runtime: &mut Value, todo_board: &TodoBoard) {
    record_runtime_agent_field(runtime, "todo_progress", todo_board.to_runtime_value());
}

#[cfg(test)]
pub(super) fn record_missing_required_input(runtime: &mut Value, payload: &Value) {
    record_runtime_agent_field(runtime, "missing_required_input", payload.clone());
}
