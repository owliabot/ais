use super::input_store::InputValueStability;
use super::ref_model::RefPath;
use super::state_summary::StateSummary;
use super::{InputStore, RuntimeFactsStore};
use serde::{Deserialize, Serialize};
use serde_json::json;
use serde_json::Value;
use std::collections::BTreeMap;
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub(crate) struct ReferenceInventory {
    pub entries: Vec<ReferenceInventoryEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ReferenceInventoryEntry {
    pub reference: RefPath,
    pub canonical_ref: String,
    pub value_available: bool,
    pub value_type: String,
    pub source: String,
    pub source_priority: u32,
    pub stability: String,
    pub reusable: bool,
    pub freshness_ms: Option<u64>,
    pub observed_at_ms: Option<u64>,
    pub producer_step: Option<String>,
}

impl ReferenceInventory {
    pub(crate) fn build(state_summary: Option<&Value>) -> Self {
        let Some(summary) = state_summary else {
            return Self::default();
        };

        let mut entries = BTreeMap::<String, ReferenceInventoryEntry>::new();
        collect_input_entries(summary, &mut entries);
        collect_runtime_fact_entries(summary, &mut entries);
        collect_node_output_entries(summary, &mut entries);
        Self {
            entries: entries.into_values().collect::<Vec<_>>(),
        }
    }

    pub(crate) fn build_typed(state_summary: Option<&StateSummary>) -> Self {
        let Some(summary) = state_summary else {
            return Self::default();
        };

        let mut entries = BTreeMap::<String, ReferenceInventoryEntry>::new();
        collect_input_entries_typed(summary, &mut entries);
        collect_runtime_fact_entries_typed(summary, &mut entries);
        collect_node_output_entries_typed(summary, &mut entries);
        Self {
            entries: entries.into_values().collect::<Vec<_>>(),
        }
    }

    pub(crate) fn from_runtime_stores(
        input_store: Option<&InputStore>,
        runtime_facts_store: Option<&RuntimeFactsStore>,
    ) -> Self {
        let mut entries = BTreeMap::<String, ReferenceInventoryEntry>::new();
        collect_input_entries_runtime(input_store, &mut entries);
        collect_runtime_fact_entries_runtime(runtime_facts_store, &mut entries);
        Self {
            entries: entries.into_values().collect::<Vec<_>>(),
        }
    }

    pub(crate) fn input_refs(&self) -> Vec<String> {
        self.entries
            .iter()
            .filter_map(|entry| match entry.reference {
                RefPath::Input { .. } => Some(entry.canonical_ref.clone()),
                RefPath::Fact { .. } | RefPath::NodeOutput { .. } => None,
            })
            .collect::<Vec<_>>()
    }

    pub(crate) fn to_reusable_outputs_projection(&self) -> Option<Value> {
        let mut entries = self
            .entries
            .iter()
            .filter_map(|entry| match entry.reference {
                RefPath::Input { .. } | RefPath::Fact { .. } => Some(entry),
                RefPath::NodeOutput { .. } => None,
            })
            .map(|entry| {
                let namespace = match entry.reference {
                    RefPath::Input { .. } => "inputs",
                    RefPath::Fact { .. } => "facts",
                    RefPath::NodeOutput { .. } => "nodes",
                };
                json!({
                    "ref": entry.canonical_ref,
                    "namespace": namespace,
                    "source": entry.source,
                    "stability": entry.stability,
                    "observed_at_ms": entry.observed_at_ms,
                    "reusable": entry.reusable,
                    "freshness": freshness_label(entry),
                })
            })
            .collect::<Vec<_>>();

        if entries.is_empty() {
            return None;
        }
        entries.sort_by(|left, right| {
            left.get("ref")
                .and_then(Value::as_str)
                .cmp(&right.get("ref").and_then(Value::as_str))
        });
        let total_refs = entries.len() as u64;
        let reusable_refs = entries
            .iter()
            .filter(|entry| entry.get("reusable").and_then(Value::as_bool) == Some(true))
            .count() as u64;
        let stable_refs = entries
            .iter()
            .filter(|entry| entry.get("stability").and_then(Value::as_str) == Some("stable"))
            .count() as u64;
        let fresh_volatile_refs = entries
            .iter()
            .filter(|entry| {
                entry.get("stability").and_then(Value::as_str) == Some("volatile")
                    && entry.get("reusable").and_then(Value::as_bool) == Some(true)
            })
            .count() as u64;
        let stale_volatile_refs = entries
            .iter()
            .filter(|entry| {
                entry.get("stability").and_then(Value::as_str) == Some("volatile")
                    && entry.get("reusable").and_then(Value::as_bool) == Some(false)
            })
            .count() as u64;
        let unknown_stability_refs = entries
            .iter()
            .filter(|entry| entry.get("stability").and_then(Value::as_str) == Some("unknown"))
            .count() as u64;
        Some(json!({
            "schema": "ais-agent-reusable-output-inventory/0.0.1",
            "entries": entries,
            "summary": {
                "total_refs": total_refs,
                "reusable_refs": reusable_refs,
                "stable_refs": stable_refs,
                "fresh_volatile_refs": fresh_volatile_refs,
                "stale_volatile_refs": stale_volatile_refs,
                "unknown_stability_refs": unknown_stability_refs,
            }
        }))
    }
}

fn collect_input_entries_runtime(
    input_store: Option<&InputStore>,
    entries: &mut BTreeMap<String, ReferenceInventoryEntry>,
) {
    let Some(store) = input_store else {
        return;
    };
    for slot in store.list_projected_ref_strings() {
        let Some(entry) = store.get_projected(slot.as_str()) else {
            continue;
        };
        let canonical_ref = format!("inputs.{slot}");
        let Some(reference) = RefPath::parse(canonical_ref.as_str()) else {
            continue;
        };
        let stability = match entry.meta.stability {
            InputValueStability::Stable => "stable",
            InputValueStability::Volatile => "volatile",
            InputValueStability::Unknown => "unknown",
        }
        .to_string();
        let observed_at_ms = entry.meta.observed_at_ms;
        entries.insert(
            canonical_ref.clone(),
            ReferenceInventoryEntry {
                reference,
                canonical_ref,
                value_available: true,
                value_type: infer_binding_value_type(&entry.value).to_string(),
                source: entry.meta.source.clone(),
                source_priority: entry.meta.source_priority,
                stability: stability.clone(),
                reusable: is_reusable(stability.as_str(), observed_at_ms),
                freshness_ms: freshness_ms(stability.as_str(), observed_at_ms),
                observed_at_ms,
                producer_step: None,
            },
        );
    }
}

fn collect_runtime_fact_entries_runtime(
    runtime_facts_store: Option<&RuntimeFactsStore>,
    entries: &mut BTreeMap<String, ReferenceInventoryEntry>,
) {
    let Some(store) = runtime_facts_store else {
        return;
    };
    for canonical_ref in store.list_ref_strings() {
        let Some(entry) = store.get(canonical_ref.as_str()) else {
            continue;
        };
        let Some(reference) = RefPath::parse(canonical_ref.as_str()) else {
            continue;
        };
        let stability = match entry.meta.stability {
            InputValueStability::Stable => "stable",
            InputValueStability::Volatile => "volatile",
            InputValueStability::Unknown => "unknown",
        }
        .to_string();
        let observed_at_ms = entry.meta.observed_at_ms;
        entries.insert(
            canonical_ref.clone(),
            ReferenceInventoryEntry {
                reference,
                canonical_ref,
                value_available: true,
                value_type: infer_binding_value_type(&entry.value).to_string(),
                source: entry.meta.source.clone(),
                source_priority: entry.meta.source_priority,
                stability: stability.clone(),
                reusable: is_reusable(stability.as_str(), observed_at_ms),
                freshness_ms: freshness_ms(stability.as_str(), observed_at_ms),
                observed_at_ms,
                producer_step: None,
            },
        );
    }
}

fn collect_input_entries(summary: &Value, entries: &mut BTreeMap<String, ReferenceInventoryEntry>) {
    let facts = summary
        .pointer("/input_store/facts")
        .and_then(Value::as_object);
    let meta = summary
        .pointer("/input_store/meta")
        .and_then(Value::as_object);
    let mut slots = BTreeSet::<String>::new();
    slots.extend(input_registry_slots(
        summary.pointer("/input_registry/known_refs"),
    ));
    if let Some(facts) = facts {
        slots.extend(
            facts
                .keys()
                .filter(|key| input_store_meta_allows_slot(meta, key.as_str()))
                .cloned(),
        );
    }
    if let Some(meta) = meta {
        slots.extend(
            meta.keys()
                .filter(|key| input_store_meta_allows_slot(Some(meta), key.as_str()))
                .cloned(),
        );
    }
    for slot in slots {
        let canonical_ref = format!("inputs.{slot}");
        let Some(reference) = RefPath::parse(canonical_ref.as_str()) else {
            continue;
        };
        let value = facts.and_then(|map| {
            map.get(slot.as_str())
                .or_else(|| value_at_dotted_path_object(map, slot.as_str()))
        });
        let meta_entry = meta.and_then(|map| {
            map.get(slot.as_str())
                .or_else(|| value_at_dotted_path_object(map, slot.as_str()))
        });
        let stability = meta_entry
            .and_then(|item| item.get("stability"))
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        let observed_at_ms = meta_entry
            .and_then(|item| item.get("observed_at_ms"))
            .and_then(Value::as_u64);
        entries.insert(
            canonical_ref.clone(),
            ReferenceInventoryEntry {
                reference,
                canonical_ref,
                value_available: value.is_some(),
                value_type: value
                    .map(infer_binding_value_type)
                    .unwrap_or("unknown")
                    .to_string(),
                source: meta_entry
                    .and_then(|item| item.get("source"))
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .to_string(),
                source_priority: meta_entry
                    .and_then(|item| item.get("source_priority"))
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as u32,
                stability: stability.clone(),
                reusable: is_reusable(stability.as_str(), observed_at_ms),
                freshness_ms: freshness_ms(stability.as_str(), observed_at_ms),
                observed_at_ms,
                producer_step: None,
            },
        );
    }
}

fn collect_input_entries_typed(
    summary: &StateSummary,
    entries: &mut BTreeMap<String, ReferenceInventoryEntry>,
) {
    let facts = summary.input_store_facts();
    let meta = summary.input_store_meta();

    let mut slots = std::collections::BTreeSet::<String>::new();
    slots.extend(input_registry_slots(Some(
        &summary.input_registry["known_refs"],
    )));
    if let Some(facts) = facts {
        slots.extend(
            facts
                .keys()
                .filter(|key| input_store_meta_allows_slot(meta, key.as_str()))
                .cloned(),
        );
    }
    if let Some(meta) = meta {
        slots.extend(
            meta.keys()
                .filter(|key| input_store_meta_allows_slot(Some(meta), key.as_str()))
                .cloned(),
        );
    }

    for slot in slots {
        let canonical_ref = format!("inputs.{slot}");
        let Some(reference) = RefPath::parse(canonical_ref.as_str()) else {
            continue;
        };
        let value = facts.and_then(|map| {
            map.get(slot.as_str())
                .or_else(|| value_at_dotted_path_object(map, slot.as_str()))
        });
        let meta_entry = meta.and_then(|map| {
            map.get(slot.as_str())
                .or_else(|| value_at_dotted_path_object(map, slot.as_str()))
        });
        let stability = meta_entry
            .and_then(|item| item.get("stability"))
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        let observed_at_ms = meta_entry
            .and_then(|item| item.get("observed_at_ms"))
            .and_then(Value::as_u64);
        if !input_store_meta_allows_slot(meta, slot.as_str()) {
            continue;
        }
        entries.insert(
            canonical_ref.clone(),
            ReferenceInventoryEntry {
                reference,
                canonical_ref,
                value_available: value.is_some(),
                value_type: value
                    .map(infer_binding_value_type)
                    .unwrap_or("unknown")
                    .to_string(),
                source: meta_entry
                    .and_then(|item| item.get("source"))
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .to_string(),
                source_priority: meta_entry
                    .and_then(|item| item.get("source_priority"))
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as u32,
                stability: stability.clone(),
                reusable: is_reusable(stability.as_str(), observed_at_ms),
                freshness_ms: freshness_ms(stability.as_str(), observed_at_ms),
                observed_at_ms,
                producer_step: None,
            },
        );
    }
}

fn collect_runtime_fact_entries(
    summary: &Value,
    entries: &mut BTreeMap<String, ReferenceInventoryEntry>,
) {
    let facts = summary
        .pointer("/runtime_facts/facts")
        .and_then(Value::as_object);
    let meta = summary
        .pointer("/runtime_facts/meta")
        .and_then(Value::as_object);
    let Some(facts) = facts else {
        return;
    };

    let mut refs = BTreeSet::<String>::new();
    refs.extend(facts.keys().cloned());
    if let Some(meta) = meta {
        refs.extend(meta.keys().cloned());
    }

    for raw_key in refs {
        let Some(canonical_ref) = canonical_fact_ref(raw_key.as_str()) else {
            continue;
        };
        let Some(reference) = RefPath::parse(canonical_ref.as_str()) else {
            continue;
        };
        let fact_key = canonical_ref
            .strip_prefix("facts.")
            .unwrap_or(canonical_ref.as_str());
        let value = facts
            .get(canonical_ref.as_str())
            .or_else(|| facts.get(fact_key))
            .or_else(|| value_at_dotted_path_object(facts, fact_key));
        let meta_entry = meta.and_then(|map| {
            map.get(canonical_ref.as_str())
                .or_else(|| map.get(fact_key))
                .or_else(|| value_at_dotted_path_object(map, fact_key))
        });
        let stability = meta_entry
            .and_then(|item| item.get("stability"))
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        let observed_at_ms = meta_entry
            .and_then(|item| item.get("observed_at_ms"))
            .and_then(Value::as_u64);
        entries.insert(
            canonical_ref.clone(),
            ReferenceInventoryEntry {
                reference,
                canonical_ref,
                value_available: value.is_some(),
                value_type: value
                    .map(infer_binding_value_type)
                    .unwrap_or("unknown")
                    .to_string(),
                source: meta_entry
                    .and_then(|item| item.get("source"))
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .to_string(),
                source_priority: meta_entry
                    .and_then(|item| item.get("source_priority"))
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as u32,
                stability: stability.clone(),
                reusable: is_reusable(stability.as_str(), observed_at_ms),
                freshness_ms: freshness_ms(stability.as_str(), observed_at_ms),
                observed_at_ms,
                producer_step: None,
            },
        );
    }
}

fn collect_node_output_entries(
    summary: &Value,
    entries: &mut BTreeMap<String, ReferenceInventoryEntry>,
) {
    for raw_ref in summary
        .pointer("/node_output_refs/known_refs")
        .and_then(Value::as_array)
        .into_iter()
        .flat_map(|items| items.iter())
        .filter_map(Value::as_str)
    {
        let Some(reference) = RefPath::parse(raw_ref) else {
            continue;
        };
        let producer_step = match &reference {
            RefPath::NodeOutput { step_id, .. } => step_id.clone(),
            RefPath::Input { .. } | RefPath::Fact { .. } => continue,
        };
        let value = runtime_node_output_value(summary, &reference)
            .filter(|value| is_readable_node_value(value));
        let canonical_ref = reference.as_canonical_str();
        entries.insert(
            canonical_ref.clone(),
            ReferenceInventoryEntry {
                reference,
                canonical_ref,
                value_available: value.is_some(),
                value_type: value
                    .map(infer_binding_value_type)
                    .unwrap_or("unknown")
                    .to_string(),
                source: "node_output_refs".to_string(),
                source_priority: 0,
                stability: "unknown".to_string(),
                reusable: false,
                freshness_ms: None,
                observed_at_ms: None,
                producer_step: Some(producer_step),
            },
        );
    }
}

fn collect_runtime_fact_entries_typed(
    summary: &StateSummary,
    entries: &mut BTreeMap<String, ReferenceInventoryEntry>,
) {
    let facts = summary.runtime_facts_facts();
    let meta = summary.runtime_facts_meta();
    let Some(facts) = facts else {
        return;
    };

    let mut refs = std::collections::BTreeSet::<String>::new();
    refs.extend(facts.keys().cloned());
    if let Some(meta) = meta {
        refs.extend(meta.keys().cloned());
    }

    for raw_key in refs {
        let Some(canonical_ref) = canonical_fact_ref(raw_key.as_str()) else {
            continue;
        };
        let Some(reference) = RefPath::parse(canonical_ref.as_str()) else {
            continue;
        };
        let fact_key = canonical_ref
            .strip_prefix("facts.")
            .unwrap_or(canonical_ref.as_str());
        let value = facts
            .get(canonical_ref.as_str())
            .or_else(|| facts.get(fact_key))
            .or_else(|| value_at_dotted_path_object(facts, fact_key));
        let meta_entry = meta.and_then(|map| {
            map.get(canonical_ref.as_str())
                .or_else(|| map.get(fact_key))
                .or_else(|| value_at_dotted_path_object(map, fact_key))
        });
        let stability = meta_entry
            .and_then(|item| item.get("stability"))
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        let observed_at_ms = meta_entry
            .and_then(|item| item.get("observed_at_ms"))
            .and_then(Value::as_u64);
        entries.insert(
            canonical_ref.clone(),
            ReferenceInventoryEntry {
                reference,
                canonical_ref,
                value_available: value.is_some(),
                value_type: value
                    .map(infer_binding_value_type)
                    .unwrap_or("unknown")
                    .to_string(),
                source: meta_entry
                    .and_then(|item| item.get("source"))
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .to_string(),
                source_priority: meta_entry
                    .and_then(|item| item.get("source_priority"))
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as u32,
                stability: stability.clone(),
                reusable: is_reusable(stability.as_str(), observed_at_ms),
                freshness_ms: freshness_ms(stability.as_str(), observed_at_ms),
                observed_at_ms,
                producer_step: None,
            },
        );
    }
}

fn collect_node_output_entries_typed(
    summary: &StateSummary,
    entries: &mut BTreeMap<String, ReferenceInventoryEntry>,
) {
    for raw_ref in summary.node_output_refs_known_refs() {
        let Some(reference) = RefPath::parse(raw_ref) else {
            continue;
        };
        let producer_step = match &reference {
            RefPath::NodeOutput { step_id, .. } => step_id.clone(),
            RefPath::Input { .. } | RefPath::Fact { .. } => continue,
        };
        let canonical_ref = reference.as_canonical_str();
        entries.insert(
            canonical_ref.clone(),
            ReferenceInventoryEntry {
                reference,
                canonical_ref,
                value_available: false,
                value_type: "unknown".to_string(),
                source: "node_output_refs".to_string(),
                source_priority: 0,
                stability: "unknown".to_string(),
                reusable: false,
                freshness_ms: None,
                observed_at_ms: None,
                producer_step: Some(producer_step),
            },
        );
    }
}

fn runtime_node_output_value<'a>(summary: &'a Value, reference: &RefPath) -> Option<&'a Value> {
    let RefPath::NodeOutput {
        step_id,
        field_path,
    } = reference
    else {
        return None;
    };
    let nodes = summary.pointer("/nodes").and_then(Value::as_object)?;
    for (node_id, node) in nodes {
        if !runtime_node_id_matches_step(node_id.as_str(), step_id.as_str()) {
            continue;
        }
        let outputs = node.get("outputs")?;
        let value = value_at_dotted_path(outputs, field_path.as_str())?;
        if is_readable_node_value(value) {
            return Some(value);
        }
    }
    None
}

fn runtime_node_id_matches_step(node_id: &str, step_id: &str) -> bool {
    let normalized_node = node_id.trim();
    let normalized_step = step_id.trim();
    if normalized_node.is_empty() || normalized_step.is_empty() {
        return false;
    }
    if normalized_node == normalized_step {
        return true;
    }
    if normalized_node
        .rsplit_once("__")
        .map(|(_, suffix)| suffix.trim())
        .is_some_and(|suffix| suffix == normalized_step)
    {
        return true;
    }
    normalized_node
        .rsplit_once('/')
        .map(|(_, suffix)| suffix.trim())
        .is_some_and(|suffix| suffix == normalized_step)
}

fn is_readable_node_value(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Array(items) => !items.is_empty(),
        Value::Object(object) => !object.is_empty(),
        Value::Bool(_) | Value::Number(_) | Value::String(_) => true,
    }
}

fn input_store_meta_entry<'a>(
    meta: Option<&'a serde_json::Map<String, Value>>,
    slot: &str,
) -> Option<&'a Value> {
    meta.and_then(|map| {
        map.get(slot)
            .or_else(|| value_at_dotted_path_object(map, slot))
    })
}

fn input_store_meta_allows_slot(meta: Option<&serde_json::Map<String, Value>>, slot: &str) -> bool {
    let Some(meta) = meta else {
        return true;
    };
    if let Some(entry) = input_store_meta_entry(Some(meta), slot) {
        return meta_entry_has_any_source(entry);
    }
    let prefix = format!("{slot}.");
    let mut saw_descendant = false;
    let mut has_any_source_descendant = false;
    for (key, entry) in meta {
        if !key.starts_with(prefix.as_str()) {
            continue;
        }
        saw_descendant = true;
        if meta_entry_has_any_source(entry) {
            has_any_source_descendant = true;
            break;
        }
    }
    if !saw_descendant {
        return true;
    }
    has_any_source_descendant
}

fn meta_entry_has_any_source(entry: &Value) -> bool {
    if let Some(source) = entry.get("source").and_then(Value::as_str) {
        return !source.trim().is_empty();
    }
    entry
        .as_object()
        .is_none_or(|object| object.values().any(meta_entry_has_any_source))
}

fn input_registry_slots(known_refs: Option<&Value>) -> BTreeSet<String> {
    known_refs
        .and_then(Value::as_array)
        .into_iter()
        .flat_map(|items| items.iter())
        .filter_map(Value::as_str)
        .filter_map(|raw_ref| match RefPath::parse(raw_ref) {
            Some(RefPath::Input { slot }) => Some(slot),
            Some(RefPath::Fact { .. } | RefPath::NodeOutput { .. }) | None => None,
        })
        .collect::<BTreeSet<_>>()
}

fn value_at_dotted_path_object<'a>(
    root: &'a serde_json::Map<String, Value>,
    dotted: &str,
) -> Option<&'a Value> {
    let mut segments = dotted.split('.').filter(|part| !part.is_empty());
    let first = segments.next()?;
    let mut current = root.get(first)?;
    for segment in segments {
        current = current.get(segment)?;
    }
    Some(current)
}

fn value_at_dotted_path<'a>(root: &'a Value, dotted: &str) -> Option<&'a Value> {
    let mut current = root;
    for segment in dotted.split('.').filter(|part| !part.is_empty()) {
        current = current.get(segment)?;
    }
    Some(current)
}

fn infer_binding_value_type(value: &Value) -> &'static str {
    match value {
        Value::Bool(_) => "boolean",
        Value::Number(_) => "numeric",
        Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.starts_with("eip155:") {
                "chain"
            } else if trimmed.len() == 42
                && trimmed.starts_with("0x")
                && trimmed
                    .as_bytes()
                    .iter()
                    .skip(2)
                    .all(|byte| byte.is_ascii_hexdigit())
            {
                "address"
            } else {
                "text"
            }
        }
        _ => "unknown",
    }
}

fn is_reusable(stability: &str, observed_at_ms: Option<u64>) -> bool {
    match stability {
        "stable" | "unknown" => true,
        "volatile" => freshness_ms(stability, observed_at_ms).is_some(),
        _ => false,
    }
}

fn freshness_ms(stability: &str, observed_at_ms: Option<u64>) -> Option<u64> {
    match stability {
        "stable" => None,
        "volatile" => {
            let observed = observed_at_ms?;
            let now_ms = current_unix_ms();
            (now_ms.saturating_sub(observed) <= 30_000).then_some(now_ms.saturating_sub(observed))
        }
        _ => None,
    }
}

fn freshness_label(entry: &ReferenceInventoryEntry) -> &'static str {
    match entry.stability.as_str() {
        "stable" => "stable",
        "volatile" if entry.reusable => "fresh",
        "volatile" => "stale",
        _ => "unknown",
    }
}

fn current_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn canonical_fact_ref(raw_key: &str) -> Option<String> {
    let trimmed = raw_key.trim();
    if trimmed.is_empty() {
        return None;
    }
    let normalized = if trimmed.starts_with("facts.") {
        trimmed.to_string()
    } else if let Some(key) = trimmed.strip_prefix("fact:") {
        format!("facts.{key}")
    } else if let Some(key) = trimmed.strip_prefix("fact.") {
        format!("facts.{key}")
    } else {
        format!("facts.{trimmed}")
    };
    let parsed = RefPath::parse(normalized.as_str())?;
    match parsed {
        RefPath::Fact { .. } => Some(parsed.as_canonical_str()),
        RefPath::Input { .. } | RefPath::NodeOutput { .. } => None,
    }
}

#[cfg(test)]
#[path = "tests/reference_inventory.rs"]
mod tests;
