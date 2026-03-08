use super::*;
use serde_json::json;
use std::collections::BTreeSet;

#[test]
fn collect_missing_refs_from_payload_normalizes_and_expands() {
    let payload = json!({
        "missing_refs": [
            " runtime.inputs.owner.value ",
            {"ref":"inputs.token", "missing_ref_fields":["inputs.token.address", "inputs.token.decimals"]},
            {"missing_ref":"params.owner"},
            {"path":"inputs.receiver"},
            {"path":"facts.quote.price"}
        ]
    });

    assert_eq!(
        collect_missing_refs_from_payload(&payload),
        vec![
            "facts.quote.price".to_string(),
            "inputs.owner".to_string(),
            "inputs.receiver".to_string(),
            "inputs.token".to_string(),
            "inputs.token.address".to_string(),
            "inputs.token.decimals".to_string(),
        ]
    );
}

#[test]
fn collect_missing_items_keeps_typed_ref_and_source() {
    let payload = json!({
        "missing_refs": ["inputs.token.decimals"],
        "questions": [{"id":"owner","question":"Need owner"}]
    });

    let items = collect_missing_items(&payload);
    assert_eq!(items.len(), 2);
    assert!(items.iter().any(|item| {
        item.missing_ref
            == super::super::ref_model::RefPath::Input {
                slot: "token.decimals".to_string(),
            }
            && item.source == MissingItemSource::MissingRefsField
    }));
    assert!(items.iter().any(|item| {
        item.missing_ref
            == super::super::ref_model::RefPath::Input {
                slot: "owner".to_string(),
            }
            && item.source == MissingItemSource::QuestionsField
    }));
}

#[test]
fn collect_todo_precheck_missing_refs_filters_available_refs() {
    let required_facts = vec![
        "inputs.owner".to_string(),
        "token.decimals".to_string(),
        "facts.quote.price".to_string(),
    ];
    let refs = collect_todo_precheck_missing_refs(required_facts.as_slice(), |reference| {
        reference == "inputs.owner"
    });
    assert_eq!(
        refs,
        vec![
            "facts.quote.price".to_string(),
            "inputs.token.decimals".to_string(),
        ]
    );
}

#[test]
fn collect_todo_precheck_missing_refs_keeps_non_input_namespaces() {
    let required_facts = vec![
        "facts.quote.price".to_string(),
        "nodes.q_balance.outputs.balance".to_string(),
    ];
    let refs = collect_todo_precheck_missing_refs(required_facts.as_slice(), |_reference| false);
    assert_eq!(
        refs,
        vec![
            "facts.quote.price".to_string(),
            "nodes.q_balance.outputs.balance".to_string(),
        ]
    );
}

#[test]
fn collect_compile_missing_input_merges_unknown_and_write_gate_sources() {
    let payload = json!({
        "message":"compile failed suggested_ref=inputs.recipient",
        "issues":[
            {
                "reference":"unknown_input_ref",
                "kind":"compile",
                "suggested_ref":"inputs.owner",
                "candidates":["token.decimals","facts.quote.price"],
                "message":"unknown input `token.decimals`"
            },
            {
                "reference":"gate",
                "kind":"write_gate_missing",
                "reason_code":"missing_required_input",
                "required_fact":"token.address",
                "message":"missing `inputs.token.decimals`"
            }
        ]
    });
    let collected = collect_compile_missing_input(&payload);
    assert_eq!(collected.issues.len(), 2);
    assert_eq!(
        collected.missing_refs,
        vec![
            "inputs.owner".to_string(),
            "inputs.recipient".to_string(),
            "inputs.token.address".to_string(),
            "inputs.token.decimals".to_string(),
        ]
    );
}

#[test]
fn collect_compile_missing_input_accepts_missing_token_decimals_without_message_refs() {
    let payload = json!({
        "issues":[
            {
                "reference":"gate",
                "kind":"write_gate_missing",
                "reason_code":"missing_token_decimals",
                "required_fact":"token.decimals",
                "message":"asset decimals unavailable"
            }
        ]
    });
    let collected = collect_compile_missing_input(&payload);
    assert_eq!(collected.issues.len(), 1);
    assert_eq!(
        collected.missing_refs,
        vec!["inputs.token.decimals".to_string()]
    );
}

#[test]
fn collect_missing_refs_from_message_supports_backtick_and_suggested_ref() {
    let mut refs = BTreeSet::<String>::new();
    collect_missing_refs_from_message(
        "need `inputs.owner` and suggested_ref=token.decimals",
        &mut refs,
    );
    assert_eq!(
        refs.into_iter().collect::<Vec<_>>(),
        vec![
            "inputs.owner".to_string(),
            "inputs.token.decimals".to_string(),
        ]
    );
}

#[test]
fn collect_missing_refs_from_payload_keeps_facts_and_nodes() {
    let payload = json!({
        "missing_refs": [
            "facts.quote.price",
            {"path":"nodes.q_balance.outputs.balance"},
            {"missing_ref":"input.owner"}
        ]
    });
    assert_eq!(
        collect_missing_refs_from_payload(&payload),
        vec![
            "facts.quote.price".to_string(),
            "nodes.q_balance.outputs.balance".to_string(),
        ]
    );
}
