use super::*;
use serde_json::json;

#[test]
fn planning_context_marks_unchanged_payloads() {
    let mut manager = PlanningContextManager::default();
    let state = EngineRunnerState::default();
    let first = manager.next_summary(&state, 0, false, None, None, None);
    assert_eq!(
        first.pointer("/context_unchanged"),
        Some(&Value::Bool(false))
    );
    let second = manager.next_summary(&state, 0, false, None, None, None);
    assert_eq!(
        second.pointer("/context_unchanged"),
        Some(&Value::Bool(true))
    );
    assert_eq!(
        second.pointer("/context_envelope/schema"),
        Some(&json!(envelope::CONTEXT_ENVELOPE_SCHEMA))
    );
    assert_eq!(
        second.pointer("/context_envelope/schema_version"),
        Some(&json!(envelope::CONTEXT_ENVELOPE_SCHEMA_VERSION))
    );
    assert!(
        second.pointer("/input_registry").is_some(),
        "unchanged summaries must still include full projected context"
    );
}

#[test]
fn planning_context_envelope_payload_projection_is_compatible() {
    let mut manager = PlanningContextManager::default();
    let state = EngineRunnerState::default();
    let summary = manager.next_summary(&state, 0, false, None, None, None);
    let envelope = envelope::ContextEnvelope::from_summary(&summary).expect("read envelope");
    assert_eq!(envelope.version, 1);
    assert!(!envelope.hash.is_empty());

    let payload = envelope::payload_from_summary(&summary);
    assert_eq!(
        payload.pointer("/context_budget/token_limit"),
        summary.pointer("/context_budget/token_limit")
    );
    assert!(
        payload.pointer("/context_envelope").is_none(),
        "payload projection must exclude envelope metadata"
    );
    let payload_tokens = summary
        .pointer("/context_budget/estimated_payload_tokens")
        .and_then(Value::as_u64)
        .expect("payload tokens");
    let emitted_tokens = summary
        .pointer("/context_budget/estimated_emitted_tokens")
        .and_then(Value::as_u64)
        .expect("emitted tokens");
    assert!(emitted_tokens >= payload_tokens);
    assert_eq!(
        summary.pointer("/context_budget/token_limit_scope"),
        Some(&json!("payload_core"))
    );
}

#[test]
fn projected_summary_includes_tool_memory_projection() {
    let state = EngineRunnerState::default();
    let tool_memory_projection = json!({
        "schema": "ais-agent-tool-memory-projection/0.0.1",
        "recent": {
            "catalog_search": [
                {"query":"transfer","top_refs":[{"ref":"erc20@0.0.2/transfer"}]}
            ],
            "candidate_detail": [],
            "guide": {"schema": {}, "topic": {"cel": {}}}
        }
    });
    let summary =
        build_projected_summary(&state, 0, false, None, None, Some(&tool_memory_projection));
    assert_eq!(
        summary.pointer("/tool_memory_projection/schema"),
        Some(&json!("ais-agent-tool-memory-projection/0.0.1"))
    );
    assert_eq!(
        summary.pointer("/tool_memory_projection/recent/guide/topic/cel"),
        Some(&json!({}))
    );
}

#[test]
fn projected_summary_includes_node_output_refs() {
    let state = EngineRunnerState {
        completed_node_ids: vec!["seg_transfer__q_balance".to_string()],
        runtime: json!({
            "nodes": {
                "seg_transfer__q_balance": {
                    "outputs": {
                        "balance": "123"
                    }
                },
                "seg_transfer__a_transfer": {
                    "outputs": {
                        "outputs": {
                            "confirmed": true,
                            "tx_hash": "0xabc"
                        }
                    }
                }
            }
        }),
        ..EngineRunnerState::default()
    };
    let summary = build_projected_summary(&state, 0, false, None, None, None);
    assert_eq!(
        summary.pointer("/node_output_refs/schema"),
        Some(&json!("ais-agent-node-output-refs/0.0.1"))
    );
    let refs = summary
        .pointer("/node_output_refs/known_refs")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert!(
        refs.iter()
            .any(|value| value.as_str() == Some("nodes.q_balance.outputs.balance")),
        "known_refs={refs:?}"
    );
}

#[test]
fn projected_summary_limits_fact_entries() {
    let mut store = InputStore::default();
    for index in 0..40 {
        store.upsert_seed(format!("k.{index}"), json!(index), "runtime.inputs");
    }
    store.upsert_seed("owner", json!("0xabc"), "runtime.inputs.owner");
    let state = EngineRunnerState::default();
    let summary = build_projected_summary(&state, 0, false, None, Some(&store), None);
    assert_eq!(
        summary.pointer("/input_store/facts/owner"),
        Some(&json!("0xabc"))
    );
    assert!(
        summary
            .pointer("/input_store/meta/_truncated_entries")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            > 0
    );
}

#[test]
fn projected_summary_includes_input_slots_and_missing_refs() {
    let mut store = InputStore::default();
    store.upsert_seed("inputs.owner", json!("0xabc"), "runtime.inputs.owner");
    let state = EngineRunnerState {
        runtime: json!({
            "inputs": {
                "owner": "0xabc",
                "token": {"address":"0xdef"}
            },
            "agent": {
                "todo_progress": {
                    "current_todo": {
                        "required_facts": ["inputs.owner", "inputs.amount"]
                    }
                }
            }
        }),
        ..EngineRunnerState::default()
    };
    let summary = build_projected_summary(&state, 0, false, None, Some(&store), None);
    assert_eq!(
        summary.pointer("/input_slots/canonical_refs/owner"),
        Some(&json!("inputs.owner"))
    );
    assert_eq!(
        summary.pointer("/input_slots/canonical_refs/token.address"),
        Some(&json!("inputs.token.address"))
    );
    assert_eq!(
        summary.pointer("/input_slots/missing/0/ref"),
        Some(&json!("inputs.amount"))
    );
    assert_eq!(
        summary.pointer("/input_registry/known_refs/0"),
        Some(&json!("inputs.amount"))
    );
    assert_eq!(
        summary.pointer("/input_registry/known_refs/1"),
        Some(&json!("inputs.owner"))
    );
    assert_eq!(
        summary.pointer("/input_registry/entries/0/status"),
        Some(&json!("missing"))
    );
    assert_eq!(
        summary.pointer("/canonical_context/account_refs/0/account_ref"),
        Some(&json!("0xabc"))
    );
    assert_eq!(
        summary.pointer("/input_binding/bindable_refs_source"),
        Some(&json!("state_summary.input_registry.known_refs"))
    );
    assert_eq!(
        summary.pointer("/input_binding/facts_bindable"),
        Some(&json!(false))
    );
}

#[test]
fn projected_summary_intent_slots_separates_bindable_inputs_from_semantic_facts() {
    let state = EngineRunnerState {
        runtime: json!({
            "agent": {
                "intent_grounding": {
                    "status":"proposed",
                    "ready_for_todos": false,
                    "resolved_inputs": {
                        "inputs.owner": "0xabc",
                        "fact:token": "USDC"
                    },
                    "intent_facts": {
                        "token":"USDC",
                        "native_balance":"1000"
                    },
                    "confidence": {
                        "inputs.owner": 97,
                        "fact:token": 88
                    }
                }
            }
        }),
        ..EngineRunnerState::default()
    };

    let summary = build_projected_summary(&state, 0, false, None, None, None);
    assert_eq!(
        summary.pointer("/intent_slots/input_binding/bindable_refs_source"),
        Some(&json!("state_summary.input_registry.known_refs"))
    );
    assert_eq!(
        summary.pointer("/intent_slots/resolved_input_refs/0"),
        Some(&json!("inputs.owner"))
    );
    assert_eq!(
        summary.pointer("/intent_slots/resolved_inputs/owner"),
        Some(&json!("0xabc"))
    );
    assert!(summary
        .pointer("/intent_slots/resolved_inputs/fact:token")
        .is_none());
    assert_eq!(
        summary.pointer("/intent_slots/confidence/inputs/inputs.owner"),
        Some(&json!(97))
    );
    assert!(summary
        .pointer("/intent_slots/confidence/facts/token")
        .is_none());
    assert!(summary.pointer("/intent_slots/intent_facts").is_none());
    assert_eq!(
        summary.pointer("/intent_context/facts/token"),
        Some(&json!("USDC"))
    );
    assert_eq!(
        summary.pointer("/intent_context/confidence/facts/token"),
        Some(&json!(88))
    );
}

#[test]
fn projected_summary_prefers_input_store_values_for_overlap_with_runtime() {
    let mut store = InputStore::default();
    store.upsert_seed(
        "inputs.owner",
        json!("0xinputstore-owner"),
        "runtime.inputs.owner",
    );
    store.upsert_seed(
        "inputs.chain_id",
        json!("eip155:2"),
        "runtime.inputs.chain_id",
    );
    let state = EngineRunnerState {
        runtime: json!({
            "inputs": {
                "owner": "0xruntime-owner",
                "chain_id": "eip155:1",
                "token": {
                    "address": "0xruntime-token"
                }
            },
            "agent": {
                "missing_required_input": {
                    "questions": [
                        {"id": "token.decimals"},
                        {"id": "   "}
                    ]
                },
                "todo_progress": {
                    "current_todo": {
                        "required_facts": ["inputs.owner", "inputs.chain_id", "inputs.token.decimals"]
                    }
                }
            }
        }),
        ..EngineRunnerState::default()
    };
    let summary = build_projected_summary(&state, 0, false, None, Some(&store), None);

    let resolved = summary
        .pointer("/input_slots/resolved")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let owner = resolved.iter().find_map(|item| {
        if item.get("id") == Some(&json!("owner")) {
            item.get("value")
        } else {
            None
        }
    });
    let chain_id = resolved.iter().find_map(|item| {
        if item.get("id") == Some(&json!("chain_id")) {
            item.get("value")
        } else {
            None
        }
    });
    assert_eq!(owner, Some(&json!("0xinputstore-owner")));
    assert_eq!(chain_id, Some(&json!("eip155:2")));

    let known_refs = summary
        .pointer("/input_registry/known_refs")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let known_refs = known_refs
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    assert!(known_refs.contains(&"inputs.owner"));
    assert!(known_refs.contains(&"inputs.chain_id"));
    assert_eq!(
        summary.pointer("/input_slots/missing/0/ref"),
        Some(&json!("inputs.token.decimals"))
    );
    assert_eq!(
        summary.pointer("/canonical_context/chain_refs/0/chain_ref"),
        Some(&json!("eip155:2"))
    );
}

#[test]
fn projected_summary_includes_chain_account_asset_and_amount_refs() {
    let state = EngineRunnerState {
        runtime: json!({
            "inputs": {
                "chain_id": "eip155:31338",
                "owner": "0x1111111111111111111111111111111111111111",
                "recipient": "0x2222222222222222222222222222222222222222",
                "token": {
                    "address":"0x8464135c8F25Da09e49BC8782676a84730C318bC",
                    "chain_id":"eip155:31338",
                    "decimals": 18,
                    "symbol": "TKN"
                },
                "amount": {
                    "human":"1.25",
                    "atomic":"1250000000000000000"
                }
            }
        }),
        ..EngineRunnerState::default()
    };
    let summary = build_projected_summary(&state, 0, false, None, None, None);
    assert_eq!(
        summary.pointer("/canonical_context/chain_refs/0/chain_ref"),
        Some(&json!("eip155:31338"))
    );
    assert_eq!(
        summary.pointer("/canonical_context/account_refs/0/account_ref"),
        Some(&json!("0x1111111111111111111111111111111111111111"))
    );
    assert_eq!(
        summary.pointer("/canonical_context/account_refs/1/account_ref"),
        Some(&json!("0x2222222222222222222222222222222222222222"))
    );
    assert_eq!(
        summary.pointer("/canonical_context/asset_refs/0/chain_ref"),
        Some(&json!("eip155:31338"))
    );
    assert_eq!(
        summary.pointer("/canonical_context/amount_refs/0/amount_atomic"),
        Some(&json!("1250000000000000000"))
    );
}

#[test]
fn projected_summary_applies_budget_and_keeps_priority_slots() {
    let mut inputs = serde_json::Map::<String, Value>::new();
    inputs.insert("owner".to_string(), json!("0xabc"));
    for index in 0..220 {
        inputs.insert(format!("extra_{index}"), json!(format!("v{index}")));
    }
    let mut protocols = Vec::<Value>::new();
    for index in 0..120 {
        protocols.push(json!({
            "protocol": format!("protocol-{index:03}"),
            "chains": ["eip155:1"],
            "actions": [{"name":"a","ref":format!("p{index}/a")}],
            "queries": [{"name":"q","ref":format!("p{index}/q")}],
            "required_inputs": ["owner", "amount"]
        }));
    }
    let state = EngineRunnerState {
        runtime: json!({
            "inputs": Value::Object(inputs),
            "agent": {
                "capability_view": {
                    "schema": "ais-agent-capability-view/0.0.1",
                    "ready": true,
                    "protocols": protocols,
                    "counts": {
                        "protocols": 120,
                        "actions": 120,
                        "queries": 120
                    }
                },
                "todo_progress": {
                    "current_todo": {
                        "required_facts": ["inputs.owner", "inputs.amount"]
                    }
                }
            }
        }),
        ..EngineRunnerState::default()
    };
    let mut store = InputStore::default();
    for index in 0..120 {
        store.upsert_seed(
            format!("inputs.extra_{index}"),
            json!(index),
            "runtime.inputs",
        );
    }
    store.upsert_seed("owner", json!("0xabc"), "runtime.inputs.owner");

    let summary = build_projected_summary(&state, 0, false, None, Some(&store), None);
    assert_eq!(
        summary.pointer("/input_registry/entries/0/ref"),
        Some(&json!("inputs.amount"))
    );
    assert_eq!(
        summary.pointer("/input_store/facts/owner"),
        Some(&json!("0xabc"))
    );
    assert_eq!(
        summary.pointer("/context_budget/token_limit"),
        Some(&json!(DEFAULT_PLANNER_CONTEXT_TOKEN_BUDGET))
    );
    assert_eq!(
        summary.pointer("/context_budget/adaptive_mode"),
        Some(&json!("default"))
    );
    assert_eq!(
        summary.pointer("/context_budget/estimator"),
        Some(&json!("chars_div_4"))
    );
    assert!(
        summary
            .pointer("/context_budget/truncated")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        "large context should be marked truncated"
    );
}

#[test]
fn projected_summary_relaxes_budget_when_context_remaining_is_high() {
    let mut inputs = serde_json::Map::<String, Value>::new();
    inputs.insert("owner".to_string(), json!("0xabc"));
    for index in 0..120 {
        inputs.insert(format!("extra_{index}"), json!(format!("v{index}")));
    }
    let state = EngineRunnerState {
        runtime: json!({
            "inputs": Value::Object(inputs),
            "agent": {
                "llm_usage": {
                    "context_soft_limit_tokens": 100000,
                    "context_remaining_tokens": 90000
                }
            }
        }),
        ..EngineRunnerState::default()
    };
    let summary = build_projected_summary(&state, 0, false, None, None, None);
    let token_limit = summary
        .pointer("/context_budget/token_limit")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    assert!(token_limit > DEFAULT_PLANNER_CONTEXT_TOKEN_BUDGET as u64);
    assert_eq!(
        summary.pointer("/context_budget/adaptive_mode"),
        Some(&json!("relaxed"))
    );
    assert_eq!(
        summary.pointer("/context_budget/adaptive/remaining_ratio_bps"),
        Some(&json!(9000))
    );
}

#[test]
fn projected_summary_uses_critical_pressure_strategy_when_usage_exceeds_ninety_percent() {
    let long_text = "x".repeat(4000);
    let state = EngineRunnerState {
        runtime: json!({
            "inputs": {
                "owner": "0xabc",
                "token": {"address":"0xdef"}
            },
            "agent": {
                "llm_usage": {
                    "context_soft_limit_tokens": 100000,
                    "context_remaining_tokens": 900
                },
                "capability_view": {
                    "schema": "ais-agent-capability-view/0.0.1",
                    "ready": true,
                    "protocols": [{
                        "protocol": "erc20@0.0.2",
                        "actions": [{"name":"transfer","ref":"erc20@0.0.2/transfer"}],
                        "queries": [{"name":"balance-of","ref":"erc20@0.0.2/balance-of"}],
                        "required_inputs": ["owner", "recipient"]
                    }],
                    "topics": ["transfer"]
                }
            }
        }),
        ..EngineRunnerState::default()
    };
    let previous_error = json!({
        "phase": "planning",
        "reason_code": "planner_invalid_tool_output",
        "last_failed_finalize": {
            "tool": "plan.revise_segment",
            "arguments": {
                "status":"proposed",
                "segment": {
                    "segment_id":"seg_1",
                    "steps":[{"id":"s1","kind":"query","inputs":{"owner":{"ref":"inputs.owner"}}}]
                }
            },
            "assistant_content": long_text
        }
    });
    let tool_memory_projection = json!({
        "schema": "ais-agent-tool-memory-projection/0.0.1",
        "recent": {
            "guide": {
                "schema": {
                    "ais-plan-sketch/0.1.0": {
                        "summary": "y".repeat(5000)
                    }
                },
                "topic": {}
            }
        }
    });
    let summary = build_projected_summary(
        &state,
        0,
        false,
        Some(&previous_error),
        None,
        Some(&tool_memory_projection),
    );
    assert_eq!(
        summary.pointer("/context_budget/pressure_mode"),
        Some(&json!("critical"))
    );
    assert_eq!(
        summary.pointer("/input_slots/canonical_refs"),
        Some(&Value::Null)
    );
    assert_eq!(
        summary.pointer("/capability_view/protocols"),
        Some(&json!([]))
    );
    assert_eq!(
        summary.pointer("/previous_error/last_failed_finalize/assistant_content"),
        Some(&Value::Null)
    );
    let actions = summary
        .pointer("/context_budget/pressure_actions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert!(actions
        .iter()
        .any(|item| item.as_str() == Some("drop_capability_protocols")));
    assert!(actions
        .iter()
        .any(|item| item.as_str() == Some("compress_tool_memory_projection")));
}
