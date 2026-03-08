use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::input_normalize::normalize_input_slot_key;
const DERIVED_ASSET_ROOTS: &[&str] = &["token"];

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum VolatileInputSignal {
    Balance,
    Allowance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VolatileSignalObservation {
    pub observed_at_ms: u64,
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
        self.upsert_semantic(key, value, meta)
    }

    fn upsert_canonical_entry(
        &mut self,
        key: impl AsRef<str>,
        value: Value,
        meta: InputValueMeta,
    ) -> InputStoreUpsertResult {
        let Some(canonical_ref) = InputRef::new(key.as_ref()) else {
            return InputStoreUpsertResult::Rejected;
        };

        let canonical = canonical_ref.as_str().to_string();
        let entry = InputStoreEntry {
            value,
            meta: normalize_store_entry_meta(canonical.as_str(), &meta),
        };

        let existing = self.entries.get(&canonical);
        if let Some(existing_entry) = existing {
            if entry.meta.source_priority < existing_entry.meta.source_priority {
                return InputStoreUpsertResult::Ignored;
            }
            if entry.meta.source_priority == existing_entry.meta.source_priority
                && !should_refresh_equal_priority(existing_entry, &entry)
            {
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
        self.upsert_semantic(
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
        self.upsert_semantic(
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

    pub fn upsert_semantic(
        &mut self,
        key: impl AsRef<str>,
        value: Value,
        meta: InputValueMeta,
    ) -> InputStoreUpsertResult {
        let normalized = normalize_semantic_entries(key.as_ref(), value, &meta);
        if normalized.is_empty() {
            return InputStoreUpsertResult::Rejected;
        }

        let mut saw_inserted = false;
        let mut saw_replaced = false;
        let mut saw_ignored = false;
        for (canonical_key, normalized_value, normalized_meta) in normalized {
            match self.upsert_canonical_entry(
                canonical_key.as_str(),
                normalized_value,
                normalized_meta,
            ) {
                InputStoreUpsertResult::Inserted => saw_inserted = true,
                InputStoreUpsertResult::Replaced => saw_replaced = true,
                InputStoreUpsertResult::Ignored => saw_ignored = true,
                InputStoreUpsertResult::Rejected => {}
            }
        }
        if saw_replaced {
            InputStoreUpsertResult::Replaced
        } else if saw_inserted {
            InputStoreUpsertResult::Inserted
        } else if saw_ignored {
            InputStoreUpsertResult::Ignored
        } else {
            InputStoreUpsertResult::Rejected
        }
    }

    pub fn get(&self, key: &str) -> Option<&InputStoreEntry> {
        let key = InputRef::new(key)?;
        self.entries.get(key.as_str())
    }

    pub fn get_semantic(&self, key: &str) -> Option<&InputStoreEntry> {
        self.get(key)
    }

    #[cfg(test)]
    pub fn has(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    pub fn list_refs(&self) -> Vec<InputRef> {
        self.entries
            .keys()
            .filter_map(InputRef::from_canonical)
            .collect()
    }

    #[cfg(test)]
    pub fn list_ref_strings(&self) -> Vec<String> {
        self.list_refs()
            .into_iter()
            .map(|item| item.as_str().to_string())
            .collect()
    }

    pub fn list_semantic_refs(&self) -> Vec<InputRef> {
        self.list_refs()
    }

    pub fn list_semantic_ref_strings(&self) -> Vec<String> {
        self.list_semantic_refs()
            .into_iter()
            .map(|item| item.as_str().to_string())
            .collect()
    }

    pub fn list_projected_ref_strings(&self) -> Vec<String> {
        self.projected_entries().into_keys().collect::<Vec<_>>()
    }

    pub fn get_projected(&self, key: &str) -> Option<InputStoreEntry> {
        let canonical = InputRef::new(key)?.as_str().to_string();
        self.entries
            .get(canonical.as_str())
            .cloned()
            .or_else(|| self.build_projected_asset_entry(canonical.as_str()))
    }

    pub fn merge(&mut self, other: &InputStore) {
        for (key, entry) in &other.entries {
            let _ = self.upsert_semantic(key.as_str(), entry.value.clone(), entry.meta.clone());
        }
    }

    /// Rehydrate an `InputStore` from the planning projection shape:
    /// `{"facts":{...},"meta":{...}}`.
    ///
    /// This is intentionally lossy with respect to unknown meta fields; it
    /// defaults missing metadata to `InputValueMeta::default()`.
    pub fn from_projected_planning_value(value: &Value) -> Option<Self> {
        let facts = value.get("facts")?.as_object()?;
        let meta = value.get("meta").and_then(Value::as_object);
        let mut out = InputStore::default();
        for (key, fact_value) in facts {
            let meta_value = meta.and_then(|map| map.get(key));
            let parsed_meta = meta_value
                .and_then(|value| serde_json::from_value::<InputValueMeta>(value.clone()).ok())
                .unwrap_or_default();
            if parsed_meta.layer == InputValueLayer::Derived {
                continue;
            }
            let _ = out.upsert_semantic(key.as_str(), fact_value.clone(), parsed_meta);
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
        for (key, entry) in self.projected_entries() {
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

    fn projected_entries(&self) -> BTreeMap<String, InputStoreEntry> {
        let mut projected = self.entries.clone();
        for root in derived_asset_roots(self.entries.keys()) {
            if let Some(entry) = self.build_projected_asset_entry(root.as_str()) {
                projected.insert(root, entry);
            }
        }
        projected
    }

    fn build_projected_asset_entry(&self, root: &str) -> Option<InputStoreEntry> {
        let prefixed_root = format!("{root}.");
        let child_entries = self
            .entries
            .iter()
            .filter(|(key, _)| key.starts_with(prefixed_root.as_str()))
            .collect::<Vec<_>>();
        if child_entries.is_empty() {
            return self.entries.get(root).cloned();
        }

        let mut object = Map::<String, Value>::new();
        for (key, entry) in &child_entries {
            let suffix = key.strip_prefix(prefixed_root.as_str())?;
            set_nested_projected_value(&mut object, suffix, entry.value.clone());
        }

        let source_priority = child_entries
            .iter()
            .map(|(_, entry)| entry.meta.source_priority)
            .max()
            .unwrap_or(0);
        let observed_at_ms = child_entries
            .iter()
            .filter_map(|(_, entry)| entry.meta.observed_at_ms)
            .max();
        let stability = if child_entries
            .iter()
            .any(|(_, entry)| entry.meta.stability == InputValueStability::Volatile)
        {
            InputValueStability::Volatile
        } else if child_entries
            .iter()
            .any(|(_, entry)| entry.meta.stability == InputValueStability::Stable)
        {
            InputValueStability::Stable
        } else {
            InputValueStability::Unknown
        };

        Some(InputStoreEntry {
            value: Value::Object(object),
            meta: InputValueMeta {
                source: "derived".to_string(),
                source_priority,
                provenance: Some(format!("input_store.projected.{root}")),
                confidence: None,
                layer: InputValueLayer::Derived,
                stability,
                observed_at_ms,
            },
        })
    }
}

fn should_refresh_equal_priority(existing: &InputStoreEntry, incoming: &InputStoreEntry) -> bool {
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

fn volatile_signal_matches_key(signal: VolatileInputSignal, key: &str) -> bool {
    let lowered = key.to_ascii_lowercase();
    match signal {
        VolatileInputSignal::Balance => lowered.contains("balance"),
        VolatileInputSignal::Allowance => lowered.contains("allowance"),
    }
}

fn normalize_semantic_entries(
    raw_key: &str,
    value: Value,
    meta: &InputValueMeta,
) -> Vec<(String, Value, InputValueMeta)> {
    let Some(canonical_key) = normalize_input_key(raw_key) else {
        return Vec::new();
    };

    if let Some(entries) = decompose_asset_root_entry(canonical_key.as_str(), &value, meta) {
        return entries;
    }

    vec![(
        canonical_key.clone(),
        normalize_leaf_value(canonical_key.as_str(), value),
        normalize_leaf_meta(canonical_key.as_str(), meta),
    )]
}

fn decompose_asset_root_entry(
    canonical_key: &str,
    value: &Value,
    meta: &InputValueMeta,
) -> Option<Vec<(String, Value, InputValueMeta)>> {
    if !DERIVED_ASSET_ROOTS.contains(&canonical_key) {
        return None;
    }

    match value {
        Value::String(_) => Some(vec![(
            format!("{canonical_key}.address"),
            value.clone(),
            meta.clone(),
        )]),
        Value::Object(object) => {
            let mut out = Vec::<(String, Value, InputValueMeta)>::new();
            for (field, field_value) in object {
                let field = field.trim();
                if field.is_empty() {
                    continue;
                }
                let Some(canonical_field) =
                    normalize_input_key(format!("{canonical_key}.{field}").as_str())
                else {
                    continue;
                };
                out.push((
                    canonical_field.clone(),
                    normalize_leaf_value(canonical_field.as_str(), field_value.clone()),
                    normalize_leaf_meta(canonical_field.as_str(), meta),
                ));
            }
            (!out.is_empty()).then_some(out)
        }
        _ => None,
    }
}

fn normalize_leaf_value(canonical_key: &str, value: Value) -> Value {
    match leaf_value_contract(canonical_key) {
        LeafValueContract::Passthrough => value,
        LeafValueContract::IntegerLike => normalize_integer_like_value(value),
    }
}

fn normalize_leaf_meta(canonical_key: &str, meta: &InputValueMeta) -> InputValueMeta {
    normalize_store_entry_meta(canonical_key, meta)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LeafValueContract {
    Passthrough,
    IntegerLike,
}

fn leaf_value_contract(canonical_key: &str) -> LeafValueContract {
    if is_decimals_slot(canonical_key) || is_integer_like_slot(canonical_key) {
        return LeafValueContract::IntegerLike;
    }
    LeafValueContract::Passthrough
}

fn is_decimals_slot(canonical_key: &str) -> bool {
    canonical_key
        .rsplit('.')
        .next()
        .is_some_and(|segment| segment == "decimals")
}

fn is_integer_like_slot(canonical_key: &str) -> bool {
    canonical_key
        .rsplit('.')
        .next()
        .is_some_and(|segment| match segment {
            "nonce" | "deadline" | "retry_limit" | "max_retries" => true,
            _ => segment == "bps" || segment.ends_with("_bps"),
        })
}

fn normalize_integer_like_value(value: Value) -> Value {
    match value {
        Value::String(raw) => raw
            .trim()
            .parse::<u64>()
            .ok()
            .map(|parsed| Value::Number(parsed.into()))
            .unwrap_or(Value::String(raw)),
        Value::Object(object) => {
            if let Some(inner) = object.get("value").cloned() {
                return normalize_integer_like_value(inner);
            }
            Value::Object(object)
        }
        other => other,
    }
}

pub(crate) fn normalize_store_entry_meta(key: &str, meta: &InputValueMeta) -> InputValueMeta {
    let mut normalized = meta.clone();
    if is_decimals_slot(key) {
        normalized.stability = InputValueStability::Stable;
        return normalized;
    }

    if volatile_signal_matches_any_key(key) {
        if is_query_observation_source(normalized.source.as_str()) {
            normalized.stability = InputValueStability::Volatile;
        }
        if normalized.stability == InputValueStability::Volatile
            && normalized.observed_at_ms.is_none()
        {
            normalized.observed_at_ms = Some(current_time_ms());
        }
    }
    normalized
}

pub(crate) fn is_query_observation_source(source: &str) -> bool {
    let lowered = source.trim().to_ascii_lowercase();
    lowered == "query" || lowered.starts_with("query.") || lowered.starts_with("host.query")
}

fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn derived_asset_roots<'a, I>(keys: I) -> BTreeSet<String>
where
    I: IntoIterator<Item = &'a String>,
{
    let mut roots = BTreeSet::<String>::new();
    for key in keys {
        for root in DERIVED_ASSET_ROOTS {
            if key == root || key.starts_with(format!("{root}.").as_str()) {
                roots.insert((*root).to_string());
            }
        }
    }
    roots
}

fn set_nested_projected_value(runtime_inputs: &mut Map<String, Value>, key: &str, value: Value) {
    let segments = key
        .split('.')
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
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

fn normalize_input_key(raw_key: &str) -> Option<String> {
    normalize_input_slot_key(raw_key).filter(|canonical| {
        !canonical
            .split('.')
            .any(|segment| segment.contains('/') || segment.contains('~'))
    })
}

fn volatile_signal_matches_any_key(key: &str) -> bool {
    volatile_signal_matches_key(VolatileInputSignal::Balance, key)
        || volatile_signal_matches_key(VolatileInputSignal::Allowance, key)
}

#[cfg(test)]
fn set_runtime_input_value(runtime_inputs: &mut Map<String, Value>, key: &str, value: Value) {
    use super::input_normalize::LEAF_VALUE_KEY;
    let segments = key.split('.').collect::<Vec<_>>();
    if segments.is_empty() {
        return;
    }
    if segments.len() == 1 {
        let k = segments[0].to_string();
        if let Some(existing) = runtime_inputs.get_mut(&k) {
            if existing.is_object() {
                if let Some(obj) = existing.as_object_mut() {
                    obj.insert(LEAF_VALUE_KEY.to_string(), value);
                }
                return;
            }
        }
        runtime_inputs.insert(k, value);
        return;
    }

    let mut current = runtime_inputs;
    for segment in &segments[..segments.len() - 1] {
        let entry = current
            .entry((*segment).to_string())
            .or_insert_with(|| Value::Object(Map::new()));
        if !entry.is_object() {
            let previous = std::mem::replace(entry, Value::Object(Map::new()));
            if let Some(obj) = entry.as_object_mut() {
                obj.insert(LEAF_VALUE_KEY.to_string(), previous);
            }
        }
        current = entry.as_object_mut().expect("nested object");
    }
    current.insert(segments[segments.len() - 1].to_string(), value);
}

#[cfg(test)]
#[path = "tests/input_store.rs"]
mod tests;
