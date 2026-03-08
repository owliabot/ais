use super::*;

#[test]
fn apply_intent_grounding_writes_inputs_namespace_only() {
    let mut state = EngineRunnerState::default();
    let mut fact_store = InputStore::default();
    let resolved_inputs = BTreeMap::from_iter([("owner".to_string(), json!("0xabc"))]);
    let confidence = BTreeMap::from_iter([("owner".to_string(), 90u8)]);

    let summary = apply_intent_grounding(
        &mut state,
        &mut fact_store,
        &resolved_inputs,
        &BTreeMap::new(),
        &confidence,
        "transfer",
    );

    assert!(summary.applied.iter().any(|item| item == "inputs.owner:90"));
    assert_eq!(
        fact_store
            .get("owner")
            .and_then(|entry| entry.value.as_str()),
        Some("0xabc")
    );
    assert_eq!(
        fact_store
            .get("inputs.owner")
            .and_then(|entry| entry.value.as_str()),
        Some("0xabc")
    );
    assert_eq!(
        state.runtime.pointer("/inputs/owner"),
        Some(&json!("0xabc"))
    );
}

#[test]
fn deterministic_balance_threshold_writes_inputs_namespace_only() {
    let mut state = EngineRunnerState::default();
    let mut fact_store = InputStore::default();

    let _ = apply_intent_grounding(
        &mut state,
        &mut fact_store,
        &BTreeMap::new(),
        &BTreeMap::new(),
        &BTreeMap::new(),
        "native_balance > 100",
    );

    assert_eq!(
        fact_store
            .get("balance_threshold")
            .and_then(|entry| entry.value.as_u64()),
        Some(100)
    );
    assert_eq!(
        fact_store
            .get("inputs.balance_threshold")
            .and_then(|entry| entry.value.as_u64()),
        Some(100)
    );
    assert_eq!(
        state.runtime.pointer("/inputs/balance_threshold"),
        Some(&json!(100))
    );
}

#[test]
fn intent_grounding_ready_for_todos_blocks_on_missing_refs() {
    let state = EngineRunnerState {
        runtime: json!({
            "agent": {
                "intent_grounding": {
                    "ready_for_todos": true,
                    "resolved_inputs": {"owner":"0xabc"},
                    "missing_refs": ["inputs.token.decimals"]
                }
            }
        }),
        ..EngineRunnerState::default()
    };
    assert!(!intent_grounding_ready_for_todos(&state));
}

#[test]
fn intent_grounding_ready_for_todos_accepts_fact_only_fallback() {
    let state = EngineRunnerState {
        runtime: json!({
            "agent": {
                "intent_grounding": {
                    "ready_for_todos": false,
                    "resolved_inputs": {},
                    "intent_facts": {"quote.price":"1.01"},
                    "questions": [],
                    "missing_refs": []
                }
            }
        }),
        ..EngineRunnerState::default()
    };
    assert!(intent_grounding_ready_for_todos(&state));
}

#[test]
fn grounding_retry_after_user_input_ignores_exhausted_autofill_budget() {
    let mut budget = 0u8;
    let action = handle_grounding_retry_outcome(
        GroundingDraftOutcome::Retry {
            state_changed: true,
            host_ready: false,
        },
        &mut budget,
    );
    assert!(matches!(action, GroundingRetryAction::RetrySilently));
    assert_eq!(budget, 0);
}

#[test]
fn grounding_retry_autofill_stops_when_budget_is_exhausted() {
    let mut budget = 0u8;
    let action = handle_grounding_retry_outcome(
        GroundingDraftOutcome::Retry {
            state_changed: false,
            host_ready: false,
        },
        &mut budget,
    );
    assert!(matches!(action, GroundingRetryAction::StopNotReady));
    assert_eq!(budget, 0);
}

#[test]
fn grounding_retry_after_host_recovery_ignores_exhausted_autofill_budget() {
    let mut budget = 0u8;
    let action = handle_grounding_retry_outcome(
        GroundingDraftOutcome::Retry {
            state_changed: true,
            host_ready: false,
        },
        &mut budget,
    );
    assert!(matches!(action, GroundingRetryAction::RetrySilently));
    assert_eq!(budget, 0);
}

#[test]
fn grounding_retry_returns_ready_when_host_is_already_satisfied() {
    let mut budget = 0u8;
    let action = handle_grounding_retry_outcome(
        GroundingDraftOutcome::Retry {
            state_changed: false,
            host_ready: true,
        },
        &mut budget,
    );
    assert!(matches!(action, GroundingRetryAction::ReturnReady));
    assert_eq!(budget, 0);
}

#[test]
fn grounding_follow_up_state_marks_empty_payload_non_actionable() {
    assert!(matches!(
        grounding_follow_up_state(&[], &[]),
        GroundingFollowUpState::NonActionable
    ));
    assert!(matches!(
        grounding_follow_up_state(&[json!({"id":"inputs.owner","question":"owner?"})], &[]),
        GroundingFollowUpState::Actionable
    ));
    assert!(matches!(
        grounding_follow_up_state(&[], &["inputs.owner".to_string()]),
        GroundingFollowUpState::Actionable
    ));
}

#[test]
fn normalize_grounding_candidate_canonicalizes_refs_once() {
    let candidate = super::super::super::grounding_resolution::normalize_grounding_candidate(
        false,
        &[
            "token.decimals".to_string(),
            "inputs.token.decimals".to_string(),
        ],
        &[json!({
            "id": "inputs.token.address",
            "question": "Need token address"
        })],
        &BTreeMap::new(),
        &BTreeMap::new(),
        &BTreeMap::new(),
        &[],
    );

    assert_eq!(candidate.missing_refs, vec!["inputs.token.decimals"]);
    assert_eq!(candidate.question_refs, vec!["inputs.token.address"]);
}

#[test]
fn reconcile_grounding_candidate_filters_host_resolved_missing_refs() {
    let typed_summary = super::super::super::state_summary::StateSummary {
        completed_segments: 0,
        completed_nodes: 0,
        plan_epoch: 0,
        paused_reason: None,
        done: false,
        previous_error: None,
        input_store: Some(json!({
            "facts": {
                "token": {
                    "decimals": 18
                }
            },
            "meta": {}
        })),
        runtime_facts: None,
        input_binding: super::super::super::state_summary::InputBindingContract {
            schema: "ais-input-binding/0.0.1",
            bindable_namespace: "inputs",
            bindable_refs_source: "state_summary.input_store",
            bindable_refs_projection: "state_summary.input_registry.known_refs",
            known_refs_only: true,
            facts_bindable: false,
        },
        input_registry: json!({
            "known_refs": ["inputs.token.decimals"]
        }),
        node_output_refs: json!({
            "known_refs": []
        }),
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

    let candidate = super::super::super::grounding_resolution::normalize_grounding_candidate(
        false,
        &["inputs.token.decimals".to_string()],
        &[],
        &BTreeMap::new(),
        &BTreeMap::new(),
        &BTreeMap::new(),
        &[],
    );
    let resolution = super::super::super::grounding_resolution::reconcile_grounding_candidate(
        Some(&typed_summary),
        &candidate,
    );

    assert!(resolution.effective_missing_refs.is_empty());
    assert!(resolution.host_recovery_satisfied);
    assert!(!resolution.user_input_required);
}

#[test]
fn reconcile_grounding_candidate_is_ready_when_host_has_signal_even_if_planner_says_false() {
    let candidate = super::super::super::grounding_resolution::normalize_grounding_candidate(
        false,
        &[],
        &[],
        &BTreeMap::from([("owner".to_string(), json!("0xabc"))]),
        &BTreeMap::new(),
        &BTreeMap::new(),
        &[],
    );
    let resolution =
        super::super::super::grounding_resolution::reconcile_grounding_candidate(None, &candidate);

    assert!(resolution.ready_for_todos);
    assert!(!resolution.planner_ready_hint);
    assert!(!resolution.user_input_required);
}

#[test]
fn grounding_payload_fact_ref_is_resolved_from_runtime_facts_not_intent_context() {
    let runtime = json!({});
    let typed_summary = super::super::super::state_summary::StateSummary {
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
            },
            "meta": {
                "facts.quote.price": {"source":"query","source_priority":80}
            }
        })),
        input_binding: super::super::super::state_summary::InputBindingContract {
            schema: "ais-input-binding/0.0.1",
            bindable_namespace: "inputs",
            bindable_refs_source: "state_summary.input_store",
            bindable_refs_projection: "state_summary.input_registry.known_refs",
            known_refs_only: true,
            facts_bindable: false,
        },
        input_registry: json!({"known_refs":[]}),
        node_output_refs: json!({"known_refs":[]}),
        reusable_outputs: None,
        tool_memory_projection: None,
        intent_slots: None,
        intent_context: Some(json!({
            "facts": {
                "quote": {"price":"9.99"}
            }
        })),
        capability_view: None,
        capability_ready: None,
        side_effect_lifecycle: None,
        todo_state: None,
        recovery_diagnostics: None,
    };

    assert!(grounding_payload_ref_resolved(
        &runtime,
        Some(&typed_summary),
        "facts.quote.price"
    ));
}

#[test]
fn reconcile_grounding_candidate_builds_fallback_questions_from_missing_refs() {
    let candidate = super::super::super::grounding_resolution::normalize_grounding_candidate(
        true,
        &["inputs.token.decimals".to_string()],
        &[],
        &BTreeMap::new(),
        &BTreeMap::new(),
        &BTreeMap::new(),
        &[],
    );
    let resolution =
        super::super::super::grounding_resolution::reconcile_grounding_candidate(None, &candidate);

    assert_eq!(resolution.effective_questions.len(), 1);
    assert_eq!(
        resolution.effective_questions[0]
            .get("id")
            .and_then(serde_json::Value::as_str),
        Some("inputs.token.decimals")
    );
}

#[test]
fn reconcile_grounding_candidate_filters_stale_explicit_questions_once_host_has_value() {
    let typed_summary = super::super::super::state_summary::StateSummary {
        completed_segments: 0,
        completed_nodes: 0,
        plan_epoch: 0,
        paused_reason: None,
        done: false,
        previous_error: None,
        input_store: Some(json!({
            "facts": {
                "recipient": {
                    "profile": "alice"
                }
            },
            "meta": {
                "recipient": {
                    "profile": {
                        "source": "user",
                        "layer": "seed"
                    }
                }
            }
        })),
        runtime_facts: None,
        input_binding: super::super::super::state_summary::InputBindingContract {
            schema: "ais-input-binding/0.0.1",
            bindable_namespace: "inputs",
            bindable_refs_source: "state_summary.input_store",
            bindable_refs_projection: "state_summary.input_registry.known_refs",
            known_refs_only: true,
            facts_bindable: false,
        },
        input_registry: json!({
            "known_refs": ["inputs.recipient.profile"]
        }),
        node_output_refs: json!({
            "known_refs": []
        }),
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
    let candidate = super::super::super::grounding_resolution::normalize_grounding_candidate(
        false,
        &["inputs.recipient.profile".to_string()],
        &[json!({
            "id":"inputs.recipient.profile",
            "question":"recipient profile?"
        })],
        &BTreeMap::new(),
        &BTreeMap::new(),
        &BTreeMap::new(),
        &[],
    );
    let resolution = super::super::super::grounding_resolution::reconcile_grounding_candidate(
        Some(&typed_summary),
        &candidate,
    );

    assert!(resolution.effective_missing_refs.is_empty());
    assert!(resolution.effective_questions.is_empty());
    assert!(resolution.ready_for_todos);
}

#[test]
fn grounding_payload_missing_ref_resolution_uses_true_input_values_not_only_known_refs() {
    let typed_summary = super::super::super::state_summary::StateSummary {
        completed_segments: 0,
        completed_nodes: 0,
        plan_epoch: 0,
        paused_reason: None,
        done: false,
        previous_error: None,
        input_store: Some(json!({
            "facts": {
                "token": {
                    "decimals": 18
                }
            },
            "meta": {
                "token": {
                    "decimals": {
                        "source": "user",
                        "layer": "seed"
                    }
                }
            }
        })),
        runtime_facts: None,
        input_binding: super::super::super::state_summary::InputBindingContract {
            schema: "ais-input-binding/0.0.1",
            bindable_namespace: "inputs",
            bindable_refs_source: "state_summary.input_store",
            bindable_refs_projection: "state_summary.input_registry.known_refs",
            known_refs_only: true,
            facts_bindable: false,
        },
        input_registry: json!({
            "known_refs": []
        }),
        node_output_refs: json!({
            "known_refs": []
        }),
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

    let unresolved = collect_unresolved_grounding_payload_missing_refs(
        &json!({}),
        Some(&typed_summary),
        &["inputs.token.decimals".to_string()],
    );

    assert!(unresolved.is_empty());
}
