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
        payload.pointer("/context_budget/pressure_mode"),
        summary.pointer("/context_budget/pressure_mode")
    );
    assert!(
        payload.pointer("/context_envelope").is_none(),
        "payload projection must exclude envelope metadata"
    );
    assert!(
        summary
            .pointer("/context_budget/pack_diagnostics")
            .is_some(),
        "context budget must expose pack diagnostics"
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
fn projected_summary_does_not_embed_prompt_compact_projection() {
    let state = EngineRunnerState::default();
    let mut manager = PlanningContextManager::default();
    let summary = manager.next_summary(&state, 0, false, None, None, None);
    assert!(
        summary.pointer("/prompt_compact").is_none(),
        "state_summary should not embed nested prompt_compact"
    );
    let compact = super::super::context::prompt_compact::build_prompt_compact(&summary);
    assert_eq!(
        compact.pointer("/schema"),
        Some(&json!("ais-agent-state-summary-prompt-compact/0.0.1"))
    );
    assert!(
        compact.pointer("/context_budget/pack_trace").is_none(),
        "prompt_compact must not include pack_trace"
    );
    assert!(
        compact
            .pointer("/context_budget/pack_diagnostics/packed_blocks_total")
            .is_some(),
        "prompt_compact must keep compact diagnostics summary"
    );
    assert!(
        compact
            .pointer("/summary_text")
            .and_then(Value::as_str)
            .is_some(),
        "prompt_compact must include summary_text"
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
fn projected_summary_keeps_full_input_store_facts_before_pack_loop() {
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
        summary.pointer("/input_store/facts/k.39").is_some(),
        "projector should not pre-truncate input_store facts"
    );
}

#[test]
fn projected_summary_includes_input_slots_and_missing_refs() {
    let mut store = InputStore::default();
    store.upsert_seed("inputs.owner", json!("0xabc"), "runtime.inputs.owner");
    store.upsert_seed(
        "inputs.token.address",
        json!("0xdef"),
        "runtime.inputs.token.address",
    );
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
    // input_registry.known_refs contains the canonical refs formerly in input_slots
    let known_refs = summary
        .pointer("/input_registry/known_refs")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let known_ref_strs = known_refs
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    assert!(
        known_ref_strs.contains(&"inputs.owner"),
        "known_refs should contain inputs.owner: {known_ref_strs:?}"
    );
    assert!(
        known_ref_strs.contains(&"inputs.token.address"),
        "known_refs should contain inputs.token.address: {known_ref_strs:?}"
    );
    // missing entries are now in input_registry.entries with status "missing"
    let entries = summary
        .pointer("/input_registry/entries")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let missing_entry = entries
        .iter()
        .find(|entry| entry.get("status").and_then(Value::as_str) == Some("missing"));
    assert!(
        missing_entry.is_some(),
        "input_registry.entries should contain a missing entry: {entries:?}"
    );
    assert_eq!(
        missing_entry.unwrap().get("ref").and_then(Value::as_str),
        Some("inputs.amount"),
        "missing entry ref should be inputs.amount"
    );
    assert_eq!(
        summary.pointer("/input_binding/bindable_refs_source"),
        Some(&json!("state_summary.input_store"))
    );
    assert_eq!(
        summary.pointer("/input_binding/bindable_refs_projection"),
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
        summary.pointer("/intent_slots/input_binding/role"),
        Some(&json!("grounding_intermediate"))
    );
    assert_eq!(
        summary.pointer("/intent_slots/input_binding/bindable"),
        Some(&json!(false))
    );
    assert_eq!(
        summary.pointer("/intent_slots/input_binding/source_of_truth"),
        Some(&json!("state_summary.input_store"))
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
fn projected_summary_selects_minimal_stage_when_token_budget_is_tiny() {
    let mut store = InputStore::default();
    for index in 0..120 {
        store.upsert_seed(
            format!("k.{index}"),
            json!("x".repeat(64)),
            "runtime.inputs",
        );
    }
    store.upsert_seed("owner", json!("0xabc"), "runtime.inputs.owner");

    let state = EngineRunnerState::default();
    let mut manager = PlanningContextManager::with_token_budget(200);
    let summary = manager.next_summary(&state, 0, false, None, Some(&store), None);
    assert_eq!(
        summary.pointer("/context_budget/pressure_mode"),
        Some(&json!("normal"))
    );
    assert_eq!(
        summary.pointer("/context_budget/pack_overflow_reason"),
        Some(&json!("must_keep_only_exceeds_budget"))
    );
    assert_eq!(
        summary.pointer("/context_budget/final_compact_applied"),
        Some(&Value::Bool(true))
    );
}

#[test]
fn projected_summary_records_pressure_mode_from_runtime_usage() {
    let state = EngineRunnerState {
        runtime: json!({
            "agent": {
                "llm_usage": {
                    "context_soft_limit_tokens": 100_000,
                    "context_remaining_tokens": 2_000
                }
            }
        }),
        ..EngineRunnerState::default()
    };

    let summary = build_projected_summary(&state, 6_000, false, None, None, None);
    assert_eq!(
        summary.pointer("/context_budget/pressure_mode"),
        Some(&json!("critical"))
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

    // input_store.facts should prefer input_store values over runtime
    // (keys are stored without the "inputs." prefix after normalization)
    assert_eq!(
        summary.pointer("/input_store/facts/owner"),
        Some(&json!("0xinputstore-owner"))
    );
    assert_eq!(
        summary.pointer("/input_store/facts/chain_id"),
        Some(&json!("eip155:2"))
    );

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
    // missing entries are now in input_registry.entries with status "missing"
    let entries = summary
        .pointer("/input_registry/entries")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let missing_entry = entries.iter().find(|entry| {
        entry.get("status").and_then(Value::as_str) == Some("missing")
            && entry.get("ref").and_then(Value::as_str) == Some("inputs.token.decimals")
    });
    assert!(
        missing_entry.is_some(),
        "input_registry.entries should contain a missing entry for inputs.token.decimals: {entries:?}"
    );
}

#[test]
fn projected_summary_includes_chain_account_asset_and_amount_refs() {
    let mut store = InputStore::default();
    store.upsert_seed("inputs.chain_id", json!("eip155:31338"), "test.seed");
    store.upsert_seed(
        "inputs.owner",
        json!("0x1111111111111111111111111111111111111111"),
        "test.seed",
    );
    store.upsert_seed(
        "inputs.recipient",
        json!("0x2222222222222222222222222222222222222222"),
        "test.seed",
    );
    store.upsert_seed(
        "inputs.token",
        json!({
            "address":"0x8464135c8F25Da09e49BC8782676a84730C318bC",
            "chain_id":"eip155:31338",
            "decimals": 18,
            "symbol": "TKN"
        }),
        "test.seed",
    );
    store.upsert_seed(
        "inputs.amount",
        json!({
            "human":"1.25",
            "atomic":"1250000000000000000"
        }),
        "test.seed",
    );
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
    let summary = build_projected_summary(&state, 0, false, None, Some(&store), None);
    // canonical_context no longer exists; verify inputs via input_registry and input_store
    let known_refs = summary
        .pointer("/input_registry/known_refs")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let known_ref_strs = known_refs
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    assert!(
        known_ref_strs.contains(&"inputs.chain_id"),
        "known_refs should contain inputs.chain_id: {known_ref_strs:?}"
    );
    assert!(
        known_ref_strs.contains(&"inputs.owner"),
        "known_refs should contain inputs.owner: {known_ref_strs:?}"
    );
    assert!(
        known_ref_strs.contains(&"inputs.recipient"),
        "known_refs should contain inputs.recipient: {known_ref_strs:?}"
    );
    assert!(
        known_ref_strs.contains(&"inputs.token"),
        "known_refs should contain inputs.token: {known_ref_strs:?}"
    );
    assert!(
        known_ref_strs.contains(&"inputs.amount"),
        "known_refs should contain inputs.amount: {known_ref_strs:?}"
    );
    // Verify the actual values are in input_store.facts
    // (keys are stored without the "inputs." prefix after normalization)
    assert_eq!(
        summary.pointer("/input_store/facts/chain_id"),
        Some(&json!("eip155:31338"))
    );
    assert_eq!(
        summary.pointer("/input_store/facts/owner"),
        Some(&json!("0x1111111111111111111111111111111111111111"))
    );
    assert_eq!(
        summary.pointer("/input_store/facts/recipient"),
        Some(&json!("0x2222222222222222222222222222222222222222"))
    );
    assert_eq!(
        summary.pointer("/input_store/facts/amount/atomic"),
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
        summary.pointer("/context_budget/pressure_mode"),
        Some(&json!("normal"))
    );
    assert!(
        summary
            .pointer("/context_budget/pack_diagnostics")
            .is_some(),
        "context budget should expose pack diagnostics"
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
    assert_eq!(
        summary.pointer("/context_budget/pressure_mode"),
        Some(&json!("normal"))
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
        summary.pointer("/capability_view/protocols"),
        Some(&json!([]))
    );
    assert!(
        summary
            .pointer("/previous_error/last_failed_finalize/assistant_content")
            .is_none()
            || summary.pointer("/previous_error/last_failed_finalize/assistant_content")
                == Some(&Value::Null)
    );
    assert!(
        summary
            .pointer("/context_budget/pack_trace")
            .and_then(Value::as_array)
            .is_some(),
        "context_budget.pack_trace must be present for observability"
    );
}

#[test]
fn pack_blocks_compresses_low_priority_blocks_before_stale_or_drop() {
    let huge_projection = json!({
        "schema": "ais-agent-tool-memory-projection/0.0.1",
        "recent": {
            "catalog_search": [
                {
                    "query": "transfer",
                    "returned_matches": 2,
                    "results": [
                        {
                            "ref": "erc20@0.0.2/transfer",
                            "kind": "action",
                            "schema_name": "erc20@0.0.2",
                            "notes": "x".repeat(4000)
                        }
                    ]
                }
            ],
            "candidate_detail": [{"signatures":[{"ref":"erc20@0.0.2/transfer","kind":"action","params":[{"name":"to","required":true},{"name":"amount","required":true}]}]}],
            "guide": {"schema": {}, "topic": {"cel": {"summary": "y".repeat(2000)}}}
        }
    });
    let state = EngineRunnerState {
        runtime: json!({
            "agent": {
                "llm_usage": {
                    "context_soft_limit_tokens": 100_000,
                    "context_remaining_tokens": 1_000
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
            "arguments": {"status":"proposed","segment":{"segment_id":"seg_1","steps":[]}},
            "assistant_content": "z".repeat(8000)
        }
    });

    let mut manager = PlanningContextManager::with_token_budget(900);
    let summary = manager.next_summary(
        &state,
        0,
        false,
        Some(&previous_error),
        None,
        Some(&huge_projection),
    );

    let trace = summary
        .pointer("/context_budget/pack_trace")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert!(
        !trace.is_empty(),
        "pack_trace must record at least one decision when over budget"
    );
    assert_eq!(
        trace
            .first()
            .and_then(|item| item.get("block_id"))
            .and_then(Value::as_str),
        Some("tool_memory_projection"),
        "expected pack loop to compress low-priority tool_memory_projection first; trace={trace:?}"
    );
    assert_eq!(
        trace
            .first()
            .and_then(|item| item.get("action"))
            .and_then(Value::as_str),
        Some("compress"),
        "expected first decision to be a compression step; trace={trace:?}"
    );
}

#[test]
fn pack_diagnostics_are_zero_when_window_is_sufficient_for_full_context() {
    let state = EngineRunnerState::default();
    let tool_memory_projection = json!({
        "schema": "ais-agent-tool-memory-projection/0.0.1",
        "recent": {
            "catalog_search": [{"query":"transfer","results": [{"ref":"erc20@0.0.2/transfer","notes":"x".repeat(2000)}]}],
            "candidate_detail": [{"signatures": [{"ref":"erc20@0.0.2/transfer","params": [{"name":"to"},{"name":"amount"}]}]}],
            "guide": {"topic": {"cel": {"summary": "y".repeat(1500)}}}
        }
    });

    // Large budget should avoid any pack-loop compression/eviction decisions.
    let mut manager = PlanningContextManager::with_token_budget(50_000);
    let summary = manager.next_summary(&state, 0, false, None, None, Some(&tool_memory_projection));
    assert_eq!(
        summary.pointer("/context_budget/pack_overflow_reason"),
        Some(&Value::Null)
    );
    assert_eq!(
        summary.pointer("/context_budget/final_compact_applied"),
        Some(&Value::Bool(false))
    );

    let diagnostics = summary
        .pointer("/context_budget/pack_diagnostics")
        .cloned()
        .unwrap_or(Value::Null);
    assert_eq!(
        diagnostics.pointer("/compressed_blocks_total"),
        Some(&json!(0))
    );
    assert_eq!(
        diagnostics.pointer("/packed_blocks_evicted"),
        Some(&json!(0))
    );
}

#[test]
fn pack_diagnostics_record_progressive_compress_then_evict_under_pressure() {
    let huge_projection = json!({
        "schema": "ais-agent-tool-memory-projection/0.0.1",
        "recent": {
            "catalog_search": [
                {
                    "query": "transfer",
                    "returned_matches": 2,
                    "results": [
                        {
                            "ref": "erc20@0.0.2/transfer",
                            "kind": "action",
                            "schema_name": "erc20@0.0.2",
                            "notes": "x".repeat(8000)
                        }
                    ]
                }
            ],
            "candidate_detail": [{"signatures":[{"ref":"erc20@0.0.2/transfer","kind":"action","params":[{"name":"to","required":true},{"name":"amount","required":true}]}]}],
            "guide": {"schema": {}, "topic": {"cel": {"summary": "y".repeat(6000)}}}
        }
    });
    let state = EngineRunnerState {
        runtime: json!({
            "agent": {
                "llm_usage": {
                    "context_soft_limit_tokens": 100_000,
                    "context_remaining_tokens": 1_000
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
            "arguments": {"status":"proposed","segment":{"segment_id":"seg_1","steps":[]}},
            "assistant_content": "z".repeat(9000)
        }
    });

    // Extremely tiny budget + critical pressure: expect compress decisions first, then evictions,
    // and an explicit overflow signal when only must-keep core remains.
    let mut manager = PlanningContextManager::with_token_budget(200);
    let summary = manager.next_summary(
        &state,
        0,
        false,
        Some(&previous_error),
        None,
        Some(&huge_projection),
    );
    assert_eq!(
        summary.pointer("/context_budget/pack_overflow_reason"),
        Some(&json!("must_keep_only_exceeds_budget"))
    );
    assert_eq!(
        summary.pointer("/context_budget/final_compact_applied"),
        Some(&Value::Bool(true))
    );
    let diagnostics = summary
        .pointer("/context_budget/pack_diagnostics")
        .cloned()
        .unwrap_or(Value::Null);
    let compressed = diagnostics
        .pointer("/compressed_blocks_total")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let evicted = diagnostics
        .pointer("/packed_blocks_evicted")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    assert!(
        compressed > 0,
        "expected compressions recorded; diagnostics={diagnostics:?}"
    );
    assert!(
        evicted > 0,
        "expected evictions recorded; diagnostics={diagnostics:?}"
    );

    let compressed_reasons = diagnostics
        .pointer("/compressed_by_reason")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    assert!(
        compressed_reasons.contains_key("pack_compress"),
        "expected pack_compress reason; reasons={compressed_reasons:?}"
    );
    let evicted_reasons = diagnostics
        .pointer("/evicted_by_reason")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    assert!(
        evicted_reasons.contains_key("pack_drop"),
        "expected pack_drop reason; reasons={evicted_reasons:?}"
    );
}

#[test]
fn pack_blocks_handles_medium_priority_blocks_after_low_and_stale() {
    let mut store = InputStore::default();
    // Make /input_store/facts large enough to pressure budgets.
    store.upsert_seed("owner", json!("0xabc"), "runtime.inputs.owner");
    for index in 0..80 {
        store.upsert_seed(
            format!("k.{index}"),
            json!("x".repeat(256)),
            "runtime.inputs",
        );
    }

    // Make /node_output_refs/entries large.
    let mut nodes = serde_json::Map::<String, Value>::new();
    for step in 0..24 {
        nodes.insert(
            format!("seg_{step:02}__q"),
            json!({
                "outputs": {
                    "a": "x",
                    "b": "y",
                    "c": {"nested": {"value": step}}
                }
            }),
        );
    }

    // Make a low-priority block huge so the pack loop must act.
    let huge_projection = json!({
        "schema": "ais-agent-tool-memory-projection/0.0.1",
        "recent": {
            "catalog_search": [{"query":"transfer","results":[{"ref":"erc20@0.0.2/transfer","notes":"z".repeat(6000)}]}],
            "candidate_detail": [{"signatures":[{"ref":"erc20@0.0.2/transfer","params":[{"name":"to"},{"name":"amount"}]}]}],
            "guide": {"topic": {"cel": {"summary": "y".repeat(4000)}}}
        }
    });

    let state = EngineRunnerState {
        runtime: json!({
            "nodes": Value::Object(nodes),
            "agent": {
                "llm_usage": {
                    "context_soft_limit_tokens": 100_000,
                    "context_remaining_tokens": 1_000
                }
            }
        }),
        ..EngineRunnerState::default()
    };

    let mut manager = PlanningContextManager::with_token_budget(700);
    let summary =
        manager.next_summary(&state, 0, false, None, Some(&store), Some(&huge_projection));

    let trace = summary
        .pointer("/context_budget/pack_trace")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert!(
        !trace.is_empty(),
        "pack_trace must record decisions when over budget"
    );

    assert!(
        trace.iter().any(|decision| {
            decision.pointer("/block_id").and_then(Value::as_str) == Some("input_store.facts")
        }),
        "pack loop should manage input_store.facts as an optional candidate; trace={trace:?}"
    );
    let fact_keys = summary
        .pointer("/input_store/facts")
        .and_then(Value::as_object)
        .map(|object| {
            object
                .keys()
                .filter(|key| !key.starts_with('_'))
                .cloned()
                .collect::<std::collections::BTreeSet<_>>()
        })
        .unwrap_or_default();
    let meta_keys = summary
        .pointer("/input_store/meta")
        .and_then(Value::as_object)
        .map(|object| {
            object
                .keys()
                .cloned()
                .collect::<std::collections::BTreeSet<_>>()
        })
        .unwrap_or_default();
    assert!(
        meta_keys.len() <= fact_keys.len(),
        "meta should not exceed packed fact cardinality: facts={fact_keys:?} meta={meta_keys:?}"
    );
    assert!(
        meta_keys.iter().all(|meta_key| {
            fact_keys.contains(meta_key)
                || meta_key
                    .strip_prefix("inputs.")
                    .is_some_and(|stripped| fact_keys.contains(stripped))
                || fact_keys
                    .iter()
                    .any(|fact_key| format!("inputs.{fact_key}") == *meta_key)
        }),
        "input_store.meta must remain coherent with packed input_store.facts: facts={fact_keys:?} meta={meta_keys:?}"
    );
}

#[test]
fn pack_blocks_can_compress_and_drop_recovery_related_context_under_pressure() {
    let state = EngineRunnerState {
        runtime: json!({
            "agent": {
                "llm_usage": {
                    "context_soft_limit_tokens": 100_000,
                    "context_remaining_tokens": 900
                },
                "missing_input_autofill": {
                    "query_autofill_round": {
                        "round": 3,
                        "terminal_reason": "attempt_exhausted"
                    },
                    "query_attempts": [
                        {
                            "status": "failed",
                            "query_ref": "erc20@0.0.2/balance-of",
                            "missing_ref": "inputs.token.address",
                            "detail": "x".repeat(2000)
                        },
                        {
                            "status": "failed",
                            "query_ref": "erc20@0.0.2/symbol",
                            "missing_ref": "inputs.token.address",
                            "detail": "y".repeat(2000)
                        }
                    ]
                }
            }
        }),
        ..EngineRunnerState::default()
    };
    let previous_error = json!({
        "phase": "ground_intent",
        "reason_code": "missing_inputs",
        "sub_reason_code": "autofill_exhausted",
        "message": "failed to recover missing refs",
        "autofill_history": {
            "attempt_keys": [
                "query_ref:erc20@0.0.2/balance-of",
                "query_ref:erc20@0.0.2/symbol"
            ],
            "trace": "z".repeat(8000)
        },
        "last_failed_finalize": {
            "tool": "plan.ground_intent",
            "assistant_content": "k".repeat(8000)
        }
    });

    let mut manager = PlanningContextManager::with_token_budget(320);
    let summary = manager.next_summary(&state, 0, false, Some(&previous_error), None, None);

    let trace = summary
        .pointer("/context_budget/pack_trace")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    assert!(
        trace.iter().any(|decision| {
            decision.pointer("/block_id").and_then(Value::as_str) == Some("recovery_diagnostics")
        }),
        "expected recovery_diagnostics to participate in pack loop; trace={trace:?}"
    );
    assert!(
        trace.iter().any(|decision| {
            decision.pointer("/block_id").and_then(Value::as_str) == Some("previous_error")
        }),
        "expected previous_error to participate in pack loop; trace={trace:?}"
    );
    assert!(
        trace.iter().any(|decision| {
            decision.pointer("/block_id").and_then(Value::as_str)
                == Some("previous_error.autofill_history")
        }),
        "expected previous_error.autofill_history to participate in pack loop; trace={trace:?}"
    );

    assert!(
        summary.pointer("/recovery_diagnostics").is_none()
            || summary.pointer("/recovery_diagnostics") == Some(&Value::Null),
        "recovery_diagnostics should be compressible/evictable under pressure"
    );
    assert!(
        summary
            .pointer("/previous_error/autofill_history")
            .is_none()
            || summary.pointer("/previous_error/autofill_history") == Some(&Value::Null),
        "previous_error.autofill_history should be compressible/evictable under pressure"
    );
}
