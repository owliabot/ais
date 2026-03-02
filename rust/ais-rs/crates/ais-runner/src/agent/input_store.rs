use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Map;
use serde_json::Value;

use super::input_normalize::normalize_input_slot_key;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct InputRef {
    canonical: String,
}

impl InputRef {
    pub fn new(raw_key: &str) -> Option<Self> {
        normalize_input_slot_key(raw_key).map(|canonical| Self { canonical })
    }

    pub fn as_str(&self) -> &str {
        self.canonical.as_str()
    }

    fn from_canonical(canonical: impl Into<String>) -> Option<Self> {
        let canonical = canonical.into();
        if canonical.trim().is_empty() {
            return None;
        }
        if normalize_input_key(canonical.as_str())?.as_str() != canonical {
            return None;
        }
        Some(Self { canonical })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InputValueMeta {
    pub source: String,
    pub source_priority: u32,
    pub provenance: Option<String>,
    pub confidence: Option<f64>,
    #[serde(default)]
    pub layer: InputValueLayer,
    #[serde(default)]
    pub stability: InputValueStability,
    #[serde(default)]
    pub observed_at_ms: Option<u64>,
}

impl Default for InputValueMeta {
    fn default() -> Self {
        Self {
            source: "unknown".to_string(),
            source_priority: 0,
            provenance: None,
            confidence: None,
            layer: InputValueLayer::Unknown,
            stability: InputValueStability::Unknown,
            observed_at_ms: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum InputValueLayer {
    Seed,
    Observed,
    Derived,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum InputValueStability {
    Stable,
    Volatile,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InputStoreEntry {
    pub value: Value,
    pub meta: InputValueMeta,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputStoreUpsertResult {
    Inserted,
    Replaced,
    Ignored,
    Rejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VolatileInputSignal {
    Balance,
    Allowance,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InputStore {
    entries: BTreeMap<String, InputStoreEntry>,
}

impl InputStore {
    pub fn upsert(
        &mut self,
        key: impl AsRef<str>,
        value: Value,
        meta: InputValueMeta,
    ) -> InputStoreUpsertResult {
        let Some(canonical_ref) = InputRef::new(key.as_ref()) else {
            return InputStoreUpsertResult::Rejected;
        };

        let canonical = canonical_ref.as_str().to_string();
        let entry = InputStoreEntry { value, meta };

        let existing = self.entries.get(&canonical);
        if let Some(existing_entry) = existing {
            if entry.meta.source_priority <= existing_entry.meta.source_priority {
                return InputStoreUpsertResult::Ignored;
            }
            self.entries.insert(canonical, entry);
            return InputStoreUpsertResult::Replaced;
        }

        self.entries.insert(canonical, entry);
        InputStoreUpsertResult::Inserted
    }

    pub fn upsert_seed(
        &mut self,
        key: impl AsRef<str>,
        value: Value,
        provenance: impl Into<String>,
    ) -> InputStoreUpsertResult {
        self.upsert(
            key,
            value,
            InputValueMeta {
                source: "seed".to_string(),
                source_priority: 10,
                provenance: Some(provenance.into()),
                confidence: None,
                layer: InputValueLayer::Seed,
                stability: InputValueStability::Unknown,
                observed_at_ms: None,
            },
        )
    }

    pub fn upsert_user(
        &mut self,
        key: impl AsRef<str>,
        value: Value,
        provenance: impl Into<String>,
    ) -> InputStoreUpsertResult {
        self.upsert(
            key,
            value,
            InputValueMeta {
                source: "user".to_string(),
                source_priority: 100,
                provenance: Some(provenance.into()),
                confidence: None,
                layer: InputValueLayer::Observed,
                stability: InputValueStability::Unknown,
                observed_at_ms: None,
            },
        )
    }

    pub fn get(&self, key: &str) -> Option<&InputStoreEntry> {
        let key = InputRef::new(key)?;
        self.entries.get(key.as_str())
    }

    pub fn has(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    pub fn list_refs(&self) -> Vec<InputRef> {
        self.entries
            .keys()
            .filter_map(InputRef::from_canonical)
            .collect()
    }

    pub fn list_ref_strings(&self) -> Vec<String> {
        self.list_refs()
            .into_iter()
            .map(|item| item.as_str().to_string())
            .collect()
    }

    pub fn merge(&mut self, other: &InputStore) {
        for (key, entry) in &other.entries {
            let _ = self.upsert(key.as_str(), entry.value.clone(), entry.meta.clone());
        }
    }

    pub fn has_fresh_volatile_signal(
        &self,
        signal: VolatileInputSignal,
        max_age_ms: u64,
        now_ms: u64,
    ) -> bool {
        self.entries.iter().any(|(key, entry)| {
            entry.meta.stability == InputValueStability::Volatile
                && entry.meta.source.eq_ignore_ascii_case("query")
                && volatile_signal_matches_key(signal, key.as_str())
                && entry
                    .meta
                    .observed_at_ms
                    .is_some_and(|timestamp| now_ms.saturating_sub(timestamp) <= max_age_ms)
        })
    }

    pub fn to_projected_planning_value(&self, max_entries: usize) -> Value {
        let max_entries = max_entries.max(1);
        let mut facts = Map::<String, Value>::new();
        let mut meta = Map::<String, Value>::new();
        let mut selected_keys = Vec::<String>::new();
        for priority_key in ["owner", "wallet.default"] {
            if self.entries.contains_key(priority_key) {
                selected_keys.push(priority_key.to_string());
            }
        }
        for key in self.entries.keys() {
            if selected_keys.len() >= max_entries {
                break;
            }
            if selected_keys.iter().any(|item| item == key) {
                continue;
            }
            selected_keys.push(key.clone());
        }
        let truncated = self.entries.len().saturating_sub(selected_keys.len());
        for key in selected_keys {
            let Some(entry) = self.entries.get(key.as_str()) else {
                continue;
            };
            facts.insert(key.clone(), entry.value.clone());
            meta.insert(
                key.clone(),
                serde_json::json!({
                    "layer": entry.meta.layer,
                    "source": entry.meta.source,
                    "source_priority": entry.meta.source_priority,
                    "provenance": entry.meta.provenance,
                    "stability": entry.meta.stability,
                    "observed_at_ms": entry.meta.observed_at_ms,
                    "confidence": entry.meta.confidence,
                }),
            );
        }
        if truncated > 0 {
            meta.insert(
                "_truncated_entries".to_string(),
                Value::Number((truncated as u64).into()),
            );
        }
        Value::Object(
            [
                ("facts".to_string(), Value::Object(facts)),
                ("meta".to_string(), Value::Object(meta)),
            ]
            .into_iter()
            .collect(),
        )
    }

    #[cfg(test)]
    pub fn to_runtime_projection(&self) -> Value {
        let mut runtime_inputs = Map::new();
        for (key, entry) in &self.entries {
            set_runtime_input_value(&mut runtime_inputs, key, entry.value.clone());
        }
        let mut root = Map::new();
        root.insert("inputs".to_string(), Value::Object(runtime_inputs));
        Value::Object(root)
    }
}

fn volatile_signal_matches_key(signal: VolatileInputSignal, key: &str) -> bool {
    let lowered = key.to_ascii_lowercase();
    match signal {
        VolatileInputSignal::Balance => lowered.contains("balance"),
        VolatileInputSignal::Allowance => lowered.contains("allowance"),
    }
}

fn normalize_input_key(raw_key: &str) -> Option<String> {
    normalize_input_slot_key(raw_key).filter(|canonical| {
        !canonical
            .split('.')
            .any(|segment| segment.contains('/') || segment.contains('~'))
    })
}

#[cfg(test)]
fn set_runtime_input_value(runtime_inputs: &mut Map<String, Value>, key: &str, value: Value) {
    let segments = key.split('.').collect::<Vec<_>>();
    if segments.is_empty() {
        return;
    }
    if segments.len() == 1 {
        runtime_inputs.insert(segments[0].to_string(), value);
        return;
    }

    let mut current = runtime_inputs;
    for segment in &segments[..segments.len() - 1] {
        let entry = current
            .entry((*segment).to_string())
            .or_insert_with(|| Value::Object(Map::new()));
        if !entry.is_object() {
            *entry = Value::Object(Map::new());
        }
        current = entry.as_object_mut().expect("nested object");
    }
    current.insert(segments[segments.len() - 1].to_string(), value);
}

#[cfg(test)]
#[path = "tests/input_store.rs"]
mod tests;
