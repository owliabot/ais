use super::input_store::InputStore;
use super::runtime_facts_store::RuntimeFactsStore;
use serde_json::{Map, Value};

const KEY_RESUME_CORE: &str = "resume_core";
const KEY_PLANNING_MEMORY: &str = "planning_memory";
const KEY_INPUT_STORE: &str = "input_store";
const KEY_RUNTIME_FACTS_STORE: &str = "runtime_facts_store";

#[derive(Debug, Clone, Default)]
pub(super) struct AgentCheckpointExtensions {
    planning_memory: Option<Value>,
    restored_input_store_projection: Option<InputStore>,
    input_store: Option<InputStore>,
    runtime_facts_store: Option<RuntimeFactsStore>,
}

impl AgentCheckpointExtensions {
    pub(super) fn decode(input: Option<&Map<String, Value>>) -> Self {
        let mut decoded = Self::default();
        let Some(raw) = input else {
            return decoded;
        };
        if let Some(section) = raw.get(KEY_RESUME_CORE).and_then(Value::as_object) {
            decode_resume_core_section(section, &mut decoded);
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

    pub(super) fn runtime_facts_store(&self) -> Option<&RuntimeFactsStore> {
        self.runtime_facts_store.as_ref()
    }

    pub(super) fn encode_updated_with_runtime_facts(
        &self,
        planning_memory: Option<Value>,
        input_store: &InputStore,
        runtime_facts_store: &RuntimeFactsStore,
    ) -> Map<String, Value> {
        let mut output = Map::new();
        let mut resume_core = Map::new();
        if let Some(memory) = planning_memory {
            resume_core.insert(KEY_PLANNING_MEMORY.to_string(), memory);
        }
        if let Ok(store) = serde_json::to_value(input_store) {
            resume_core.insert(KEY_INPUT_STORE.to_string(), store);
        }
        if let Ok(store) = serde_json::to_value(runtime_facts_store) {
            resume_core.insert(KEY_RUNTIME_FACTS_STORE.to_string(), store);
        }
        if !resume_core.is_empty() {
            output.insert(KEY_RESUME_CORE.to_string(), Value::Object(resume_core));
        }
        output
    }
}

fn decode_resume_core_section(
    section: &Map<String, Value>,
    decoded: &mut AgentCheckpointExtensions,
) {
    if decoded.planning_memory.is_none() {
        decoded.planning_memory = section.get(KEY_PLANNING_MEMORY).cloned();
    }
    if decoded.input_store.is_none() {
        if let Some(value) = section.get(KEY_INPUT_STORE) {
            if let Ok(store) = serde_json::from_value::<InputStore>(value.clone()) {
                decoded.input_store = Some(store);
            }
        }
    }
    if decoded.runtime_facts_store.is_none() {
        if let Some(value) = section.get(KEY_RUNTIME_FACTS_STORE) {
            if let Ok(store) = serde_json::from_value::<RuntimeFactsStore>(value.clone()) {
                decoded.runtime_facts_store = Some(store);
            }
        }
    }
}

#[cfg(test)]
#[path = "tests/checkpoint_ext_module.rs"]
mod tests;
