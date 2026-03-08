use ais_engine::EngineRunnerState;
use serde_json::Value;
use std::collections::BTreeSet;

use super::input_store::{InputStore, InputValueStability, VolatileInputSignal};
use super::reference_inventory::ReferenceInventory;
use super::runtime_facts_store::RuntimeFactsStore;
use super::state_summary::StateSummary;
use crate::policy::VolatileFactsPolicy;

pub(crate) struct ExecutionView<'a> {
    state: Option<&'a EngineRunnerState>,
}

pub(crate) struct ConfirmationView<'a> {
    execution: ExecutionView<'a>,
}

pub(crate) struct ReusableOutputInventory<'a> {
    typed_summary: Option<&'a StateSummary>,
    runtime_facts_store: Option<&'a RuntimeFactsStore>,
    input_store: Option<&'a InputStore>,
    volatile_facts_policy: VolatileFactsPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReusableOutputMatch {
    pub reference: String,
    pub source: String,
    pub stability: InputValueStability,
    pub observed_at_ms: Option<u64>,
}

pub(crate) fn build_reusable_output_inventory_projection(
    runtime_facts_store: Option<&RuntimeFactsStore>,
    input_store: Option<&InputStore>,
) -> Option<Value> {
    ReferenceInventory::from_runtime_stores(input_store, runtime_facts_store)
        .to_reusable_outputs_projection()
}

impl<'a> ExecutionView<'a> {
    pub(crate) fn new(state: Option<&'a EngineRunnerState>) -> Self {
        Self { state }
    }

    pub(crate) fn completed_node_ids(&self) -> BTreeSet<String> {
        self.state
            .map(|runtime| {
                runtime
                    .completed_node_ids
                    .iter()
                    .cloned()
                    .collect::<BTreeSet<_>>()
            })
            .unwrap_or_default()
    }

    pub(crate) fn approved_node_ids(&self) -> BTreeSet<String> {
        self.state
            .map(|runtime| {
                runtime
                    .approved_node_ids
                    .iter()
                    .cloned()
                    .collect::<BTreeSet<_>>()
            })
            .unwrap_or_default()
    }

    pub(crate) fn resolve_input_reference_display(&self, reference: &str) -> Option<String> {
        let state = self.state?;
        let key = reference.strip_prefix("inputs.")?;
        state
            .runtime
            .pointer(format!("/inputs/{}", key.replace('.', "/")).as_str())
            .and_then(value_to_text)
            .or_else(|| {
                state
                    .runtime
                    .pointer("/agent/state_summary/intent_slots/resolved_inputs")
                    .and_then(|inputs| value_at_dotted_path(inputs, key))
                    .and_then(value_to_text)
            })
            .or_else(|| {
                state
                    .runtime
                    .pointer("/agent/state_summary/input_store/facts")
                    .and_then(|facts| value_at_dotted_path(facts, key))
                    .and_then(value_to_text)
            })
    }
}

impl<'a> ConfirmationView<'a> {
    pub(crate) fn new(state: Option<&'a EngineRunnerState>) -> Self {
        Self {
            execution: ExecutionView::new(state),
        }
    }

    pub(crate) fn completed_node_ids(&self) -> BTreeSet<String> {
        self.execution.completed_node_ids()
    }

    pub(crate) fn approved_node_ids(&self) -> BTreeSet<String> {
        self.execution.approved_node_ids()
    }

    pub(crate) fn render_param_value(&self, value: &Value) -> Option<String> {
        if let Some(text) = value.as_str() {
            return Some(text.to_string());
        }
        if value.is_number() || value.is_boolean() {
            return Some(value.to_string());
        }
        let object = value.as_object()?;
        if let Some(lit) = object.get("lit") {
            if let Some(text) = lit.as_str() {
                return Some(text.to_string());
            }
            return Some(lit.to_string());
        }
        if let Some(reference) = object.get("ref").and_then(Value::as_str) {
            return self
                .execution
                .resolve_input_reference_display(reference)
                .or_else(|| Some(format!("ref:{reference}")));
        }
        if let Some(inner) = object.get("object") {
            if let Some(address_ref) = inner
                .get("address")
                .and_then(Value::as_object)
                .and_then(|entry| entry.get("ref"))
                .and_then(Value::as_str)
            {
                return self
                    .execution
                    .resolve_input_reference_display(address_ref)
                    .or_else(|| Some(format!("ref:{address_ref}")));
            }
            if let Some(text) = inner.as_str() {
                return Some(text.to_string());
            }
        }
        if object.get("cel").is_some() {
            return Some("computed(cel)".to_string());
        }
        Some(value.to_string())
    }
}

impl<'a> ReusableOutputInventory<'a> {
    #[cfg(test)]
    pub(crate) fn new(
        typed_summary: Option<&'a StateSummary>,
        runtime_facts_store: Option<&'a RuntimeFactsStore>,
        input_store: Option<&'a InputStore>,
    ) -> Self {
        Self::with_policy(
            typed_summary,
            runtime_facts_store,
            input_store,
            VolatileFactsPolicy::default(),
        )
    }

    pub(crate) fn with_policy(
        typed_summary: Option<&'a StateSummary>,
        runtime_facts_store: Option<&'a RuntimeFactsStore>,
        input_store: Option<&'a InputStore>,
        volatile_facts_policy: VolatileFactsPolicy,
    ) -> Self {
        Self {
            typed_summary,
            runtime_facts_store,
            input_store,
            volatile_facts_policy,
        }
    }

    pub(crate) fn reusable_reference(
        &self,
        reference: &str,
        now_ms: u64,
    ) -> Option<ReusableOutputMatch> {
        if let Some(slot) = reference.strip_prefix("inputs.") {
            return self.reusable_input_reference(slot, reference, now_ms);
        }
        if let Some(fact_key) = reference.strip_prefix("facts.") {
            return self.reusable_fact_reference(fact_key, reference);
        }
        None
    }

    pub(crate) fn has_fresh_volatile_signal(
        &self,
        signal: VolatileInputSignal,
        now_ms: u64,
    ) -> bool {
        let max_age_ms = self.volatile_facts_policy.max_age_ms;
        if let Some(store) = self.runtime_facts_store {
            if store.has_fresh_volatile_signal(signal, max_age_ms, now_ms) {
                return true;
            }
        }
        if let Some(store) = self.input_store {
            if input_store_has_fresh_true_input_signal(store, signal, max_age_ms, now_ms) {
                return true;
            }
        }
        let Some(summary) = self.typed_summary else {
            return false;
        };
        let Some(facts) = summary.runtime_facts_facts() else {
            return false;
        };
        let Some(meta) = summary.runtime_facts_meta() else {
            return false;
        };
        facts.keys().any(|key| {
            volatile_signal_matches_key(signal, key.as_str())
                && meta_for_slot(meta, key.as_str()).is_some_and(|meta_entry| {
                    meta_entry
                        .get("source")
                        .and_then(Value::as_str)
                        .is_some_and(|source| source.eq_ignore_ascii_case("query"))
                        && meta_entry
                            .get("stability")
                            .and_then(Value::as_str)
                            .is_some_and(|stability| stability == "volatile")
                        && meta_entry
                            .get("observed_at_ms")
                            .and_then(Value::as_u64)
                            .is_some_and(|observed_at_ms| {
                                now_ms.saturating_sub(observed_at_ms) <= max_age_ms
                            })
                })
        })
    }

    fn reusable_input_reference(
        &self,
        slot: &str,
        reference: &str,
        now_ms: u64,
    ) -> Option<ReusableOutputMatch> {
        if let Some(store) = self.input_store {
            if let Some(entry) = store.get_projected(slot) {
                if !is_reusable(
                    entry.meta.stability,
                    entry.meta.observed_at_ms,
                    now_ms,
                    self.volatile_facts_policy.max_age_ms,
                ) {
                    return None;
                }
                return Some(ReusableOutputMatch {
                    reference: reference.to_string(),
                    source: entry.meta.source.clone(),
                    stability: entry.meta.stability,
                    observed_at_ms: entry.meta.observed_at_ms,
                });
            }
        }

        let summary = self.typed_summary?;
        let facts = summary.input_store_facts()?;
        let value = facts
            .get(slot)
            .or_else(|| value_at_dotted_path_object(facts, slot))?;
        let meta = summary
            .input_store_meta()
            .and_then(|meta| meta_for_slot(meta, slot));
        let stability = meta
            .and_then(|entry| entry.get("stability"))
            .and_then(Value::as_str)
            .map(parse_stability)
            .unwrap_or(InputValueStability::Unknown);
        let observed_at_ms = meta
            .and_then(|entry| entry.get("observed_at_ms"))
            .and_then(Value::as_u64);
        if !value.is_null()
            && is_reusable(
                stability,
                observed_at_ms,
                now_ms,
                self.volatile_facts_policy.max_age_ms,
            )
        {
            return Some(ReusableOutputMatch {
                reference: reference.to_string(),
                source: meta
                    .and_then(|entry| entry.get("source"))
                    .and_then(Value::as_str)
                    .unwrap_or("input_store_projection")
                    .to_string(),
                stability,
                observed_at_ms,
            });
        }
        None
    }

    fn reusable_fact_reference(
        &self,
        fact_key: &str,
        reference: &str,
    ) -> Option<ReusableOutputMatch> {
        if let Some(store) = self.runtime_facts_store {
            let full_ref = format!("facts.{fact_key}");
            if let Some(entry) = store.get(full_ref.as_str()) {
                if entry.value.is_null() {
                    return None;
                }
                return Some(ReusableOutputMatch {
                    reference: reference.to_string(),
                    source: entry.meta.source.clone(),
                    stability: entry.meta.stability,
                    observed_at_ms: entry.meta.observed_at_ms,
                });
            }
        }
        let summary = self.typed_summary?;
        let value = summary.intent_context_facts().and_then(|facts| {
            facts
                .get(fact_key)
                .or_else(|| value_at_dotted_path_object(facts, fact_key))
        })?;
        if value.is_null() {
            return None;
        }
        Some(ReusableOutputMatch {
            reference: reference.to_string(),
            source: "intent_context".to_string(),
            stability: InputValueStability::Stable,
            observed_at_ms: None,
        })
    }
}

fn value_at_dotted_path<'a>(root: &'a Value, dotted: &str) -> Option<&'a Value> {
    let mut current = root;
    for segment in dotted.split('.').filter(|part| !part.is_empty()) {
        current = current.get(segment)?;
    }
    Some(current)
}

fn value_to_text(value: &Value) -> Option<String> {
    if let Some(text) = value.as_str() {
        return Some(text.to_string());
    }
    if value.is_number() || value.is_boolean() {
        return Some(value.to_string());
    }
    None
}

fn meta_for_slot<'a>(meta: &'a serde_json::Map<String, Value>, slot: &str) -> Option<&'a Value> {
    meta.get(slot)
        .or_else(|| value_at_dotted_path_object(meta, slot))
}

fn parse_stability(raw: &str) -> InputValueStability {
    match raw.trim() {
        "stable" => InputValueStability::Stable,
        "volatile" => InputValueStability::Volatile,
        _ => InputValueStability::Unknown,
    }
}

fn is_reusable(
    stability: InputValueStability,
    observed_at_ms: Option<u64>,
    now_ms: u64,
    max_age_ms: u64,
) -> bool {
    match stability {
        InputValueStability::Stable | InputValueStability::Unknown => true,
        InputValueStability::Volatile => {
            observed_at_ms.is_some_and(|timestamp| now_ms.saturating_sub(timestamp) <= max_age_ms)
        }
    }
}

fn volatile_signal_matches_key(signal: VolatileInputSignal, key: &str) -> bool {
    let lowered = key.to_ascii_lowercase();
    match signal {
        VolatileInputSignal::Balance => lowered.contains("balance"),
        VolatileInputSignal::Allowance => lowered.contains("allowance"),
    }
}

fn input_store_has_fresh_true_input_signal(
    store: &InputStore,
    signal: VolatileInputSignal,
    max_age_ms: u64,
    now_ms: u64,
) -> bool {
    store.list_semantic_ref_strings().into_iter().any(|slot| {
        store.get_semantic(slot.as_str()).is_some_and(|entry| {
            entry.meta.stability == InputValueStability::Volatile
                && volatile_signal_matches_key(signal, slot.as_str())
                && entry.meta.observed_at_ms.is_some_and(|observed_at_ms| {
                    now_ms.saturating_sub(observed_at_ms) <= max_age_ms
                })
        })
    })
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::input_store::{InputValueLayer, InputValueMeta};
    use serde_json::json;

    #[test]
    fn resolve_input_reference_prefers_runtime_inputs() {
        let state = EngineRunnerState {
            runtime: json!({
                "inputs": {
                    "recipient": "0xruntime",
                },
                "agent": {
                    "state_summary": {
                        "input_store": {
                            "facts": {
                                "recipient": "0xstore"
                            }
                        }
                    }
                }
            }),
            ..EngineRunnerState::default()
        };

        let view = ExecutionView::new(Some(&state));
        assert_eq!(
            view.resolve_input_reference_display("inputs.recipient")
                .as_deref(),
            Some("0xruntime")
        );
    }

    #[test]
    fn resolve_input_reference_prefers_input_store_over_runtime_facts_for_input_display() {
        let state = EngineRunnerState {
            runtime: json!({
                "agent": {
                    "state_summary": {
                        "runtime_facts": {
                            "facts": {
                                "inputs.token.address": "0xfacts"
                            }
                        },
                        "input_store": {
                            "facts": {
                                "token": {
                                    "address": "0xstore"
                                }
                            }
                        }
                    }
                }
            }),
            ..EngineRunnerState::default()
        };

        let view = ExecutionView::new(Some(&state));
        assert_eq!(
            view.resolve_input_reference_display("inputs.token.address"),
            Some("0xstore".to_string())
        );
    }

    #[test]
    fn resolve_input_reference_accepts_query_observed_input_store_projection() {
        let state = EngineRunnerState {
            runtime: json!({
                "agent": {
                    "state_summary": {
                        "input_store": {
                            "facts": {
                                "token": {
                                    "address": "0xmirrored"
                                }
                            },
                            "meta": {
                                "token": {
                                    "address": {
                                        "source": "query.auto_project"
                                    }
                                }
                            }
                        }
                    }
                }
            }),
            ..EngineRunnerState::default()
        };

        let view = ExecutionView::new(Some(&state));
        assert_eq!(
            view.resolve_input_reference_display("inputs.token.address"),
            Some("0xmirrored".to_string())
        );
    }

    #[test]
    fn resolve_input_reference_does_not_fall_through_to_intent_context_facts() {
        let state = EngineRunnerState {
            runtime: json!({
                "agent": {
                    "state_summary": {
                        "intent_context": {
                            "facts": {
                                "recipient": "0xfact-only"
                            }
                        }
                    }
                }
            }),
            ..EngineRunnerState::default()
        };

        let view = ExecutionView::new(Some(&state));
        assert_eq!(
            view.resolve_input_reference_display("inputs.recipient"),
            None
        );
    }

    #[test]
    fn reusable_inventory_accepts_query_observed_input_store_entries() {
        let mut input_store = InputStore::default();
        input_store.upsert(
            "inputs.native_balance",
            json!("100"),
            InputValueMeta {
                source: "query".to_string(),
                source_priority: 80,
                provenance: None,
                confidence: None,
                layer: InputValueLayer::Observed,
                stability: InputValueStability::Volatile,
                observed_at_ms: Some(100_000),
            },
        );

        let inventory = ReusableOutputInventory::new(None, None, Some(&input_store));
        let matched = inventory
            .reusable_reference("inputs.native_balance", 110_000)
            .expect("query-observed input should stay reusable");
        assert_eq!(matched.reference, "inputs.native_balance");
        assert_eq!(matched.source, "query");
    }

    #[test]
    fn reusable_inventory_accepts_true_input_store_entries() {
        let mut input_store = InputStore::default();
        input_store.upsert(
            "inputs.recipient",
            json!("0xabc"),
            InputValueMeta {
                source: "user".to_string(),
                source_priority: 80,
                provenance: None,
                confidence: None,
                layer: InputValueLayer::Seed,
                stability: InputValueStability::Stable,
                observed_at_ms: None,
            },
        );

        let inventory = ReusableOutputInventory::new(None, None, Some(&input_store));
        let matched = inventory
            .reusable_reference("inputs.recipient", 100_000)
            .expect("true input should remain reusable");
        assert_eq!(matched.reference, "inputs.recipient");
        assert_eq!(matched.source, "user");
    }

    #[test]
    fn reusable_inventory_uses_newest_equal_priority_input_store_freshness() {
        let mut input_store = InputStore::default();
        input_store.upsert(
            "inputs.native_balance",
            json!("100"),
            InputValueMeta {
                source: "query".to_string(),
                source_priority: 90,
                provenance: Some("segment_store.seg_1/q_balance.balance".to_string()),
                confidence: None,
                layer: InputValueLayer::Observed,
                stability: InputValueStability::Volatile,
                observed_at_ms: Some(10_000),
            },
        );
        input_store.upsert(
            "inputs.native_balance",
            json!("100"),
            InputValueMeta {
                source: "query".to_string(),
                source_priority: 90,
                provenance: Some("segment_store.seg_2/q_balance.balance".to_string()),
                confidence: None,
                layer: InputValueLayer::Observed,
                stability: InputValueStability::Volatile,
                observed_at_ms: Some(20_000),
            },
        );

        let inventory = ReusableOutputInventory::new(None, None, Some(&input_store));
        let matched = inventory
            .reusable_reference("inputs.native_balance", 49_000)
            .expect("newest equal-priority observation should stay fresh");
        assert_eq!(matched.reference, "inputs.native_balance");
        assert_eq!(matched.observed_at_ms, Some(20_000));
    }

    #[test]
    fn reusable_inventory_respects_pack_volatile_fact_policy_threshold() {
        let mut input_store = InputStore::default();
        input_store.upsert(
            "inputs.native_balance",
            json!("100"),
            InputValueMeta {
                source: "query".to_string(),
                source_priority: 90,
                provenance: Some("segment_store.seg_1/q_balance.balance".to_string()),
                confidence: None,
                layer: InputValueLayer::Observed,
                stability: InputValueStability::Volatile,
                observed_at_ms: Some(20_000),
            },
        );

        let inventory = ReusableOutputInventory::with_policy(
            None,
            None,
            Some(&input_store),
            VolatileFactsPolicy { max_age_ms: 60_000 },
        );
        let matched = inventory
            .reusable_reference("inputs.native_balance", 70_000)
            .expect("custom threshold should keep observation reusable");
        assert_eq!(matched.observed_at_ms, Some(20_000));
    }

    #[test]
    fn reusable_inventory_projection_deduplicates_fact_refs_by_precedence() {
        let mut runtime_facts = RuntimeFactsStore::default();
        runtime_facts.upsert(
            "facts.quote.price",
            json!("101.25"),
            InputValueMeta {
                source: "query".to_string(),
                source_priority: 90,
                provenance: Some("segment_store.seg_1/q_quote.price".to_string()),
                confidence: None,
                layer: InputValueLayer::Observed,
                stability: InputValueStability::Stable,
                observed_at_ms: Some(20_000),
            },
        );

        let projection = build_reusable_output_inventory_projection(Some(&runtime_facts), None)
            .expect("projection");

        assert_eq!(projection.pointer("/summary/total_refs"), Some(&json!(1)));
        assert_eq!(
            projection.pointer("/entries/0/ref"),
            Some(&json!("facts.quote.price"))
        );
        assert_eq!(
            projection.pointer("/entries/0/source"),
            Some(&json!("query"))
        );
    }
}
