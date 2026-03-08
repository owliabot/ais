use serde_json::{json, Map, Value};
use std::collections::BTreeSet;

/// Non-input semantic context extracted from intent grounding.
#[derive(Debug, Clone)]
pub(super) struct IntentContext {
    projection: Value,
}

impl IntentContext {
    pub(super) fn from_runtime(runtime: &Value) -> Option<Self> {
        let grounding = runtime.pointer("/agent/intent_grounding")?;
        let grounding_obj = grounding.as_object()?;

        let mut out = Map::<String, Value>::new();
        for key in [
            "status",
            "summary",
            "ready_for_todos",
            "reason_code",
            "message",
            "issues",
            "questions",
            "answers",
        ] {
            if let Some(value) = grounding_obj.get(key) {
                out.insert(key.to_string(), value.clone());
            }
        }

        out.insert(
            "facts".to_string(),
            grounding_obj
                .get("intent_facts")
                .cloned()
                .unwrap_or(Value::Object(Map::new())),
        );
        out.insert(
            "confidence".to_string(),
            json!({
                "facts": normalize_fact_confidence(
                    grounding_obj.get("confidence").and_then(Value::as_object),
                ),
            }),
        );

        Some(Self {
            projection: Value::Object(out),
        })
    }

    pub(super) fn projection(&self) -> &Value {
        &self.projection
    }
}

pub(super) fn grounding_fact_keys_from_state_summary(state_summary: Option<&Value>) -> Vec<String> {
    let mut keys = BTreeSet::<String>::new();
    for raw_key in state_summary
        .and_then(|summary| summary.pointer("/intent_context/facts"))
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|entries| entries.keys())
    {
        keys.insert(raw_key.to_string());
    }
    keys.into_iter().collect::<Vec<_>>()
}

pub(super) fn grounding_fact_keys_from_typed_summary(
    state_summary: Option<&super::state_summary::StateSummary>,
) -> Vec<String> {
    let mut keys = BTreeSet::<String>::new();
    for raw_key in state_summary
        .and_then(|summary| summary.intent_context_facts())
        .into_iter()
        .flat_map(|entries| entries.keys())
    {
        keys.insert(raw_key.to_string());
    }
    keys.into_iter().collect::<Vec<_>>()
}

fn normalize_fact_confidence(confidence: Option<&Map<String, Value>>) -> Map<String, Value> {
    let mut fact_confidence = Map::<String, Value>::new();
    let Some(confidence) = confidence else {
        return fact_confidence;
    };

    for (key, score) in confidence {
        let Some(score_u64) = score.as_u64() else {
            continue;
        };
        if let Some(fact_key) = key.strip_prefix("fact:") {
            fact_confidence.insert(fact_key.to_string(), Value::Number(score_u64.into()));
        }
    }
    fact_confidence
}
