use super::*;
use crate::agent::missing_resolution::policy::MissingResolutionDecision;
use crate::agent::ref_model::RefPath;
use crate::agent::InputStore;
use serde_json::json;

#[test]
fn build_execution_plan_collects_bind_query_and_abort() {
    let decisions = vec![
        MissingResolutionDecision::BindFromRef {
            target: RefPath::Input {
                slot: "token.decimals".to_string(),
            },
            source: RefPath::Fact {
                key: "token.decimals".to_string(),
            },
        },
        MissingResolutionDecision::RunProducer {
            target: RefPath::Input {
                slot: "token.decimals".to_string(),
            },
            query_ref: "erc20@0.0.2/decimals".to_string(),
        },
        MissingResolutionDecision::RunProducer {
            target: RefPath::Input {
                slot: "token.symbol".to_string(),
            },
            query_ref: "erc20@0.0.2/decimals".to_string(),
        },
        MissingResolutionDecision::Abort {
            reason: "cannot recover".to_string(),
        },
    ];
    let plan = build_missing_resolution_execution_plan(decisions.as_slice());
    assert_eq!(plan.bindings.len(), 1);
    assert_eq!(plan.run_producers.len(), 2);
    assert_eq!(
        plan.run_producers[0].target.as_canonical_str(),
        "inputs.token.decimals"
    );
    assert_eq!(
        plan.run_producers[0].query_ref,
        "erc20@0.0.2/decimals".to_string()
    );
    assert_eq!(
        plan.run_producers[1].target.as_canonical_str(),
        "inputs.token.symbol"
    );
    assert_eq!(
        plan.run_producers[1].query_ref,
        "erc20@0.0.2/decimals".to_string()
    );
    assert_eq!(plan.query_refs, vec!["erc20@0.0.2/decimals".to_string()]);
    assert_eq!(plan.abort_reason, Some("cannot recover".to_string()));
}

#[test]
fn apply_bindings_supports_fact_to_input() {
    let mut runtime = json!({});
    let mut input_store = InputStore::default();
    let state_summary = json!({
        "runtime_facts": {
            "facts": {
                "facts.quote.price": "1.01"
            }
        }
    });
    let bindings = vec![MissingResolutionBindAction {
        target: RefPath::Input {
            slot: "price_limit".to_string(),
        },
        source: RefPath::Fact {
            key: "quote.price".to_string(),
        },
    }];
    let execution = apply_missing_resolution_bindings(
        &mut runtime,
        &mut input_store,
        Some(&state_summary),
        bindings.as_slice(),
        "test",
    );
    assert_eq!(
        execution.resolved_targets,
        vec!["inputs.price_limit".to_string()]
    );
    assert!(execution.unresolved_targets.is_empty());
    assert_eq!(runtime.pointer("/inputs/price_limit"), Some(&json!("1.01")));
    assert_eq!(
        input_store
            .get("price_limit")
            .and_then(|entry| entry.value.as_str()),
        Some("1.01")
    );
}

#[test]
fn apply_bindings_supports_input_to_fact() {
    let mut runtime = json!({});
    let mut input_store = InputStore::default();
    let state_summary = json!({
        "input_store": {
            "facts": {
                "owner": "0x1111111111111111111111111111111111111111"
            },
            "meta": {
                "owner": {
                    "source": "user"
                }
            }
        }
    });
    let bindings = vec![MissingResolutionBindAction {
        target: RefPath::Fact {
            key: "wallet.owner".to_string(),
        },
        source: RefPath::Input {
            slot: "owner".to_string(),
        },
    }];
    let execution = apply_missing_resolution_bindings(
        &mut runtime,
        &mut input_store,
        Some(&state_summary),
        bindings.as_slice(),
        "test",
    );
    assert_eq!(
        execution.resolved_targets,
        vec!["facts.wallet.owner".to_string()]
    );
    assert_eq!(
        runtime.pointer("/agent/intent_grounding/intent_facts/wallet/owner"),
        Some(&json!("0x1111111111111111111111111111111111111111"))
    );
    assert!(input_store.get("wallet.owner").is_none());
}

#[test]
fn apply_bindings_prefers_canonical_input_store_for_input_sources() {
    let mut runtime = json!({});
    let mut input_store = InputStore::default();
    let state_summary = json!({
        "runtime_facts": {
            "facts": {
                "inputs.owner": "0x2222222222222222222222222222222222222222"
            }
        },
        "input_store": {
            "facts": {
                "owner": "0x1111111111111111111111111111111111111111"
            },
            "meta": {
                "owner": {
                    "source": "user"
                }
            }
        }
    });
    let bindings = vec![MissingResolutionBindAction {
        target: RefPath::Fact {
            key: "wallet.owner".to_string(),
        },
        source: RefPath::Input {
            slot: "owner".to_string(),
        },
    }];
    let execution = apply_missing_resolution_bindings(
        &mut runtime,
        &mut input_store,
        Some(&state_summary),
        bindings.as_slice(),
        "test",
    );
    assert_eq!(
        runtime.pointer("/agent/intent_grounding/intent_facts/wallet/owner"),
        Some(&json!("0x1111111111111111111111111111111111111111"))
    );
    assert_eq!(
        execution.resolved_targets,
        vec!["facts.wallet.owner".to_string()]
    );
}

#[test]
fn apply_bindings_accepts_query_observed_input_store_sources() {
    let mut runtime = json!({});
    let mut input_store = InputStore::default();
    let state_summary = json!({
        "input_store": {
            "facts": {
                "owner": "0x1111111111111111111111111111111111111111"
            },
            "meta": {
                "owner": {
                    "source": "query.auto_project"
                }
            }
        }
    });
    let bindings = vec![MissingResolutionBindAction {
        target: RefPath::Fact {
            key: "wallet.owner".to_string(),
        },
        source: RefPath::Input {
            slot: "owner".to_string(),
        },
    }];
    let execution = apply_missing_resolution_bindings(
        &mut runtime,
        &mut input_store,
        Some(&state_summary),
        bindings.as_slice(),
        "test",
    );
    assert_eq!(
        execution.resolved_targets,
        vec!["facts.wallet.owner".to_string()]
    );
    assert_eq!(
        runtime.pointer("/agent/intent_grounding/intent_facts/wallet/owner"),
        Some(&json!("0x1111111111111111111111111111111111111111"))
    );
}

#[test]
fn apply_bindings_fact_targets_do_not_backfill_input_store() {
    let mut runtime = json!({});
    let mut input_store = InputStore::default();
    let state_summary = json!({
        "runtime_facts": {
            "facts": {
                "facts.quote.price": "1.01"
            }
        }
    });
    let bindings = vec![MissingResolutionBindAction {
        target: RefPath::Fact {
            key: "quote.accepted_price".to_string(),
        },
        source: RefPath::Fact {
            key: "quote.price".to_string(),
        },
    }];

    let execution = apply_missing_resolution_bindings(
        &mut runtime,
        &mut input_store,
        Some(&state_summary),
        bindings.as_slice(),
        "test",
    );

    assert_eq!(
        execution.resolved_targets,
        vec!["facts.quote.accepted_price".to_string()]
    );
    assert_eq!(
        runtime.pointer("/agent/intent_grounding/intent_facts/quote/accepted_price"),
        Some(&json!("1.01"))
    );
    assert!(input_store.get("quote.accepted_price").is_none());
}

#[test]
fn apply_bindings_fact_sources_do_not_fall_through_to_intent_context() {
    let mut runtime = json!({});
    let mut input_store = InputStore::default();
    let state_summary = json!({
        "intent_context": {
            "facts": {
                "quote": {"price":"1.01"}
            }
        }
    });
    let bindings = vec![MissingResolutionBindAction {
        target: RefPath::Input {
            slot: "price_limit".to_string(),
        },
        source: RefPath::Fact {
            key: "quote.price".to_string(),
        },
    }];

    let execution = apply_missing_resolution_bindings(
        &mut runtime,
        &mut input_store,
        Some(&state_summary),
        bindings.as_slice(),
        "test",
    );

    assert!(execution.resolved_targets.is_empty());
    assert_eq!(
        execution.unresolved_targets,
        vec!["inputs.price_limit".to_string()]
    );
    assert!(runtime.pointer("/inputs/price_limit").is_none());
    assert!(input_store.get("price_limit").is_none());
}
