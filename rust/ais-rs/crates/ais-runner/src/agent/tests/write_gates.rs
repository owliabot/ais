#[test]
fn write_gate_validation_rejects_transfer_without_assert_branch_chain() {
    let segment: ais_sdk::documents::PlanSketchSegment = serde_json::from_value(json!({
        "segment_id": "seg-transfer",
        "cursor_in": "0",
        "cursor_out": "1",
        "done": false,
        "steps": [
            {
                "id": "a_transfer_native_5",
                "kind": "action",
                "candidate_ref": "demo-bank@0.0.1/native-transfer",
                "inputs": {
                    "to": "0xabc",
                    "amount": "5"
                }
            }
        ]
    }))
    .expect("segment");

    let mut candidate_context = super::candidates::CandidateContext::default();
    candidate_context.detail_by_ref.insert(
        "demo-bank@0.0.1/native-transfer".to_string(),
        json!({
            "kind":"action",
            "id":"native-transfer",
            "risk_tags":["transfer"],
            "params":[
                {"name":"to","type":"address"},
                {"name":"amount","type":"uint256"}
            ]
        }),
    );

    let error = super::validate_segment_write_gates(&segment, &candidate_context, None, None)
        .expect_err("transfer without gate chain must fail");
    assert_eq!(
        error.get("reason_code").and_then(Value::as_str),
        Some("write_gate_missing")
    );
    assert_eq!(
        error.pointer("/issues/0/reason_code"),
        Some(&json!("missing_action_gate_dep"))
    );
    assert_eq!(
        error.pointer("/issues/0/family_reason_code"),
        Some(&json!("missing_query_assert_branch_chain"))
    );
}


#[test]
fn write_gate_validation_requires_token_decimals_when_asset_input_lacks_decimals() {
    let segment: ais_sdk::documents::PlanSketchSegment = serde_json::from_value(json!({
        "segment_id": "seg-transfer",
        "cursor_in": "0",
        "cursor_out": "1",
        "done": false,
        "steps": [
            {
                "id": "assert_q_native_balance",
                "kind": "assert",
                "candidate_ref": "demo-bank@0.0.1/native-balance",
                "inputs": {
                    "owner": "0xabc"
                }
            },
            {
                "id": "a_transfer_erc20",
                "kind": "action",
                "candidate_ref": "erc20@0.0.2/transfer",
                "depends_on": ["assert_q_native_balance"],
                "inputs": {
                    "token": "0x8464135c8F25Da09e49BC8782676a84730C318bC",
                    "to": "0xabc",
                    "amount": "1000"
                }
            }
        ]
    }))
    .expect("segment");

    let mut candidate_context = super::candidates::CandidateContext::default();
    candidate_context.detail_by_ref.insert(
        "demo-bank@0.0.1/native-balance".to_string(),
        json!({
            "kind":"query",
            "id":"native-balance",
            "returns":[{"name":"balance","type":"uint256"}]
        }),
    );
    candidate_context.detail_by_ref.insert(
        "erc20@0.0.2/transfer".to_string(),
        json!({
            "kind":"action",
            "id":"transfer",
            "risk_tags":["transfer"],
            "params":[
                {"name":"token","type":"asset"},
                {"name":"to","type":"address"},
                {"name":"amount","type":"uint256"}
            ]
        }),
    );

    let input_store = InputStore::default();
    let error =
        super::validate_segment_write_gates(&segment, &candidate_context, None, Some(&input_store))
            .expect_err("missing decimals must fail");
    assert_eq!(
        error.get("reason_code").and_then(Value::as_str),
        Some("write_gate_missing")
    );
    assert!(error.to_string().contains("missing_token_decimals"));
}


#[test]
fn write_gate_validation_rejects_stale_volatile_facts_without_refresh_query() {
    let segment: ais_sdk::documents::PlanSketchSegment = serde_json::from_value(json!({
        "segment_id": "seg-transfer",
        "cursor_in": "0",
        "cursor_out": "1",
        "done": false,
        "steps": [
            {
                "id": "assert_q_balance_gate",
                "kind": "assert",
                "candidate_ref": "demo-bank@0.0.1/native-balance",
                "inputs": {
                    "owner": "0xabc"
                }
            },
            {
                "id": "a_transfer_erc20",
                "kind": "action",
                "candidate_ref": "erc20@0.0.2/transfer",
                "depends_on": ["assert_q_balance_gate"],
                "inputs": {
                    "token": {
                        "object": {
                            "address": {"lit":"0x8464135c8F25Da09e49BC8782676a84730C318bC"},
                            "chain_id": {"lit":"eip155:31338"},
                            "decimals": 18
                        }
                    },
                    "to": "0xabc",
                    "amount": "1000"
                }
            }
        ]
    }))
    .expect("segment");

    let mut candidate_context = super::candidates::CandidateContext::default();
    candidate_context.detail_by_ref.insert(
        "demo-bank@0.0.1/native-balance".to_string(),
        json!({
            "kind":"query",
            "id":"native-balance",
            "returns":[{"name":"balance","type":"uint256"}]
        }),
    );
    candidate_context.detail_by_ref.insert(
        "erc20@0.0.2/transfer".to_string(),
        json!({
            "kind":"action",
            "id":"transfer",
            "risk_tags":["transfer"],
            "params":[
                {"name":"token","type":"asset"},
                {"name":"to","type":"address"},
                {"name":"amount","type":"uint256","role":"spend_amount"}
            ]
        }),
    );

    let mut input_store = InputStore::default();
    input_store.upsert(
        "wallet.balance.native",
        json!("100"),
        InputValueMeta {
            source: "query".to_string(),
            source_priority: 90,
            provenance: Some("query:native-balance".to_string()),
            confidence: None,
            layer: InputValueLayer::Observed,
            stability: InputValueStability::Volatile,
            observed_at_ms: Some(1),
        },
    );
    let error =
        super::validate_segment_write_gates(&segment, &candidate_context, None, Some(&input_store))
            .expect_err("stale volatile balance without refresh query must fail");
    assert_eq!(
        error.get("reason_code").and_then(Value::as_str),
        Some("write_gate_missing")
    );
    assert!(
        error.to_string().contains("stale_volatile_fact"),
        "error should report stale_volatile_fact"
    );
}


#[test]
fn write_gate_validation_accepts_refresh_query_for_volatile_facts() {
    let segment: ais_sdk::documents::PlanSketchSegment = serde_json::from_value(json!({
        "segment_id": "seg-transfer",
        "cursor_in": "0",
        "cursor_out": "1",
        "done": false,
        "steps": [
            {
                "id": "q_native_balance",
                "kind": "query",
                "candidate_ref": "demo-bank@0.0.1/native-balance",
                "inputs": {
                    "owner": "0xabc"
                }
            },
            {
                "id": "assert_q_balance_gate",
                "kind": "assert",
                "candidate_ref": "demo-bank@0.0.1/native-balance",
                "depends_on": ["q_native_balance"],
                "inputs": {
                    "owner": "0xabc"
                }
            },
            {
                "id": "a_transfer_erc20",
                "kind": "action",
                "candidate_ref": "erc20@0.0.2/transfer",
                "depends_on": ["assert_q_balance_gate"],
                "inputs": {
                    "token": {
                        "object": {
                            "address": {"lit":"0x8464135c8F25Da09e49BC8782676a84730C318bC"},
                            "chain_id": {"lit":"eip155:31338"},
                            "decimals": 18
                        }
                    },
                    "to": "0xabc",
                    "amount": "1000"
                }
            }
        ]
    }))
    .expect("segment");

    let mut candidate_context = super::candidates::CandidateContext::default();
    candidate_context.detail_by_ref.insert(
        "demo-bank@0.0.1/native-balance".to_string(),
        json!({
            "kind":"query",
            "id":"native-balance",
            "returns":[{"name":"balance","type":"uint256"}]
        }),
    );
    candidate_context.detail_by_ref.insert(
        "erc20@0.0.2/transfer".to_string(),
        json!({
            "kind":"action",
            "id":"transfer",
            "risk_tags":["transfer"],
            "params":[
                {"name":"token","type":"asset"},
                {"name":"to","type":"address"},
                {"name":"amount","type":"uint256","role":"spend_amount"}
            ]
        }),
    );

    let input_store = InputStore::default();
    super::validate_segment_write_gates(&segment, &candidate_context, None, Some(&input_store))
        .expect("same segment refresh query should satisfy volatile freshness check");
}


#[test]
fn write_gate_validation_rejects_unexecuted_query_store_backfill_for_token_decimals() {
    let segment: ais_sdk::documents::PlanSketchSegment = serde_json::from_value(json!({
        "segment_id": "seg-transfer",
        "cursor_in": "0",
        "cursor_out": "1",
        "done": false,
        "steps": [
            {
                "id": "q_balance",
                "kind": "query",
                "candidate_ref": "native@0.0.1/balance",
                "inputs": {
                    "owner": "0xabc"
                }
            },
            {
                "id": "q_decimals",
                "kind": "query",
                "candidate_ref": "erc20@0.0.2/decimals",
                "stores": {
                    "decimals": "inputs.token.decimals"
                },
                "inputs": {
                    "token": "0xabc"
                }
            },
            {
                "id": "g_assert",
                "kind": "assert",
                "depends_on": ["q_decimals", "q_balance"],
                "candidate_ref": "erc20@0.0.2/decimals",
                "inputs": {
                    "condition": {"lit":true}
                }
            },
            {
                "id": "g_branch",
                "kind": "branch",
                "depends_on": ["g_assert"],
                "inputs": {
                    "condition": {"lit": true}
                }
            },
            {
                "id": "a_transfer_erc20",
                "kind": "action",
                "candidate_ref": "erc20@0.0.2/transfer",
                "depends_on": ["g_branch"],
                "inputs": {
                    "token": {
                        "object": {
                            "address": {"lit":"0x8464135c8F25Da09e49BC8782676a84730C318bC"},
                            "chain_id": {"lit":"eip155:31338"}
                        }
                    },
                    "to": "0xabc",
                    "amount": "1000"
                }
            }
        ]
    }))
    .expect("segment");

    let mut candidate_context = super::candidates::CandidateContext::default();
    candidate_context.detail_by_ref.insert(
        "native@0.0.1/balance".to_string(),
        json!({
            "kind":"query",
            "id":"balance",
            "returns":[{"name":"balance","type":"uint256"}]
        }),
    );
    candidate_context.detail_by_ref.insert(
        "erc20@0.0.2/decimals".to_string(),
        json!({
            "kind":"query",
            "id":"decimals",
            "returns":[{"name":"value","type":"uint8"}]
        }),
    );
    candidate_context.detail_by_ref.insert(
        "erc20@0.0.2/transfer".to_string(),
        json!({
            "kind":"action",
            "id":"transfer",
            "risk_tags":["transfer"],
            "params":[
                {"name":"token","type":"asset"},
                {"name":"to","type":"address"},
                {"name":"amount","type":"uint256"}
            ]
        }),
    );

    let input_store = InputStore::default();
    let error = super::validate_segment_write_gates(&segment, &candidate_context, None, Some(&input_store))
        .expect_err("query step declaration alone must not satisfy decimals availability");
    assert!(error.to_string().contains("missing_token_decimals"));
}

#[test]
fn write_gate_validation_accepts_when_token_decimals_already_bound_in_input_store() {
    let segment: ais_sdk::documents::PlanSketchSegment = serde_json::from_value(json!({
        "segment_id": "seg-transfer",
        "cursor_in": "0",
        "cursor_out": "1",
        "done": false,
        "steps": [
            {
                "id": "q_balance",
                "kind": "query",
                "candidate_ref": "native@0.0.1/balance",
                "inputs": {
                    "owner": "0xabc"
                }
            },
            {
                "id": "g_assert",
                "kind": "assert",
                "depends_on": ["q_balance"],
                "inputs": {
                    "condition": {"lit":true}
                }
            },
            {
                "id": "a_transfer_erc20",
                "kind": "action",
                "candidate_ref": "erc20@0.0.2/transfer",
                "depends_on": ["g_assert"],
                "inputs": {
                    "token": {
                        "object": {
                            "address": {"lit":"0x8464135c8F25Da09e49BC8782676a84730C318bC"},
                            "chain_id": {"lit":"eip155:31338"}
                        }
                    },
                    "to": "0xabc",
                    "amount": "1000"
                }
            }
        ]
    }))
    .expect("segment");

    let mut candidate_context = super::candidates::CandidateContext::default();
    candidate_context.detail_by_ref.insert(
        "native@0.0.1/balance".to_string(),
        json!({
            "kind":"query",
            "id":"balance",
            "returns":[{"name":"balance","type":"uint256"}]
        }),
    );
    candidate_context.detail_by_ref.insert(
        "erc20@0.0.2/transfer".to_string(),
        json!({
            "kind":"action",
            "id":"transfer",
            "risk_tags":["transfer"],
            "params":[
                {"name":"token","type":"asset"},
                {"name":"to","type":"address"},
                {"name":"amount","type":"uint256"}
            ]
        }),
    );

    let mut input_store = InputStore::default();
    input_store.upsert_user(
        "inputs.token.decimals",
        json!(6),
        "test.prefilled.token.decimals",
    );
    super::validate_segment_write_gates(&segment, &candidate_context, None, Some(&input_store))
        .expect("bound inputs.token.decimals should satisfy strict decimals availability");
}

#[test]
fn write_gate_validation_accepts_asset_object_decimals_ref_from_input_store() {
    let segment: ais_sdk::documents::PlanSketchSegment = serde_json::from_value(json!({
        "segment_id": "seg-transfer",
        "cursor_in": "0",
        "cursor_out": "1",
        "done": false,
        "steps": [
            {
                "id": "q_balance",
                "kind": "query",
                "candidate_ref": "native@0.0.1/balance",
                "inputs": {
                    "owner": "0xabc"
                }
            },
            {
                "id": "g_assert",
                "kind": "assert",
                "depends_on": ["q_balance"],
                "inputs": {
                    "condition": {"lit":true}
                }
            },
            {
                "id": "a_transfer_erc20",
                "kind": "action",
                "candidate_ref": "erc20@0.0.2/transfer",
                "depends_on": ["g_assert"],
                "inputs": {
                    "token": {
                        "object": {
                            "address": {"lit":"0x8464135c8F25Da09e49BC8782676a84730C318bC"},
                            "decimals": {"ref":"inputs.token.decimals"}
                        }
                    },
                    "to": "0xabc",
                    "amount": "1000"
                }
            }
        ]
    }))
    .expect("segment");

    let mut candidate_context = super::candidates::CandidateContext::default();
    candidate_context.detail_by_ref.insert(
        "native@0.0.1/balance".to_string(),
        json!({
            "kind":"query",
            "id":"balance",
            "returns":[{"name":"balance","type":"uint256"}]
        }),
    );
    candidate_context.detail_by_ref.insert(
        "erc20@0.0.2/transfer".to_string(),
        json!({
            "kind":"action",
            "id":"transfer",
            "risk_tags":["transfer"],
            "params":[
                {"name":"token","type":"asset"},
                {"name":"to","type":"address"},
                {"name":"amount","type":"uint256"}
            ]
        }),
    );

    let mut input_store = InputStore::default();
    input_store.upsert_user(
        "inputs.token.decimals",
        json!(6),
        "test.prefilled.token.decimals",
    );
    super::validate_segment_write_gates(&segment, &candidate_context, None, Some(&input_store))
        .expect("decimals ref into bound input should satisfy write-gate availability");
}

#[test]
fn write_gate_validation_accepts_token_ref_when_bound_token_input_contains_decimals() {
    let segment: ais_sdk::documents::PlanSketchSegment = serde_json::from_value(json!({
        "segment_id": "seg-transfer",
        "cursor_in": "0",
        "cursor_out": "1",
        "done": false,
        "steps": [
            {
                "id": "q_balance",
                "kind": "query",
                "candidate_ref": "native@0.0.1/balance",
                "inputs": {
                    "owner": "0xabc"
                }
            },
            {
                "id": "g_assert",
                "kind": "assert",
                "depends_on": ["q_balance"],
                "inputs": {
                    "condition": {"lit":true}
                }
            },
            {
                "id": "a_transfer_erc20",
                "kind": "action",
                "candidate_ref": "erc20@0.0.2/transfer",
                "depends_on": ["g_assert"],
                "inputs": {
                    "token": {"ref":"inputs.token"},
                    "to": "0xabc",
                    "amount": "1000"
                }
            }
        ]
    }))
    .expect("segment");

    let mut candidate_context = super::candidates::CandidateContext::default();
    candidate_context.detail_by_ref.insert(
        "native@0.0.1/balance".to_string(),
        json!({
            "kind":"query",
            "id":"balance",
            "returns":[{"name":"balance","type":"uint256"}]
        }),
    );
    candidate_context.detail_by_ref.insert(
        "erc20@0.0.2/transfer".to_string(),
        json!({
            "kind":"action",
            "id":"transfer",
            "risk_tags":["transfer"],
            "params":[
                {"name":"token","type":"asset"},
                {"name":"to","type":"address"},
                {"name":"amount","type":"uint256"}
            ]
        }),
    );

    let mut input_store = InputStore::default();
    input_store.upsert_user(
        "inputs.token",
        json!({
            "address": {"lit":"0x8464135c8F25Da09e49BC8782676a84730C318bC"},
            "decimals": "6"
        }),
        "test.prefilled.token",
    );
    super::validate_segment_write_gates(&segment, &candidate_context, None, Some(&input_store))
        .expect("token ref should satisfy write-gate availability when bound token carries decimals");
}

#[test]
fn write_gate_validation_rejects_out_of_range_token_decimals_in_input_store() {
    let segment: ais_sdk::documents::PlanSketchSegment = serde_json::from_value(json!({
        "segment_id": "seg-transfer",
        "cursor_in": "0",
        "cursor_out": "1",
        "done": false,
        "steps": [
            {
                "id": "q_balance",
                "kind": "query",
                "candidate_ref": "native@0.0.1/balance",
                "inputs": {
                    "owner": "0xabc"
                }
            },
            {
                "id": "g_assert",
                "kind": "assert",
                "depends_on": ["q_balance"],
                "inputs": {
                    "condition": {"lit":true}
                }
            },
            {
                "id": "a_transfer_erc20",
                "kind": "action",
                "candidate_ref": "erc20@0.0.2/transfer",
                "depends_on": ["g_assert"],
                "inputs": {
                    "token": {
                        "object": {
                            "address": {"lit":"0x8464135c8F25Da09e49BC8782676a84730C318bC"},
                            "chain_id": {"lit":"eip155:31338"}
                        }
                    },
                    "to": "0xabc",
                    "amount": "1000"
                }
            }
        ]
    }))
    .expect("segment");

    let mut candidate_context = super::candidates::CandidateContext::default();
    candidate_context.detail_by_ref.insert(
        "native@0.0.1/balance".to_string(),
        json!({
            "kind":"query",
            "id":"balance",
            "returns":[{"name":"balance","type":"uint256"}]
        }),
    );
    candidate_context.detail_by_ref.insert(
        "erc20@0.0.2/transfer".to_string(),
        json!({
            "kind":"action",
            "id":"transfer",
            "risk_tags":["transfer"],
            "params":[
                {"name":"token","type":"asset"},
                {"name":"to","type":"address"},
                {"name":"amount","type":"uint256"}
            ]
        }),
    );

    let mut input_store = InputStore::default();
    input_store.upsert_user("inputs.token.decimals", json!(255), "test.invalid.decimals");
    let error = super::validate_segment_write_gates(&segment, &candidate_context, None, Some(&input_store))
        .expect_err("out-of-range decimals must not satisfy write-gate availability");
    assert!(error.to_string().contains("missing_token_decimals"));
}

#[test]
fn write_gate_validation_rejects_non_integer_inline_asset_decimals() {
    let segment: ais_sdk::documents::PlanSketchSegment = serde_json::from_value(json!({
        "segment_id": "seg-transfer",
        "cursor_in": "0",
        "cursor_out": "1",
        "done": false,
        "steps": [
            {
                "id": "assert_q_native_balance",
                "kind": "assert",
                "candidate_ref": "demo-bank@0.0.1/native-balance",
                "inputs": {
                    "owner": "0xabc"
                }
            },
            {
                "id": "a_transfer_erc20",
                "kind": "action",
                "candidate_ref": "erc20@0.0.2/transfer",
                "depends_on": ["assert_q_native_balance"],
                "inputs": {
                    "token": {
                        "object": {
                            "address": {"lit":"0x8464135c8F25Da09e49BC8782676a84730C318bC"},
                            "decimals": "18.5"
                        }
                    },
                    "to": "0xabc",
                    "amount": "1000"
                }
            }
        ]
    }))
    .expect("segment");

    let mut candidate_context = super::candidates::CandidateContext::default();
    candidate_context.detail_by_ref.insert(
        "demo-bank@0.0.1/native-balance".to_string(),
        json!({
            "kind":"query",
            "id":"native-balance",
            "returns":[{"name":"balance","type":"uint256"}]
        }),
    );
    candidate_context.detail_by_ref.insert(
        "erc20@0.0.2/transfer".to_string(),
        json!({
            "kind":"action",
            "id":"transfer",
            "risk_tags":["transfer"],
            "params":[
                {"name":"token","type":"asset"},
                {"name":"to","type":"address"},
                {"name":"amount","type":"uint256"}
            ]
        }),
    );

    let input_store = InputStore::default();
    let error =
        super::validate_segment_write_gates(&segment, &candidate_context, None, Some(&input_store))
            .expect_err("non-integer inline decimals should fail typed decimals validation");
    assert!(error.to_string().contains("missing_token_decimals"));
}


#[test]
fn write_gate_validation_ignores_candidate_name_heuristics_without_structured_markers() {
    let segment: ais_sdk::documents::PlanSketchSegment = serde_json::from_value(json!({
        "segment_id": "seg-transfer",
        "cursor_in": "0",
        "cursor_out": "1",
        "done": false,
        "steps": [
            {
                "id": "a_transfer_native_5",
                "kind": "action",
                "candidate_ref": "demo-bank@0.0.1/native-transfer",
                "inputs": {
                    "to": "0xabc",
                    "amount": "5"
                }
            }
        ]
    }))
    .expect("segment");

    let mut candidate_context = super::candidates::CandidateContext::default();
    candidate_context.detail_by_ref.insert(
        "demo-bank@0.0.1/native-transfer".to_string(),
        json!({
            "kind":"action",
            "id":"native-transfer",
            "risk_tags":[],
            "params":[
                {"name":"to","type":"address"},
                {"name":"amount","type":"uint256"}
            ]
        }),
    );

    super::validate_segment_write_gates(&segment, &candidate_context, None, None)
        .expect("name-only heuristic should not trigger write gate");
}


#[test]
fn write_gate_validation_supports_explicit_profile_override() {
    let segment: ais_sdk::documents::PlanSketchSegment = serde_json::from_value(json!({
        "segment_id": "seg-transfer",
        "cursor_in": "0",
        "cursor_out": "1",
        "done": false,
        "steps": [
            {
                "id": "a_write_native_5",
                "kind": "action",
                "candidate_ref": "demo-bank@0.0.1/native-write",
                "inputs": {
                    "to": "0xabc",
                    "amount": "5"
                }
            }
        ]
    }))
    .expect("segment");

    let mut candidate_context = super::candidates::CandidateContext::default();
    candidate_context.detail_by_ref.insert(
        "demo-bank@0.0.1/native-write".to_string(),
        json!({
            "kind":"action",
            "id":"native-write",
            "params":[
                {"name":"to","type":"address"},
                {"name":"amount","type":"uint256"}
            ],
            "write_gate":{"mode":"required"}
        }),
    );

    let error = super::validate_segment_write_gates(&segment, &candidate_context, None, None)
        .expect_err("explicit write_gate required must enforce gate chain");
    assert_eq!(
        error.get("reason_code").and_then(Value::as_str),
        Some("write_gate_missing")
    );
    assert_eq!(
        error.pointer("/issues/0/reason_code"),
        Some(&json!("missing_action_gate_dep"))
    );
    assert_eq!(
        error.pointer("/issues/0/family_reason_code"),
        Some(&json!("missing_query_assert_branch_chain"))
    );
}
use super::input_store::{InputStore, InputValueLayer, InputValueMeta, InputValueStability};
