use super::*;
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

    let selected = build_query_param_value(Some(&summary), "inputs.recipient", "recipient", "address")
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

    let selected = build_query_param_value(Some(&summary), "inputs.recipient", "recipient", "address");
    assert!(selected.is_none(), "ambiguous address candidates should not auto-bind");
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
