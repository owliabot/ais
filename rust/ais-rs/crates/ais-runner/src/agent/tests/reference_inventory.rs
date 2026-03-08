use super::super::reference_inventory::ReferenceInventory;
use super::super::state_summary::{InputBindingContract, StateSummary};
use serde_json::json;

#[test]
fn reference_inventory_build_typed_collects_inputs_facts_and_node_outputs() {
    let typed = StateSummary {
        completed_segments: 0,
        completed_nodes: 0,
        plan_epoch: 0,
        paused_reason: None,
        done: false,
        previous_error: None,
        input_store: Some(json!({
            "facts": {"token.decimals": 6},
            "meta": {"token.decimals": {"source":"query","source_priority":90,"observed_at_ms":123}}
        })),
        runtime_facts: Some(json!({
            "facts": {"facts.quote.price": "1.00"},
            "meta": {"facts.quote.price": {"source":"query","source_priority":80,"observed_at_ms":456}}
        })),
        input_binding: InputBindingContract {
            schema: "ais-agent-input-binding-contract/0.0.1",
            bindable_namespace: "inputs",
            bindable_refs_source: "state_summary.input_store",
            bindable_refs_projection: "state_summary.input_registry.known_refs",
            known_refs_only: true,
            facts_bindable: false,
        },
        input_registry: json!({"known_refs":["inputs.token.decimals"]}),
        node_output_refs: json!({"known_refs":["nodes.q_balance.outputs.balance"]}),
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

    let inventory = ReferenceInventory::build_typed(Some(&typed));
    assert!(inventory
        .entries
        .iter()
        .any(|entry| entry.canonical_ref == "inputs.token.decimals"
            && entry.source == "query"
            && entry.value_available));
    assert!(inventory
        .entries
        .iter()
        .any(|entry| entry.canonical_ref == "facts.quote.price"
            && entry.source == "query"
            && entry.value_available));
    assert!(inventory.entries.iter().any(|entry| entry.canonical_ref
        == "nodes.q_balance.outputs.balance"
        && entry.source == "node_output_refs"
        && !entry.value_available));
}

#[test]
fn reference_inventory_input_refs_returns_only_inputs_namespace() {
    let typed = StateSummary {
        completed_segments: 0,
        completed_nodes: 0,
        plan_epoch: 0,
        paused_reason: None,
        done: false,
        previous_error: None,
        input_store: Some(json!({
            "facts": {"owner": "0xabc"},
            "meta": {"owner": {"source":"user","source_priority":100}}
        })),
        runtime_facts: Some(json!({
            "facts": {"facts.quote.price": "1.00"},
            "meta": {"facts.quote.price": {"source":"query","source_priority":80}}
        })),
        input_binding: InputBindingContract {
            schema: "ais-agent-input-binding-contract/0.0.1",
            bindable_namespace: "inputs",
            bindable_refs_source: "state_summary.input_store",
            bindable_refs_projection: "state_summary.input_registry.known_refs",
            known_refs_only: true,
            facts_bindable: false,
        },
        input_registry: json!({"known_refs":["inputs.owner"]}),
        node_output_refs: json!({"known_refs":["nodes.q_balance.outputs.balance"]}),
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

    let refs = ReferenceInventory::build_typed(Some(&typed)).input_refs();
    assert_eq!(refs, vec!["inputs.owner".to_string()]);
}
