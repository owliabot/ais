use super::*;
use crate::agent::ref_model::RefPath;
use serde_json::json;

#[test]
fn build_run_producer_decisions_reads_first_query_candidate() {
    let payload = json!({
        "resolved":[
            {
                "missing_ref":"inputs.token.decimals",
                "query_candidates":[
                    {"query_ref":"erc20@0.0.2/decimals"},
                    {"query_ref":"alt@0.0.1/read-decimals"}
                ]
            },
            {
                "missing_ref":"facts.quote.price",
                "query_candidates":[{"query_ref":"dex@1/quote"}]
            }
        ]
    });
    let decisions = build_missing_resolution_run_producer_decisions(&payload);
    assert_eq!(decisions.len(), 2);
    assert_eq!(
        selected_query_refs_from_missing_resolution_decisions(decisions.as_slice()),
        vec![
            "dex@1/quote".to_string(),
            "erc20@0.0.2/decimals".to_string(),
        ]
    );
}

#[test]
fn build_recovery_decisions_prefers_explicit_decisions_payload() {
    let payload = json!({
        "decisions":[
            {
                "kind":"bind_from_ref",
                "target":"token.decimals",
                "source":"facts.token.decimals"
            },
            {
                "kind":"ask_user",
                "target":"inputs.owner",
                "question":"Need owner address"
            }
        ],
        "resolved":[
            {
                "missing_ref":"inputs.token.decimals",
                "query_candidates":[{"query_ref":"erc20@0.0.2/decimals"}]
            }
        ]
    });
    let decisions = build_missing_resolution_decisions(&payload);
    assert_eq!(decisions.len(), 2);
    assert!(matches!(
        &decisions[0],
        MissingResolutionDecision::BindFromRef { target, source }
            if target.as_canonical_str() == "inputs.token.decimals"
            && source.as_canonical_str() == "facts.token.decimals"
    ));
    assert!(matches!(
        &decisions[1],
        MissingResolutionDecision::AskUser { target, question }
            if target.as_canonical_str() == "inputs.owner"
            && question == "Need owner address"
    ));
}

#[test]
fn validate_recovery_decisions_rejects_empty() {
    let validation = validate_missing_resolution_decisions(&[], &[], &RefCatalog::default());
    assert!(!validation.accepted);
    assert!(validation.accepted_decisions.is_empty());
    assert!(validation.rejected_decisions.is_empty());
    assert!(validation
        .issues
        .iter()
        .any(|item| item.code == "empty_decision_set"));
}

#[test]
fn validate_recovery_decisions_rejects_bind_source_not_available() {
    let decisions = vec![MissingResolutionDecision::BindFromRef {
        target: RefPath::Input {
            slot: "token.decimals".to_string(),
        },
        source: RefPath::Input {
            slot: "fallback_decimals".to_string(),
        },
    }];
    let catalog = RefCatalog::build(Some(&json!({
        "input_store": {
            "facts": {"fallback_decimals": 6},
            "meta": {"fallback_decimals":{"source":"query","source_priority":90}}
        }
    })));
    let validation = validate_missing_resolution_decisions(
        decisions.as_slice(),
        &["inputs.token.decimals".to_string()],
        &catalog,
    );
    assert!(validation.accepted);

    let unavailable_catalog = RefCatalog::build(Some(&json!({
        "input_store": {"facts": {}, "meta": {}}
    })));
    let unavailable = validate_missing_resolution_decisions(
        decisions.as_slice(),
        &["inputs.token.decimals".to_string()],
        &unavailable_catalog,
    );
    assert!(!unavailable.accepted);
    assert!(unavailable
        .issues
        .iter()
        .any(|item| item.code == "bind_source_not_in_catalog"));
}

#[test]
fn validate_recovery_decisions_rejects_target_outside_missing_set() {
    let decisions = vec![MissingResolutionDecision::RunProducer {
        target: RefPath::Fact {
            key: "quote.price".to_string(),
        },
        query_ref: "dex@1/quote".to_string(),
    }];
    let validation = validate_missing_resolution_decisions(
        decisions.as_slice(),
        &["inputs.token.decimals".to_string()],
        &RefCatalog::default(),
    );
    assert!(!validation.accepted);
    assert!(validation.accepted_decisions.is_empty());
    assert_eq!(validation.rejected_decisions.len(), 1);
    assert!(validation
        .issues
        .iter()
        .any(|item| item.code == "target_not_missing"));
}

#[test]
fn validate_recovery_decisions_rejects_type_mismatch() {
    let decisions = vec![MissingResolutionDecision::BindFromRef {
        target: RefPath::Input {
            slot: "token.decimals".to_string(),
        },
        source: RefPath::Input {
            slot: "owner".to_string(),
        },
    }];
    let catalog = RefCatalog::build(Some(&json!({
        "input_store": {
            "facts": {
                "owner": "0x1111111111111111111111111111111111111111",
                "token.decimals": 6
            },
            "meta": {
                "owner": {"source":"user","source_priority":100},
                "token.decimals": {"source":"query","source_priority":88}
            }
        }
    })));
    let validation = validate_missing_resolution_decisions(
        decisions.as_slice(),
        &["inputs.token.decimals".to_string()],
        &catalog,
    );
    assert!(!validation.accepted);
    assert!(validation.accepted_decisions.is_empty());
    assert_eq!(validation.rejected_decisions.len(), 1);
    assert!(validation
        .issues
        .iter()
        .any(|item| item.code == "bind_type_incompatible"));
}

#[test]
fn validate_recovery_decisions_rejects_reverse_dependency_cycle() {
    let decisions = vec![
        MissingResolutionDecision::BindFromRef {
            target: RefPath::Input {
                slot: "a".to_string(),
            },
            source: RefPath::Input {
                slot: "b".to_string(),
            },
        },
        MissingResolutionDecision::BindFromRef {
            target: RefPath::Input {
                slot: "b".to_string(),
            },
            source: RefPath::Input {
                slot: "a".to_string(),
            },
        },
    ];
    let catalog = RefCatalog::build(Some(&json!({
        "input_store": {
            "facts": {
                "a": 1,
                "b": 2
            },
            "meta": {
                "a": {"source":"seed","source_priority":80},
                "b": {"source":"seed","source_priority":80}
            }
        }
    })));
    let validation = validate_missing_resolution_decisions(
        decisions.as_slice(),
        &["inputs.a".to_string(), "inputs.b".to_string()],
        &catalog,
    );
    assert!(!validation.accepted);
    assert!(validation.accepted_decisions.is_empty());
    assert_eq!(validation.rejected_decisions.len(), 2);
    assert!(validation
        .issues
        .iter()
        .any(|item| item.code == "bind_reverse_dependency"));
    assert!(validation
        .issues
        .iter()
        .any(|item| item.code == "bind_cycle_detected"));
}

#[test]
fn validate_recovery_decisions_keeps_valid_subset_when_one_decision_invalid() {
    let decisions = vec![
        MissingResolutionDecision::RunProducer {
            target: RefPath::Input {
                slot: "token.decimals".to_string(),
            },
            query_ref: String::new(),
        },
        MissingResolutionDecision::RunProducer {
            target: RefPath::Input {
                slot: "owner".to_string(),
            },
            query_ref: "wallet@0.0.1/defaultOwner".to_string(),
        },
    ];
    let validation = validate_missing_resolution_decisions(
        decisions.as_slice(),
        &[
            "inputs.token.decimals".to_string(),
            "inputs.owner".to_string(),
        ],
        &RefCatalog::default(),
    );
    assert!(!validation.accepted);
    assert_eq!(validation.accepted_decisions.len(), 1);
    assert!(matches!(
        validation.accepted_decisions.first(),
        Some(MissingResolutionDecision::RunProducer { target, query_ref })
            if target.as_canonical_str() == "inputs.owner"
            && query_ref == "wallet@0.0.1/defaultOwner"
    ));
    assert_eq!(validation.rejected_decisions.len(), 1);
    assert!(validation
        .rejected_decisions
        .first()
        .is_some_and(|item| item
            .issues
            .iter()
            .any(|issue| issue.code == "run_producer_query_ref_empty")));
}

#[test]
fn validate_recovery_decisions_rejects_duplicate_target_but_keeps_other_targets() {
    let decisions = vec![
        MissingResolutionDecision::RunProducer {
            target: RefPath::Input {
                slot: "token.decimals".to_string(),
            },
            query_ref: "erc20@0.0.2/decimals".to_string(),
        },
        MissingResolutionDecision::AskUser {
            target: RefPath::Input {
                slot: "token.decimals".to_string(),
            },
            question: "provide decimals".to_string(),
        },
        MissingResolutionDecision::RunProducer {
            target: RefPath::Input {
                slot: "owner".to_string(),
            },
            query_ref: "wallet@0.0.1/defaultOwner".to_string(),
        },
    ];
    let validation = validate_missing_resolution_decisions(
        decisions.as_slice(),
        &[
            "inputs.token.decimals".to_string(),
            "inputs.owner".to_string(),
        ],
        &RefCatalog::default(),
    );
    assert!(!validation.accepted);
    assert_eq!(validation.accepted_decisions.len(), 1);
    assert!(matches!(
        validation.accepted_decisions.first(),
        Some(MissingResolutionDecision::RunProducer { target, query_ref })
            if target.as_canonical_str() == "inputs.owner"
            && query_ref == "wallet@0.0.1/defaultOwner"
    ));
    assert_eq!(validation.rejected_decisions.len(), 2);
    assert!(validation.rejected_decisions.iter().all(|item| {
        item.issues
            .iter()
            .any(|issue| issue.code == "duplicate_target_decision")
    }));
}
