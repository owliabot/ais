use serde::Deserialize;
use serde_json::{json, Value};

use super::super::candidates::CandidateContext;
use super::super::input_store::InputStore;
use super::super::intent_segmented::resolve_missing_facts_for_refs;
use super::super::runtime_facts_store::RuntimeFactsStore;
use super::super::state_summary::StateSummary;

#[derive(Debug, Deserialize)]
pub(crate) struct RuntimeQueryArgs {
    pub(crate) action: String,
    #[serde(default)]
    pub(crate) refs: Vec<String>,
}

/// Handle `runtime.query(action=inspect)` by looking up ref values from
/// the state_summary and input_store that the planner already has access to.
pub(crate) fn handle_inspect(
    args: &RuntimeQueryArgs,
    typed_summary: Option<&StateSummary>,
    runtime_facts_store: Option<&RuntimeFactsStore>,
    input_store: Option<&InputStore>,
) -> Value {
    let mut results = Vec::<Value>::new();
    for ref_key in &args.refs {
        results.push(resolve_ref_value(
            ref_key.as_str(),
            typed_summary,
            runtime_facts_store,
            input_store,
        ));
    }
    json!({
        "action": "inspect",
        "results": results,
    })
}

/// Handle `runtime.query(action=resolve)` — return resolution status for
/// each ref: whether it's already resolved (with value), or what query
/// candidates are available if it's missing.
pub(crate) fn handle_resolve(
    args: &RuntimeQueryArgs,
    typed_summary: Option<&StateSummary>,
    runtime_facts_store: Option<&RuntimeFactsStore>,
    input_store: Option<&InputStore>,
    candidate_context: Option<&CandidateContext>,
) -> Value {
    let mut results = Vec::<Value>::new();
    for ref_key in &args.refs {
        let inspect_result = resolve_ref_value(
            ref_key.as_str(),
            typed_summary,
            runtime_facts_store,
            input_store,
        );
        if inspect_result.get("status").and_then(Value::as_str) == Some("resolved") {
            results.push(inspect_result);
            continue;
        }

        // Ref is unresolved — enrich with query candidate info
        let mut entry = json!({
            "ref": ref_key,
            "status": "unresolved",
        });
        if let Some(context) = candidate_context {
            let resolution = resolve_missing_facts_for_refs(context, &[ref_key.clone()], 3);
            let has_candidates = resolution
                .pointer("/resolved")
                .and_then(Value::as_array)
                .is_some_and(|arr| !arr.is_empty());
            entry["resolution"] = json!({
                "has_candidates": has_candidates,
                "detail": resolution,
            });
        }
        results.push(entry);
    }
    json!({
        "action": "resolve",
        "results": results,
    })
}

fn resolve_ref_value(
    ref_key: &str,
    typed_summary: Option<&StateSummary>,
    runtime_facts_store: Option<&RuntimeFactsStore>,
    input_store: Option<&InputStore>,
) -> Value {
    let trimmed = ref_key.trim();
    if trimmed.is_empty() {
        return json!({
            "ref": ref_key,
            "status": "error",
            "error": "empty ref key",
        });
    }

    if let Some(slot) = trimmed.strip_prefix("inputs.") {
        return resolve_input_ref(
            trimmed,
            slot,
            typed_summary,
            runtime_facts_store,
            input_store,
        );
    }

    if let Some(fact_key) = trimmed.strip_prefix("facts.") {
        return resolve_fact_ref(trimmed, fact_key, typed_summary, runtime_facts_store);
    }

    if trimmed.starts_with("nodes.") {
        return resolve_node_output_ref(trimmed, typed_summary);
    }

    json!({
        "ref": ref_key,
        "status": "error",
        "error": "unsupported ref namespace; expected inputs.*, facts.*, or nodes.*.outputs.*",
    })
}

fn resolve_input_ref(
    full_ref: &str,
    slot: &str,
    typed_summary: Option<&StateSummary>,
    _runtime_facts_store: Option<&RuntimeFactsStore>,
    input_store: Option<&InputStore>,
) -> Value {
    // Primary: check InputStore directly
    if let Some(store) = input_store {
        if let Some(entry) = store.get_projected(slot) {
            return json!({
                "ref": full_ref,
                "status": "resolved",
                "value": entry.value,
                "source": entry.meta.source,
                "layer": entry.meta.layer,
                "stability": entry.meta.stability,
            });
        }
    }

    if let Some(summary) = typed_summary {
        let input_meta = summary.input_store_meta().and_then(|meta| {
            meta.get(slot)
                .or_else(|| value_at_dotted_path_object(meta, slot))
        });
        if input_meta.is_some() {
            if let Some(value) = summary.input_store_facts().and_then(|facts| {
                facts
                    .get(slot)
                    .or_else(|| value_at_dotted_path_object(facts, slot))
            }) {
                return json!({
                    "ref": full_ref,
                    "status": "resolved",
                    "value": value,
                    "source": "input_store_projection",
                });
            }
        }
    }

    if let Some(value) =
        typed_summary.and_then(|summary| summary.intent_slots_view().resolved_input(slot))
    {
        return json!({
            "ref": full_ref,
            "status": "resolved",
            "value": value,
            "source": "grounding",
        });
    }

    json!({
        "ref": full_ref,
        "status": "unresolved",
    })
}

fn resolve_fact_ref(
    full_ref: &str,
    _fact_key: &str,
    typed_summary: Option<&StateSummary>,
    runtime_facts_store: Option<&RuntimeFactsStore>,
) -> Value {
    if let Some(store) = runtime_facts_store {
        if let Some(entry) = store.get(full_ref) {
            return json!({
                "ref": full_ref,
                "status": "resolved",
                "value": entry.value,
                "source": entry.meta.source,
                "layer": entry.meta.layer,
                "stability": entry.meta.stability,
            });
        }
    }
    if let Some((value, source)) = typed_summary.and_then(|summary| {
        let view = summary.runtime_facts_view();
        view.fact(full_ref).map(|value| {
            let source = view
                .meta(full_ref)
                .and_then(|meta| meta.get("source"))
                .and_then(Value::as_str)
                .unwrap_or("runtime_facts_projection");
            (value, source)
        })
    }) {
        return json!({
            "ref": full_ref,
            "status": "resolved",
            "value": value,
            "source": source,
        });
    }

    json!({
        "ref": full_ref,
        "status": "unresolved",
    })
}

fn resolve_node_output_ref(full_ref: &str, typed_summary: Option<&StateSummary>) -> Value {
    // Expected: nodes.<step_id>.outputs.<field_path...>
    let parts: Vec<&str> = full_ref.splitn(4, '.').collect();
    if parts.len() < 4 || parts[2] != "outputs" {
        return json!({
            "ref": full_ref,
            "status": "error",
            "error": "invalid node output ref format; expected nodes.<step_id>.outputs.<field>",
        });
    }

    let known_typed = typed_summary
        .map(|summary| {
            summary
                .node_output_refs_known_refs()
                .iter()
                .any(|raw| *raw == full_ref)
        })
        .unwrap_or(false);
    if known_typed {
        return json!({
            "ref": full_ref,
            "status": "resolved",
            "source": "node_output",
        });
    }

    json!({
        "ref": full_ref,
        "status": "unresolved",
    })
}

fn value_at_dotted_path_object<'a>(
    map: &'a serde_json::Map<String, Value>,
    dotted: &str,
) -> Option<&'a Value> {
    let mut segments = dotted.split('.').filter(|part| !part.is_empty());
    let first = segments.next()?;
    let mut current = map.get(first)?;
    for segment in segments {
        current = current.get(segment)?;
    }
    Some(current)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::input_store::{InputStore, InputValueMeta};
    use crate::agent::runtime_facts_store::RuntimeFactsStore;
    use crate::agent::state_summary::InputBindingContract;

    fn build_test_summary(
        input_store: Option<Value>,
        runtime_facts: Option<Value>,
        intent_slots: Option<Value>,
        intent_context: Option<Value>,
        node_output_refs: Option<Value>,
    ) -> StateSummary {
        StateSummary {
            completed_segments: 0,
            completed_nodes: 0,
            plan_epoch: 0,
            paused_reason: None,
            done: false,
            previous_error: None,
            input_store,
            runtime_facts,
            input_binding: InputBindingContract {
                schema: "ais-agent-input-binding-contract/0.0.1",
                bindable_namespace: "inputs",
                bindable_refs_source: "state_summary.input_store",
                bindable_refs_projection: "state_summary.input_registry.known_refs",
                known_refs_only: true,
                facts_bindable: false,
            },
            input_registry: json!({"known_refs": []}),
            node_output_refs: node_output_refs.unwrap_or_else(|| json!({"known_refs": []})),
            reusable_outputs: None,
            tool_memory_projection: None,
            intent_slots,
            intent_context,
            capability_view: None,
            capability_ready: None,
            side_effect_lifecycle: None,
            todo_state: None,
            recovery_diagnostics: None,
        }
    }

    #[test]
    fn inspect_input_from_store() {
        let mut store = InputStore::default();
        store.upsert(
            "owner",
            json!("0xABC"),
            InputValueMeta {
                source: "user".to_string(),
                ..Default::default()
            },
        );
        let args = RuntimeQueryArgs {
            action: "inspect".to_string(),
            refs: vec!["inputs.owner".to_string()],
        };
        let result = handle_inspect(&args, None, None, Some(&store));
        let entry = &result["results"][0];
        assert_eq!(entry["status"], "resolved");
        assert_eq!(entry["value"], "0xABC");
        assert_eq!(entry["source"], "user");
    }

    #[test]
    fn inspect_input_reads_canonical_input_store_even_when_runtime_facts_carries_fact_only_values()
    {
        let mut input_store = InputStore::default();
        input_store.upsert(
            "token.decimals",
            json!(6),
            InputValueMeta {
                source: "seed".to_string(),
                source_priority: 10,
                ..Default::default()
            },
        );
        let mut runtime_facts = RuntimeFactsStore::default();
        runtime_facts.upsert(
            "facts.token.decimals",
            json!(18),
            InputValueMeta {
                source: "query".to_string(),
                source_priority: 90,
                ..Default::default()
            },
        );
        let args = RuntimeQueryArgs {
            action: "inspect".to_string(),
            refs: vec!["inputs.token.decimals".to_string()],
        };
        let result = handle_inspect(&args, None, Some(&runtime_facts), Some(&input_store));
        let entry = &result["results"][0];
        assert_eq!(entry["status"], "resolved");
        assert_eq!(entry["value"], 6);
        assert_eq!(entry["source"], "seed");
    }

    #[test]
    fn inspect_input_accepts_query_observed_input_store_entries() {
        let mut input_store = InputStore::default();
        input_store.upsert(
            "token.decimals",
            json!(6),
            InputValueMeta {
                source: "query".to_string(),
                source_priority: 90,
                ..Default::default()
            },
        );
        let args = RuntimeQueryArgs {
            action: "inspect".to_string(),
            refs: vec!["inputs.token.decimals".to_string()],
        };
        let result = handle_inspect(&args, None, None, Some(&input_store));
        let entry = &result["results"][0];
        assert_eq!(entry["status"], "resolved");
        assert_eq!(entry["value"], 6);
        assert_eq!(entry["source"], "query");
    }

    #[test]
    fn inspect_input_reads_projected_asset_root_from_input_store() {
        let mut input_store = InputStore::default();
        input_store.upsert(
            "token",
            json!({
                "address": "0xabc",
                "decimals": "6"
            }),
            InputValueMeta {
                source: "query".to_string(),
                source_priority: 90,
                ..Default::default()
            },
        );
        let args = RuntimeQueryArgs {
            action: "inspect".to_string(),
            refs: vec!["inputs.token".to_string()],
        };
        let result = handle_inspect(&args, None, None, Some(&input_store));
        let entry = &result["results"][0];
        assert_eq!(entry["status"], "resolved");
        assert_eq!(entry["value"]["address"], "0xabc");
        assert_eq!(entry["value"]["decimals"], 6);
        assert_eq!(entry["source"], "derived");
    }

    #[test]
    fn inspect_input_from_state_summary_fallback() {
        let summary = build_test_summary(
            None,
            None,
            Some(json!({
                "resolved_inputs": {
                    "owner": "0x70997970C51812dc3A010C7d01b50e0d17dc79C8"
                }
            })),
            None,
            None,
        );
        let args = RuntimeQueryArgs {
            action: "inspect".to_string(),
            refs: vec!["inputs.owner".to_string()],
        };
        let result = handle_inspect(&args, Some(&summary), None, None);
        let entry = &result["results"][0];
        assert_eq!(entry["status"], "resolved");
        assert_eq!(entry["source"], "grounding");
    }

    #[test]
    fn inspect_fact_does_not_treat_input_store_projection_as_runtime_fact_truth() {
        let summary = build_test_summary(
            Some(json!({"facts": {"token": {"decimals": 18}}, "meta": {}})),
            None,
            None,
            None,
            None,
        );
        let args = RuntimeQueryArgs {
            action: "inspect".to_string(),
            refs: vec!["facts.token.decimals".to_string()],
        };
        let result = handle_inspect(&args, Some(&summary), None, None);
        let entry = &result["results"][0];
        assert_eq!(entry["status"], "unresolved");
    }

    #[test]
    fn inspect_node_output_known() {
        let summary = build_test_summary(
            None,
            None,
            None,
            None,
            Some(json!({"known_refs": ["nodes.q_balance.outputs.balance"]})),
        );
        let args = RuntimeQueryArgs {
            action: "inspect".to_string(),
            refs: vec!["nodes.q_balance.outputs.balance".to_string()],
        };
        let result = handle_inspect(&args, Some(&summary), None, None);
        let entry = &result["results"][0];
        assert_eq!(entry["status"], "resolved");
    }

    #[test]
    fn inspect_unresolved_refs() {
        let args = RuntimeQueryArgs {
            action: "inspect".to_string(),
            refs: vec![
                "inputs.nonexistent".to_string(),
                "facts.no_such_fact".to_string(),
                "nodes.missing.outputs.x".to_string(),
            ],
        };
        let result = handle_inspect(&args, None, None, None);
        for entry in result["results"].as_array().unwrap() {
            assert_eq!(entry["status"], "unresolved");
        }
    }

    #[test]
    fn inspect_mixed_namespaces() {
        let mut store = InputStore::default();
        store.upsert(
            "amount",
            json!("500"),
            InputValueMeta {
                source: "user".to_string(),
                ..Default::default()
            },
        );
        let summary = build_test_summary(
            Some(json!({"facts": {"token": {"decimals": 18}}, "meta": {}})),
            None,
            None,
            None,
            Some(json!({"known_refs": ["nodes.q_balance.outputs.balance"]})),
        );
        let args = RuntimeQueryArgs {
            action: "inspect".to_string(),
            refs: vec![
                "inputs.amount".to_string(),
                "facts.token.decimals".to_string(),
                "nodes.q_balance.outputs.balance".to_string(),
                "inputs.nonexistent".to_string(),
            ],
        };
        let result = handle_inspect(&args, Some(&summary), None, Some(&store));
        let results = result["results"].as_array().unwrap();
        assert_eq!(results[0]["status"], "resolved");
        assert_eq!(results[0]["value"], "500");
        assert_eq!(results[1]["status"], "unresolved");
        assert_eq!(results[2]["status"], "resolved");
        assert_eq!(results[3]["status"], "unresolved");
    }

    #[test]
    fn inspect_fact_does_not_fall_through_to_intent_context_facts() {
        let summary = build_test_summary(
            None,
            None,
            None,
            Some(json!({
                "facts": {
                    "quote": {
                        "price": "123.45"
                    }
                }
            })),
            None,
        );
        let args = RuntimeQueryArgs {
            action: "inspect".to_string(),
            refs: vec!["facts.quote.price".to_string()],
        };
        let result = handle_inspect(&args, Some(&summary), None, None);
        let entry = &result["results"][0];
        assert_eq!(entry["status"], "unresolved");
    }

    #[test]
    fn inspect_fact_typed_view_reads_runtime_facts_meta_source() {
        let summary = build_test_summary(
            None,
            Some(json!({
                "facts": {
                    "facts.quote.price": "123.45"
                },
                "meta": {
                    "facts.quote.price": {
                        "source": "query.quote"
                    }
                }
            })),
            None,
            None,
            None,
        );
        let args = RuntimeQueryArgs {
            action: "inspect".to_string(),
            refs: vec!["facts.quote.price".to_string()],
        };
        let result = handle_inspect(&args, Some(&summary), None, None);
        let entry = &result["results"][0];
        assert_eq!(entry["status"], "resolved");
        assert_eq!(entry["source"], "query.quote");
    }

    #[test]
    fn inspect_fact_does_not_treat_resolved_inputs_as_facts() {
        let summary = build_test_summary(
            None,
            None,
            Some(json!({
                "resolved_inputs": {
                    "token.decimals": 18
                }
            })),
            None,
            None,
        );
        let args = RuntimeQueryArgs {
            action: "inspect".to_string(),
            refs: vec!["facts.token.decimals".to_string()],
        };
        let result = handle_inspect(&args, Some(&summary), None, None);
        let entry = &result["results"][0];
        assert_eq!(entry["status"], "unresolved");
    }

    #[test]
    fn inspect_invalid_namespace() {
        let args = RuntimeQueryArgs {
            action: "inspect".to_string(),
            refs: vec!["unknown.ref".to_string()],
        };
        let result = handle_inspect(&args, None, None, None);
        assert_eq!(result["results"][0]["status"], "error");
    }

    #[test]
    fn resolve_already_resolved_returns_value() {
        let mut store = InputStore::default();
        store.upsert(
            "owner",
            json!("0xABC"),
            InputValueMeta {
                source: "user".to_string(),
                ..Default::default()
            },
        );
        let args = RuntimeQueryArgs {
            action: "resolve".to_string(),
            refs: vec!["inputs.owner".to_string()],
        };
        let result = handle_resolve(&args, None, None, Some(&store), None);
        let entry = &result["results"][0];
        assert_eq!(entry["status"], "resolved");
        assert_eq!(entry["value"], "0xABC");
    }

    #[test]
    fn resolve_unresolved_without_candidates() {
        let args = RuntimeQueryArgs {
            action: "resolve".to_string(),
            refs: vec!["inputs.nonexistent".to_string()],
        };
        let result = handle_resolve(&args, None, None, None, None);
        let entry = &result["results"][0];
        assert_eq!(entry["status"], "unresolved");
        assert!(entry.get("resolution").is_none());
    }

    #[test]
    fn inspect_invalid_node_output_format() {
        let args = RuntimeQueryArgs {
            action: "inspect".to_string(),
            refs: vec!["nodes.step_id.wrong_key".to_string()],
        };
        let result = handle_inspect(&args, None, None, None);
        assert_eq!(result["results"][0]["status"], "error");
    }
}
