use super::*;
use crate::agent::missing_resolution::static_refill;
use crate::agent::ref_model::RefPath;
use serde_json::json;

#[test]
fn preserve_autofill_context_copies_previous_envelope() {
    let previous_error = json!({
        "reason_code": "missing_required_input",
        "autofill": {
            "mode": "host_missing_input_round",
            "selected_query_refs": ["erc20@0.0.2/decimals"]
        }
    });
    let mut payload = json!({
        "reason_code": "schema_invalid"
    });

    preserve_autofill_context(Some(&previous_error), &mut payload);

    assert_eq!(
        payload.pointer("/autofill/mode").and_then(Value::as_str),
        Some("host_missing_input_round")
    );
    assert_eq!(
        payload
            .pointer("/autofill/selected_query_refs/0")
            .and_then(Value::as_str),
        Some("erc20@0.0.2/decimals")
    );
}

#[test]
fn missing_required_input_refs_normalizes_and_expands() {
    let payload = json!({
        "missing_refs": [
            " runtime.inputs.owner.value ",
            {"ref":"inputs.token", "missing_ref_fields":["inputs.token.address", "inputs.token.decimals"]},
            {"missing_ref":"params.owner"},
            {"path":"inputs.receiver"}
        ]
    });

    assert_eq!(
        missing_required_input_refs(&payload),
        vec![
            "inputs.owner".to_string(),
            "inputs.receiver".to_string(),
            "inputs.token".to_string(),
            "inputs.token.address".to_string(),
            "inputs.token.decimals".to_string(),
        ]
    );
}

#[test]
fn missing_required_refs_keeps_non_input_namespaces() {
    let payload = json!({
        "missing_refs": [
            "facts.quote.price",
            {"path":"nodes.q_balance.outputs.balance"}
        ],
        "questions": [
            {"id":"facts.quote.price","question":"Need quote"}
        ]
    });
    assert_eq!(
        missing_required_refs(&payload),
        vec![
            "facts.quote.price".to_string(),
            "nodes.q_balance.outputs.balance".to_string(),
        ]
    );
    assert_eq!(
        payload_question_refs(&payload),
        vec!["facts.quote.price".to_string()]
    );
}

#[test]
fn query_recoverable_missing_refs_returns_only_refs_with_candidates() {
    let payload = json!({
        "resolved": [
            {"missing_ref":"inputs.token.decimals","query_candidates":[{"query_ref":"erc20@0.0.2/decimals"}]},
            {"missing_ref":"inputs.owner","query_candidates":[]}
        ]
    });

    assert_eq!(
        query_recoverable_missing_refs(&payload)
            .into_iter()
            .collect::<Vec<_>>(),
        vec!["inputs.token.decimals".to_string()]
    );
}

#[test]
fn selected_query_refs_from_missing_resolution_dedups_first_candidates() {
    let payload = json!({
        "resolved": [
            {
                "missing_ref":"inputs.token.decimals",
                "query_candidates":[
                    {"query_ref":"erc20@0.0.2/decimals"},
                    {"query_ref":"alt@0.0.1/read-decimals"}
                ]
            },
            {
                "missing_ref":"inputs.token.symbol",
                "query_candidates":[{"query_ref":"erc20@0.0.2/decimals"}]
            },
            {"missing_ref":"inputs.owner","query_candidates":[]}
        ]
    });

    assert_eq!(
        selected_query_refs_from_missing_resolution(&payload),
        vec!["erc20@0.0.2/decimals".to_string()]
    );
}

#[test]
fn selected_query_refs_from_missing_resolution_prefers_explicit_policy_decisions() {
    let payload = json!({
        "decisions": [
            {
                "kind":"run_producer",
                "target":"inputs.token.decimals",
                "query_ref":"erc20@0.0.2/decimals"
            },
            {
                "kind":"run_producer",
                "target":"facts.quote.price",
                "query_ref":"dex@1/quote"
            }
        ],
        "resolved": [
            {
                "missing_ref":"inputs.token.decimals",
                "query_candidates":[{"query_ref":"legacy@0/decimals"}]
            }
        ]
    });
    assert_eq!(
        selected_query_refs_from_missing_resolution(&payload),
        vec![
            "dex@1/quote".to_string(),
            "erc20@0.0.2/decimals".to_string()
        ]
    );
}

#[test]
fn split_query_recoverable_questions_partitions_by_resolver_candidates() {
    let mut context = CandidateContext::default();
    context.executable_candidates.queries.push(json!({
        "ref": "erc20@0.0.2/decimals",
        "kind": "query"
    }));
    context.detail_by_ref.insert(
        "erc20@0.0.2/decimals".to_string(),
        json!({
            "ref":"erc20@0.0.2/decimals",
            "kind":"query",
            "returns":[{"name":"decimals","type":"uint8"}]
        }),
    );

    let questions = vec![
        json!({"id":"token.decimals","question":"Need decimals"}),
        json!({"id":"owner","question":"Need owner"}),
        json!({"question":"Missing id should remain unresolved"}),
    ];
    let (recoverable, unresolved) = split_query_recoverable_questions(&context, &questions, 3);

    assert_eq!(recoverable.len(), 1);
    assert_eq!(
        recoverable[0].get("id").and_then(Value::as_str),
        Some("token.decimals")
    );
    assert_eq!(unresolved.len(), 2);
    assert_eq!(
        unresolved[0].get("id").and_then(Value::as_str),
        Some("owner")
    );
    assert!(unresolved[1].get("id").is_none());
}

#[test]
fn build_query_param_value_prefers_matching_token_asset_with_multiple_tokens() {
    let summary = json!({
        "input_store": {
            "facts": {
                "tst_token": {
                    "value": {
                        "address": "0x1111111111111111111111111111111111111111",
                        "symbol": "TST",
                        "chain_id": "eip155:31338"
                    }
                },
                "usdc_token": {
                    "value": {
                        "address": "0x2222222222222222222222222222222222222222",
                        "symbol": "USDC",
                        "chain_id": "eip155:31338"
                    }
                }
            },
            "meta": {
                "tst_token": {"source_priority": 90},
                "usdc_token": {"source_priority": 90}
            }
        }
    });

    let selected = build_query_param_value(
        Some(&summary),
        "inputs.tst_token.decimals",
        "token",
        "asset",
    )
    .expect("must select token binding");
    assert_eq!(selected.pointer("/ref"), Some(&json!("inputs.tst_token")));
}

#[test]
fn build_query_param_value_prefers_exact_address_slot_with_multiple_addresses() {
    let summary = json!({
        "input_store": {
            "facts": {
                "owner": "0xaAaAaAaaAaAaAaaAaAAAAAAAAaaaAaAaAaaAaaAa",
                "recipient": "0xbBbBBBBbbBBBbbbBbbBbbbbBBbBbbbbBbBbbBBbB",
                "treasury": "0xCcCCccccCCCCcCCCCCCcCcCccCcCCCcCcccccccC"
            },
            "meta": {
                "owner": {"source_priority": 90},
                "recipient": {"source_priority": 90},
                "treasury": {"source_priority": 90}
            }
        }
    });

    let selected =
        build_query_param_value(Some(&summary), "inputs.recipient", "recipient", "address")
            .expect("must select recipient binding");
    assert_eq!(selected.pointer("/ref"), Some(&json!("inputs.recipient")));
}

#[test]
fn build_query_param_value_returns_none_for_ambiguous_address_candidates() {
    let summary = json!({
        "input_store": {
            "facts": {
                "recipient_primary": "0xbBbBBBBbbBBBbbbBbbBbbbbBBbBbbbbBbBbbBBbB",
                "recipient_backup": "0xCcCCccccCCCCcCCCCCCcCcCccCcCCCcCcccccccC"
            },
            "meta": {
                "recipient_primary": {"source_priority": 90},
                "recipient_backup": {"source_priority": 90}
            }
        }
    });

    let selected =
        build_query_param_value(Some(&summary), "inputs.recipient", "recipient", "address");
    assert!(
        selected.is_none(),
        "ambiguous address candidates should not auto-bind"
    );
}

#[test]
fn build_query_param_value_selects_non_token_numeric_slot() {
    let summary = json!({
        "input_store": {
            "facts": {
                "balance_threshold": 100,
                "min_required": 50
            },
            "meta": {
                "balance_threshold": {"source_priority": 90},
                "min_required": {"source_priority": 70}
            }
        }
    });

    let selected = build_query_param_value(
        Some(&summary),
        "inputs.balance_threshold",
        "threshold",
        "uint256",
    )
    .expect("must select threshold binding");
    assert_eq!(
        selected.pointer("/ref"),
        Some(&json!("inputs.balance_threshold"))
    );
}

#[test]
fn query_param_fallback_slots_include_erc20_token_alias_for_token_inputs() {
    let slots = query_param_fallback_slots("inputs.token.decimals", "token", "asset");
    assert!(slots.iter().any(|slot| slot == "erc20_token"));
    assert!(slots.iter().any(|slot| slot == "erc20_token.address"));
}

#[test]
fn build_query_param_value_supports_erc20_token_alias_for_token_asset_param() {
    let summary = json!({
        "input_store": {
            "facts": {
                "erc20_token": "0x8464135c8F25Da09e49BC8782676a84730C318bC"
            },
            "meta": {
                "erc20_token": {"source_priority": 90}
            }
        }
    });
    let selected =
        build_query_param_value(Some(&summary), "inputs.token.decimals", "token", "asset")
            .expect("must resolve token asset input via erc20 alias");
    let resolved_ref = selected.pointer("/address/ref").and_then(Value::as_str);
    let resolved_lit = selected.pointer("/address").and_then(Value::as_str);
    assert!(
        resolved_ref == Some("inputs.erc20_token")
            || resolved_lit == Some("0x8464135c8F25Da09e49BC8782676a84730C318bC")
    );
}

#[test]
fn build_query_param_value_supports_erc20_token_alias_for_node_output_missing_ref() {
    let summary = json!({
        "input_store": {
            "facts": {
                "erc20_token": "0x8464135c8F25Da09e49BC8782676a84730C318bC"
            },
            "meta": {
                "erc20_token": {"source_priority": 90}
            }
        }
    });
    let selected = build_query_param_value(
        Some(&summary),
        "nodes.q_erc20_balance.outputs.balance",
        "token",
        "asset",
    )
    .expect("must resolve token asset input via erc20 alias for node output targets");
    let resolved_ref = selected.pointer("/address/ref").and_then(Value::as_str);
    let resolved_lit = selected.pointer("/address").and_then(Value::as_str);
    assert!(
        resolved_ref == Some("inputs.erc20_token")
            || resolved_lit == Some("0x8464135c8F25Da09e49BC8782676a84730C318bC")
    );
}

#[test]
fn query_autofill_chain_scope_prefers_runtime_chain_over_default_mainnet() {
    let summary = json!({
        "input_store": {
            "facts": {
                "chain": {
                    "value": "eip155:31338"
                }
            }
        }
    });
    let query_detail = json!({
        "execution_chains": ["eip155:*"]
    });
    assert_eq!(
        query_autofill_chain_scope(Some(&summary), &query_detail),
        vec!["eip155:31338".to_string()]
    );
}

#[test]
fn query_autofill_chain_scope_reads_runtime_facts_chain_without_intent_context() {
    let summary = json!({
        "runtime_facts": {
            "facts": {
                "facts.chain": "eip155:8453"
            }
        },
        "intent_context": {
            "facts": {
                "chain": "eip155:1"
            }
        }
    });
    let query_detail = json!({
        "execution_chains": ["eip155:*"]
    });
    assert_eq!(
        query_autofill_chain_scope(Some(&summary), &query_detail),
        vec!["eip155:8453".to_string()]
    );
}

#[test]
fn query_autofill_runtime_chain_scope_prefers_typed_intent_slots_view() {
    let typed_summary = crate::agent::state_summary::StateSummary {
        completed_segments: 0,
        completed_nodes: 0,
        plan_epoch: 0,
        paused_reason: None,
        done: false,
        previous_error: None,
        input_store: None,
        runtime_facts: None,
        input_binding: crate::agent::state_summary::InputBindingContract {
            schema: "ais-agent-input-binding-contract/0.0.1",
            bindable_namespace: "inputs",
            bindable_refs_source: "state_summary.input_store",
            bindable_refs_projection: "state_summary.input_registry.known_refs",
            known_refs_only: true,
            facts_bindable: false,
        },
        input_registry: json!({"known_refs": []}),
        node_output_refs: json!({"known_refs": []}),
        reusable_outputs: None,
        tool_memory_projection: None,
        intent_slots: Some(json!({
            "resolved_inputs": {
                "chain": "eip155:8453"
            }
        })),
        intent_context: None,
        capability_view: None,
        capability_ready: None,
        side_effect_lifecycle: None,
        todo_state: None,
        recovery_diagnostics: None,
    };
    assert_eq!(
        query_autofill_runtime_chain_scope_typed(Some(&typed_summary)),
        Some("eip155:8453".to_string())
    );
}

#[test]
fn build_query_param_value_typed_reads_typed_intent_slots_fallback() {
    let typed_summary = crate::agent::state_summary::StateSummary {
        completed_segments: 0,
        completed_nodes: 0,
        plan_epoch: 0,
        paused_reason: None,
        done: false,
        previous_error: None,
        input_store: None,
        runtime_facts: None,
        input_binding: crate::agent::state_summary::InputBindingContract {
            schema: "ais-agent-input-binding-contract/0.0.1",
            bindable_namespace: "inputs",
            bindable_refs_source: "state_summary.input_store",
            bindable_refs_projection: "state_summary.input_registry.known_refs",
            known_refs_only: true,
            facts_bindable: false,
        },
        input_registry: json!({"known_refs": []}),
        node_output_refs: json!({"known_refs": []}),
        reusable_outputs: None,
        tool_memory_projection: None,
        intent_slots: Some(json!({
            "resolved_inputs": {
                "recipient": "0x1111111111111111111111111111111111111111"
            }
        })),
        intent_context: None,
        capability_view: None,
        capability_ready: None,
        side_effect_lifecycle: None,
        todo_state: None,
        recovery_diagnostics: None,
    };

    let selected = build_query_param_value_typed(
        Some(&typed_summary),
        "inputs.recipient",
        "recipient",
        "address",
    );

    assert_eq!(
        selected,
        Some(json!("0x1111111111111111111111111111111111111111"))
    );
}

#[test]
fn set_runtime_intent_fact_writes_nested_fact_path() {
    let mut runtime = json!({});
    super::super::executor::set_runtime_intent_fact(&mut runtime, "quote.price.usd", json!("1.01"));
    assert_eq!(
        runtime.pointer("/agent/intent_grounding/intent_facts/quote/price/usd"),
        Some(&json!("1.01"))
    );
}

#[test]
fn merge_missing_resolution_decisions_injects_explicit_autofill_decisions() {
    let resolution = json!({
        "resolved":[
            {
                "missing_ref":"inputs.token.decimals",
                "query_candidates":[{"query_ref":"erc20@0.0.2/decimals"}]
            }
        ]
    });
    let payload = json!({
        "autofill": {
            "decisions": [
                {
                    "kind": "run_producer",
                    "target": "inputs.token.decimals",
                    "query_ref": "alt@0.0.1/read-decimals"
                }
            ]
        }
    });

    let merged = merge_missing_resolution_decisions(&resolution, &payload);
    let decisions = super::super::policy::build_missing_resolution_decisions(&merged);
    assert_eq!(decisions.len(), 1);
    assert!(matches!(
        &decisions[0],
        super::super::policy::MissingResolutionDecision::RunProducer { target, query_ref }
            if target.as_canonical_str() == "inputs.token.decimals"
            && query_ref == "alt@0.0.1/read-decimals"
    ));
}

#[test]
fn merge_missing_resolution_decisions_reads_error_details_decisions() {
    let resolution = json!({
        "resolved":[
            {
                "missing_ref":"inputs.token.decimals",
                "query_candidates":[{"query_ref":"erc20@0.0.2/decimals"}]
            }
        ]
    });
    let payload = json!({
        "error_details": {
            "decisions": [
                {
                    "kind": "run_producer",
                    "target": "inputs.token.decimals",
                    "query_ref": "alt@0.0.1/read-decimals"
                }
            ]
        }
    });

    let merged = merge_missing_resolution_decisions(&resolution, &payload);
    let decisions = super::super::policy::build_missing_resolution_decisions(&merged);
    assert_eq!(decisions.len(), 1);
    assert!(matches!(
        &decisions[0],
        super::super::policy::MissingResolutionDecision::RunProducer { target, query_ref }
            if target.as_canonical_str() == "inputs.token.decimals"
            && query_ref == "alt@0.0.1/read-decimals"
    ));
}

#[test]
fn build_query_candidate_for_run_producer_keeps_target_query_ref_binding() {
    let resolution = json!({
        "resolved":[
            {
                "missing_ref":"inputs.token.decimals",
                "query_candidates":[
                    {
                        "query_ref":"erc20@0.0.2/decimals",
                        "matched_return_fields":["decimals"]
                    }
                ]
            }
        ]
    });
    let run = super::super::executor::MissingResolutionRunProducerAction {
        target: RefPath::Input {
            slot: "token.decimals".to_string(),
        },
        query_ref: "erc20@0.0.2/decimals".to_string(),
    };
    let query_candidate = build_query_candidate_for_run_producer(&resolution, &run);
    assert_eq!(
        query_candidate.get("query_ref").and_then(Value::as_str),
        Some("erc20@0.0.2/decimals")
    );
    assert_eq!(
        query_candidate.pointer("/matched_return_fields/0"),
        Some(&json!("decimals"))
    );
}

#[test]
fn precheck_missing_input_payload_skips_node_output_questions() {
    let payload = precheck_missing_input_payload(
        &[
            "inputs.owner".to_string(),
            "facts.quote.price".to_string(),
            "nodes.q_balance.outputs.balance".to_string(),
        ],
        3,
    );
    let question_ids = payload
        .get("questions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|item| item.get("id").and_then(Value::as_str).map(str::to_string))
        .collect::<Vec<_>>();
    assert_eq!(
        question_ids,
        vec!["inputs.owner".to_string(), "facts.quote.price".to_string()]
    );
}

#[test]
fn set_runtime_node_output_value_prefers_existing_runtime_node_suffix() {
    let mut runtime = json!({
        "nodes": {
            "seg_1/q_balance": {"outputs": {}}
        }
    });
    set_runtime_node_output_value(&mut runtime, "q_balance", "balance", json!("100"));
    assert_eq!(
        runtime.pointer("/nodes/seg_1~1q_balance/outputs/balance"),
        Some(&json!("100"))
    );
}

#[test]
fn set_runtime_node_output_value_creates_autofill_node_when_missing() {
    let mut runtime = json!({});
    set_runtime_node_output_value(&mut runtime, "q_quote", "quote.price", json!("1.01"));
    assert_eq!(
        runtime.pointer("/nodes/autofill~1q_quote/outputs/quote/price"),
        Some(&json!("1.01"))
    );
}

#[test]
fn runtime_has_ref_for_node_output_requires_readable_value() {
    let summary = json!({
        "node_output_refs": {
            "known_refs": ["nodes.q_balance.outputs.balance"]
        },
        "nodes": {
            "seg_1/q_balance": {
                "outputs": {
                    "balance": null
                }
            }
        }
    });
    assert!(!super::super::runtime_has_ref(
        Some(&summary),
        "nodes.q_balance.outputs.balance"
    ));

    let readable = json!({
        "node_output_refs": {
            "known_refs": ["nodes.q_balance.outputs.balance"]
        },
        "nodes": {
            "seg_1/q_balance": {
                "outputs": {
                    "balance": 0
                }
            }
        }
    });
    assert!(super::super::runtime_has_ref(
        Some(&readable),
        "nodes.q_balance.outputs.balance"
    ));
}

#[test]
fn resolve_static_input_value_reads_canonical_input_store_for_inputs() {
    let summary = json!({
        "input_store": {
            "facts": {
                "token": {
                    "address": "0x1111111111111111111111111111111111111111"
                }
            },
            "meta": {
                "token": {
                    "address": {
                        "source": "user"
                    }
                }
            }
        }
    });

    let resolved =
        static_refill::resolve_static_input_value_for_slot(Some(&summary), "token.address");
    assert_eq!(
        resolved,
        Some(json!("0x1111111111111111111111111111111111111111"))
    );
}

#[test]
fn resolve_static_input_value_accepts_query_observed_input_store_projection() {
    let summary = json!({
        "input_store": {
            "facts": {
                "token": {
                    "address": "0x1111111111111111111111111111111111111111"
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
    });

    let resolved =
        static_refill::resolve_static_input_value_for_slot(Some(&summary), "token.address");
    assert_eq!(
        resolved,
        Some(json!("0x1111111111111111111111111111111111111111"))
    );
}

#[test]
fn resolve_static_input_value_does_not_fall_through_to_intent_context_facts() {
    let summary = json!({
        "intent_context": {
            "facts": {
                "token": {
                    "address": "0x1111111111111111111111111111111111111111"
                }
            }
        }
    });

    let resolved =
        static_refill::resolve_static_input_value_for_slot(Some(&summary), "token.address");
    assert_eq!(resolved, None);
}

#[test]
fn resolve_static_input_value_typed_reads_typed_intent_slots_view() {
    let typed_summary = crate::agent::state_summary::StateSummary {
        completed_segments: 0,
        completed_nodes: 0,
        plan_epoch: 0,
        paused_reason: None,
        done: false,
        previous_error: None,
        input_store: None,
        runtime_facts: None,
        input_binding: crate::agent::state_summary::InputBindingContract {
            schema: "ais-agent-input-binding-contract/0.0.1",
            bindable_namespace: "inputs",
            bindable_refs_source: "state_summary.input_store",
            bindable_refs_projection: "state_summary.input_registry.known_refs",
            known_refs_only: true,
            facts_bindable: false,
        },
        input_registry: json!({"known_refs": []}),
        node_output_refs: json!({"known_refs": []}),
        reusable_outputs: None,
        tool_memory_projection: None,
        intent_slots: Some(json!({
            "resolved_inputs": {
                "token": {
                    "address": "0x1111111111111111111111111111111111111111"
                }
            }
        })),
        intent_context: None,
        capability_view: None,
        capability_ready: None,
        side_effect_lifecycle: None,
        todo_state: None,
        recovery_diagnostics: None,
    };

    let resolved = static_refill::resolve_static_input_value_for_slot_typed(
        Some(&typed_summary),
        None,
        "token.address",
    );
    assert_eq!(
        resolved,
        Some(json!("0x1111111111111111111111111111111111111111"))
    );
}

#[test]
fn runtime_has_ref_typed_reads_runtime_facts_view() {
    let typed_summary = crate::agent::state_summary::StateSummary {
        completed_segments: 0,
        completed_nodes: 0,
        plan_epoch: 0,
        paused_reason: None,
        done: false,
        previous_error: None,
        input_store: None,
        runtime_facts: Some(json!({
            "facts": {
                "facts.quote.price": "1.01"
            }
        })),
        input_binding: crate::agent::state_summary::InputBindingContract {
            schema: "ais-agent-input-binding-contract/0.0.1",
            bindable_namespace: "inputs",
            bindable_refs_source: "state_summary.input_store",
            bindable_refs_projection: "state_summary.input_registry.known_refs",
            known_refs_only: true,
            facts_bindable: false,
        },
        input_registry: json!({"known_refs": []}),
        node_output_refs: json!({"known_refs": []}),
        reusable_outputs: None,
        tool_memory_projection: None,
        intent_slots: None,
        intent_context: Some(json!({
            "facts": {
                "quote": {
                    "price": "9.99"
                }
            }
        })),
        capability_view: None,
        capability_ready: None,
        side_effect_lifecycle: None,
        todo_state: None,
        recovery_diagnostics: None,
    };

    assert!(static_refill::runtime_has_ref_typed(
        Some(&typed_summary),
        "facts.quote.price"
    ));
}

#[test]
fn runtime_has_fact_ref_reads_runtime_facts_without_intent_context_fallback() {
    let summary = json!({
        "runtime_facts": {
            "facts": {
                "facts.quote.price": "1.01"
            }
        },
        "intent_context": {
            "facts": {
                "quote": {
                    "price": "9.99"
                }
            }
        }
    });
    assert!(super::super::runtime_has_ref(
        Some(&summary),
        "facts.quote.price"
    ));

    let intent_context_only = json!({
        "intent_context": {
            "facts": {
                "quote": {
                    "price": "9.99"
                }
            }
        }
    });
    assert!(!super::super::runtime_has_ref(
        Some(&intent_context_only),
        "facts.quote.price"
    ));
}
