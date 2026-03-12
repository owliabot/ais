use super::*;

#[test]
fn truncate_candidates_limits_total_index_cards() {
    let mut candidates = ExecutableCandidates {
        schema: "ais-executable-candidates/0.0.1".to_string(),
        created_at: None,
        hash: "x".to_string(),
        catalog_schema: "ais-catalog/0.0.1".to_string(),
        catalog_hash: "y".to_string(),
        pack: None,
        chain_scope: None,
        actions: vec![
            json!({"ref":"a1"}),
            json!({"ref":"a2"}),
            json!({"ref":"a3"}),
        ],
        queries: vec![json!({"ref":"q1"}), json!({"ref":"q2"})],
        execution_plugins: vec![],
    };
    truncate_candidates(&mut candidates, 4);
    assert_eq!(candidates.actions.len(), 3);
    assert_eq!(candidates.queries.len(), 1);
}

#[test]
fn build_detail_lookup_applies_pack_merged_requires_queries_and_constraints() {
    let mut resolver = ResolverContext::new();
    resolver.register_protocol(serde_json::from_value(json!({
        "schema":"ais/0.0.2",
        "meta":{"protocol":"demo","version":"0.0.2"},
        "deployments":[{"chain":"eip155:*","contracts":{"router":"0x1"}}],
        "actions":{
            "swap":{
                "description":"base",
                "risk_level":3,
                "params":[],
                "requires_queries":["quote"],
                "execution":{"eip155:*":{"type":"evm_call","to":{"lit":"0x1"},"abi":{"type":"function","name":"swap","inputs":[],"outputs":[]},"args":{}}}
            }
        },
        "queries":{}
    }))
    .expect("protocol"));
    let pack: PackDocument = serde_json::from_value(json!({
        "schema":"ais-pack/0.0.2",
        "name":"safe-defi",
        "version":"0.0.2",
        "includes":[{"protocol":"demo","version":"0.0.2","source":"registry"}],
        "policy":{"constraints":[{"id":"global","effect":"hard_block","assert":"inputs.x"}]},
        "overrides":{
            "action_rules":[{"id":"swap-rule","actions":["demo.swap"],"constraints":[{"id":"rule","effect":"hard_block","assert":"params.y"}]}],
            "actions":{
                "demo.swap":{
                    "requires_queries":["quote","allowance"],
                    "constraints":[{"id":"action","effect":"hard_block","assert":"params.z"}]
                }
            }
        }
    }))
    .expect("pack");
    resolver.register_pack(pack.clone());

    let details = build_detail_lookup_from_catalog(
        vec![json!({
            "ref":"demo@0.0.2/swap",
            "id":"swap",
            "description":"base",
            "risk_level":3,
            "risk_tags":[],
            "params":[],
            "returns":[],
            "requires_queries":["quote"],
            "execution_types":["evm_call"],
            "execution_chains":["eip155:*"]
        })],
        vec![],
        &resolver,
        Some(&pack),
    );

    let detail = details.get("demo@0.0.2/swap").expect("detail");
    assert_eq!(
        detail
            .get("requires_queries")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(2)
    );
    assert_eq!(
        detail
            .pointer("/pack_constraints/matched_action_rule_ids/0")
            .and_then(Value::as_str),
        Some("swap-rule")
    );
    assert_eq!(
        detail
            .pointer("/semantic_hints/deployment/selected_chain")
            .and_then(Value::as_str),
        Some("eip155:*")
    );
    assert_eq!(
        detail
            .pointer("/semantic_hints/deployment/contract_keys/0")
            .and_then(Value::as_str),
        Some("router")
    );
    assert_eq!(
        detail
            .pointer("/semantic_hints/prerequisites/requires_query_count")
            .and_then(Value::as_u64),
        Some(2)
    );
    assert_eq!(
        detail
            .pointer("/semantic_hints/pack/has_action_override")
            .and_then(Value::as_bool),
        Some(true)
    );
    assert!(detail.get("providers").is_none());
}

#[test]
fn build_detail_lookup_exposes_composite_lowering_hints() {
    let mut resolver = ResolverContext::new();
    resolver.register_protocol(serde_json::from_value(json!({
        "schema":"ais/0.0.2",
        "meta":{"protocol":"demo","version":"0.0.2"},
        "deployments":[{"chain":"eip155:1","contracts":{"router":"0x1"}}],
        "actions":{
            "supply":{
                "description":"composite supply",
                "params":[],
                "execution":{
                    "eip155:*":{
                        "type":"composite",
                        "steps":[
                            {"id":"approve","execution":{"type":"evm_call","to":{"lit":"0x1"},"abi":{"type":"function","name":"approve","inputs":[],"outputs":[]},"args":{}}},
                            {"id":"supply","execution":{"type":"evm_call","to":{"lit":"0x1"},"abi":{"type":"function","name":"supply","inputs":[],"outputs":[]},"args":{}}}
                        ]
                    }
                }
            }
        },
        "queries":{}
    }))
    .expect("protocol"));

    let details = build_detail_lookup_from_catalog(
        vec![json!({
            "ref":"demo@0.0.2/supply",
            "id":"supply",
            "description":"composite supply",
            "risk_tags":[],
            "params":[],
            "returns":[],
            "execution_types":["composite"],
            "execution_chains":["eip155:1"]
        })],
        vec![],
        &resolver,
        None,
    );

    let detail = details.get("demo@0.0.2/supply").expect("detail");
    assert_eq!(
        detail
            .pointer("/semantic_hints/execution/selected_type")
            .and_then(Value::as_str),
        Some("composite")
    );
    assert_eq!(
        detail
            .pointer("/semantic_hints/execution/requires_lowering")
            .and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        detail
            .pointer("/semantic_hints/execution/composite_step_ids/0")
            .and_then(Value::as_str),
        Some("approve")
    );
    assert_eq!(
        detail
            .pointer("/semantic_hints/execution/composite_step_count")
            .and_then(Value::as_u64),
        Some(2)
    );
}

#[test]
fn build_detail_lookup_prefers_pack_chain_scope_for_wildcard_execution_resolution() {
    let mut resolver = ResolverContext::new();
    resolver.register_protocol(
        serde_json::from_value(json!({
            "schema":"ais/0.0.2",
            "meta":{"protocol":"aave-demo","version":"0.0.2"},
            "deployments":[
                {"chain":"eip155:1","contracts":{"pool":"0x1"}},
                {"chain":"eip155:8453","contracts":{"pool":"0x8453"}}
            ],
            "actions":{
                "supply":{
                    "description":"supply",
                    "params":[],
                    "execution":{
                        "eip155:*":{
                            "type":"evm_call",
                            "to":{"ref":"contracts.pool"},
                            "abi":{"type":"function","name":"supply","inputs":[],"outputs":[]},
                            "args":{}
                        }
                    }
                }
            },
            "queries":{}
        }))
        .expect("protocol"),
    );
    let pack: PackDocument = serde_json::from_value(json!({
        "schema":"ais-pack/0.0.2",
        "name":"safe-defi",
        "version":"0.0.2",
        "includes":[{"protocol":"aave-demo","version":"0.0.2","source":"registry","chain_scope":["eip155:8453"]}]
    }))
    .expect("pack");

    let details = build_detail_lookup_from_catalog(
        vec![json!({
            "ref":"aave-demo@0.0.2/supply",
            "id":"supply",
            "description":"supply",
            "risk_tags":[],
            "params":[],
            "returns":[],
            "execution_types":["evm_call"],
            "execution_chains":["eip155:*"]
        })],
        vec![],
        &resolver,
        Some(&pack),
    );

    let detail = details.get("aave-demo@0.0.2/supply").expect("detail");
    assert_eq!(
        detail
            .pointer("/semantic_hints/resolution_chain")
            .and_then(Value::as_str),
        Some("eip155:8453")
    );
    assert_eq!(
        detail
            .pointer("/semantic_hints/deployment/contracts/pool")
            .and_then(Value::as_str),
        Some("0x8453")
    );
}

#[test]
fn build_detail_lookup_omits_semantic_hints_without_deterministic_concrete_chain() {
    let mut resolver = ResolverContext::new();
    resolver.register_protocol(
        serde_json::from_value(json!({
            "schema":"ais/0.0.2",
            "meta":{"protocol":"aave-demo","version":"0.0.2"},
            "deployments":[
                {"chain":"eip155:1","contracts":{"pool":"0x1"}},
                {"chain":"eip155:8453","contracts":{"pool":"0x8453"}}
            ],
            "actions":{
                "supply":{
                    "description":"supply",
                    "params":[],
                    "execution":{
                        "eip155:*":{
                            "type":"evm_call",
                            "to":{"ref":"contracts.pool"},
                            "abi":{"type":"function","name":"supply","inputs":[],"outputs":[]},
                            "args":{}
                        }
                    }
                }
            },
            "queries":{}
        }))
        .expect("protocol"),
    );

    let details = build_detail_lookup_from_catalog(
        vec![json!({
            "ref":"aave-demo@0.0.2/supply",
            "id":"supply",
            "description":"supply",
            "risk_tags":[],
            "params":[],
            "returns":[],
            "execution_types":["evm_call"],
            "execution_chains":["eip155:*"]
        })],
        vec![],
        &resolver,
        None,
    );

    let detail = details.get("aave-demo@0.0.2/supply").expect("detail");
    assert!(detail.get("semantic_hints").is_none());
}

#[test]
fn get_details_for_refs_returns_only_known_refs() {
    let mut context = CandidateContext::default();
    context
        .detail_by_ref
        .insert("a@1/swap".to_string(), json!({"ref":"a@1/swap"}));
    let value = context.get_details_for_refs(&["a@1/swap".to_string(), "missing".to_string()]);
    assert_eq!(value.get("count").and_then(Value::as_u64), Some(1));
}

#[test]
fn get_details_for_refs_enforces_budgeted_ref_limit() {
    let mut context = CandidateContext::default();
    for index in 0..20 {
        let reference = format!("p@1/q{index}");
        context
            .detail_by_ref
            .insert(reference.clone(), json!({"ref": reference}));
    }
    let refs = (0..20)
        .map(|index| format!("p@1/q{index}"))
        .collect::<Vec<_>>();
    let value = context.get_details_for_refs(&refs);
    assert_eq!(
        value.get("returned_refs").and_then(Value::as_u64),
        Some(DEFAULT_MAX_DETAIL_REFS as u64)
    );
    assert_eq!(value.get("truncated").and_then(Value::as_bool), Some(true));
}

#[test]
fn truncate_candidates_limits_large_catalog_to_budget_window() {
    let mut candidates = ExecutableCandidates {
        schema: "ais-executable-candidates/0.0.1".to_string(),
        created_at: None,
        hash: "x".to_string(),
        catalog_schema: "ais-catalog/0.0.1".to_string(),
        catalog_hash: "y".to_string(),
        pack: None,
        chain_scope: None,
        actions: (0..200)
            .map(|index| json!({ "ref": format!("a{index}") }))
            .collect(),
        queries: (0..200)
            .map(|index| json!({ "ref": format!("q{index}") }))
            .collect(),
        execution_plugins: vec![],
    };
    truncate_candidates(&mut candidates, DEFAULT_MAX_INDEX_CANDIDATES);
    assert_eq!(candidates.actions.len(), DEFAULT_MAX_INDEX_CANDIDATES);
    assert!(candidates.queries.is_empty());
}

#[test]
fn search_candidates_filters_by_keyword_chain_and_risk() {
    let context = CandidateContext {
        executable_candidates: ExecutableCandidates {
            schema: "ais-executable-candidates/0.0.1".to_string(),
            created_at: None,
            hash: "x".to_string(),
            catalog_schema: "ais-catalog/0.0.1".to_string(),
            catalog_hash: "y".to_string(),
            pack: None,
            chain_scope: None,
            actions: vec![
                json!({
                    "ref":"dex@1/swap",
                    "id":"swap",
                    "description":"swap tokens on dex",
                    "risk_level":3,
                    "execution_chains":["eip155:*"]
                }),
                json!({
                    "ref":"vault@1/deposit",
                    "id":"deposit",
                    "description":"deposit into vault",
                    "risk_level":1,
                    "execution_chains":["eip155:1"]
                }),
            ],
            queries: vec![json!({
                "ref":"wallet@1/native-balance",
                "id":"native-balance",
                "description":"read wallet native balance",
                "execution_chains":["eip155:*"]
            })],
            execution_plugins: vec![],
        },
        ..CandidateContext::default()
    };
    let result = context.search_candidates(&CandidateSearchRequest {
        query: Some("swap".to_string()),
        kind: Some("action".to_string()),
        chain: Some("eip155:31338".to_string()),
        min_risk_level: Some(2),
        max_risk_level: Some(4),
        limit: Some(10),
    });
    assert_eq!(result.get("total_matches").and_then(Value::as_u64), Some(1));
    assert_eq!(
        result.pointer("/results/0/ref").and_then(Value::as_str),
        Some("dex@1/swap")
    );
    assert_eq!(
        result.pointer("/results/0/kind").and_then(Value::as_str),
        Some("action")
    );
    assert_eq!(
        result
            .pointer("/results/0/risk_level")
            .and_then(Value::as_u64),
        Some(3)
    );
    assert_eq!(
        result
            .pointer("/results/0/chains/0")
            .and_then(Value::as_str),
        Some("eip155:*")
    );
    assert!(result.pointer("/results/0/description").is_none());
}

#[test]
fn search_candidates_enforces_limit_and_truncated_flag() {
    let context = CandidateContext {
        executable_candidates: ExecutableCandidates {
            schema: "ais-executable-candidates/0.0.1".to_string(),
            created_at: None,
            hash: "x".to_string(),
            catalog_schema: "ais-catalog/0.0.1".to_string(),
            catalog_hash: "y".to_string(),
            pack: None,
            chain_scope: None,
            actions: (0..30)
                .map(|index| {
                    json!({
                        "ref":format!("demo@1/action-{index}"),
                        "id":format!("action-{index}"),
                        "description":"stress action",
                        "risk_level":2,
                        "execution_chains":["eip155:*"]
                    })
                })
                .collect(),
            queries: vec![],
            execution_plugins: vec![],
        },
        ..CandidateContext::default()
    };
    let result = context.search_candidates(&CandidateSearchRequest {
        kind: Some("action".to_string()),
        limit: Some(7),
        ..CandidateSearchRequest::default()
    });
    assert_eq!(
        result.get("returned_matches").and_then(Value::as_u64),
        Some(7)
    );
    assert_eq!(result.get("truncated").and_then(Value::as_bool), Some(true));
}

#[test]
fn search_candidates_matches_normalized_token_synonyms() {
    let context = CandidateContext {
        executable_candidates: ExecutableCandidates {
            schema: "ais-executable-candidates/0.0.1".to_string(),
            created_at: None,
            hash: "x".to_string(),
            catalog_schema: "ais-catalog/0.0.1".to_string(),
            catalog_hash: "y".to_string(),
            pack: None,
            chain_scope: None,
            actions: vec![json!({
                "ref":"evm-native-utils@0.0.1/native-transfer",
                "id":"native-transfer",
                "description":"Send native ETH",
                "risk_level":3,
                "execution_chains":["eip155:*"]
            })],
            queries: vec![json!({
                "ref":"erc20@0.0.2/balance-of",
                "id":"balance-of",
                "description":"Read ERC-20 balanceOf(owner).",
                "execution_chains":["eip155:*"]
            })],
            execution_plugins: vec![],
        },
        ..CandidateContext::default()
    };
    let token_balance = context.search_candidates(&CandidateSearchRequest {
        kind: Some("query".to_string()),
        query: Some("token balance".to_string()),
        ..CandidateSearchRequest::default()
    });
    assert_eq!(
        token_balance
            .pointer("/results/0/ref")
            .and_then(Value::as_str),
        Some("erc20@0.0.2/balance-of")
    );
    let native_transfer = context.search_candidates(&CandidateSearchRequest {
        kind: Some("action".to_string()),
        query: Some("eth transfer".to_string()),
        ..CandidateSearchRequest::default()
    });
    assert_eq!(
        native_transfer
            .pointer("/results/0/ref")
            .and_then(Value::as_str),
        Some("evm-native-utils@0.0.1/native-transfer")
    );
}

#[test]
fn capability_view_groups_by_protocol_and_collects_required_inputs() {
    let mut context = CandidateContext {
        executable_candidates: ExecutableCandidates {
            schema: "ais-executable-candidates/0.0.1".to_string(),
            created_at: None,
            hash: "x".to_string(),
            catalog_schema: "ais-catalog/0.0.1".to_string(),
            catalog_hash: "y".to_string(),
            pack: None,
            chain_scope: None,
            actions: vec![json!({
                "ref":"erc20@0.0.2/transfer",
                "execution_chains":["eip155:1"]
            })],
            queries: vec![json!({
                "ref":"erc20@0.0.2/balance-of",
                "execution_chains":["eip155:1"]
            })],
            execution_plugins: vec![],
        },
        protocols: vec![serde_json::from_value(json!({
            "schema":"ais/0.0.2",
            "meta":{"protocol":"erc20","version":"0.0.2","tags":["token"]},
            "deployments":[],
            "actions":{
                "transfer":{
                    "description":"Transfer ERC-20 tokens",
                    "risk_level":3,
                    "risk_tags":["transfer"],
                    "execution":{"eip155:*":{"type":"evm_call","to":{"lit":"0x0"}}},
                    "extensions":{"agent":{"topic":"payments.token_transfer"}}
                }
            },
            "queries":{
                "balance-of":{
                    "description":"Read ERC-20 balance",
                    "execution":{"eip155:*":{"type":"evm_read","to":{"lit":"0x0"}}},
                    "extensions":{"agent":{"topics":["balance.token"]}}
                }
            },
            "risks":[],
            "supported_assets":[],
            "capabilities_required":[],
            "tests":[],
            "extensions":{}
        }))
        .expect("protocol fixture should parse")],
        ..CandidateContext::default()
    };
    context.detail_by_ref.insert(
        "erc20@0.0.2/transfer".to_string(),
        json!({
            "params":[
                {"name":"to","required":true},
                {"name":"amount","required":true},
                {"name":"memo","required":false}
            ]
        }),
    );

    let view = context.capability_view();
    assert_eq!(
        view.pointer("/schema").and_then(Value::as_str),
        Some("ais-agent-capability-view/0.0.2")
    );
    assert_eq!(view.pointer("/ready").and_then(Value::as_bool), Some(true));
    assert_eq!(
        view.pointer("/protocols/0/protocol")
            .and_then(Value::as_str),
        Some("erc20@0.0.2")
    );
    assert_eq!(
        view.pointer("/protocols/0/actions/0/ref")
            .and_then(Value::as_str),
        Some("erc20@0.0.2/transfer")
    );
    assert_eq!(
        view.pointer("/protocols/0/required_inputs/0")
            .and_then(Value::as_str),
        Some("amount")
    );
    assert_eq!(
        view.pointer("/protocols/0/actions/0/topic")
            .and_then(Value::as_str),
        Some("payments.token_transfer")
    );
    assert_eq!(
        view.pointer("/protocols/0/topic_cards/0/topic")
            .and_then(Value::as_str),
        Some("balance.token")
    );
    assert_eq!(
        view.pointer("/counts/topics").and_then(Value::as_u64),
        Some(2)
    );
}

#[test]
fn discovery_card_is_ref_first_and_compact() {
    let card = json!({
        "ref":"demo@0.0.1/native-transfer",
        "risk_level":3,
        "execution_chains":["eip155:*"]
    });
    let discovery = to_discovery_card(&card, "action");
    assert_eq!(
        discovery.get("ref").and_then(Value::as_str),
        Some("demo@0.0.1/native-transfer")
    );
    assert_eq!(
        discovery.get("kind").and_then(Value::as_str),
        Some("action")
    );
    assert_eq!(discovery.get("risk_level").and_then(Value::as_u64), Some(3));
    assert_eq!(
        discovery.pointer("/chains/0").and_then(Value::as_str),
        Some("eip155:*")
    );
    assert!(discovery.get("name").is_none());
    assert!(discovery.get("schema_name").is_none());
}
