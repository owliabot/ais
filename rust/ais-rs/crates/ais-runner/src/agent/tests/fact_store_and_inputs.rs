#[test]
fn initial_input_store_derives_owner_from_evm_signer() {
    let mut chains = BTreeMap::new();
    chains.insert(
        "eip155:31338".to_string(),
        ChainConfig {
            rpc_url: "https://rpc.example".to_string(),
            timeout_ms: None,
            wait_for_receipt: None,
            receipt_poll: None,
            commitment: None,
            wait_for_confirmation: None,
            confirmation_poll: None,
            signer: Some(SignerConfig::EvmPrivateKey {
                private_key: "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d"
                    .to_string(),
            }),
        },
    );
    let config = RunnerConfig {
        schema: "ais-runner/0.0.1".to_string(),
        engine: RunnerEngineConfig::default(),
        llm: None,
        chains,
        plugins: RunnerPluginsConfig::default(),
    };

    let input_store =
        super::build_initial_input_store(&json!({}), &config, &["eip155:31338".to_string()])
            .expect("fact store");
    let owner = input_store.get("owner").expect("owner fact");
    assert_eq!(
        owner.value.as_str(),
        Some("0x70997970c51812dc3a010c7d01b50e0d17dc79c8")
    );
    assert_eq!(owner.meta.source, "config");
    assert_eq!(
        owner.meta.provenance.as_deref(),
        Some("runner_config.chains.eip155:31338.signer")
    );
    assert_eq!(
        input_store
            .get("owner_by_chain.eip155:31338")
            .and_then(|entry| entry.value.as_str()),
        Some("0x70997970c51812dc3a010c7d01b50e0d17dc79c8")
    );
}


#[test]
fn initial_input_store_uses_runtime_owner_when_signer_missing() {
    let mut chains = BTreeMap::new();
    chains.insert(
        "eip155:1".to_string(),
        ChainConfig {
            rpc_url: "https://rpc.example".to_string(),
            timeout_ms: None,
            wait_for_receipt: None,
            receipt_poll: None,
            commitment: None,
            wait_for_confirmation: None,
            confirmation_poll: None,
            signer: None,
        },
    );
    let config = RunnerConfig {
        schema: "ais-runner/0.0.1".to_string(),
        engine: RunnerEngineConfig::default(),
        llm: None,
        chains,
        plugins: RunnerPluginsConfig::default(),
    };

    let runtime = json!({
        "inputs": {
            "wallet": "0x1111111111111111111111111111111111111111"
        }
    });
    let input_store = super::build_initial_input_store(&runtime, &config, &["eip155:1".to_string()])
        .expect("fact store");
    let owner = input_store.get("owner").expect("owner fact");
    assert_eq!(
        owner.value.as_str(),
        Some("0x1111111111111111111111111111111111111111")
    );
    assert_eq!(owner.meta.source, "runtime");
    assert_eq!(owner.meta.provenance.as_deref(), Some("runtime.inputs.wallet"));
}


#[test]
fn initial_input_store_prefers_signer_over_runtime_owner() {
    let mut chains = BTreeMap::new();
    chains.insert(
        "eip155:31338".to_string(),
        ChainConfig {
            rpc_url: "https://rpc.example".to_string(),
            timeout_ms: None,
            wait_for_receipt: None,
            receipt_poll: None,
            commitment: None,
            wait_for_confirmation: None,
            confirmation_poll: None,
            signer: Some(SignerConfig::EvmPrivateKey {
                private_key: "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d"
                    .to_string(),
            }),
        },
    );
    let config = RunnerConfig {
        schema: "ais-runner/0.0.1".to_string(),
        engine: RunnerEngineConfig::default(),
        llm: None,
        chains,
        plugins: RunnerPluginsConfig::default(),
    };

    let runtime = json!({
        "inputs": {
            "owner": "0x1111111111111111111111111111111111111111"
        }
    });
    let input_store =
        super::build_initial_input_store(&runtime, &config, &["eip155:31338".to_string()])
            .expect("fact store");
    let owner = input_store.get("owner").expect("owner fact");
    assert_eq!(
        owner.value.as_str(),
        Some("0x70997970c51812dc3a010c7d01b50e0d17dc79c8")
    );
    assert_eq!(owner.meta.source, "config");
    assert_eq!(
        owner.meta.provenance.as_deref(),
        Some("runner_config.chains.eip155:31338.signer")
    );
}


#[test]
fn initial_input_store_seeds_runtime_inputs_under_inputs_namespace() {
    let config = RunnerConfig {
        schema: "ais-runner/0.0.1".to_string(),
        engine: RunnerEngineConfig::default(),
        llm: None,
        chains: BTreeMap::new(),
        plugins: RunnerPluginsConfig::default(),
    };
    let runtime = json!({
        "inputs": {
            "owner": "0x1111111111111111111111111111111111111111",
            "token": {
                "address": "0x2222222222222222222222222222222222222222",
                "decimals": 6
            },
            "amount": "1.5"
        }
    });
    let input_store = super::build_initial_input_store(&runtime, &config, &["eip155:1".to_string()])
        .expect("fact store");
    assert_eq!(
        input_store
            .get("owner")
            .and_then(|entry| entry.value.as_str()),
        Some("0x1111111111111111111111111111111111111111")
    );
    assert_eq!(
        input_store
            .get("inputs.owner")
            .and_then(|entry| entry.value.as_str()),
        Some("0x1111111111111111111111111111111111111111")
    );
    assert_eq!(
        input_store
            .get("token.address")
            .and_then(|entry| entry.value.as_str()),
        Some("0x2222222222222222222222222222222222222222")
    );
    assert_eq!(
        input_store
            .get("inputs.token.address")
            .and_then(|entry| entry.value.as_str()),
        Some("0x2222222222222222222222222222222222222222")
    );
    assert_eq!(
        input_store
            .get("inputs.token.decimals")
            .and_then(|entry| entry.value.as_i64()),
        Some(6)
    );
    assert_eq!(
        input_store
            .get("inputs.amount")
            .and_then(|entry| entry.value.as_str()),
        Some("1.5")
    );
}


#[test]
fn state_summary_includes_input_store_payload() {
    let mut input_store = super::InputStore::default();
    input_store.upsert_user(
        "owner",
        json!("0x2222222222222222222222222222222222222222"),
        "user.prompt",
    );
    let state = EngineRunnerState::default();
    let summary = super::build_state_summary(
        &state,
        0,
        false,
        None,
        Some(&input_store,
        ),
    );
    assert_eq!(
        summary.pointer("/input_store/facts/owner"),
        Some(&json!("0x2222222222222222222222222222222222222222"))
    );
    assert_eq!(
        summary.pointer("/input_store/meta/owner/source"),
        Some(&json!("user"))
    );
    assert_eq!(
        summary.pointer("/input_store/meta/owner/layer"),
        Some(&json!("observed"))
    );
    assert_eq!(
        summary.pointer("/input_store/meta/owner/source_priority"),
        Some(&json!(100))
    );
    assert_eq!(
        summary.pointer("/input_store/meta/owner/provenance"),
        Some(&json!("user.prompt"))
    );
    assert_eq!(
        summary.pointer("/input_store/meta/owner/stability"),
        Some(&json!("unknown"))
    );
}


#[test]
fn state_summary_includes_todo_state_payload_from_runtime() {
    let state = EngineRunnerState {
        runtime: json!({
            "agent": {
                "todo_progress": {
                    "schema": "ais-agent-todo-progress/0.0.1",
                    "current_todo": {"id":"todo_1","status":"in_progress"},
                    "progress": {"todo":0,"in_progress":1,"done":0,"blocked":0,"total":1},
                    "next_seq": 2
                }
            }
        }),
        ..EngineRunnerState::default()
    };
    let summary = super::build_state_summary(&state, 0, false, None, None);
    assert_eq!(
        summary.pointer("/todo_state/schema"),
        Some(&json!("ais-agent-todo-progress/0.0.1"))
    );
    assert_eq!(
        summary.pointer("/todo_state/current_todo/id"),
        Some(&json!("todo_1"))
    );
    assert_eq!(
        summary.pointer("/todo_state/current_todo/status"),
        Some(&json!("in_progress"))
    );
    assert_eq!(
        summary.pointer("/todo_state/progress/total"),
        Some(&json!(1))
    );
    assert_eq!(summary.pointer("/todo_state/next_seq"), Some(&json!(2)));
}

#[test]
fn state_summary_projects_input_registry_missing_slots_from_todo_and_questions() {
    let mut input_store = super::InputStore::default();
    input_store.upsert_seed(
        "inputs.owner",
        json!("0xabc"),
        "runtime.inputs.owner",
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
                        "id":"todo_1",
                        "required_facts": ["inputs.owner", "inputs.amount"]
                    }
                },
                "missing_required_input": {
                    "questions": [
                        {"id":"token.decimals", "question":"token decimals?"}
                    ]
                }
            }
        }),
        ..EngineRunnerState::default()
    };

    let summary = super::build_state_summary(
        &state,
        0,
        false,
        None,
        Some(&input_store,
        ),
    );
    assert_eq!(
        summary.pointer("/input_registry/schema"),
        Some(&json!("ais-agent-input-registry/0.0.1"))
    );
    assert_eq!(
        summary.pointer("/todo_state/current_todo/id"),
        Some(&json!("todo_1"))
    );
    assert_eq!(
        summary.pointer("/input_slots/canonical_refs/owner"),
        Some(&json!("inputs.owner"))
    );

    let missing_items = summary
        .pointer("/input_slots/missing")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let missing_refs = missing_items
        .iter()
        .filter_map(|item| item.get("ref"))
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    assert!(missing_refs.contains(&"inputs.amount"), "missing_refs={missing_refs:?}");
    assert!(
        missing_refs.contains(&"inputs.token.decimals"),
        "missing_refs={missing_refs:?}"
    );

    let registry_entries = summary
        .pointer("/input_registry/entries")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert!(registry_entries.iter().any(|entry| {
        entry.get("ref") == Some(&json!("inputs.owner"))
            && entry.get("status") == Some(&json!("resolved"))
    }));
    assert!(registry_entries.iter().any(|entry| {
        entry.get("ref") == Some(&json!("inputs.amount"))
            && entry.get("status") == Some(&json!("missing"))
    }));
    assert!(registry_entries.iter().any(|entry| {
        entry.get("ref") == Some(&json!("inputs.token.decimals"))
            && entry.get("status") == Some(&json!("missing"))
    }));
}

#[test]
fn state_summary_includes_node_output_refs_projection_for_segment_outputs() {
    let state = EngineRunnerState {
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
    let summary = super::build_state_summary(&state, 0, false, None, None);
    assert_eq!(
        summary.pointer("/node_output_refs/schema"),
        Some(&json!("ais-agent-node-output-refs/0.0.1"))
    );
    let known_ref_items = summary
        .pointer("/node_output_refs/known_refs")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let known_refs = known_ref_items
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    assert!(
        known_refs.contains(&"nodes.q_balance.outputs.balance"),
        "known_refs={known_refs:?}"
    );
    assert!(
        known_refs.contains(&"nodes.a_transfer.outputs.outputs.tx_hash"),
        "known_refs={known_refs:?}"
    );
}

#[test]
fn context_envelope_keeps_projected_summary_contract_compatible() {
    let mut input_store = super::InputStore::default();
    input_store.upsert_seed(
        "inputs.owner",
        json!("0xabc"),
        "runtime.inputs.owner",
    );
    let state = EngineRunnerState {
        runtime: json!({
            "inputs": {
                "owner": "0xabc",
                "token": {"address":"0xdef"}
            }
        }),
        ..EngineRunnerState::default()
    };

    let mut manager = super::context_view::PlanningContextManager::default();
    let envelope = manager.next_summary(
        &state,
        0,
        false,
        None,
        Some(&input_store,
        ),
        None,
    );

    assert_eq!(envelope.get("context_version").and_then(Value::as_u64), Some(1));
    assert_eq!(
        envelope.get("context_unchanged").and_then(Value::as_bool),
        Some(false)
    );
    assert!(
        envelope
            .get("context_hash")
            .and_then(Value::as_str)
            .is_some_and(|value| !value.is_empty())
    );
    let context_hash = envelope
        .get("context_hash")
        .and_then(Value::as_str)
        .expect("context hash");
    assert_eq!(
        envelope.pointer("/context_envelope/schema"),
        Some(&json!("ais-agent-context-envelope"))
    );
    assert_eq!(envelope.pointer("/context_envelope/schema_version"), Some(&json!(1)));
    assert_eq!(envelope.pointer("/context_envelope/version"), Some(&json!(1)));
    assert_eq!(
        envelope.pointer("/context_envelope/hash"),
        Some(&json!(context_hash))
    );
    assert_eq!(envelope.pointer("/context_envelope/unchanged"), Some(&json!(false)));
    assert_eq!(
        envelope.pointer("/input_registry/schema"),
        Some(&json!("ais-agent-input-registry/0.0.1"))
    );
    assert_eq!(
        envelope.pointer("/node_output_refs/schema"),
        Some(&json!("ais-agent-node-output-refs/0.0.1"))
    );
    assert_eq!(
        envelope.pointer("/input_slots/canonical_refs/owner"),
        Some(&json!("inputs.owner"))
    );
    let meta_base = if envelope.pointer("/input_store/meta/inputs.owner").is_some() {
        "/input_store/meta/inputs.owner"
    } else {
        "/input_store/meta/owner"
    };
    assert_eq!(
        envelope.pointer(&format!("{meta_base}/source")),
        Some(&json!("seed"))
    );
    assert_eq!(
        envelope.pointer(&format!("{meta_base}/layer")),
        Some(&json!("seed"))
    );
    assert_eq!(
        envelope.pointer(&format!("{meta_base}/provenance")),
        Some(&json!("runtime.inputs.owner"))
    );
}

#[test]
fn context_envelope_hash_and_unchanged_flags_track_payload_mutations() {
    let mut manager = super::context_view::PlanningContextManager::default();
    let state = EngineRunnerState::default();
    let mut input_store_base = super::InputStore::default();
    input_store_base.upsert_seed("inputs.owner", json!("0xabc"), "runtime.inputs.owner");
    let mut input_store_changed = input_store_base.clone();
    input_store_changed.upsert_seed("inputs.amount", json!("1.0"), "runtime.inputs.amount");
    let first = manager.next_summary(&state, 0, false, None, Some(&input_store_base), None);
    let second = manager.next_summary(&state, 0, false, None, Some(&input_store_base), None);
    let third = manager.next_summary(&state, 0, false, None, Some(&input_store_changed), None);

    assert_eq!(first.get("context_version").and_then(Value::as_u64), Some(1));
    assert_eq!(second.get("context_version").and_then(Value::as_u64), Some(2));
    assert_eq!(third.get("context_version").and_then(Value::as_u64), Some(3));
    assert_eq!(
        first.pointer("/context_envelope/schema"),
        Some(&json!("ais-agent-context-envelope"))
    );
    assert_eq!(first.pointer("/context_envelope/schema_version"), Some(&json!(1)));
    assert_eq!(first.pointer("/context_envelope/version"), Some(&json!(1)));
    assert_eq!(second.pointer("/context_envelope/version"), Some(&json!(2)));
    assert_eq!(third.pointer("/context_envelope/version"), Some(&json!(3)));
    assert_eq!(
        second.get("context_unchanged").and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        third.get("context_unchanged").and_then(Value::as_bool),
        Some(false)
    );
    assert_eq!(
        second.pointer("/context_envelope/unchanged"),
        Some(&json!(true))
    );
    assert_eq!(
        third.pointer("/context_envelope/unchanged"),
        Some(&json!(false))
    );
    assert_eq!(first.get("context_hash"), second.get("context_hash"));
    assert_ne!(second.get("context_hash"), third.get("context_hash"));
    let first_hash = first
        .get("context_hash")
        .and_then(Value::as_str)
        .expect("first hash");
    let third_hash = third
        .get("context_hash")
        .and_then(Value::as_str)
        .expect("third hash");
    assert_eq!(
        first.pointer("/context_envelope/hash"),
        Some(&json!(first_hash))
    );
    assert_eq!(
        third.pointer("/context_envelope/hash"),
        Some(&json!(third_hash))
    );
    assert_eq!(
        third.pointer("/input_slots/canonical_refs/amount"),
        Some(&json!("inputs.amount"))
    );
}

#[test]
fn context_budget_reports_payload_vs_emitted_estimates_under_tight_budget() {
    let mut inputs = serde_json::Map::<String, Value>::new();
    inputs.insert("owner".to_string(), json!("0xabc"));
    for index in 0..220 {
        inputs.insert(format!("extra_{index}"), json!(format!("v{index}")));
    }
    let state = EngineRunnerState {
        runtime: json!({
            "inputs": Value::Object(inputs),
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

    let mut manager = super::context_view::PlanningContextManager::with_token_budget(64);
    let summary = manager.next_summary(&state, 0, false, None, None, None);
    assert!(summary.pointer("/context_budget/pressure_mode").is_some());
    assert!(summary.pointer("/context_budget/pack_diagnostics").is_some());
    assert!(summary.pointer("/context_budget/pack_trace").is_some());
    assert!(
        summary
            .pointer("/context_budget/final_compact_applied")
            .and_then(Value::as_bool)
            .is_some()
    );
}

#[test]
fn typed_context_core_path_switch_keeps_projection_and_envelope_contract_parity() {
    let mut input_store = super::InputStore::default();
    input_store.upsert_seed(
        "inputs.owner",
        json!("0xabc"),
        "runtime.inputs.owner",
    );
    let state = EngineRunnerState {
        runtime: json!({
            "inputs": {
                "owner": "0xabc",
                "token": {"address": "0xdef"}
            },
            "nodes": {
                "seg_transfer__q_balance": {
                    "outputs": {"balance": "123"}
                }
            }
        }),
        ..EngineRunnerState::default()
    };
    let previous_error = json!({
        "phase": "planning",
        "reason_code": "planner_invalid_tool_output"
    });
    let tool_memory_projection = json!({
        "schema": "ais-agent-tool-memory-projection/0.0.1",
        "recent": {"catalog_search": []}
    });

    let via_compat_facade = super::context_view::build_projected_summary(
        &state,
        2,
        false,
        Some(&previous_error),
        Some(&input_store,
        ),
        Some(&tool_memory_projection),
    );
    let core_payload = super::context::projector::build_projected_summary_base(
        &state,
        2,
        false,
        Some(&previous_error),
        Some(&input_store,
        ),
        Some(&tool_memory_projection),
    );
    let via_typed_core = super::context::budgeter::budget_and_compact_summary(
        core_payload,
        &state,
        super::context_view::DEFAULT_PLANNER_CONTEXT_TOKEN_BUDGET,
    );
    assert_eq!(
        via_compat_facade.pointer("/input_registry/schema"),
        Some(&json!("ais-agent-input-registry/0.0.1"))
    );
    assert_eq!(
        via_typed_core.pointer("/input_registry/schema"),
        Some(&json!("ais-agent-input-registry/0.0.1"))
    );
    assert_eq!(
        via_compat_facade.pointer("/node_output_refs/schema"),
        Some(&json!("ais-agent-node-output-refs/0.0.1"))
    );
    assert_eq!(
        via_typed_core.pointer("/node_output_refs/schema"),
        Some(&json!("ais-agent-node-output-refs/0.0.1"))
    );
    assert_eq!(
        via_typed_core.pointer("/input_slots/canonical_refs/owner"),
        Some(&json!("inputs.owner"))
    );
    let typed_owner_meta_base = if via_typed_core.pointer("/input_store/meta/inputs.owner").is_some()
    {
        "/input_store/meta/inputs.owner"
    } else {
        "/input_store/meta/owner"
    };
    assert_eq!(
        via_typed_core.pointer(&format!("{typed_owner_meta_base}/source")),
        Some(&json!("seed"))
    );
    assert_eq!(
        via_typed_core.pointer(&format!("{typed_owner_meta_base}/provenance")),
        Some(&json!("runtime.inputs.owner"))
    );
    assert_eq!(
        via_typed_core.pointer("/tool_memory_projection/schema"),
        Some(&json!("ais-agent-tool-memory-projection/0.0.1"))
    );
    assert_eq!(
        via_typed_core.pointer("/previous_error/reason_code"),
        Some(&json!("planner_invalid_tool_output"))
    );

    let envelope_v1 =
        super::context::envelope::ContextEnvelope::from_payload(&via_typed_core, 1, None);
    let summary_v1 = envelope_v1.to_compat_summary(via_typed_core.clone());
    let parsed_v1 =
        super::context::envelope::ContextEnvelope::from_summary(&summary_v1).expect("envelope");
    assert_eq!(parsed_v1.schema, "ais-agent-context-envelope");
    assert_eq!(parsed_v1.schema_version, 1);
    assert_eq!(parsed_v1.version, 1);
    assert_eq!(parsed_v1.hash, envelope_v1.hash);
    let payload_v1 = super::context::envelope::payload_from_summary(&summary_v1);
    assert!(payload_v1.get("context_envelope").is_none());
    assert!(payload_v1.get("context_hash").is_none());
    assert_eq!(
        payload_v1.pointer("/input_registry/schema"),
        Some(&json!("ais-agent-input-registry/0.0.1"))
    );
    let payload_owner_meta_base = if payload_v1.pointer("/input_store/meta/inputs.owner").is_some()
    {
        "/input_store/meta/inputs.owner"
    } else {
        "/input_store/meta/owner"
    };
    assert_eq!(
        payload_v1.pointer(&format!("{payload_owner_meta_base}/provenance")),
        Some(&json!("runtime.inputs.owner"))
    );

    let envelope_v2 = super::context::envelope::ContextEnvelope::from_payload(
        &via_typed_core,
        2,
        Some(envelope_v1.hash.as_str()),
    );
    let summary_v2 = envelope_v2.to_compat_summary(via_typed_core);
    assert_eq!(
        summary_v2.get("context_unchanged").and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        summary_v2.pointer("/context_envelope/unchanged"),
        Some(&json!(true))
    );
}

#[test]
fn context_envelope_foreign_schema_falls_back_to_legacy_contract() {
    let payload = json!({
        "done": false,
        "input_registry": {"known_refs": ["inputs.owner"]}
    });
    let envelope = super::context::envelope::ContextEnvelope::from_payload(&payload, 7, None);
    let mut summary = envelope.to_compat_summary(payload);
    summary["context_envelope"]["schema"] = json!("foreign-context-envelope");
    summary["context_envelope"]["schema_version"] = json!(999);

    let parsed = super::context::envelope::ContextEnvelope::from_summary(&summary)
        .expect("legacy compatibility fallback");
    assert_eq!(parsed.schema, super::context::envelope::CONTEXT_ENVELOPE_SCHEMA);
    assert_eq!(
        parsed.schema_version,
        super::context::envelope::CONTEXT_ENVELOPE_SCHEMA_VERSION
    );
    assert_eq!(parsed.version, 7);
    assert_eq!(parsed.hash, envelope.hash);
}

#[test]
fn context_envelope_foreign_schema_without_legacy_contract_is_rejected() {
    let summary = json!({
        "done": false,
        "context_envelope": {
            "schema": "foreign-context-envelope",
            "schema_version": 1,
            "version": 1,
            "hash": "x",
            "unchanged": false
        }
    });
    assert!(super::context::envelope::ContextEnvelope::from_summary(&summary).is_none());
}

#[test]
fn context_envelope_hash_verification_rejects_tampered_payload() {
    let payload = json!({
        "done": false,
        "input_registry": {"known_refs": ["inputs.owner"]}
    });
    let envelope = super::context::envelope::ContextEnvelope::from_payload(&payload, 3, None);
    let mut summary = envelope.to_compat_summary(payload);
    summary["done"] = json!(true);

    assert!(super::context::envelope::ContextEnvelope::from_summary(&summary).is_some());
    assert!(
        super::context::envelope::ContextEnvelope::from_summary_with_options(&summary, true)
            .is_none(),
        "strict hash verification should reject tampered summaries"
    );
}

#[test]
fn context_budget_prefers_worst_case_usage_signal_when_window_and_remaining_diverge() {
    let state = EngineRunnerState {
        runtime: json!({
            "inputs": {
                "owner": "0xabc"
            },
            "agent": {
                "llm_usage": {
                    "context_soft_limit_tokens": 100000,
                    "context_window_input_tokens": 60000,
                    "context_remaining_tokens": 12000
                }
            }
        }),
        ..EngineRunnerState::default()
    };

    let summary = super::context_view::build_projected_summary(&state, 0, false, None, None, None);
    assert_eq!(
        summary.pointer("/context_budget/pressure_mode"),
        Some(&json!("medium"))
    );
}

#[test]
fn context_pressure_uses_absolute_remaining_guard_even_when_usage_ratio_is_low() {
    let state = EngineRunnerState {
        runtime: json!({
            "inputs": {
                "owner": "0xabc"
            },
            "agent": {
                "llm_usage": {
                    "context_soft_limit_tokens": 4000,
                    "context_window_input_tokens": 1000,
                    "context_remaining_tokens": 2500
                }
            }
        }),
        ..EngineRunnerState::default()
    };

    let summary = super::context_view::build_projected_summary(&state, 0, false, None, None, None);
    assert_eq!(
        summary.pointer("/context_budget/pressure_mode"),
        Some(&json!("critical"))
    );
}


#[test]
fn record_runtime_agent_field_initializes_runtime_and_repairs_corrupt_agent_entry() {
    let mut runtime = json!(null);
    super::record_runtime_agent_field(&mut runtime, "capability_ready", json!(true));
    assert_eq!(
        runtime.pointer("/agent/capability_ready"),
        Some(&json!(true))
    );

    runtime
        .as_object_mut()
        .expect("runtime must be object")
        .insert("agent".to_string(), json!("corrupted"));
    super::record_runtime_agent_field(&mut runtime, "capability_view", json!({"ready": true}));
    assert_eq!(
        runtime.pointer("/agent/capability_view/ready"),
        Some(&json!(true))
    );
    assert!(
        runtime.pointer("/agent").and_then(Value::as_object).is_some(),
        "runtime.agent should be repaired into object"
    );
}


#[test]
fn record_todo_progress_tracks_follow_up_todo_after_completion() {
    let mut runtime = json!({"agent":"corrupted"});
    let mut board = super::todos::TodoBoard::bootstrap("transfer 1 token");
    board.mark_current_in_progress(Some("query balances"), "seg_1");
    super::record_todo_progress(&mut runtime, &board);
    assert_eq!(
        runtime.pointer("/agent/todo_progress/current_todo/id"),
        Some(&json!("todo_1"))
    );
    assert_eq!(
        runtime.pointer("/agent/todo_progress/current_todo/status"),
        Some(&json!("in_progress"))
    );
    assert!(runtime.pointer("/todo_progress").is_none());

    board.mark_current_done();
    board.open_follow_up_todo();
    super::record_todo_progress(&mut runtime, &board);
    assert_eq!(
        runtime.pointer("/agent/todo_progress/current_todo/id"),
        Some(&json!("todo_2"))
    );
    assert_eq!(
        runtime.pointer("/agent/todo_progress/progress/done"),
        Some(&json!(1))
    );
    assert!(runtime.pointer("/todo_progress").is_none());
}


#[test]
fn missing_required_input_payload_roundtrip_records_questions() {
    let payload = super::missing_required_input_payload(
        Some("missing token decimals"),
        &[json!({
            "id": "token.decimals",
            "question": "token decimals?",
            "options": [{"label":"18","value":18}]
        })],
        &[json!({"kind":"schema_error","reason_code":"missing_input","message":"x"})],
        2,
    );
    assert_eq!(
        payload.pointer("/reason_code"),
        Some(&json!("missing_required_input"))
    );
    assert_eq!(
        payload.pointer("/questions/0/id"),
        Some(&json!("token.decimals"))
    );
    assert_eq!(payload.pointer("/round"), Some(&json!(2)));

    let mut runtime = json!({});
    super::record_missing_required_input(&mut runtime, &payload);
    assert_eq!(
        runtime.pointer("/agent/missing_required_input/reason_code"),
        Some(&json!("missing_required_input"))
    );
    assert_eq!(
        runtime.pointer("/agent/missing_required_input/questions/0/id"),
        Some(&json!("token.decimals"))
    );
    assert!(runtime.pointer("/missing_required_input").is_none());
}


#[test]
fn apply_missing_input_answers_backfills_runtime_and_input_store() {
    let mut state = EngineRunnerState {
        runtime: json!({}),
        ..EngineRunnerState::default()
    };
    let mut store = super::InputStore::default();
    let answers = Map::from_iter([
        ("owner".to_string(), json!("0xabc")),
        ("token.decimals".to_string(), json!(18)),
    ]);
    super::apply_missing_input_answers(&mut state, &mut store, &answers);

    assert_eq!(
        state.runtime.pointer("/inputs/owner"),
        Some(&json!("0xabc"))
    );
    assert_eq!(
        state.runtime.pointer("/inputs/token/decimals"),
        Some(&json!(18))
    );
    assert_eq!(
        store.get("owner").and_then(|entry| entry.value.as_str()),
        Some("0xabc")
    );
    assert_eq!(
        store
            .get("wallet.default")
            .and_then(|entry| entry.value.as_str()),
        Some("0xabc")
    );
    assert_eq!(
        store
            .get("token.decimals")
            .and_then(|entry| entry.value.as_i64()),
        Some(18)
    );
    assert_eq!(
        store
            .get("inputs.token.decimals")
            .and_then(|entry| entry.value.as_i64()),
        Some(18)
    );
    let decimals = store
        .get("inputs.token.decimals")
        .expect("inputs.token.decimals");
    assert_eq!(decimals.meta.source, "user");
    assert_eq!(
        decimals.meta.provenance.as_deref(),
        Some("user.prompt.token.decimals")
    );
}


#[test]
fn apply_missing_input_answers_normalizes_inputs_prefixed_keys() {
    let mut state = EngineRunnerState {
        runtime: json!({}),
        ..EngineRunnerState::default()
    };
    let mut store = super::InputStore::default();
    let answers = Map::from_iter([("inputs.owner".to_string(), json!("0xabc"))]);
    super::apply_missing_input_answers(&mut state, &mut store, &answers);

    assert_eq!(
        state.runtime.pointer("/inputs/owner"),
        Some(&json!("0xabc"))
    );
    assert!(state.runtime.pointer("/inputs/inputs/owner").is_none());
    assert!(store.get("inputs.inputs.owner").is_none());
    assert_eq!(
        store
            .get("inputs.owner")
            .and_then(|entry| entry.value.as_str()),
        Some("0xabc")
    );
}


#[test]
fn apply_missing_input_answers_normalizes_runtime_inputs_prefixed_keys() {
    let mut state = EngineRunnerState {
        runtime: json!({}),
        ..EngineRunnerState::default()
    };
    let mut store = super::InputStore::default();
    let answers = Map::from_iter([("runtime.inputs.owner".to_string(), json!("0xdef"))]);
    super::apply_missing_input_answers(&mut state, &mut store, &answers);

    assert_eq!(
        state.runtime.pointer("/inputs/owner"),
        Some(&json!("0xdef"))
    );
    assert!(state
        .runtime
        .pointer("/inputs/runtime/inputs/owner")
        .is_none());
    assert!(store.get("inputs.runtime.inputs.owner").is_none());
    assert_eq!(
        store
            .get("inputs.owner")
            .and_then(|entry| entry.value.as_str()),
        Some("0xdef")
    );
}

#[test]
fn auto_answer_missing_input_question_selects_single_query_option() {
    let questions = super::parse_missing_input_questions(&[json!({
        "id":"inputs.token.decimals",
        "question":"What is token decimals?",
        "required":true,
        "options":[
            {"label":"Query decimals","value":"query"},
            {"label":"Use default 18","value":18}
        ]
    })]);
    assert_eq!(questions.len(), 1);
    let answer = super::auto_answer_missing_input_question(&questions[0]);
    assert_eq!(answer, Some(json!("query")));
}

#[test]
fn auto_answer_missing_input_question_skips_ambiguous_query_options() {
    let questions = super::parse_missing_input_questions(&[json!({
        "id":"inputs.token.decimals",
        "question":"What is token decimals?",
        "required":true,
        "options":[
            {"label":"Query via erc20","value":"query_erc20"},
            {"label":"Query via metadata","value":"query_metadata"}
        ]
    })]);
    assert_eq!(questions.len(), 1);
    let answer = super::auto_answer_missing_input_question(&questions[0]);
    assert!(answer.is_none());
}

#[test]
fn maybe_collect_missing_input_answers_auto_query_works_without_tty_prompt() {
    let questions = vec![json!({
        "id":"inputs.token.decimals",
        "question":"What is token decimals?",
        "required":true,
        "options":[
            {"label":"Query decimals","value":"query"}
        ]
    })];
    let answers = super::maybe_collect_missing_input_answers(&questions)
        .expect("collect should succeed")
        .expect("query option should be auto-selected");
    assert_eq!(answers.get("inputs.token.decimals"), Some(&json!("query")));
}

#[test]
fn apply_missing_input_answers_resolves_token_decimals_missing_slot_in_summary() {
    let mut state = EngineRunnerState {
        runtime: json!({
            "agent": {
                "missing_required_input": {
                    "questions": [
                        {"id":"token.decimals","question":"token decimals?"}
                    ]
                }
            }
        }),
        ..EngineRunnerState::default()
    };
    let mut store = super::InputStore::default();
    let summary_before = super::build_state_summary(
        &state,
        0,
        false,
        None,
        Some(&store,
        ),
    );
    assert!(summary_before
        .pointer("/input_slots/missing")
        .and_then(Value::as_array)
        .is_some_and(|items| {
            items
                .iter()
                .any(|item| item.get("ref") == Some(&json!("inputs.token.decimals")))
        }));

    let answers = Map::from_iter([("token.decimals".to_string(), json!(6))]);
    super::apply_missing_input_answers(&mut state, &mut store, &answers);

    let summary_after = super::build_state_summary(
        &state,
        0,
        false,
        None,
        Some(&store,
        ),
    );
    assert_eq!(
        summary_after.pointer("/input_slots/canonical_refs/token.decimals"),
        Some(&json!("inputs.token.decimals"))
    );
    assert!(!summary_after
        .pointer("/input_slots/missing")
        .and_then(Value::as_array)
        .is_some_and(|items| {
            items
                .iter()
                .any(|item| item.get("ref") == Some(&json!("inputs.token.decimals")))
        }));
    assert!(summary_after
        .pointer("/input_registry/entries")
        .and_then(Value::as_array)
        .is_some_and(|entries| {
            entries.iter().any(|entry| {
                entry.get("ref") == Some(&json!("inputs.token.decimals"))
                    && entry.get("status") == Some(&json!("resolved"))
            })
        }));
}

#[test]
fn apply_missing_input_answers_resolves_non_token_object_missing_slot_in_summary() {
    let mut state = EngineRunnerState {
        runtime: json!({
            "agent": {
                "missing_required_input": {
                    "questions": [
                        {"id":"recipient.profile","question":"recipient profile?"}
                    ]
                }
            }
        }),
        ..EngineRunnerState::default()
    };
    let mut store = super::InputStore::default();
    let summary_before = super::build_state_summary(
        &state,
        0,
        false,
        None,
        Some(&store,
        ),
    );
    assert!(summary_before
        .pointer("/input_slots/missing")
        .and_then(Value::as_array)
        .is_some_and(|items| {
            items
                .iter()
                .any(|item| item.get("ref") == Some(&json!("inputs.recipient.profile")))
        }));

    let answers = Map::from_iter([(
        "recipient.profile".to_string(),
        json!({
            "address":"0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
            "chain_ref":"eip155:31338",
            "label":"ops-wallet"
        }),
    )]);
    super::apply_missing_input_answers(&mut state, &mut store, &answers);

    assert_eq!(
        state.runtime.pointer("/inputs/recipient/profile/address"),
        Some(&json!("0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"))
    );
    assert_eq!(
        state.runtime.pointer("/inputs/recipient/profile/chain_ref"),
        Some(&json!("eip155:31338"))
    );
    assert_eq!(
        store
            .get("recipient.profile")
            .and_then(|entry| entry.value.pointer("/address"))
            .and_then(Value::as_str),
        Some("0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266")
    );
    assert_eq!(
        store
            .get("inputs.recipient.profile")
            .and_then(|entry| entry.value.pointer("/chain_ref"))
            .and_then(Value::as_str),
        Some("eip155:31338")
    );
    assert!(store.get("token.address").is_none());
    assert!(store.get("inputs.token.decimals").is_none());
    let profile = store
        .get("inputs.recipient.profile")
        .expect("inputs.recipient.profile");
    assert_eq!(profile.meta.source, "user");
    assert_eq!(
        profile.meta.provenance.as_deref(),
        Some("user.prompt.recipient.profile")
    );

    let summary_after = super::build_state_summary(
        &state,
        0,
        false,
        None,
        Some(&store,
        ),
    );
    assert_eq!(
        summary_after.pointer("/input_slots/canonical_refs/recipient.profile"),
        Some(&json!("inputs.recipient.profile"))
    );
    assert!(!summary_after
        .pointer("/input_slots/missing")
        .and_then(Value::as_array)
        .is_some_and(|items| {
            items
                .iter()
                .any(|item| item.get("ref") == Some(&json!("inputs.recipient.profile")))
        }));
    assert!(summary_after
        .pointer("/input_registry/entries")
        .and_then(Value::as_array)
        .is_some_and(|entries| {
            entries.iter().any(|entry| {
                entry.get("ref") == Some(&json!("inputs.recipient.profile"))
                    && entry.get("status") == Some(&json!("resolved"))
            })
        }));
}

#[test]
fn query_stores_backfill_multiple_fields_to_inputs_with_query_meta() {
    let segment: ais_sdk::documents::PlanSketchSegment = serde_json::from_value(json!({
        "segment_id":"seg_query",
        "cursor_in":"0",
        "cursor_out":"1",
        "done":false,
        "steps":[
            {
                "id":"q_token",
                "kind":"query",
                "candidate_ref":"erc20@0.0.2/token_meta",
                "inputs":{},
                "stores":{
                    "decimals":"token.decimals",
                    "symbol":"runtime.inputs.token.symbol",
                    "address":"inputs.token.address"
                }
            }
        ]
    }))
    .expect("segment");
    let state = EngineRunnerState {
        runtime: json!({
            "nodes": {
                "seg_query/q_token": {
                    "outputs":{"decimals":6,"symbol":"USDC","address":"0x2222222222222222222222222222222222222222"}
                }
            }
        }),
        ..EngineRunnerState::default()
    };
    let mut input_store = super::InputStore::default();

    super::phase_machine::segment_exec::apply_segment_stores_from_runtime(
        &segment,
        &state,
        &mut input_store,
        false,
    );

    assert_eq!(
        input_store
            .get("inputs.token.decimals")
            .and_then(|entry| entry.value.as_i64()),
        Some(6)
    );
    assert_eq!(
        input_store
            .get("inputs.token.symbol")
            .and_then(|entry| entry.value.as_str()),
        Some("USDC")
    );
    assert_eq!(
        input_store
            .get("inputs.token.address")
            .and_then(|entry| entry.value.as_str()),
        Some("0x2222222222222222222222222222222222222222")
    );
    assert_eq!(
        input_store
            .get("token.decimals")
            .and_then(|entry| entry.value.as_i64()),
        Some(6)
    );
    assert_eq!(
        input_store
            .get("inputs.token.decimals")
            .map(|entry| entry.meta.source.as_str()),
        Some("query")
    );
    assert_eq!(
        input_store
            .get("inputs.token.decimals")
            .and_then(|entry| entry.meta.provenance.as_deref()),
        Some("segment_store.seg_query/q_token.decimals")
    );

    let mut runtime = json!({});
    let projection = input_store.to_runtime_projection();
    runtime
        .as_object_mut()
        .expect("runtime object")
        .insert(
            "inputs".to_string(),
            projection
                .pointer("/inputs")
                .cloned()
                .unwrap_or_else(|| json!({})),
        );
    assert_eq!(runtime.pointer("/inputs/token/decimals"), Some(&json!(6)));
    assert_eq!(runtime.pointer("/inputs/token/symbol"), Some(&json!("USDC")));
    assert_eq!(
        runtime.pointer("/inputs/token/address"),
        Some(&json!("0x2222222222222222222222222222222222222222"))
    );
}

#[test]
fn checkpoint_roundtrip_restores_query_backfilled_inputs() {
    let mut input_store = super::InputStore::default();
    let _ = super::upsert_store_value_with_source(
        &mut input_store,
        "token.decimals",
        json!(6),
        super::input_store::InputValueLayer::Observed,
        "query",
        90,
        "segment_store.seg_query/q_token.decimals",
    );
    let _ = super::upsert_store_value_with_source(
        &mut input_store,
        "token.symbol",
        json!("USDC"),
        super::input_store::InputValueLayer::Observed,
        "query",
        90,
        "segment_store.seg_query/q_token.symbol",
    );

    let extensions = super::checkpoint_ext::AgentCheckpointExtensions::decode(None).encode_updated(
        None,
        &input_store,
        None,
        None,
    );
    let mut restored_runtime = json!({});
    let restored = super::decode_agent_checkpoint_extensions(
        &mut restored_runtime,
        Some(&extensions),
        false,
    );
    let restored_store = restored.input_store().expect("restored input_store");
    assert_eq!(
        restored_store
            .get("inputs.token.decimals")
            .and_then(|entry| entry.value.as_i64()),
        Some(6)
    );
    assert_eq!(
        restored_store
            .get("inputs.token.symbol")
            .and_then(|entry| entry.value.as_str()),
        Some("USDC")
    );
    assert_eq!(
        restored_store
            .get("inputs.token.decimals")
            .map(|entry| entry.meta.source.as_str()),
        Some("query")
    );
    assert_eq!(
        restored_store
            .get("inputs.token.decimals")
            .and_then(|entry| entry.meta.provenance.as_deref()),
        Some("segment_store.seg_query/q_token.decimals")
    );

    let mut projected_runtime = json!({});
    let projection = restored_store.to_runtime_projection();
    projected_runtime
        .as_object_mut()
        .expect("runtime object")
        .insert(
            "inputs".to_string(),
            projection
                .pointer("/inputs")
                .cloned()
                .unwrap_or_else(|| json!({})),
        );
    assert_eq!(
        projected_runtime.pointer("/inputs/token/decimals"),
        Some(&json!(6))
    );
    assert_eq!(
        projected_runtime.pointer("/inputs/token/symbol"),
        Some(&json!("USDC"))
    );
}


#[test]
fn input_normalize_module_keeps_prefix_canonicalization_and_runtime_write_behavior() {
    assert_eq!(
        super::input_normalize::canonical_input_slot_key("inputs.owner"),
        "owner"
    );
    assert_eq!(
        super::input_normalize::canonical_input_slot_key("runtime.inputs.owner"),
        "owner"
    );
    assert_eq!(
        super::input_normalize::normalize_grounding_input_key("runtime.inputs.token.address"),
        "token.address"
    );

    let mut runtime = json!({});
    super::input_normalize::set_runtime_input_value(
        &mut runtime,
        "runtime.inputs.token.decimals",
        json!(18),
    );
    super::input_normalize::set_runtime_input_value(&mut runtime, "inputs.owner", json!("0xabc"));

    assert_eq!(runtime.pointer("/inputs/token/decimals"), Some(&json!(18)));
    assert_eq!(runtime.pointer("/inputs/owner"), Some(&json!("0xabc")));
    assert!(runtime.pointer("/inputs/inputs/owner").is_none());
}


#[test]
fn parse_user_supplied_answer_value_prefers_json_literal() {
    assert_eq!(super::parse_user_supplied_answer_value("18"), json!(18));
    assert_eq!(
        super::parse_user_supplied_answer_value("{\"a\":1}"),
        json!({"a":1})
    );
    assert_eq!(
        super::parse_user_supplied_answer_value("0xabc"),
        json!("0xabc")
    );
}
