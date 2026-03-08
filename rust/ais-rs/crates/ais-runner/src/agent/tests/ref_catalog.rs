use super::*;
use serde_json::json;

#[test]
fn build_ref_catalog_collects_input_store_and_node_output_refs() {
    let summary = json!({
        "input_store": {
            "facts": {
                "owner": "0x1111111111111111111111111111111111111111",
                "token": {"address":"0x2222222222222222222222222222222222222222"}
            },
            "meta": {
                "owner": {"source":"user","source_priority":100,"observed_at_ms":123},
                "token.address": {"source":"query","source_priority":90}
            }
        },
        "node_output_refs": {
            "known_refs": [
                "nodes.q_balance.outputs.balance",
                "nodes.q_allowance.outputs.amount"
            ]
        },
        "nodes": {
            "seg_1/q_balance": {
                "outputs": {
                    "balance": "100"
                }
            },
            "seg_1/q_allowance": {
                "outputs": {
                    "amount": "20"
                }
            }
        }
    });

    let catalog = RefCatalog::build(Some(&summary));
    assert!(catalog
        .entries
        .iter()
        .any(|entry| entry.canonical_ref == "inputs.owner"
            && entry.value_available
            && entry.source == "user"
            && entry.source_priority == 100));
    assert!(catalog.entries.iter().any(|entry| {
        entry.canonical_ref == "inputs.token.address"
            && entry.source == "query"
            && entry.source_priority == 90
            && entry.value_available
    }));
    assert!(catalog.entries.iter().any(|entry| {
        entry.canonical_ref == "nodes.q_balance.outputs.balance"
            && entry.value_available
            && entry.producer_step.as_deref() == Some("q_balance")
    }));
}

#[test]
fn build_ref_catalog_does_not_treat_intent_context_as_fact_inventory() {
    let summary = json!({
        "intent_context": {
            "facts": {
                "quote": {
                    "price": "1.02"
                }
            }
        }
    });

    let catalog = RefCatalog::build(Some(&summary));
    assert!(catalog.entries.is_empty());
}

#[test]
fn available_input_ref_catalog_filters_non_input_refs() {
    let summary = json!({
        "input_store": {
            "facts": {"owner":"0xabc"},
            "meta": {"owner":{"source":"user","source_priority":100}}
        },
        "node_output_refs": {
            "known_refs": ["nodes.q_balance.outputs.balance"]
        },
        "nodes": {
            "seg_1/q_balance": {
                "outputs": {
                    "balance": 1
                }
            }
        }
    });

    let refs = available_input_ref_catalog(Some(&summary));
    assert_eq!(refs.len(), 1);
    assert_eq!(
        refs[0].get("ref").and_then(serde_json::Value::as_str),
        Some("inputs.owner")
    );
}

#[test]
fn build_ref_catalog_reads_query_observed_inputs_from_input_store() {
    let typed = StateSummary {
        completed_segments: 0,
        completed_nodes: 0,
        plan_epoch: 0,
        paused_reason: None,
        done: false,
        previous_error: None,
        input_store: Some(json!({
            "facts": {"token.decimals": 6},
            "meta": {"token.decimals": {"source":"seed","source_priority":10}}
        })),
        runtime_facts: Some(json!({
            "facts": {"facts.quote.price": "1.00"},
            "meta": {"facts.quote.price": {"source":"query","source_priority":90,"observed_at_ms":123}}
        })),
        input_binding: super::super::state_summary::InputBindingContract {
            schema: "ais-agent-input-binding-contract/0.0.1",
            bindable_namespace: "inputs",
            bindable_refs_source: "state_summary.input_store",
            bindable_refs_projection: "state_summary.input_registry.known_refs",
            known_refs_only: true,
            facts_bindable: false,
        },
        input_registry: json!({"known_refs": ["inputs.token.decimals"]}),
        node_output_refs: json!({"known_refs": []}),
        reusable_outputs: None,
        tool_memory_projection: None,
        intent_slots: None,
        intent_context: None,
        capability_view: None,
        capability_ready: None,
        side_effect_lifecycle: None,
        todo_state: None,
        recovery_diagnostics: None,
    };

    let catalog = RefCatalog::build_typed(Some(&typed));
    let entry = catalog
        .entries
        .iter()
        .find(|entry| entry.canonical_ref == "inputs.token.decimals")
        .expect("inputs.token.decimals ref");
    assert_eq!(entry.source, "seed");
    assert_eq!(entry.source_priority, 10);
}

#[test]
fn build_ref_catalog_accepts_query_observed_input_store_entries_without_runtime_facts() {
    let typed = StateSummary {
        completed_segments: 0,
        completed_nodes: 0,
        plan_epoch: 0,
        paused_reason: None,
        done: false,
        previous_error: None,
        input_store: Some(json!({
            "facts": {"token.decimals": 6},
            "meta": {"token.decimals": {"source":"query","source_priority":90}}
        })),
        runtime_facts: None,
        input_binding: super::super::state_summary::InputBindingContract {
            schema: "ais-agent-input-binding-contract/0.0.1",
            bindable_namespace: "inputs",
            bindable_refs_source: "state_summary.input_store",
            bindable_refs_projection: "state_summary.input_registry.known_refs",
            known_refs_only: true,
            facts_bindable: false,
        },
        input_registry: json!({"known_refs": ["inputs.token.decimals"]}),
        node_output_refs: json!({"known_refs": []}),
        reusable_outputs: None,
        tool_memory_projection: None,
        intent_slots: None,
        intent_context: None,
        capability_view: None,
        capability_ready: None,
        side_effect_lifecycle: None,
        todo_state: None,
        recovery_diagnostics: None,
    };

    let catalog = RefCatalog::build_typed(Some(&typed));
    assert!(catalog
        .entries
        .iter()
        .any(|entry| entry.canonical_ref == "inputs.token.decimals"
            && entry.source == "query"
            && entry.source_priority == 90));
}

#[test]
fn build_ref_catalog_keeps_node_output_refs_without_input_store() {
    let summary = json!({
        "node_output_refs": {
            "known_refs": [
                "nodes.q_quote.outputs.amount_out",
                "nodes.q_quote.outputs.price_impact_bps"
            ]
        },
        "nodes": {
            "seg_quote__q_quote": {
                "outputs": {
                    "amount_out": 1000,
                    "price_impact_bps": 25
                }
            }
        }
    });
    let catalog = RefCatalog::build(Some(&summary));
    assert!(catalog.entries.iter().any(|entry| {
        entry.canonical_ref == "nodes.q_quote.outputs.amount_out"
            && entry.value_available
            && entry.producer_step.as_deref() == Some("q_quote")
            && entry.source == "node_output_refs"
    }));
    assert!(catalog.entries.iter().any(|entry| {
        entry.canonical_ref == "nodes.q_quote.outputs.price_impact_bps" && entry.value_available
    }));
}

#[test]
fn build_ref_catalog_marks_node_ref_unavailable_for_null_or_empty_outputs() {
    let summary = json!({
        "node_output_refs": {
            "known_refs": [
                "nodes.q_quote.outputs.price",
                "nodes.q_quote.outputs.meta",
                "nodes.q_quote.outputs.path"
            ]
        },
        "nodes": {
            "seg_quote__q_quote": {
                "outputs": {
                    "price": null,
                    "meta": {},
                    "path": []
                }
            }
        }
    });
    let catalog = RefCatalog::build(Some(&summary));
    assert!(catalog.entries.iter().all(|entry| {
        if entry.canonical_ref.starts_with("nodes.q_quote.outputs.") {
            !entry.value_available
        } else {
            true
        }
    }));
}
