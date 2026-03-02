use super::input_store::InputStore;
use serde_json::{Map, Value};
use std::collections::BTreeMap;

const KEY_PLANNING_MEMORY: &str = "planning_memory";
const KEY_INPUT_STORE: &str = "input_store";
const KEY_TODO_PROGRESS: &str = "todo_progress";
const KEY_INTENT_FACTS: &str = "intent_facts";

#[derive(Debug, Clone, Default)]
pub(super) struct AgentCheckpointExtensions {
    planning_memory: Option<Value>,
    restored_input_store_projection: Option<InputStore>,
    input_store: Option<InputStore>,
    todo_progress: Option<Value>,
    intent_facts: Option<BTreeMap<String, Value>>,
}

impl AgentCheckpointExtensions {
    pub(super) fn decode(input: Option<&Map<String, Value>>) -> Self {
        let mut decoded = Self::default();
        let Some(raw) = input else {
            return decoded;
        };
        for (key, value) in raw {
            match key.as_str() {
                KEY_PLANNING_MEMORY => decoded.planning_memory = Some(value.clone()),
                KEY_INPUT_STORE => match serde_json::from_value::<InputStore>(value.clone()) {
                    Ok(store) => decoded.input_store = Some(store),
                    Err(_) => {}
                },
                KEY_TODO_PROGRESS => {
                    decoded.todo_progress = Some(normalize_todo_progress_receipt_tx_hashes(value));
                }
                KEY_INTENT_FACTS => {
                    match serde_json::from_value::<BTreeMap<String, Value>>(value.clone()) {
                        Ok(intent_facts) => decoded.intent_facts = Some(intent_facts),
                        Err(_) => {}
                    }
                }
                _ => {}
            }
        }
        if let Some(input_store) = decoded.input_store.as_ref() {
            decoded.restored_input_store_projection = Some(input_store.clone());
        }
        decoded
    }

    pub(super) fn planning_memory(&self) -> Option<&Value> {
        self.planning_memory.as_ref()
    }

    pub(super) fn input_store(&self) -> Option<&InputStore> {
        self.restored_input_store_projection.as_ref()
    }

    pub(super) fn todo_progress(&self) -> Option<&Value> {
        self.todo_progress.as_ref()
    }

    pub(super) fn intent_facts(&self) -> Option<&BTreeMap<String, Value>> {
        self.intent_facts.as_ref()
    }

    pub(super) fn encode_updated(
        &self,
        planning_memory: Option<Value>,
        input_store: &InputStore,
        todo_progress: Option<&Value>,
        intent_facts: Option<&BTreeMap<String, Value>>,
    ) -> Map<String, Value> {
        let mut output = Map::new();
        if let Some(memory) = planning_memory {
            output.insert(KEY_PLANNING_MEMORY.to_string(), memory);
        }
        if let Ok(store) = serde_json::to_value(input_store) {
            output.insert(KEY_INPUT_STORE.to_string(), store);
        }
        if let Some(todo_progress) = todo_progress {
            output.insert(KEY_TODO_PROGRESS.to_string(), todo_progress.clone());
        }
        if let Some(intent_facts) = intent_facts {
            if let Ok(value) = serde_json::to_value(intent_facts) {
                output.insert(KEY_INTENT_FACTS.to_string(), value);
            }
        }
        output
    }
}

fn normalize_todo_progress_receipt_tx_hashes(value: &Value) -> Value {
    let mut normalized = value.clone();
    let Some(progress) = normalized.as_object_mut() else {
        return normalized;
    };
    if let Some(current_todo) = progress.get_mut("current_todo") {
        normalize_todo_receipt_tx_hashes(current_todo);
    }
    if let Some(todos) = progress.get_mut("todos").and_then(Value::as_array_mut) {
        for todo in todos {
            normalize_todo_receipt_tx_hashes(todo);
        }
    }
    normalized
}

fn normalize_todo_receipt_tx_hashes(todo: &mut Value) {
    let Some(receipt) = todo
        .as_object_mut()
        .and_then(|todo_obj| todo_obj.get_mut("receipt"))
    else {
        return;
    };
    let Some(receipt_obj) = receipt.as_object_mut() else {
        return;
    };
    let tx_hashes = match receipt_obj.get("tx_hashes") {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| Value::String(value.to_string()))
            .collect::<Vec<_>>(),
        Some(Value::String(value)) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                Vec::new()
            } else {
                vec![Value::String(trimmed.to_string())]
            }
        }
        Some(Value::Null) => Vec::new(),
        Some(_) => Vec::new(),
        None => return,
    };
    receipt_obj.insert("tx_hashes".to_string(), Value::Array(tx_hashes));
}

#[cfg(test)]
#[path = "tests/checkpoint_ext_module.rs"]
mod tests;
