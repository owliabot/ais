use super::facts::FactStore;
use serde_json::{Map, Value};
use std::collections::BTreeMap;

const KEY_PLANNING_MEMORY: &str = "planning_memory";
const KEY_FACT_STORE: &str = "fact_store";
const KEY_TODO_PROGRESS: &str = "todo_progress";
const KEY_INTENT_FACTS: &str = "intent_facts";

#[derive(Debug, Clone, Default)]
pub(super) struct AgentCheckpointExtensions {
    planning_memory: Option<Value>,
    fact_store: Option<FactStore>,
    todo_progress: Option<Value>,
    intent_facts: Option<BTreeMap<String, Value>>,
    passthrough: Map<String, Value>,
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
                KEY_FACT_STORE => match serde_json::from_value::<FactStore>(value.clone()) {
                    Ok(store) => decoded.fact_store = Some(store),
                    Err(_) => {
                        decoded.passthrough.insert(key.clone(), value.clone());
                    }
                },
                KEY_TODO_PROGRESS => decoded.todo_progress = Some(value.clone()),
                KEY_INTENT_FACTS => {
                    match serde_json::from_value::<BTreeMap<String, Value>>(value.clone()) {
                        Ok(intent_facts) => decoded.intent_facts = Some(intent_facts),
                        Err(_) => {
                            decoded.passthrough.insert(key.clone(), value.clone());
                        }
                    }
                }
                _ => {
                    decoded.passthrough.insert(key.clone(), value.clone());
                }
            }
        }
        decoded
    }

    pub(super) fn planning_memory(&self) -> Option<&Value> {
        self.planning_memory.as_ref()
    }

    pub(super) fn fact_store(&self) -> Option<&FactStore> {
        self.fact_store.as_ref()
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
        fact_store: &FactStore,
        todo_progress: Option<&Value>,
        intent_facts: Option<&BTreeMap<String, Value>>,
    ) -> Map<String, Value> {
        let mut output = self.passthrough.clone();
        if let Some(memory) = planning_memory {
            output.insert(KEY_PLANNING_MEMORY.to_string(), memory);
        }
        if let Ok(store) = serde_json::to_value(fact_store) {
            output.insert(KEY_FACT_STORE.to_string(), store);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::facts::{FactLayer, FactSource};
    use serde_json::json;

    #[test]
    fn decode_and_encode_preserves_unknown_extensions() {
        let mut input = Map::<String, Value>::new();
        input.insert("planning_memory".to_string(), json!({"snapshot":"abc"}));
        input.insert("fact_store".to_string(), json!({"entries":{}}));
        input.insert(
            "todo_progress".to_string(),
            json!({"current_todo":{"id":"todo_1"}}),
        );
        input.insert(
            "intent_facts".to_string(),
            json!({"recipient":"0xabc","amount":"1"}),
        );
        input.insert(
            "vendor.custom".to_string(),
            json!({"schema":"vendor-ext/0.0.1","x":1}),
        );
        let decoded = AgentCheckpointExtensions::decode(Some(&input));
        assert!(decoded.planning_memory().is_some());
        assert!(decoded.fact_store().is_some());
        assert!(decoded.todo_progress().is_some());
        assert!(decoded.intent_facts().is_some());

        let mut fact_store = FactStore::default();
        fact_store.upsert(
            "owner",
            json!("0xabc"),
            FactLayer::Seed,
            FactSource::RuntimeProvided,
            "runtime.inputs.owner",
        );
        let output = decoded.encode_updated(
            Some(json!({"snapshot":"next"})),
            &fact_store,
            Some(&json!({"current_todo":{"id":"todo_2"}})),
            Some(&BTreeMap::from([("recipient".to_string(), json!("0xdef"))])),
        );
        assert_eq!(
            output.get("vendor.custom"),
            Some(&json!({"schema":"vendor-ext/0.0.1","x":1}))
        );
        assert_eq!(
            output.get("planning_memory"),
            Some(&json!({"snapshot":"next"}))
        );
        assert_eq!(
            output
                .get("todo_progress")
                .and_then(|value| value.pointer("/current_todo/id")),
            Some(&json!("todo_2"))
        );
        assert_eq!(
            output
                .get("intent_facts")
                .and_then(|value| value.get("recipient")),
            Some(&json!("0xdef"))
        );
    }

    #[test]
    fn decode_keeps_invalid_fact_store_passthrough() {
        let mut input = Map::<String, Value>::new();
        input.insert("fact_store".to_string(), json!({"entries":"invalid"}));
        let decoded = AgentCheckpointExtensions::decode(Some(&input));
        assert!(decoded.fact_store().is_none());
        let output = decoded.encode_updated(None, &FactStore::default(), None, None);
        assert!(output.get("fact_store").is_some());
    }
}
