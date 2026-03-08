use super::ref_model::RefPath;
use super::reference_inventory::{ReferenceInventory, ReferenceInventoryEntry};
use super::state_summary::StateSummary;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub(crate) struct RefCatalog {
    pub entries: Vec<RefCatalogEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RefCatalogEntry {
    pub reference: RefPath,
    pub canonical_ref: String,
    pub value_available: bool,
    pub value_type: String,
    pub source: String,
    pub source_priority: u32,
    pub freshness_ms: Option<u64>,
    pub producer_step: Option<String>,
}

impl RefCatalog {
    #[cfg(test)]
    pub(crate) fn build(state_summary: Option<&Value>) -> Self {
        Self {
            entries: ReferenceInventory::build(state_summary)
                .entries
                .into_iter()
                .map(ref_catalog_entry_from_inventory)
                .collect::<Vec<_>>(),
        }
    }

    pub(crate) fn build_typed(state_summary: Option<&StateSummary>) -> Self {
        Self {
            entries: ReferenceInventory::build_typed(state_summary)
                .entries
                .into_iter()
                .map(ref_catalog_entry_from_inventory)
                .collect::<Vec<_>>(),
        }
    }
}

fn ref_catalog_entry_from_inventory(entry: ReferenceInventoryEntry) -> RefCatalogEntry {
    RefCatalogEntry {
        reference: entry.reference,
        canonical_ref: entry.canonical_ref,
        value_available: entry.value_available,
        value_type: entry.value_type,
        source: entry.source,
        source_priority: entry.source_priority,
        freshness_ms: entry.freshness_ms,
        producer_step: entry.producer_step,
    }
}

#[cfg(test)]
pub(super) fn available_input_ref_catalog(state_summary: Option<&Value>) -> Vec<Value> {
    RefCatalog::build(state_summary)
        .entries
        .into_iter()
        .filter_map(ref_catalog_input_value)
        .collect::<Vec<_>>()
}

pub(super) fn available_input_ref_catalog_typed(
    state_summary: Option<&StateSummary>,
) -> Vec<Value> {
    RefCatalog::build_typed(state_summary)
        .entries
        .into_iter()
        .filter_map(ref_catalog_input_value)
        .collect::<Vec<_>>()
}

fn ref_catalog_input_value(entry: RefCatalogEntry) -> Option<Value> {
    if matches!(entry.reference, RefPath::Input { .. }) {
        Some(serde_json::json!({
            "ref": entry.canonical_ref,
            "has_value": entry.value_available,
            "value_type": entry.value_type,
            "source_priority": entry.source_priority,
            "source": entry.source,
        }))
    } else {
        None
    }
}

#[cfg(test)]
#[path = "tests/ref_catalog.rs"]
mod tests;
