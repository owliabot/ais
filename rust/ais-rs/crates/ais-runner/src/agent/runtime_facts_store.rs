use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::input_store::{
    is_query_observation_source, normalize_store_entry_meta, InputStoreUpsertResult,
    InputValueMeta, InputValueStability, VolatileInputSignal, VolatileSignalObservation,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RuntimeFactEntry {
    pub value: Value,
    pub meta: InputValueMeta,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RuntimeFactsStore {
    entries: BTreeMap<String, RuntimeFactEntry>,
}

impl RuntimeFactsStore {
    pub fn upsert(
        &mut self,
        key: impl AsRef<str>,
        value: Value,
        meta: InputValueMeta,
    ) -> InputStoreUpsertResult {
        let Some(canonical_key) = normalize_runtime_fact_key(key.as_ref()) else {
            return InputStoreUpsertResult::Rejected;
        };
        let entry = RuntimeFactEntry {
            value,
            meta: normalize_store_entry_meta(canonical_key.as_str(), &meta),
        };
        if let Some(existing) = self.entries.get(canonical_key.as_str()) {
            if entry.meta.source_priority < existing.meta.source_priority {
                return InputStoreUpsertResult::Ignored;
            }
            if entry.meta.source_priority == existing.meta.source_priority
                && !should_refresh_equal_priority(existing, &entry)
            {
                return InputStoreUpsertResult::Ignored;
            }
            self.entries.insert(canonical_key, entry);
            return InputStoreUpsertResult::Replaced;
        }
        self.entries.insert(canonical_key, entry);
        InputStoreUpsertResult::Inserted
    }

    pub fn get(&self, key: &str) -> Option<&RuntimeFactEntry> {
        let canonical_key = normalize_runtime_fact_key(key)?;
        self.entries.get(canonical_key.as_str())
    }

    pub fn list_ref_strings(&self) -> Vec<String> {
        self.entries.keys().cloned().collect()
    }

    pub fn merge(&mut self, other: &RuntimeFactsStore) {
        for (key, entry) in &other.entries {
            let _ = self.upsert(key.as_str(), entry.value.clone(), entry.meta.clone());
        }
    }

    /// Rehydrate a `RuntimeFactsStore` from the planning projection shape:
    /// `{"facts":{...},"meta":{...}}`.
    ///
    /// This is intentionally lossy with respect to unknown meta fields; it
    /// defaults missing metadata to `InputValueMeta::default()`.
    pub fn from_projected_planning_value(value: &Value) -> Option<Self> {
        let facts = value.get("facts")?.as_object()?;
        let meta = value.get("meta").and_then(Value::as_object);
        let mut out = RuntimeFactsStore::default();
        for (key, fact_value) in facts {
            let meta_value = meta.and_then(|map| map.get(key));
            let parsed_meta = meta_value
                .and_then(|value| serde_json::from_value::<InputValueMeta>(value.clone()).ok())
                .unwrap_or_default();
            let _ = out.upsert(key.as_str(), fact_value.clone(), parsed_meta);
        }
        Some(out)
    }

    pub fn has_fresh_volatile_signal(
        &self,
        signal: VolatileInputSignal,
        max_age_ms: u64,
        now_ms: u64,
    ) -> bool {
        self.entries.iter().any(|(key, entry)| {
            entry.meta.stability == InputValueStability::Volatile
                && is_query_observation_source(entry.meta.source.as_str())
                && volatile_signal_matches_key(signal, key.as_str())
                && entry
                    .meta
                    .observed_at_ms
                    .is_some_and(|timestamp| now_ms.saturating_sub(timestamp) <= max_age_ms)
        })
    }

    pub fn newest_volatile_signal_observation(
        &self,
        signal: VolatileInputSignal,
    ) -> Option<VolatileSignalObservation> {
        self.entries
            .iter()
            .filter(|(key, entry)| {
                entry.meta.stability == InputValueStability::Volatile
                    && is_query_observation_source(entry.meta.source.as_str())
                    && volatile_signal_matches_key(signal, key.as_str())
            })
            .filter_map(|(_, entry)| {
                entry
                    .meta
                    .observed_at_ms
                    .map(|observed_at_ms| VolatileSignalObservation { observed_at_ms })
            })
            .max_by_key(|observation| observation.observed_at_ms)
    }

    pub fn invalidate_volatile_signals(&mut self, signals: &[VolatileInputSignal]) -> Vec<String> {
        if signals.is_empty() {
            return Vec::new();
        }

        let mut invalidated = Vec::<String>::new();
        for (key, entry) in &mut self.entries {
            if entry.meta.stability != InputValueStability::Volatile
                || !is_query_observation_source(entry.meta.source.as_str())
                || entry.meta.observed_at_ms.is_none()
                || !signals
                    .iter()
                    .copied()
                    .any(|signal| volatile_signal_matches_key(signal, key.as_str()))
            {
                continue;
            }
            entry.meta.observed_at_ms = None;
            invalidated.push(key.clone());
        }
        invalidated
    }

    pub fn to_projected_planning_value(&self) -> Value {
        let mut facts = Map::<String, Value>::new();
        let mut meta = Map::<String, Value>::new();
        for (key, entry) in &self.entries {
            facts.insert(key.clone(), entry.value.clone());
            meta.insert(
                key.clone(),
                serde_json::json!({
                    "source": entry.meta.source,
                    "source_priority": entry.meta.source_priority,
                    "provenance": entry.meta.provenance,
                    "confidence": entry.meta.confidence,
                    "layer": entry.meta.layer,
                    "stability": entry.meta.stability,
                    "observed_at_ms": entry.meta.observed_at_ms,
                }),
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
}

fn should_refresh_equal_priority(existing: &RuntimeFactEntry, incoming: &RuntimeFactEntry) -> bool {
    if is_newer_observation(incoming.meta.observed_at_ms, existing.meta.observed_at_ms) {
        return true;
    }

    let same_observation_time = incoming.meta.observed_at_ms == existing.meta.observed_at_ms;
    same_observation_time
        && (incoming.value != existing.value
            || incoming.meta.source != existing.meta.source
            || incoming.meta.provenance != existing.meta.provenance
            || incoming.meta.confidence != existing.meta.confidence
            || incoming.meta.layer != existing.meta.layer
            || incoming.meta.stability != existing.meta.stability)
}

fn is_newer_observation(incoming: Option<u64>, existing: Option<u64>) -> bool {
    match (incoming, existing) {
        (Some(incoming), Some(existing)) => incoming > existing,
        (Some(_), None) => true,
        _ => false,
    }
}

pub fn normalize_runtime_fact_key(raw_key: &str) -> Option<String> {
    let trimmed = raw_key.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(key) = trimmed.strip_prefix("facts.") {
        let key = key.trim().trim_matches('.');
        return (!key.is_empty()).then(|| format!("facts.{key}"));
    }
    if let Some(key) = trimmed.strip_prefix("fact:") {
        let key = key.trim().trim_matches('.');
        return (!key.is_empty()).then(|| format!("facts.{key}"));
    }
    None
}

fn volatile_signal_matches_key(signal: VolatileInputSignal, key: &str) -> bool {
    let lowered = key.to_ascii_lowercase();
    match signal {
        VolatileInputSignal::Balance => lowered.contains("balance"),
        VolatileInputSignal::Allowance => lowered.contains("allowance"),
    }
}

#[cfg(test)]
#[path = "tests/runtime_facts_store.rs"]
mod tests;
