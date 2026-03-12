use super::*;
use ais_llm::{CompleteWithToolsResponse, ScriptedLlmProvider};
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

fn large_catalog_candidate_context(action_count: usize, query_count: usize) -> CandidateContext {
    let mut detail_by_ref = BTreeMap::<String, Value>::new();
    let mut actions = Vec::<Value>::with_capacity(action_count);
    let mut queries = Vec::<Value>::with_capacity(query_count);
    let long_desc = "y".repeat(700);

    for index in 0..action_count {
        let reference = format!("demo@0.0.1/action-{index}");
        let action = json!({
            "ref": reference,
            "id": format!("action-{index}"),
            "description": long_desc.as_str(),
            "params": [{"name":"amount","type":"token_amount","required":true}],
            "execution_types": ["evm_call"],
            "execution_chains": ["eip155:*"]
        });
        detail_by_ref.insert(reference.clone(), action.clone());
        actions.push(action);
    }
    for index in 0..query_count {
        let reference = format!("demo@0.0.1/query-{index}");
        let query = json!({
            "ref": reference,
            "id": format!("query-{index}"),
            "description": long_desc.as_str(),
            "params": [{"name":"owner","type":"address","required":true}],
            "returns": [{"name":"balance","type":"uint256"}],
            "execution_types": ["evm_read"],
            "execution_chains": ["eip155:*"]
        });
        detail_by_ref.insert(reference.clone(), query.clone());
        queries.push(query);
    }

    let index_actions = actions
        .iter()
        .map(|action| {
            json!({
                "kind":"action",
                "schema_name":"demo@0.0.1",
                "name": action.get("id").and_then(Value::as_str).unwrap_or_default(),
                "ref": action.get("ref").and_then(Value::as_str).unwrap_or_default()
            })
        })
        .collect::<Vec<_>>();
    let index_queries = queries
        .iter()
        .map(|query| {
            json!({
                "kind":"query",
                "schema_name":"demo@0.0.1",
                "name": query.get("id").and_then(Value::as_str).unwrap_or_default(),
                "ref": query.get("ref").and_then(Value::as_str).unwrap_or_default()
            })
        })
        .collect::<Vec<_>>();
    let executable_actions = actions.clone();
    let executable_queries = queries.clone();

    CandidateContext {
        index_candidates: json!({
            "schema":"ais-executable-candidates/0.0.1",
            "level":"name_only",
            "hash":"x",
            "catalog_schema":"ais-catalog/0.0.1",
            "catalog_hash":"y",
            "actions": index_actions,
            "queries": index_queries,
            "execution_plugins":[{"type":"evm_call","chain":"eip155:1"}]
        }),
        detail_by_ref,
        executable_candidates: ais_sdk::ExecutableCandidates {
            schema: "ais-executable-candidates/0.0.1".to_string(),
            created_at: None,
            hash: "x".to_string(),
            catalog_schema: "ais-catalog/0.0.1".to_string(),
            catalog_hash: "y".to_string(),
            pack: None,
            chain_scope: None,
            actions: executable_actions,
            queries: executable_queries,
            execution_plugins: vec![],
        },
        protocols: vec![],
        pack: None,
    }
}

fn filtered_list_candidate_context() -> CandidateContext {
    let actions = vec![
        json!({
            "ref":"dex@1/swap",
            "id":"swap",
            "description":"swap on dex",
            "execution_chains":["eip155:1"]
        }),
        json!({
            "ref":"lending@1/deposit",
            "id":"deposit",
            "description":"deposit into lending",
            "execution_chains":["eip155:31338"]
        }),
        json!({
            "ref":"solana-dex@1/swap",
            "id":"swap",
            "description":"swap on solana",
            "execution_chains":["solana:101"]
        }),
    ];
    let queries = vec![
        json!({
            "ref":"dex@1/quote",
            "id":"quote",
            "description":"get dex quote",
            "execution_chains":["eip155:*"]
        }),
        json!({
            "ref":"solana-dex@1/pool",
            "id":"pool",
            "description":"read solana pool",
            "execution_chains":["solana:*"]
        }),
    ];
    let index_actions = vec![
        json!({"kind":"action","schema_name":"dex@1","ref":"dex@1/swap"}),
        json!({"kind":"action","schema_name":"lending@1","ref":"lending@1/deposit"}),
        json!({"kind":"action","schema_name":"solana-dex@1","ref":"solana-dex@1/swap"}),
    ];
    let index_queries = vec![
        json!({"kind":"query","schema_name":"dex@1","ref":"dex@1/quote"}),
        json!({"kind":"query","schema_name":"solana-dex@1","ref":"solana-dex@1/pool"}),
    ];

    let mut detail_by_ref = BTreeMap::<String, Value>::new();
    for reference in [
        "dex@1/swap",
        "lending@1/deposit",
        "solana-dex@1/swap",
        "dex@1/quote",
        "solana-dex@1/pool",
    ] {
        detail_by_ref.insert(
            reference.to_string(),
            json!({
                "ref": reference,
                "params":[{"name":"amount","required":true}]
            }),
        );
    }

    CandidateContext {
        index_candidates: json!({
            "schema":"ais-executable-candidates/0.0.1",
            "level":"name_only",
            "hash":"x",
            "catalog_schema":"ais-catalog/0.0.1",
            "catalog_hash":"y",
            "actions": index_actions,
            "queries": index_queries,
            "execution_plugins":[{"type":"evm_call","chain":"eip155:1"}]
        }),
        detail_by_ref,
        executable_candidates: ais_sdk::ExecutableCandidates {
            schema: "ais-executable-candidates/0.0.1".to_string(),
            created_at: None,
            hash: "x".to_string(),
            catalog_schema: "ais-catalog/0.0.1".to_string(),
            catalog_hash: "y".to_string(),
            pack: None,
            chain_scope: None,
            actions,
            queries,
            execution_plugins: vec![],
        },
        protocols: vec![],
        pack: None,
    }
}

fn snapshot_refs(payload: &Value) -> BTreeSet<String> {
    let mut refs = BTreeSet::<String>::new();
    let Some(protocols) = payload.get("protocols").and_then(Value::as_array) else {
        return refs;
    };
    for protocol in protocols {
        for key in ["actions", "queries"] {
            let Some(items) = protocol.get(key).and_then(Value::as_array) else {
                continue;
            };
            for item in items {
                if let Some(reference) = item.get("ref").and_then(Value::as_str) {
                    refs.insert(reference.to_string());
                }
            }
        }
    }
    refs
}

#[test]
fn segmented_planner_begin_session_decodes_tool_payload() {
    let provider = ScriptedLlmProvider::from_responses(vec![Ok(CompleteWithToolsResponse {
        assistant_content: Some("begin".to_string()),
        tool_calls: vec![ToolCall {
            id: "tool-1".to_string(),
            name: "plan.begin".to_string(),
            arguments: json!({
                "session_id":"sess-1",
                "snapshot_hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "cursor":"cursor-0",
                "limits":{"max_rounds":4,"max_segments":3}
            }),
        }],
    })]);
    let mut planner = LlmSegmentedIntentPlanner::new(provider);
    let session = planner
        .begin_session(SegmentBeginRequest {
            intent: "check and transfer".to_string(),
            snapshot_hash: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                .to_string(),
            pack_snapshot_hash: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                .to_string(),
            catalog_hash: "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
                .to_string(),
            chain_scope: vec!["eip155:1".to_string()],
        })
        .expect("must decode begin session");
    assert_eq!(session.session_id, "sess-1");
    assert_eq!(session.cursor, "cursor-0");
    assert_eq!(session.max_rounds, 4);
    assert_eq!(session.max_segments, 3);
}

#[test]
fn segmented_planner_begin_session_coerces_numeric_cursor() {
    let provider = ScriptedLlmProvider::from_responses(vec![Ok(CompleteWithToolsResponse {
        assistant_content: Some("begin".to_string()),
        tool_calls: vec![ToolCall {
            id: "tool-1".to_string(),
            name: "plan.begin".to_string(),
            arguments: json!({
                "session_id":"sess-1",
                "snapshot_hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "cursor":0,
                "limits":{"max_rounds":4,"max_segments":3}
            }),
        }],
    })]);
    let mut planner = LlmSegmentedIntentPlanner::new(provider);
    let session = planner
        .begin_session(SegmentBeginRequest {
            intent: "check and transfer".to_string(),
            snapshot_hash: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                .to_string(),
            pack_snapshot_hash: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                .to_string(),
            catalog_hash: "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
                .to_string(),
            chain_scope: vec!["eip155:1".to_string()],
        })
        .expect("must decode begin session");
    assert_eq!(session.cursor, "0");
}

#[test]
fn segmented_planner_propose_segment_roundtrip() {
    let provider = ScriptedLlmProvider::from_responses(vec![
        Ok(CompleteWithToolsResponse {
            assistant_content: Some("list".to_string()),
            tool_calls: vec![ToolCall {
                id: "tool-1".to_string(),
                name: "catalog.discover".to_string(),
                arguments: json!({}),
            }],
        }),
        Ok(CompleteWithToolsResponse {
            assistant_content: Some("propose".to_string()),
            tool_calls: vec![ToolCall {
                id: "tool-2".to_string(),
                name: "plan.propose_segment".to_string(),
                arguments: json!({
                    "status":"proposed",
                    "done":false,
                    "cursor_next":"cursor-1",
                    "segment":{
                        "segment_id":"seg-1",
                        "cursor_in":"cursor-0",
                        "cursor_out":"cursor-1",
                        "done":false,
                        "steps":[{
                            "id":"q1",
                            "kind":"query",
                            "candidate_ref":"evm-native-utils@0.0.1/native-balance",
                            "inputs":{"addr":"0x1111111111111111111111111111111111111111"}
                        }]
                    }
                }),
            }],
        }),
    ]);
    let mut planner = LlmSegmentedIntentPlanner::new(provider)
        .with_candidate_context(Some(CandidateContext::default()));
    let draft = planner
        .propose_segment(SegmentPlanningRequest {
            intent: "read balance".to_string(),
            session: SegmentPlanningSession {
                session_id: "sess-1".to_string(),
                snapshot_hash: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    .to_string(),
                cursor: "cursor-0".to_string(),
                max_rounds: 4,
                max_segments: 3,
            },
            state_summary: None,
            typed_summary: None,
            previous_error: None,
            last_segment: None,
        })
        .expect("must decode proposed segment");
    match draft {
        SegmentDraft::Proposed {
            segment,
            cursor_next,
            done,
            ..
        } => {
            assert_eq!(segment.segment_id, "seg-1");
            assert_eq!(cursor_next, "cursor-1");
            assert!(!done);
        }
        _ => panic!("expected proposed draft"),
    }
}

#[test]
fn segmented_planner_propose_segment_accepts_stringified_segment_json_object() {
    let provider = ScriptedLlmProvider::from_responses(vec![Ok(CompleteWithToolsResponse {
        assistant_content: Some("propose".to_string()),
        tool_calls: vec![ToolCall {
            id: "tool-2".to_string(),
            name: "plan.propose_segment".to_string(),
            arguments: json!({
                "status":"proposed",
                "done":false,
                "cursor_next":1,
                "segment": serde_json::to_string(&json!({
                    "segment_id":"seg-1",
                    "cursor_in":"cursor-0",
                    "cursor_out":"cursor-1",
                    "done":false,
                    "steps":[{
                        "id":"q1",
                        "kind":"query",
                        "candidate_ref":"evm-native-utils@0.0.1/native-balance",
                        "inputs":{"addr":"0x1111111111111111111111111111111111111111"}
                    }]
                }))
                .expect("segment json string")
            }),
        }],
    })]);
    let mut planner = LlmSegmentedIntentPlanner::new(provider)
        .with_candidate_context(Some(CandidateContext::default()));
    let draft = planner
        .propose_segment(SegmentPlanningRequest {
            intent: "read balance".to_string(),
            session: SegmentPlanningSession {
                session_id: "sess-1".to_string(),
                snapshot_hash: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    .to_string(),
                cursor: "cursor-0".to_string(),
                max_rounds: 4,
                max_segments: 3,
            },
            state_summary: None,
            typed_summary: None,
            previous_error: None,
            last_segment: None,
        })
        .expect("stringified segment object must be decoded");
    match draft {
        SegmentDraft::Proposed { segment, .. } => {
            assert_eq!(segment.segment_id, "seg-1");
        }
        _ => panic!("expected proposed draft"),
    }
}

#[test]
fn segmented_planner_propose_segment_repairs_missing_status_in_round() {
    let provider = ScriptedLlmProvider::from_responses(vec![
        Ok(CompleteWithToolsResponse {
            assistant_content: Some("missing status".to_string()),
            tool_calls: vec![ToolCall {
                id: "tool-missing-status".to_string(),
                name: "plan.propose_segment".to_string(),
                arguments: json!({
                    "done":false,
                    "segment":{
                        "segment_id":"seg-1",
                        "cursor_in":"cursor-0",
                        "cursor_out":"cursor-1",
                        "done":false,
                        "steps":[]
                    }
                }),
            }],
        }),
        Ok(CompleteWithToolsResponse {
            assistant_content: Some("fixed".to_string()),
            tool_calls: vec![ToolCall {
                id: "tool-fixed".to_string(),
                name: "plan.propose_segment".to_string(),
                arguments: json!({
                    "status":"invalid",
                    "done":false,
                    "error":{"reason_code":"schema_error","message":"fixed after repair"}
                }),
            }],
        }),
    ]);
    let mut planner = LlmSegmentedIntentPlanner::new(provider).with_max_tool_rounds(4);
    let draft = planner
        .propose_segment(SegmentPlanningRequest {
            intent: "read balance".to_string(),
            session: SegmentPlanningSession {
                session_id: "sess-1".to_string(),
                snapshot_hash: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    .to_string(),
                cursor: "cursor-0".to_string(),
                max_rounds: 4,
                max_segments: 3,
            },
            state_summary: None,
            typed_summary: None,
            previous_error: None,
            last_segment: None,
        })
        .expect("missing status should be repaired in-round");
    match draft {
        SegmentDraft::Invalid { reason_code, .. } => {
            assert_eq!(reason_code, "schema_error");
        }
        _ => panic!("expected invalid draft after repair"),
    }
    let usage = planner.llm_usage_value();
    assert_eq!(usage.pointer("/calls"), Some(&json!(2)));
}

#[test]
fn segmented_planner_propose_segment_repairs_invalid_done_type_in_round() {
    let provider = ScriptedLlmProvider::from_responses(vec![
        Ok(CompleteWithToolsResponse {
            assistant_content: Some("invalid done type".to_string()),
            tool_calls: vec![ToolCall {
                id: "tool-invalid-done-type".to_string(),
                name: "plan.propose_segment".to_string(),
                arguments: json!({
                    "status":"invalid",
                    "done":"false",
                    "error":{"reason_code":"schema_error","message":"first attempt"}
                }),
            }],
        }),
        Ok(CompleteWithToolsResponse {
            assistant_content: Some("fixed done type".to_string()),
            tool_calls: vec![ToolCall {
                id: "tool-fixed-done-type".to_string(),
                name: "plan.propose_segment".to_string(),
                arguments: json!({
                    "status":"invalid",
                    "done":false,
                    "error":{"reason_code":"schema_error","message":"fixed after type repair"}
                }),
            }],
        }),
    ]);
    let mut planner = LlmSegmentedIntentPlanner::new(provider).with_max_tool_rounds(4);
    let draft = planner
        .propose_segment(SegmentPlanningRequest {
            intent: "read balance".to_string(),
            session: SegmentPlanningSession {
                session_id: "sess-1".to_string(),
                snapshot_hash: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    .to_string(),
                cursor: "cursor-0".to_string(),
                max_rounds: 4,
                max_segments: 3,
            },
            state_summary: None,
            typed_summary: None,
            previous_error: None,
            last_segment: None,
        })
        .expect("invalid done type should be repaired in-round");
    match draft {
        SegmentDraft::Invalid { reason_code, .. } => {
            assert_eq!(reason_code, "schema_error");
        }
        _ => panic!("expected invalid draft after type repair"),
    }
    let usage = planner.llm_usage_value();
    assert_eq!(usage.pointer("/calls"), Some(&json!(2)));
    assert_eq!(
        usage.pointer("/diagnostics/finalize_schema_repair_attempts_total"),
        Some(&json!(1))
    );
    assert_eq!(
        usage.pointer("/diagnostics/finalize_schema_repair_by_sub_reason/invalid_boolean_type"),
        Some(&json!(1))
    );
}

#[test]
fn segmented_planner_propose_segment_finalize_schema_repair_retry_is_bounded() {
    let provider = ScriptedLlmProvider::from_responses(vec![
        Ok(CompleteWithToolsResponse {
            assistant_content: Some("invalid done type 1".to_string()),
            tool_calls: vec![ToolCall {
                id: "tool-invalid-done-type-1".to_string(),
                name: "plan.propose_segment".to_string(),
                arguments: json!({
                    "status":"invalid",
                    "done":"false",
                    "error":{"reason_code":"schema_error","message":"attempt-1"}
                }),
            }],
        }),
        Ok(CompleteWithToolsResponse {
            assistant_content: Some("invalid done type 2".to_string()),
            tool_calls: vec![ToolCall {
                id: "tool-invalid-done-type-2".to_string(),
                name: "plan.propose_segment".to_string(),
                arguments: json!({
                    "status":"invalid",
                    "done":"false",
                    "error":{"reason_code":"schema_error","message":"attempt-2"}
                }),
            }],
        }),
        Ok(CompleteWithToolsResponse {
            assistant_content: Some("invalid done type 3".to_string()),
            tool_calls: vec![ToolCall {
                id: "tool-invalid-done-type-3".to_string(),
                name: "plan.propose_segment".to_string(),
                arguments: json!({
                    "status":"invalid",
                    "done":"false",
                    "error":{"reason_code":"schema_error","message":"attempt-3"}
                }),
            }],
        }),
    ]);
    let mut planner = LlmSegmentedIntentPlanner::new(provider).with_max_tool_rounds(6);
    let error = planner
        .propose_segment(SegmentPlanningRequest {
            intent: "read balance".to_string(),
            session: SegmentPlanningSession {
                session_id: "sess-1".to_string(),
                snapshot_hash: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    .to_string(),
                cursor: "cursor-0".to_string(),
                max_rounds: 4,
                max_segments: 3,
            },
            state_summary: None,
            typed_summary: None,
            previous_error: None,
            last_segment: None,
        })
        .expect_err("schema repair retries must be bounded");
    assert!(
        error
            .to_string()
            .contains("invalid plan.propose_segment args: invalid type"),
        "unexpected error: {error}"
    );
    let usage = planner.llm_usage_value();
    assert_eq!(usage.pointer("/calls"), Some(&json!(3)));
    assert_eq!(
        usage.pointer("/diagnostics/finalize_schema_repair_attempts_total"),
        Some(&json!(2))
    );
    assert_eq!(
        usage.pointer("/diagnostics/finalize_schema_repair_exhausted_total"),
        Some(&json!(1))
    );
}

#[test]
fn segmented_planner_ground_intent_repairs_non_actionable_not_ready_in_round() {
    let provider = ScriptedLlmProvider::from_responses(vec![
        Ok(CompleteWithToolsResponse {
            assistant_content: Some("not ready without actions".to_string()),
            tool_calls: vec![ToolCall {
                id: "tool-grounding-non-actionable".to_string(),
                name: "plan.ground_intent".to_string(),
                arguments: json!({
                    "status":"proposed",
                    "ready_for_todos":false,
                    "summary":"need something else but no details",
                    "resolved_inputs":{"owner":"0x1111"}
                }),
            }],
        }),
        Ok(CompleteWithToolsResponse {
            assistant_content: Some("fixed not-ready".to_string()),
            tool_calls: vec![ToolCall {
                id: "tool-grounding-fixed".to_string(),
                name: "plan.ground_intent".to_string(),
                arguments: json!({
                    "status":"proposed",
                    "ready_for_todos":false,
                    "summary":"need token decimals",
                    "missing_refs":["inputs.token.decimals"],
                    "questions":[{"id":"inputs.token","question":"Which token should be transferred?"}],
                    "resolved_inputs":{"owner":"0x1111"}
                }),
            }],
        }),
    ]);
    let mut planner = LlmSegmentedIntentPlanner::new(provider).with_max_tool_rounds(4);
    let draft = planner
        .ground_intent(IntentGroundingRequest {
            intent: "transfer token".to_string(),
            session: SegmentPlanningSession {
                session_id: "sess-1".to_string(),
                snapshot_hash: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    .to_string(),
                cursor: "cursor-0".to_string(),
                max_rounds: 4,
                max_segments: 3,
            },
            state_summary: None,
            typed_summary: None,
        })
        .expect("non-actionable not-ready output should be repaired in-round");
    match draft {
        IntentGroundingDraft::Proposed {
            ready_for_todos,
            questions,
            ..
        } => {
            assert!(!ready_for_todos);
            assert_eq!(questions.len(), 1);
        }
        _ => panic!("expected proposed grounding draft"),
    }
    let usage = planner.llm_usage_value();
    assert_eq!(usage.pointer("/calls"), Some(&json!(2)));
    assert_eq!(
        usage.pointer("/diagnostics/finalize_schema_repair_attempts_total"),
        Some(&json!(1))
    );
    assert_eq!(
        usage.pointer(
            "/diagnostics/finalize_schema_repair_by_sub_reason/grounding_not_ready_non_actionable"
        ),
        Some(&json!(1))
    );
}

#[test]
fn segmented_planner_retries_no_toolcall_and_recovers() {
    let provider = ScriptedLlmProvider::from_responses(vec![
        Ok(CompleteWithToolsResponse {
            assistant_content: Some("thinking".to_string()),
            tool_calls: vec![],
        }),
        Ok(CompleteWithToolsResponse {
            assistant_content: Some("fixed".to_string()),
            tool_calls: vec![ToolCall {
                id: "tool-fixed".to_string(),
                name: "plan.propose_todos".to_string(),
                arguments: json!({
                    "status":"proposed",
                    "summary":"ok",
                    "todos":[{"title":"t1"}]
                }),
            }],
        }),
    ]);
    let mut planner = LlmSegmentedIntentPlanner::new(provider).with_max_tool_rounds(4);
    let draft = planner
        .propose_todos(TodoPlanningRequest {
            intent: "plan todos".to_string(),
            session: SegmentPlanningSession {
                session_id: "sess-1".to_string(),
                snapshot_hash: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    .to_string(),
                cursor: "0".to_string(),
                max_rounds: 8,
                max_segments: 8,
            },
            state_summary: Some(json!({})),
            typed_summary: None,
        })
        .expect("no-toolcall should recover via retry");
    assert!(matches!(draft, TodoDraft::Proposed { .. }));
    let usage = planner.llm_usage_value();
    assert_eq!(usage.pointer("/calls"), Some(&json!(2)));
    assert_eq!(
        usage.pointer("/diagnostics/no_toolcall_retries_total"),
        Some(&json!(1))
    );
    assert_eq!(
        usage.pointer("/diagnostics/no_toolcall_retries_exhausted_total"),
        Some(&json!(0))
    );
}

#[test]
fn segmented_planner_no_toolcall_retry_is_bounded() {
    let provider = ScriptedLlmProvider::from_responses(vec![
        Ok(CompleteWithToolsResponse {
            assistant_content: Some("thinking-1".to_string()),
            tool_calls: vec![],
        }),
        Ok(CompleteWithToolsResponse {
            assistant_content: Some("thinking-2".to_string()),
            tool_calls: vec![],
        }),
        Ok(CompleteWithToolsResponse {
            assistant_content: Some("thinking-3".to_string()),
            tool_calls: vec![],
        }),
    ]);
    let mut planner = LlmSegmentedIntentPlanner::new(provider).with_max_tool_rounds(6);
    let error = planner
        .propose_todos(TodoPlanningRequest {
            intent: "plan todos".to_string(),
            session: SegmentPlanningSession {
                session_id: "sess-1".to_string(),
                snapshot_hash: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    .to_string(),
                cursor: "0".to_string(),
                max_rounds: 8,
                max_segments: 8,
            },
            state_summary: Some(json!({})),
            typed_summary: None,
        })
        .expect_err("no-toolcall retries must be bounded");
    assert!(
        error
            .to_string()
            .contains("no_tool_calls_retries_exhausted"),
        "unexpected error: {error}"
    );
    let usage = planner.llm_usage_value();
    assert_eq!(usage.pointer("/calls"), Some(&json!(3)));
    assert_eq!(
        usage.pointer("/diagnostics/no_toolcall_retries_total"),
        Some(&json!(2))
    );
    assert_eq!(
        usage.pointer("/diagnostics/no_toolcall_retries_exhausted_total"),
        Some(&json!(1))
    );
}

#[test]
fn segmented_planner_propose_segment_uses_cursor_out_when_cursor_next_missing() {
    let provider = ScriptedLlmProvider::from_responses(vec![Ok(CompleteWithToolsResponse {
        assistant_content: Some("propose".to_string()),
        tool_calls: vec![ToolCall {
            id: "tool-2".to_string(),
            name: "plan.propose_segment".to_string(),
            arguments: json!({
                "status":"proposed",
                "done":false,
                "segment":{
                    "segment_id":"seg-1",
                    "cursor_in":"cursor-0",
                    "cursor_out":"cursor-2",
                    "done":false,
                    "steps":[{
                        "id":"q1",
                        "kind":"query",
                        "candidate_ref":"evm-native-utils@0.0.1/native-balance",
                        "inputs":{"addr":"0x1111111111111111111111111111111111111111"}
                    }]
                }
            }),
        }],
    })]);
    let mut planner = LlmSegmentedIntentPlanner::new(provider)
        .with_candidate_context(Some(CandidateContext::default()));
    let draft = planner
        .propose_segment(SegmentPlanningRequest {
            intent: "read balance".to_string(),
            session: SegmentPlanningSession {
                session_id: "sess-1".to_string(),
                snapshot_hash: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    .to_string(),
                cursor: "cursor-0".to_string(),
                max_rounds: 4,
                max_segments: 3,
            },
            state_summary: None,
            typed_summary: None,
            previous_error: None,
            last_segment: None,
        })
        .expect("must decode proposed segment");
    match draft {
        SegmentDraft::Proposed {
            cursor_next, done, ..
        } => {
            assert_eq!(cursor_next, "cursor-2");
            assert!(!done);
        }
        _ => panic!("expected proposed draft"),
    }
}

#[test]
fn segmented_planner_blocks_finalize_until_check_segment_ok() {
    let provider = ScriptedLlmProvider::from_responses(vec![
        Ok(CompleteWithToolsResponse {
            assistant_content: Some("begin".to_string()),
            tool_calls: vec![ToolCall {
                id: "tool-begin".to_string(),
                name: "plan.begin".to_string(),
                arguments: json!({
                    "session_id":"sess-1",
                    "snapshot_hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    "cursor":"cursor-0",
                    "limits":{"max_rounds":4,"max_segments":3}
                }),
            }],
        }),
        Ok(CompleteWithToolsResponse {
            assistant_content: Some("finalize without check".to_string()),
            tool_calls: vec![ToolCall {
                id: "tool-finalize-before-check".to_string(),
                name: "plan.propose_segment".to_string(),
                arguments: json!({
                    "status":"proposed",
                    "done":false,
                    "cursor_next":"cursor-1",
                    "segment":{
                        "segment_id":"seg-1",
                        "cursor_in":"cursor-0",
                        "cursor_out":"cursor-1",
                        "done":false,
                        "steps":[]
                    }
                }),
            }],
        }),
        Ok(CompleteWithToolsResponse {
            assistant_content: Some("check".to_string()),
            tool_calls: vec![ToolCall {
                id: "tool-check".to_string(),
                name: "plan.check_segment".to_string(),
                arguments: json!({
                    "segment":{
                        "segment_id":"seg-1",
                        "cursor_in":"cursor-0",
                        "cursor_out":"cursor-1",
                        "done":false,
                        "steps":[]
                    }
                }),
            }],
        }),
        Ok(CompleteWithToolsResponse {
            assistant_content: Some("finalize after check".to_string()),
            tool_calls: vec![ToolCall {
                id: "tool-finalize-after-check".to_string(),
                name: "plan.propose_segment".to_string(),
                arguments: json!({
                    "status":"proposed",
                    "done":false,
                    "cursor_next":"cursor-1",
                    "segment":{
                        "segment_id":"seg-1",
                        "cursor_in":"cursor-0",
                        "cursor_out":"cursor-1",
                        "done":false,
                        "steps":[]
                    }
                }),
            }],
        }),
    ]);
    let mut planner = LlmSegmentedIntentPlanner::new(provider)
        .with_candidate_context(Some(CandidateContext::default()));
    let session = planner
        .begin_session(SegmentBeginRequest {
            intent: "read balance".to_string(),
            snapshot_hash: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                .to_string(),
            pack_snapshot_hash: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                .to_string(),
            catalog_hash: "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
                .to_string(),
            chain_scope: vec!["eip155:1".to_string()],
        })
        .expect("begin session");
    let draft = planner
        .propose_segment(SegmentPlanningRequest {
            intent: "read balance".to_string(),
            session,
            state_summary: None,
            typed_summary: None,
            previous_error: None,
            last_segment: None,
        })
        .expect("must finalize after check_segment ok");
    match draft {
        SegmentDraft::Proposed {
            segment,
            cursor_next,
            done,
            ..
        } => {
            assert_eq!(segment.segment_id, "seg-1");
            assert_eq!(cursor_next, "cursor-1");
            assert!(!done);
        }
        _ => panic!("expected proposed draft"),
    }
}

#[test]
fn segmented_planner_repairs_check_segment_missing_segment_in_round() {
    let provider = ScriptedLlmProvider::from_responses(vec![
        Ok(CompleteWithToolsResponse {
            assistant_content: Some("begin".to_string()),
            tool_calls: vec![ToolCall {
                id: "tool-begin".to_string(),
                name: "plan.begin".to_string(),
                arguments: json!({
                    "session_id":"sess-1",
                    "snapshot_hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    "cursor":"cursor-0",
                    "limits":{"max_rounds":4,"max_segments":3}
                }),
            }],
        }),
        Ok(CompleteWithToolsResponse {
            assistant_content: Some("bad check args".to_string()),
            tool_calls: vec![ToolCall {
                id: "tool-check-bad".to_string(),
                name: "plan.check_segment".to_string(),
                arguments: json!({
                    "raw":"{\"segment\":{\"segment_id\":\"seg-1\",\"cursor_in\":\"cursor-0\",\"cursor_out\":\"cursor-1\",\"done\":false,\"steps\":[]}}"
                }),
            }],
        }),
        Ok(CompleteWithToolsResponse {
            assistant_content: Some("fixed check args".to_string()),
            tool_calls: vec![ToolCall {
                id: "tool-check-fixed".to_string(),
                name: "plan.check_segment".to_string(),
                arguments: json!({
                    "segment":{
                        "segment_id":"seg-1",
                        "cursor_in":"cursor-0",
                        "cursor_out":"cursor-1",
                        "done":false,
                        "steps":[]
                    }
                }),
            }],
        }),
        Ok(CompleteWithToolsResponse {
            assistant_content: Some("finalize after fixed check".to_string()),
            tool_calls: vec![ToolCall {
                id: "tool-finalize".to_string(),
                name: "plan.propose_segment".to_string(),
                arguments: json!({
                    "status":"proposed",
                    "done":false,
                    "cursor_next":"cursor-1",
                    "segment":{
                        "segment_id":"seg-1",
                        "cursor_in":"cursor-0",
                        "cursor_out":"cursor-1",
                        "done":false,
                        "steps":[]
                    }
                }),
            }],
        }),
    ]);
    let mut planner = LlmSegmentedIntentPlanner::new(provider)
        .with_candidate_context(Some(CandidateContext::default()));
    let session = planner
        .begin_session(SegmentBeginRequest {
            intent: "read balance".to_string(),
            snapshot_hash: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                .to_string(),
            pack_snapshot_hash: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                .to_string(),
            catalog_hash: "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
                .to_string(),
            chain_scope: vec!["eip155:1".to_string()],
        })
        .expect("begin session");
    let draft = planner
        .propose_segment(SegmentPlanningRequest {
            intent: "read balance".to_string(),
            session,
            state_summary: None,
            typed_summary: None,
            previous_error: None,
            last_segment: None,
        })
        .expect("check-segment args should be repaired in-round");
    match draft {
        SegmentDraft::Proposed { segment, .. } => {
            assert_eq!(segment.segment_id, "seg-1");
        }
        _ => panic!("expected proposed draft"),
    }
    let usage = planner.llm_usage_value();
    assert_eq!(usage.pointer("/calls"), Some(&json!(4)));
}

#[test]
fn segmented_planner_blocks_finalize_when_segment_differs_from_checked_draft() {
    let provider = ScriptedLlmProvider::from_responses(vec![
        Ok(CompleteWithToolsResponse {
            assistant_content: Some("begin".to_string()),
            tool_calls: vec![ToolCall {
                id: "tool-begin".to_string(),
                name: "plan.begin".to_string(),
                arguments: json!({
                    "session_id":"sess-1",
                    "snapshot_hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    "cursor":"cursor-0",
                    "limits":{"max_rounds":4,"max_segments":3}
                }),
            }],
        }),
        Ok(CompleteWithToolsResponse {
            assistant_content: Some("check a".to_string()),
            tool_calls: vec![ToolCall {
                id: "tool-check-a".to_string(),
                name: "plan.check_segment".to_string(),
                arguments: json!({
                    "segment":{
                        "segment_id":"seg-a",
                        "cursor_in":"cursor-0",
                        "cursor_out":"cursor-1",
                        "done":false,
                        "steps":[]
                    }
                }),
            }],
        }),
        Ok(CompleteWithToolsResponse {
            assistant_content: Some("finalize b without re-check".to_string()),
            tool_calls: vec![ToolCall {
                id: "tool-finalize-b".to_string(),
                name: "plan.propose_segment".to_string(),
                arguments: json!({
                    "status":"proposed",
                    "done":false,
                    "cursor_next":"cursor-1",
                    "segment":{
                        "segment_id":"seg-b",
                        "cursor_in":"cursor-0",
                        "cursor_out":"cursor-1",
                        "done":false,
                        "steps":[]
                    }
                }),
            }],
        }),
        Ok(CompleteWithToolsResponse {
            assistant_content: Some("check b".to_string()),
            tool_calls: vec![ToolCall {
                id: "tool-check-b".to_string(),
                name: "plan.check_segment".to_string(),
                arguments: json!({
                    "segment":{
                        "segment_id":"seg-b",
                        "cursor_in":"cursor-0",
                        "cursor_out":"cursor-1",
                        "done":false,
                        "steps":[]
                    }
                }),
            }],
        }),
        Ok(CompleteWithToolsResponse {
            assistant_content: Some("finalize b after check".to_string()),
            tool_calls: vec![ToolCall {
                id: "tool-finalize-b-ok".to_string(),
                name: "plan.propose_segment".to_string(),
                arguments: json!({
                    "status":"proposed",
                    "done":false,
                    "cursor_next":"cursor-1",
                    "segment":{
                        "segment_id":"seg-b",
                        "cursor_in":"cursor-0",
                        "cursor_out":"cursor-1",
                        "done":false,
                        "steps":[]
                    }
                }),
            }],
        }),
    ]);
    let mut planner = LlmSegmentedIntentPlanner::new(provider)
        .with_candidate_context(Some(CandidateContext::default()));
    let session = planner
        .begin_session(SegmentBeginRequest {
            intent: "read balance".to_string(),
            snapshot_hash: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                .to_string(),
            pack_snapshot_hash: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                .to_string(),
            catalog_hash: "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
                .to_string(),
            chain_scope: vec!["eip155:1".to_string()],
        })
        .expect("begin session");
    let draft = planner
        .propose_segment(SegmentPlanningRequest {
            intent: "read balance".to_string(),
            session,
            state_summary: None,
            typed_summary: None,
            previous_error: None,
            last_segment: None,
        })
        .expect("must finalize only after matching check_segment");
    match draft {
        SegmentDraft::Proposed { segment, .. } => {
            assert_eq!(segment.segment_id, "seg-b");
        }
        _ => panic!("expected proposed draft"),
    }
}

#[test]
fn decode_segmented_tool_call_large_catalog_stays_compact_and_budgeted() {
    let context = large_catalog_candidate_context(260, 260);
    let list_call = ToolCall {
        id: "tool-list".to_string(),
        name: "catalog.discover".to_string(),
        arguments: json!({}),
    };
    let list_result = decode_segmented_tool_call(
        &list_call,
        "plan.propose_segment",
        PlannerRoundPhase::ProposeSegment,
        Some(&context),
    )
    .expect("list call");
    let list_content = match list_result {
        DecodedSegmentedToolCall::ToolMessage { content, .. } => content,
        _ => panic!("list must return tool message"),
    };
    let list_json: Value = serde_json::from_str(list_content.as_str()).expect("valid json");
    assert_eq!(
        list_json
            .pointer("/protocols/0/actions/24")
            .and_then(Value::as_str)
            .map(|value| value.starts_with("[TRUNCATED_ARRAY_ITEMS:")),
        Some(true)
    );
    assert_eq!(
        list_json
            .pointer("/protocols/0/queries/24")
            .and_then(Value::as_str)
            .map(|value| value.starts_with("[TRUNCATED_ARRAY_ITEMS:")),
        Some(true)
    );
    assert_eq!(
        list_json
            .pointer("/protocols/0/actions/0/ref")
            .and_then(Value::as_str),
        Some("demo@0.0.1/action-0")
    );
    assert!(
        list_json
            .pointer("/protocols/0/actions/0/description")
            .is_none(),
        "name-only index cards must not include description"
    );
    let raw_list_content =
        serde_json::to_string(&context.index_candidates).expect("raw index candidates");
    assert!(list_content.len() < raw_list_content.len());

    let search_call = ToolCall {
        id: "tool-search".to_string(),
        name: "catalog.discover".to_string(),
        arguments: json!({
            "query":"query-1",
            "kind":"query",
            "chain":"eip155:1",
            "limit":5
        }),
    };
    let search_result = decode_segmented_tool_call(
        &search_call,
        "plan.propose_segment",
        PlannerRoundPhase::ProposeSegment,
        Some(&context),
    )
    .expect("search call");
    let search_content = match search_result {
        DecodedSegmentedToolCall::ToolMessage { content, .. } => content,
        _ => panic!("search must return tool message"),
    };
    let search_json: Value =
        serde_json::from_str(search_content.as_str()).expect("valid search json");
    assert_eq!(
        search_json.get("returned_matches").and_then(Value::as_u64),
        Some(5)
    );
    assert_eq!(
        search_json.get("truncated").and_then(Value::as_bool),
        Some(true)
    );

    let detail_refs = (0..48)
        .map(|index| format!("demo@0.0.1/query-{index}"))
        .collect::<Vec<_>>();
    let detail_call = ToolCall {
        id: "tool-detail".to_string(),
        name: "get_candidate_detail".to_string(),
        arguments: json!({ "refs": detail_refs }),
    };
    let detail_result = decode_segmented_tool_call(
        &detail_call,
        "plan.propose_segment",
        PlannerRoundPhase::ProposeSegment,
        Some(&context),
    )
    .expect("detail call");
    let detail_content = match detail_result {
        DecodedSegmentedToolCall::ToolMessage { content, .. } => content,
        _ => panic!("detail must return tool message"),
    };
    let detail_json: Value =
        serde_json::from_str(detail_content.as_str()).expect("valid detail json");
    assert_eq!(
        detail_json.get("requested_refs").and_then(Value::as_u64),
        Some(48)
    );
    assert_eq!(
        detail_json.get("returned_refs").and_then(Value::as_u64),
        Some(super::super::candidates::DEFAULT_MAX_DETAIL_REFS as u64)
    );
    assert_eq!(
        detail_json.get("truncated").and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        detail_json.pointer("/details/0/params/0/name"),
        Some(&json!("owner"))
    );
    assert_eq!(
        detail_json.pointer("/details/0/params/0/type"),
        Some(&json!("address"))
    );
    assert_eq!(
        detail_json.pointer("/details/0/params/0/required"),
        Some(&json!(true))
    );
    assert_eq!(
        detail_json.pointer("/details/0/returns/0/name"),
        Some(&json!("balance"))
    );
    assert_eq!(
        detail_json.pointer("/details/0/returns/0/type"),
        Some(&json!("uint256"))
    );
    let raw_detail_content =
        serde_json::to_string(&context.get_details_for_refs(&detail_refs)).expect("raw detail");
    assert!(detail_content.len() < raw_detail_content.len());
}

#[test]
fn catalog_search_control_semantics_query_returns_guide_hint() {
    let context = large_catalog_candidate_context(8, 8);
    let call = ToolCall {
        id: "tool-search-control".to_string(),
        name: "catalog.discover".to_string(),
        arguments: json!({
            "query":"assert",
            "limit":12
        }),
    };
    let result = decode_segmented_tool_call(
        &call,
        "plan.propose_segment",
        PlannerRoundPhase::ProposeSegment,
        Some(&context),
    )
    .expect("search call");
    let content = match result {
        DecodedSegmentedToolCall::ToolMessage { content, .. } => content,
        _ => panic!("search must return tool message"),
    };
    let value: Value = serde_json::from_str(content.as_str()).expect("valid search json");
    assert_eq!(value.pointer("/query"), Some(&json!("assert")));
    assert_eq!(value.pointer("/returned_matches"), Some(&json!(0)));
    assert_eq!(
        value.pointer("/hint/reason_code"),
        Some(&json!("control_semantics_not_catalog_candidate"))
    );
    assert_eq!(value.pointer("/hint/next_tool"), Some(&json!("guide.get")));
    assert_eq!(
        value.pointer("/hint/guide_requests/0/schema"),
        Some(&json!("ais-plan-sketch/0.1.0"))
    );
    assert_eq!(
        value.pointer("/hint/guide_requests/1/topic"),
        Some(&json!("cel"))
    );
}

#[test]
fn resolve_missing_facts_tool_matches_query_returns() {
    let mut context = CandidateContext::default();
    context.executable_candidates.queries.push(json!({
        "ref": "erc20@0.0.2/decimals",
        "kind": "query"
    }));
    context.executable_candidates.queries.push(json!({
        "ref": "erc20@0.0.2/balanceOf",
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
    context.detail_by_ref.insert(
        "erc20@0.0.2/balanceOf".to_string(),
        json!({
            "ref":"erc20@0.0.2/balanceOf",
            "kind":"query",
            "returns":[{"name":"balance","type":"uint256"}]
        }),
    );

    let call = ToolCall {
        id: "tool-resolve-facts".to_string(),
        name: "catalog.resolve_missing_facts".to_string(),
        arguments: json!({
            "missing_refs": ["inputs.token.decimals"],
            "limit_per_ref": 3
        }),
    };
    let result = decode_segmented_tool_call(
        &call,
        "plan.propose_segment",
        PlannerRoundPhase::ProposeSegment,
        Some(&context),
    )
    .expect("resolve missing facts call");
    let content = match result {
        DecodedSegmentedToolCall::ToolMessage { content, .. } => content,
        _ => panic!("resolve_missing_facts must return tool message"),
    };
    let value: Value = serde_json::from_str(content.as_str()).expect("valid json");
    assert_eq!(
        value.pointer("/normalized_missing_refs/0"),
        Some(&json!("inputs.token.decimals"))
    );
    assert_eq!(
        value.pointer("/resolved/0/query_candidates/0/query_ref"),
        Some(&json!("erc20@0.0.2/decimals"))
    );
    assert_eq!(
        value.pointer("/resolved/0/query_candidates/0/matched_return_fields/0"),
        Some(&json!("decimals"))
    );
}

#[test]
fn resolve_missing_facts_for_refs_exposes_host_facing_resolution() {
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

    let payload = resolve_missing_facts_for_refs(&context, &[String::from("token.decimals")], 2);
    assert_eq!(
        payload.pointer("/normalized_missing_refs/0"),
        Some(&json!("inputs.token.decimals"))
    );
    assert_eq!(
        payload.pointer("/resolved/0/query_candidates/0/query_ref"),
        Some(&json!("erc20@0.0.2/decimals"))
    );
}

#[test]
fn resolve_missing_facts_for_refs_preserves_namespace_canonicalization() {
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

    let payload = resolve_missing_facts_for_refs(
        &context,
        &[
            String::from("runtime.inputs.token.decimals"),
            String::from("facts.quote.price"),
            String::from("nodes.q_balance.outputs.balance"),
        ],
        2,
    );
    assert_eq!(
        payload.pointer("/normalized_missing_refs/0"),
        Some(&json!("facts.quote.price"))
    );
    assert_eq!(
        payload.pointer("/normalized_missing_refs/1"),
        Some(&json!("inputs.token.decimals"))
    );
    assert_eq!(
        payload.pointer("/normalized_missing_refs/2"),
        Some(&json!("nodes.q_balance.outputs.balance"))
    );
}

#[test]
fn catalog_discover_cards_include_minimum_ref_metadata() {
    let context = large_catalog_candidate_context(2, 2);
    let list_call = ToolCall {
        id: "tool-list-meta".to_string(),
        name: "catalog.discover".to_string(),
        arguments: json!({}),
    };
    let list_result = decode_segmented_tool_call(
        &list_call,
        "plan.propose_segment",
        PlannerRoundPhase::ProposeSegment,
        Some(&context),
    )
    .expect("list call");
    let list_content = match list_result {
        DecodedSegmentedToolCall::ToolMessage { content, .. } => content,
        _ => panic!("list must return tool message"),
    };
    let list_json: Value = serde_json::from_str(list_content.as_str()).expect("valid json");
    assert_eq!(
        list_json.pointer("/protocols/0/actions/0/ref"),
        Some(&json!("demo@0.0.1/action-0"))
    );
    assert_eq!(
        list_json.pointer("/protocols/0/actions/0/chains/0"),
        Some(&json!("eip155:*"))
    );
    assert_eq!(
        list_json.pointer("/protocols/0/actions/0/required_inputs/0"),
        Some(&json!("amount"))
    );
    assert!(
        list_json.pointer("/protocols/0/actions/0/name").is_none(),
        "compact list cards should not duplicate action name when ref already encodes it"
    );
}

#[test]
fn catalog_discover_filters_by_exact_chain() {
    let context = filtered_list_candidate_context();
    let list_call = ToolCall {
        id: "tool-list-chain-exact".to_string(),
        name: "catalog.discover".to_string(),
        arguments: json!({"chain":"eip155:31338"}),
    };
    let list_result = decode_segmented_tool_call(
        &list_call,
        "plan.propose_segment",
        PlannerRoundPhase::ProposeSegment,
        Some(&context),
    )
    .expect("list call");
    let list_content = match list_result {
        DecodedSegmentedToolCall::ToolMessage { content, .. } => content,
        _ => panic!("list must return tool message"),
    };
    let list_json: Value = serde_json::from_str(list_content.as_str()).expect("valid json");
    assert_eq!(
        list_json.pointer("/filters/chain"),
        Some(&json!("eip155:31338"))
    );
    let refs = snapshot_refs(&list_json);
    assert!(refs.contains("lending@1/deposit"));
    assert!(refs.contains("dex@1/quote"));
    assert!(!refs.contains("solana-dex@1/swap"));
}

#[test]
fn catalog_discover_filters_by_chain_namespace_wildcard() {
    let context = filtered_list_candidate_context();
    let list_call = ToolCall {
        id: "tool-list-chain-wildcard".to_string(),
        name: "catalog.discover".to_string(),
        arguments: json!({"chain":"eip155:*"}),
    };
    let list_result = decode_segmented_tool_call(
        &list_call,
        "plan.propose_segment",
        PlannerRoundPhase::ProposeSegment,
        Some(&context),
    )
    .expect("list call");
    let list_content = match list_result {
        DecodedSegmentedToolCall::ToolMessage { content, .. } => content,
        _ => panic!("list must return tool message"),
    };
    let list_json: Value = serde_json::from_str(list_content.as_str()).expect("valid json");
    let refs = snapshot_refs(&list_json);
    assert!(refs.contains("dex@1/swap"));
    assert!(refs.contains("lending@1/deposit"));
    assert!(refs.contains("dex@1/quote"));
    assert!(!refs.contains("solana-dex@1/swap"));
    assert!(!refs.contains("solana-dex@1/pool"));
}

#[test]
fn catalog_discover_filters_by_protocol_contains_case_insensitive() {
    let context = filtered_list_candidate_context();
    let list_call = ToolCall {
        id: "tool-list-protocol".to_string(),
        name: "catalog.discover".to_string(),
        arguments: json!({"protocol":"LEND"}),
    };
    let list_result = decode_segmented_tool_call(
        &list_call,
        "plan.propose_segment",
        PlannerRoundPhase::ProposeSegment,
        Some(&context),
    )
    .expect("list call");
    let list_content = match list_result {
        DecodedSegmentedToolCall::ToolMessage { content, .. } => content,
        _ => panic!("list must return tool message"),
    };
    let list_json: Value = serde_json::from_str(list_content.as_str()).expect("valid json");
    assert_eq!(list_json.pointer("/filters/protocol"), Some(&json!("lend")));
    assert_eq!(
        list_json.pointer("/protocols/0/protocol"),
        Some(&json!("lending@1"))
    );
    assert!(list_json.pointer("/protocols/1").is_none());
}

#[test]
fn catalog_discover_filters_with_combined_chain_and_protocol() {
    let context = filtered_list_candidate_context();
    let list_call = ToolCall {
        id: "tool-list-combined".to_string(),
        name: "catalog.discover".to_string(),
        arguments: json!({"chain":"eip155:31338","protocol":"DeX"}),
    };
    let list_result = decode_segmented_tool_call(
        &list_call,
        "plan.propose_segment",
        PlannerRoundPhase::ProposeSegment,
        Some(&context),
    )
    .expect("list call");
    let list_content = match list_result {
        DecodedSegmentedToolCall::ToolMessage { content, .. } => content,
        _ => panic!("list must return tool message"),
    };
    let list_json: Value = serde_json::from_str(list_content.as_str()).expect("valid json");
    let refs = snapshot_refs(&list_json);
    assert_eq!(refs, BTreeSet::from(["dex@1/quote".to_string()]));
}

#[test]
fn planning_memory_caches_catalog_discover_per_snapshot_scope() {
    let context = large_catalog_candidate_context(8, 8);
    let call = ToolCall {
        id: "tool-list".to_string(),
        name: "catalog.discover".to_string(),
        arguments: json!({}),
    };
    let mut memory = PlanningMemory::default();
    memory.ensure_scope("session-1", "snapshot-1");

    let first = decode_segmented_tool_call_with_memory(
        &call,
        "plan.propose_segment",
        PlannerRoundPhase::ProposeSegment,
        Some(&context),
        None,
        Some(&mut memory),
        None,
        None,
        None,
    )
    .expect("first list");
    let second = decode_segmented_tool_call_with_memory(
        &call,
        "plan.propose_segment",
        PlannerRoundPhase::ProposeSegment,
        Some(&context),
        None,
        Some(&mut memory),
        None,
        None,
        None,
    )
    .expect("second list");

    let (first_content, first_cached) = match first {
        DecodedSegmentedToolCall::ToolMessage {
            content, cached, ..
        } => (content, cached),
        _ => panic!("must return tool message"),
    };
    let (second_content, second_cached) = match second {
        DecodedSegmentedToolCall::ToolMessage {
            content, cached, ..
        } => (content, cached),
        _ => panic!("must return tool message"),
    };
    assert!(!first_cached);
    assert!(second_cached);
    assert_eq!(first_content, second_content);

    memory.ensure_scope("session-2", "snapshot-1");
    let third_same_snapshot = decode_segmented_tool_call_with_memory(
        &call,
        "plan.propose_segment",
        PlannerRoundPhase::ProposeSegment,
        Some(&context),
        None,
        Some(&mut memory),
        None,
        None,
        None,
    )
    .expect("third list in same snapshot");
    match third_same_snapshot {
        DecodedSegmentedToolCall::ToolMessage { cached, .. } => assert!(cached),
        _ => panic!("must return tool message"),
    }

    memory.ensure_scope("session-3", "snapshot-2");
    let fourth_new_snapshot = decode_segmented_tool_call_with_memory(
        &call,
        "plan.propose_segment",
        PlannerRoundPhase::ProposeSegment,
        Some(&context),
        None,
        Some(&mut memory),
        None,
        None,
        None,
    )
    .expect("fourth list after snapshot reset");
    match fourth_new_snapshot {
        DecodedSegmentedToolCall::ToolMessage { cached, .. } => assert!(!cached),
        _ => panic!("must return tool message"),
    }
}

#[test]
fn planning_memory_normalizes_detail_ref_order_for_cache_key() {
    let context = large_catalog_candidate_context(2, 6);
    let mut memory = PlanningMemory::default();
    memory.ensure_scope("session-1", "snapshot-1");
    let call_first = ToolCall {
        id: "tool-detail-1".to_string(),
        name: "get_candidate_detail".to_string(),
        arguments: json!({
            "refs": ["demo@0.0.1/query-3", "demo@0.0.1/query-1"]
        }),
    };
    let call_second = ToolCall {
        id: "tool-detail-2".to_string(),
        name: "get_candidate_detail".to_string(),
        arguments: json!({
            "refs": ["demo@0.0.1/query-1", "demo@0.0.1/query-3"]
        }),
    };

    let first = decode_segmented_tool_call_with_memory(
        &call_first,
        "plan.propose_segment",
        PlannerRoundPhase::ProposeSegment,
        Some(&context),
        None,
        Some(&mut memory),
        None,
        None,
        None,
    )
    .expect("first detail");
    let second = decode_segmented_tool_call_with_memory(
        &call_second,
        "plan.propose_segment",
        PlannerRoundPhase::ProposeSegment,
        Some(&context),
        None,
        Some(&mut memory),
        None,
        None,
        None,
    )
    .expect("second detail");

    let (first_content, first_cached) = match first {
        DecodedSegmentedToolCall::ToolMessage {
            content, cached, ..
        } => (content, cached),
        _ => panic!("must return tool message"),
    };
    let (second_content, second_cached) = match second {
        DecodedSegmentedToolCall::ToolMessage {
            content, cached, ..
        } => (content, cached),
        _ => panic!("must return tool message"),
    };
    assert!(!first_cached);
    assert!(second_cached);
    assert_eq!(first_content, second_content);
}

#[test]
fn planning_memory_guide_get_full_request_refreshes_digest_cache_entry() {
    let mut memory = PlanningMemory::default();
    memory.ensure_scope("session-1", "snapshot-1");
    let digest_call = ToolCall {
        id: "tool-guide-digest".to_string(),
        name: "guide.get".to_string(),
        arguments: json!({
            "schema": "ais-plan-sketch/0.1.0"
        }),
    };
    let full_call = ToolCall {
        id: "tool-guide-full".to_string(),
        name: "guide.get".to_string(),
        arguments: json!({
            "schema": "ais-plan-sketch/0.1.0",
            "full": true
        }),
    };

    let first = decode_segmented_tool_call_with_memory(
        &digest_call,
        "plan.propose_segment",
        PlannerRoundPhase::ProposeSegment,
        None,
        None,
        Some(&mut memory),
        None,
        None,
        None,
    )
    .expect("digest schema lookup");
    let second = decode_segmented_tool_call_with_memory(
        &full_call,
        "plan.propose_segment",
        PlannerRoundPhase::ProposeSegment,
        None,
        None,
        Some(&mut memory),
        None,
        None,
        None,
    )
    .expect("full schema lookup");
    let third = decode_segmented_tool_call_with_memory(
        &full_call,
        "plan.propose_segment",
        PlannerRoundPhase::ProposeSegment,
        None,
        None,
        Some(&mut memory),
        None,
        None,
        None,
    )
    .expect("cached full schema lookup");

    let (first_content, first_cached) = match first {
        DecodedSegmentedToolCall::ToolMessage {
            content, cached, ..
        } => (content, cached),
        _ => panic!("must return tool message"),
    };
    let (second_content, second_cached) = match second {
        DecodedSegmentedToolCall::ToolMessage {
            content, cached, ..
        } => (content, cached),
        _ => panic!("must return tool message"),
    };
    let (third_content, third_cached) = match third {
        DecodedSegmentedToolCall::ToolMessage {
            content, cached, ..
        } => (content, cached),
        _ => panic!("must return tool message"),
    };

    let first_json = serde_json::from_str::<Value>(first_content.as_str()).expect("json");
    let second_json = serde_json::from_str::<Value>(second_content.as_str()).expect("json");
    let third_json = serde_json::from_str::<Value>(third_content.as_str()).expect("json");

    assert!(!first_cached);
    assert_eq!(first_json.pointer("/schema/mode"), Some(&json!("digest")));
    assert!(first_json.pointer("/schema/json").is_none());

    assert!(!second_cached);
    assert_eq!(second_json.pointer("/schema/mode"), Some(&json!("full")));
    assert!(second_json.pointer("/schema/json").is_some());

    assert!(third_cached);
    assert_eq!(third_json.pointer("/schema/mode"), Some(&json!("full")));
    assert!(third_json.pointer("/schema/json").is_some());
}

#[test]
fn phase_tools_are_scoped_by_round() {
    let begin_tools = segmented_planner_tools_for_phase(PlannerRoundPhase::Begin)
        .into_iter()
        .map(|tool| tool.name)
        .collect::<BTreeSet<_>>();
    assert_eq!(begin_tools, BTreeSet::from_iter(["plan.begin".to_string()]));

    let grounding_tools = segmented_planner_tools_for_phase(PlannerRoundPhase::GroundIntent)
        .into_iter()
        .map(|tool| tool.name)
        .collect::<BTreeSet<_>>();
    assert!(grounding_tools.contains("catalog.discover"));
    assert!(grounding_tools.contains("catalog.resolve_missing_facts"));
    assert!(grounding_tools.contains("get_candidate_detail"));
    assert!(grounding_tools.contains("guide.get"));
    assert!(grounding_tools.contains("plan.abort_intent"));
    assert!(grounding_tools.contains("plan.ground_intent"));
    assert!(!grounding_tools.contains("plan.begin"));
    assert!(!grounding_tools.contains("plan.propose_todos"));
    assert!(!grounding_tools.contains("plan.propose_segment"));
    assert!(!grounding_tools.contains("plan.revise_segment"));

    let todos_tools = segmented_planner_tools_for_phase(PlannerRoundPhase::ProposeTodos)
        .into_iter()
        .map(|tool| tool.name)
        .collect::<BTreeSet<_>>();
    assert!(todos_tools.contains("catalog.discover"));
    assert!(todos_tools.contains("catalog.resolve_missing_facts"));
    assert!(todos_tools.contains("get_candidate_detail"));
    assert!(todos_tools.contains("guide.get"));
    assert!(todos_tools.contains("plan.abort_intent"));
    assert!(todos_tools.contains("plan.propose_todos"));
    assert!(!todos_tools.contains("plan.begin"));
    assert!(!todos_tools.contains("plan.propose_segment"));
    assert!(!todos_tools.contains("plan.revise_segment"));

    let propose_tools = segmented_planner_tools_for_phase(PlannerRoundPhase::ProposeSegment)
        .into_iter()
        .map(|tool| tool.name)
        .collect::<BTreeSet<_>>();
    assert!(propose_tools.contains("catalog.discover"));
    assert!(propose_tools.contains("catalog.resolve_missing_facts"));
    assert!(propose_tools.contains("get_candidate_detail"));
    assert!(propose_tools.contains("guide.get"));
    assert!(propose_tools.contains("plan.check_segment"));
    assert!(propose_tools.contains("plan.abort_intent"));
    assert!(propose_tools.contains("plan.propose_segment"));
    assert!(!propose_tools.contains("plan.propose_todos"));
    assert!(!propose_tools.contains("plan.begin"));
    assert!(!propose_tools.contains("plan.revise_segment"));

    let revise_tools = segmented_planner_tools_for_phase(PlannerRoundPhase::ReviseSegment)
        .into_iter()
        .map(|tool| tool.name)
        .collect::<BTreeSet<_>>();
    assert!(revise_tools.contains("catalog.discover"));
    assert!(revise_tools.contains("catalog.resolve_missing_facts"));
    assert!(revise_tools.contains("get_candidate_detail"));
    assert!(revise_tools.contains("guide.get"));
    assert!(revise_tools.contains("plan.check_segment"));
    assert!(revise_tools.contains("plan.abort_intent"));
    assert!(revise_tools.contains("plan.revise_segment"));
    assert!(!revise_tools.contains("plan.propose_todos"));
    assert!(!revise_tools.contains("plan.begin"));
    assert!(!revise_tools.contains("plan.propose_segment"));
}

#[test]
fn default_prompt_rules_keep_candidate_ref_optional_for_control_steps() {
    let builder = SegmentedPromptContextBuilder::default();
    assert!(builder.base_rules.iter().any(|rule| rule
        .contains("candidate_ref is required for query/action")
        && rule.contains("optional for assert/branch")));
    assert!(!builder
        .phase_rules_propose
        .iter()
        .any(|rule| rule.contains("candidate_ref is required")));
    assert!(!builder
        .phase_rules_revise
        .iter()
        .any(|rule| rule.contains("candidate_ref is required")));
}

#[test]
fn fixture_prompt_rules_align_with_runtime_prompt_sources_of_truth() {
    let builder = SegmentedPromptContextBuilder::default();

    let candidate_ref_rule = builder
        .base_rules
        .iter()
        .find(|rule| rule.contains("candidate_ref is required for query/action"))
        .expect("default base candidate_ref semantics rule");
    assert_anchor_tokens(
        candidate_ref_rule,
        &["candidate_ref", "query/action", "optional", "assert/branch"],
    );

    let base_fixture = include_str!(
        "../../../../../fixtures/runner-local/llm-prompts/prompts/segmented.base_rules.md"
    );
    assert_anchor_tokens(
        base_fixture,
        &["candidate_ref", "query/action", "optional", "assert/branch"],
    );

    let discover_policy_rule = builder
        .base_rules
        .iter()
        .find(|rule| {
            rule.contains("Discovery basis contract")
                && rule.contains("catalog.discover/get_candidate_detail")
        })
        .expect("default base catalog.discover discovery basis rule");
    assert!(discover_policy_rule
        .contains("exact chain+protocol -> exact chain -> chain namespace wildcard"));
    assert!(base_fixture.contains("`catalog.discover` policy template (filter-first):"));
    assert!(
        base_fixture.contains("exact chain+protocol -> exact chain -> chain namespace wildcard")
    );

    let grounding_fixture = include_str!(
        "../../../../../fixtures/runner-local/llm-prompts/prompts/segmented.phase.grounding.md"
    );
    assert_phase_anchor_alignment(
        PlannerRoundPhase::GroundIntent,
        builder.phase_rules_grounding.as_slice(),
        grounding_fixture,
    );

    let todos_fixture = include_str!(
        "../../../../../fixtures/runner-local/llm-prompts/prompts/segmented.phase.todos.md"
    );
    assert_phase_anchor_alignment(
        PlannerRoundPhase::ProposeTodos,
        builder.phase_rules_todos.as_slice(),
        todos_fixture,
    );

    let propose_fixture = include_str!(
        "../../../../../fixtures/runner-local/llm-prompts/prompts/segmented.phase.propose.md"
    );
    assert_phase_anchor_alignment(
        PlannerRoundPhase::ProposeSegment,
        builder.phase_rules_propose.as_slice(),
        propose_fixture,
    );

    let revise_fixture = include_str!(
        "../../../../../fixtures/runner-local/llm-prompts/prompts/segmented.phase.revise.md"
    );
    assert_phase_anchor_alignment(
        PlannerRoundPhase::ReviseSegment,
        builder.phase_rules_revise.as_slice(),
        revise_fixture,
    );

    let contracts_fixture = include_str!(
        "../../../../../fixtures/runner-local/llm-prompts/prompts/segmented.contracts_summary.md"
    );
    let contracts_summary = builder.contracts_summary.join("\n");
    assert_anchor_tokens(
        &contracts_summary,
        &[
            "semantic_hints",
            "token_policy_signal",
            "query",
            "calculated",
            "policy",
            "contracts",
        ],
    );
    assert_anchor_tokens(
        contracts_fixture,
        &[
            "semantic_hints",
            "token_policy_signal",
            "query",
            "calculated",
            "policy",
            "contracts",
        ],
    );
}

fn assert_phase_anchor_alignment(
    phase: PlannerRoundPhase,
    default_phase_rules: &[String],
    fixture_prompt: &str,
) {
    let default_allowlist_line = default_phase_rules
        .iter()
        .find(|rule| rule.starts_with("Allowed tools:"))
        .expect("default phase allowlist rule");
    let fixture_allowlist_line = fixture_prompt
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with("- Allowed tools:"))
        .expect("fixture phase allowlist line");
    let expected_tools = segmented_planner_tools_for_phase(phase)
        .into_iter()
        .map(|tool| tool.name)
        .collect::<BTreeSet<_>>();

    for tool_name in expected_tools {
        assert!(
            default_allowlist_line.contains(tool_name.as_str()),
            "default phase allowlist for `{}` must contain `{tool_name}`",
            phase_name(phase)
        );
        assert!(
            fixture_allowlist_line.contains(format!("`{tool_name}`").as_str())
                || fixture_allowlist_line.contains(tool_name.as_str()),
            "fixture phase allowlist for `{}` must contain `{tool_name}`",
            phase_name(phase)
        );
    }
    // Discovery basis and filter-first rules are now consolidated in base_rules only,
    // not duplicated in each phase.
}

fn assert_anchor_tokens(text: &str, tokens: &[&str]) {
    let normalized = text.to_ascii_lowercase().replace('`', "");
    for token in tokens {
        let normalized_token = token.to_ascii_lowercase();
        assert!(
            normalized.contains(normalized_token.as_str()),
            "missing anchor token `{token}` in `{text}`"
        );
    }
}

#[test]
fn plan_propose_todos_tool_schema_requires_todo_title() {
    let tools = segmented_planner_tools_for_phase(PlannerRoundPhase::ProposeTodos);
    let schema = tools
        .into_iter()
        .find(|tool| tool.name == "plan.propose_todos")
        .map(|tool| tool.input_schema)
        .expect("plan.propose_todos schema");
    assert_eq!(
        schema.pointer("/properties/status/enum/0"),
        Some(&json!("proposed"))
    );
    assert_eq!(
        schema.pointer("/properties/todos/items/$ref"),
        Some(&json!("#/$defs/todo_item"))
    );
    assert_eq!(
        schema.pointer("/$defs/todo_item/required/0"),
        Some(&json!("title"))
    );
}

#[test]
fn propose_todo_draft_roundtrip_decodes_todos() {
    let call = ToolCall {
        id: "tool-todos-final".to_string(),
        name: "plan.propose_todos".to_string(),
        arguments: json!({
            "status": "proposed",
            "summary": "split into 2 todos",
            "todos": [
                {
                    "title": "Query token decimals",
                    "required_facts": ["token.address"],
                    "produced_facts": ["token.decimals"]
                },
                {
                    "title": "Execute transfer",
                    "required_facts": ["token.decimals", "amount.human"],
                    "produced_facts": ["tx_hash"]
                }
            ]
        }),
    };
    let result = decode_segmented_tool_call(
        &call,
        "plan.propose_todos",
        PlannerRoundPhase::ProposeTodos,
        None,
    )
    .expect("todo finalize call");
    match result {
        DecodedSegmentedToolCall::Final(PlannerToolOutput::TodoDraft(TodoDraft::Proposed {
            summary,
            todos,
            ..
        })) => {
            assert_eq!(summary.as_deref(), Some("split into 2 todos"));
            assert_eq!(todos.len(), 2);
            assert_eq!(todos[0].title, "Query token decimals");
            assert_eq!(todos[1].produced_facts, vec!["tx_hash".to_string()]);
        }
        _ => panic!("expected proposed todo draft"),
    }
}

#[test]
fn ground_intent_tool_schema_requires_ready_for_todos() {
    let tools = segmented_planner_tools_for_phase(PlannerRoundPhase::GroundIntent);
    let schema = tools
        .into_iter()
        .find(|tool| tool.name == "plan.ground_intent")
        .map(|tool| tool.input_schema)
        .expect("plan.ground_intent schema");
    assert_eq!(
        schema.pointer("/properties/status/enum/0"),
        Some(&json!("proposed"))
    );
    assert_eq!(
        schema.pointer("/allOf/0/then/required/0"),
        Some(&json!("ready_for_todos"))
    );
    assert_eq!(
        schema.pointer("/properties/missing_refs/minItems"),
        Some(&json!(1))
    );
    assert_eq!(
        schema.pointer("/allOf/1/then/anyOf/0/required/0"),
        Some(&json!("questions"))
    );
    assert_eq!(
        schema.pointer("/allOf/1/then/anyOf/1/required/0"),
        Some(&json!("missing_refs"))
    );
}

#[test]
fn grounding_draft_roundtrip_decodes_proposed_payload() {
    let call = ToolCall {
        id: "tool-grounding-final".to_string(),
        name: "plan.ground_intent".to_string(),
        arguments: json!({
            "status": "proposed",
            "summary": "extracted transfer fields",
            "ready_for_todos": true,
            "resolved_inputs": {
                "owner": "0x1111",
                "recipient": "0x2222",
                "amount": "1.25"
            },
            "intent_facts": {
                "intent.action": "transfer"
            },
            "confidence": {
                "owner": 95,
                "recipient": 93
            }
        }),
    };
    let result = decode_segmented_tool_call(
        &call,
        "plan.ground_intent",
        PlannerRoundPhase::GroundIntent,
        None,
    )
    .expect("grounding finalize call");
    match result {
        DecodedSegmentedToolCall::Final(PlannerToolOutput::IntentGrounding(
            IntentGroundingDraft::Proposed {
                ready_for_todos,
                resolved_inputs,
                intent_facts,
                ..
            },
        )) => {
            assert!(ready_for_todos);
            assert_eq!(
                resolved_inputs.get("recipient").and_then(Value::as_str),
                Some("0x2222")
            );
            assert_eq!(
                intent_facts.get("intent.action").and_then(Value::as_str),
                Some("transfer")
            );
        }
        _ => panic!("expected proposed grounding draft"),
    }
}

#[test]
fn grounding_draft_infers_ready_when_flag_missing_and_no_questions() {
    let call = ToolCall {
        id: "tool-grounding-final".to_string(),
        name: "plan.ground_intent".to_string(),
        arguments: json!({
            "status": "proposed",
            "summary": "extracted transfer fields",
            "resolved_inputs": {
                "owner": "0x1111",
                "recipient": "0x2222"
            },
            "intent_facts": {},
            "confidence": {
                "owner": 95,
                "recipient": 93
            }
        }),
    };
    let result = decode_segmented_tool_call(
        &call,
        "plan.ground_intent",
        PlannerRoundPhase::GroundIntent,
        None,
    )
    .expect("grounding finalize call");
    match result {
        DecodedSegmentedToolCall::Final(PlannerToolOutput::IntentGrounding(
            IntentGroundingDraft::Proposed {
                ready_for_todos, ..
            },
        )) => {
            assert!(ready_for_todos);
        }
        _ => panic!("expected proposed grounding draft"),
    }
}

#[test]
fn grounding_draft_infers_ready_from_intent_facts_only() {
    let call = ToolCall {
        id: "tool-grounding-final".to_string(),
        name: "plan.ground_intent".to_string(),
        arguments: json!({
            "status": "proposed",
            "summary": "facts are sufficient",
            "resolved_inputs": {},
            "intent_facts": {
                "quote.price": "1.01"
            }
        }),
    };
    let result = decode_segmented_tool_call(
        &call,
        "plan.ground_intent",
        PlannerRoundPhase::GroundIntent,
        None,
    )
    .expect("grounding finalize call");
    match result {
        DecodedSegmentedToolCall::Final(PlannerToolOutput::IntentGrounding(
            IntentGroundingDraft::Proposed {
                ready_for_todos, ..
            },
        )) => {
            assert!(ready_for_todos);
        }
        _ => panic!("expected proposed grounding draft"),
    }
}

#[test]
fn grounding_draft_keeps_not_ready_when_flag_missing_and_questions_exist() {
    let call = ToolCall {
        id: "tool-grounding-final".to_string(),
        name: "plan.ground_intent".to_string(),
        arguments: json!({
            "status": "proposed",
            "summary": "need more inputs",
            "resolved_inputs": {
                "owner": "0x1111"
            },
            "questions": [
                {"id":"recipient","question":"recipient?"}
            ],
            "confidence": {
                "owner": 95
            }
        }),
    };
    let result = decode_segmented_tool_call(
        &call,
        "plan.ground_intent",
        PlannerRoundPhase::GroundIntent,
        None,
    )
    .expect("grounding finalize call");
    match result {
        DecodedSegmentedToolCall::Final(PlannerToolOutput::IntentGrounding(
            IntentGroundingDraft::Proposed {
                ready_for_todos,
                missing_refs,
                ..
            },
        )) => {
            assert!(!ready_for_todos);
            assert!(missing_refs.is_empty());
        }
        _ => panic!("expected proposed grounding draft"),
    }
}

#[test]
fn grounding_draft_rejects_not_ready_without_questions_or_missing_refs() {
    let call = ToolCall {
        id: "tool-grounding-final".to_string(),
        name: "plan.ground_intent".to_string(),
        arguments: json!({
            "status": "proposed",
            "ready_for_todos": false,
            "summary": "not ready but no actionable follow-up",
            "resolved_inputs": {
                "owner": "0x1111"
            }
        }),
    };
    let error = decode_segmented_tool_call(
        &call,
        "plan.ground_intent",
        PlannerRoundPhase::GroundIntent,
        None,
    )
    .expect_err("must reject non-actionable not-ready grounding payload");
    assert!(error.to_string().contains(
            "status=proposed with ready_for_todos=false requires non-empty `questions` or `missing_refs`"
        ));
}

#[test]
fn grounding_draft_accepts_not_ready_with_missing_refs_only() {
    let call = ToolCall {
        id: "tool-grounding-final".to_string(),
        name: "plan.ground_intent".to_string(),
        arguments: json!({
            "status": "proposed",
            "ready_for_todos": false,
            "summary": "need one more lookup",
            "missing_refs": ["inputs.token.decimals"],
            "resolved_inputs": {
                "owner": "0x1111"
            }
        }),
    };
    let result = decode_segmented_tool_call(
        &call,
        "plan.ground_intent",
        PlannerRoundPhase::GroundIntent,
        None,
    )
    .expect("grounding finalize call");
    match result {
        DecodedSegmentedToolCall::Final(PlannerToolOutput::IntentGrounding(
            IntentGroundingDraft::Proposed {
                ready_for_todos,
                missing_refs,
                ..
            },
        )) => {
            assert!(!ready_for_todos);
            assert_eq!(missing_refs, vec!["inputs.token.decimals".to_string()]);
        }
        _ => panic!("expected proposed grounding draft"),
    }
}

#[test]
fn grounding_draft_canonicalizes_and_dedups_missing_refs() {
    let call = ToolCall {
        id: "tool-grounding-final".to_string(),
        name: "plan.ground_intent".to_string(),
        arguments: json!({
            "status": "proposed",
            "ready_for_todos": false,
            "summary": "need more refs",
            "missing_refs": [
                "runtime.inputs.token.decimals",
                "token.decimals",
                "fact:quote.price",
                "nodes.q_balance.outputs.balance"
            ],
            "resolved_inputs": {
                "owner": "0x1111"
            }
        }),
    };
    let result = decode_segmented_tool_call(
        &call,
        "plan.ground_intent",
        PlannerRoundPhase::GroundIntent,
        None,
    )
    .expect("grounding finalize call");
    match result {
        DecodedSegmentedToolCall::Final(PlannerToolOutput::IntentGrounding(
            IntentGroundingDraft::Proposed { missing_refs, .. },
        )) => {
            assert_eq!(
                missing_refs,
                vec![
                    "facts.quote.price".to_string(),
                    "inputs.token.decimals".to_string(),
                    "nodes.q_balance.outputs.balance".to_string()
                ]
            );
        }
        _ => panic!("expected proposed grounding draft"),
    }
}

#[test]
fn grounding_draft_decodes_stringified_maps() {
    let call = ToolCall {
        id: "tool-grounding-final".to_string(),
        name: "plan.ground_intent".to_string(),
        arguments: json!({
            "status": "proposed",
            "ready_for_todos": "true",
            "resolved_inputs": "{\"inputs.owner\":\"0x1111\"}",
            "intent_facts": "{\"chain\":\"eip155:31338\",\"owner\":\"0x1111\"}",
            "confidence": "{\"inputs.owner\":100}"
        }),
    };
    let result = decode_segmented_tool_call(
        &call,
        "plan.ground_intent",
        PlannerRoundPhase::GroundIntent,
        None,
    )
    .expect("grounding finalize call");
    match result {
        DecodedSegmentedToolCall::Final(PlannerToolOutput::IntentGrounding(
            IntentGroundingDraft::Proposed {
                ready_for_todos,
                resolved_inputs,
                intent_facts,
                confidence,
                ..
            },
        )) => {
            assert!(ready_for_todos);
            assert_eq!(
                resolved_inputs.get("inputs.owner").and_then(Value::as_str),
                Some("0x1111")
            );
            assert_eq!(
                intent_facts.get("chain").and_then(Value::as_str),
                Some("eip155:31338")
            );
            assert_eq!(confidence.get("inputs.owner").copied(), Some(100u8));
        }
        _ => panic!("expected proposed grounding draft"),
    }
}

#[test]
fn grounding_draft_decodes_payload_stringified_inside_intent_facts() {
    let payload = json!({
        "status": "proposed",
        "ready_for_todos": true,
        "resolved_inputs": {"inputs.owner":"0x1111"},
        "intent_facts": {"owner":"0x1111","chain":"eip155:31338"},
        "confidence": {"inputs.owner": 95}
    });
    let call = ToolCall {
        id: "tool-grounding-final".to_string(),
        name: "plan.ground_intent".to_string(),
        arguments: json!({
            "cursor": "0",
            "intent_facts": payload.to_string()
        }),
    };
    let result = decode_segmented_tool_call(
        &call,
        "plan.ground_intent",
        PlannerRoundPhase::GroundIntent,
        None,
    )
    .expect("grounding finalize call");
    match result {
        DecodedSegmentedToolCall::Final(PlannerToolOutput::IntentGrounding(
            IntentGroundingDraft::Proposed {
                ready_for_todos,
                resolved_inputs,
                intent_facts,
                ..
            },
        )) => {
            assert!(ready_for_todos);
            assert_eq!(
                resolved_inputs.get("inputs.owner").and_then(Value::as_str),
                Some("0x1111")
            );
            assert_eq!(
                intent_facts.get("owner").and_then(Value::as_str),
                Some("0x1111")
            );
        }
        _ => panic!("expected proposed grounding draft"),
    }
}

#[test]
fn segment_draft_tool_schema_requires_step_id() {
    let step_schema = propose_segment_step_schema();
    let required = step_schema
        .get("required")
        .and_then(Value::as_array)
        .expect("required fields");
    let required_set = required
        .iter()
        .filter_map(Value::as_str)
        .collect::<BTreeSet<_>>();
    assert!(required_set.contains("id"));
    assert!(required_set.contains("kind"));
    assert!(required_set.contains("inputs"));
}

#[test]
fn segment_draft_tool_schema_includes_runtime_controls() {
    let step_schema = propose_segment_step_schema();
    let step_props = step_schema
        .get("properties")
        .and_then(Value::as_object)
        .expect("step properties");
    assert!(step_props.contains_key("until"));
    assert!(step_props.contains_key("retry"));
    assert!(step_props.contains_key("timeout_ms"));
}

#[test]
fn segment_draft_tool_schema_accepts_control_step_kinds() {
    let step_schema = propose_segment_step_schema();
    let kind_enum = step_schema
        .pointer("/properties/kind/enum")
        .and_then(Value::as_array)
        .expect("kind enum");
    let kind_set = kind_enum
        .iter()
        .filter_map(Value::as_str)
        .collect::<BTreeSet<_>>();
    assert!(kind_set.contains("action"));
    assert!(kind_set.contains("query"));
    assert!(kind_set.contains("assert"));
    assert!(kind_set.contains("branch"));
}

#[test]
fn plan_check_segment_tool_returns_compile_issues_without_candidate_match() {
    let call = ToolCall {
        id: "tool-check-segment".to_string(),
        name: "plan.check_segment".to_string(),
        arguments: json!({
            "segment": {
                "segment_id": "seg-check",
                "cursor_in": "c0",
                "cursor_out": "c1",
                "done": false,
                "steps": [{
                    "id": "q1",
                    "kind": "query",
                    "candidate_ref": "missing@0.0.1/query",
                    "inputs": {}
                }]
            }
        }),
    };
    let check_context = SegmentCheckContext {
        intent: "check segment".to_string(),
        session_id: "s-1".to_string(),
        cursor: "0".to_string(),
        pack_snapshot_hash: "a".repeat(64),
        chain_scope: vec!["eip155:1".to_string()],
        volatile_facts_policy: crate::policy::VolatileFactsPolicy::default(),
        known_input_refs: vec![],
        grounding_fact_keys: vec![],
        current_todo_scope: None,
    };
    let result = decode_segmented_tool_call_with_memory(
        &call,
        "plan.propose_segment",
        PlannerRoundPhase::ProposeSegment,
        Some(&CandidateContext::default()),
        Some(&check_context),
        None,
        None,
        None,
        None,
    )
    .expect("check call");
    let content = match result {
        DecodedSegmentedToolCall::ToolMessage { content, .. } => content,
        _ => panic!("check must return tool message"),
    };
    let value: Value = serde_json::from_str(content.as_str()).expect("json payload");
    assert_eq!(value.pointer("/ok"), Some(&json!(false)));
    assert_eq!(value.pointer("/reason_code"), Some(&json!("compile_error")));
    let issues = value
        .pointer("/issues")
        .and_then(Value::as_array)
        .expect("issues array");
    assert!(!issues.is_empty());
}

#[test]
fn decode_plan_sketch_segment_arg_fills_missing_step_inputs() {
    let raw = json!({
        "segment_id": "seg_1",
        "cursor_in": "0",
        "cursor_out": "1",
        "done": false,
        "steps": [
            {
                "id": "q1",
                "kind": "query",
                "candidate_ref": "demo@0.0.1/read"
            },
            {
                "id": "g1",
                "kind": "assert",
                "when": {"cel": "true"}
            }
        ]
    });

    let segment = decode_plan_sketch_segment_arg(&raw).expect("segment");
    let value = serde_json::to_value(segment).expect("segment json");
    assert_eq!(value.pointer("/steps/0/inputs"), Some(&json!({})));
    assert_eq!(value.pointer("/steps/1/inputs"), Some(&json!({})));
}

fn propose_segment_step_schema() -> Value {
    let tools = segmented_planner_tools_for_phase(PlannerRoundPhase::ProposeSegment);
    let schema = tools
        .into_iter()
        .find(|tool| tool.name == "plan.propose_segment")
        .map(|tool| tool.input_schema)
        .expect("plan.propose_segment schema");
    let segment_ref = schema
        .pointer("/properties/segment/$ref")
        .and_then(Value::as_str)
        .expect("segment ref");
    let segment_schema = schema
        .pointer(segment_ref.trim_start_matches('#'))
        .cloned()
        .expect("segment schema");
    let step_ref = segment_schema
        .pointer("/properties/steps/items/$ref")
        .and_then(Value::as_str)
        .expect("steps item ref");
    schema
        .pointer(step_ref.trim_start_matches('#'))
        .cloned()
        .expect("segment step schema")
}

#[test]
fn guide_get_tool_returns_topic_guide() {
    let call = ToolCall {
        id: "tool-schema-topic".to_string(),
        name: "guide.get".to_string(),
        arguments: json!({"topic":"cel"}),
    };
    let result = decode_segmented_tool_call(
        &call,
        "plan.propose_segment",
        PlannerRoundPhase::ProposeSegment,
        None,
    )
    .expect("guide.get topic call");
    let content = match result {
        DecodedSegmentedToolCall::ToolMessage { content, .. } => content,
        _ => panic!("guide.get must return tool message"),
    };
    let value: Value = serde_json::from_str(content.as_str()).expect("json payload");
    assert_eq!(value.get("kind"), Some(&json!("topic")));
    assert_eq!(value.pointer("/topic/topic"), Some(&json!("cel")));
    assert_eq!(
        value.pointer("/topic/common_runtime_roots/0"),
        Some(&json!("inputs"))
    );
    assert_eq!(
        value.pointer("/topic/common_helpers/0"),
        Some(&json!("to_atomic(amount, decimals)"))
    );
    assert_eq!(
        value.pointer("/topic/runtime_root_contract"),
        Some(&json!("CEL can read host-exposed runtime roots from ResolverContext.runtime plus root_overrides. In segmented planning the common roots are inputs, params, nodes, and other host-projected runtime roots when present. Do not invent roots that are not present in host context."))
    );
}

#[test]
fn guide_get_tool_returns_embedded_schema() {
    let call = ToolCall {
        id: "tool-schema-id".to_string(),
        name: "guide.get".to_string(),
        arguments: json!({"schema":"ais-plan-sketch/0.1.0"}),
    };
    let result = decode_segmented_tool_call(
        &call,
        "plan.propose_segment",
        PlannerRoundPhase::ProposeSegment,
        None,
    )
    .expect("guide.get schema_id call");
    let content = match result {
        DecodedSegmentedToolCall::ToolMessage { content, .. } => content,
        _ => panic!("guide.get must return tool message"),
    };
    let value: Value = serde_json::from_str(content.as_str()).expect("json payload");
    assert_eq!(value.get("kind"), Some(&json!("schema")));
    assert_eq!(
        value.pointer("/schema/id"),
        Some(&json!("ais-plan-sketch/0.1.0"))
    );
    assert_eq!(value.pointer("/schema/mode"), Some(&json!("digest")));
    assert!(value.pointer("/schema/digest").is_some());
    assert!(value.pointer("/schema/json").is_none());
}

#[test]
fn guide_get_tool_returns_full_schema_when_requested() {
    let call = ToolCall {
        id: "tool-schema-id-full".to_string(),
        name: "guide.get".to_string(),
        arguments: json!({"schema":"ais-plan-sketch/0.1.0","full":true}),
    };
    let result = decode_segmented_tool_call(
        &call,
        "plan.propose_segment",
        PlannerRoundPhase::ProposeSegment,
        None,
    )
    .expect("guide.get full schema call");
    let content = match result {
        DecodedSegmentedToolCall::ToolMessage { content, .. } => content,
        _ => panic!("guide.get must return tool message"),
    };
    let value: Value = serde_json::from_str(content.as_str()).expect("json payload");
    assert_eq!(value.get("kind"), Some(&json!("schema")));
    assert_eq!(value.pointer("/schema/mode"), Some(&json!("full")));
    assert!(value.pointer("/schema/digest").is_some());
    assert!(value.pointer("/schema/json").is_some());
}

#[test]
fn guide_get_tool_normalizes_stringified_full_boolean() {
    let call = ToolCall {
        id: "tool-schema-id-full-string".to_string(),
        name: "guide.get".to_string(),
        arguments: json!({"schema":"ais-plan-sketch/0.1.0","full":"True"}),
    };
    let result = decode_segmented_tool_call(
        &call,
        "plan.propose_segment",
        PlannerRoundPhase::ProposeSegment,
        None,
    )
    .expect("guide.get full schema call with string bool");
    let content = match result {
        DecodedSegmentedToolCall::ToolMessage { content, .. } => content,
        _ => panic!("guide.get must return tool message"),
    };
    let value: Value = serde_json::from_str(content.as_str()).expect("json payload");
    assert_eq!(value.pointer("/schema/mode"), Some(&json!("full")));
    assert!(value.pointer("/schema/json").is_some());
}

#[test]
fn guide_get_tool_rejects_object_schema_arg() {
    let call = ToolCall {
        id: "tool-guide-schema-object".to_string(),
        name: "guide.get".to_string(),
        arguments: json!({
            "schema": {"id":"ais-plan-sketch/0.1.0"}
        }),
    };
    let error = decode_segmented_tool_call(
        &call,
        "plan.propose_segment",
        PlannerRoundPhase::ProposeSegment,
        None,
    )
    .expect_err("object schema arg must be rejected");
    assert!(error.to_string().contains("invalid guide.get args"));
}

#[test]
fn guide_get_tool_rejects_object_topic_arg() {
    let call = ToolCall {
        id: "tool-guide-topic-object".to_string(),
        name: "guide.get".to_string(),
        arguments: json!({
            "topic": {"name":"cel"}
        }),
    };
    let error = decode_segmented_tool_call(
        &call,
        "plan.propose_segment",
        PlannerRoundPhase::ProposeSegment,
        None,
    )
    .expect_err("object topic arg must be rejected");
    assert!(error.to_string().contains("invalid guide.get args"));
}

#[test]
fn guide_get_tool_rejects_stringified_schema_object_arg() {
    let call = ToolCall {
        id: "tool-guide-schema-stringified-object".to_string(),
        name: "guide.get".to_string(),
        arguments: json!({
            "schema": "{\"id\":\"ais-plan-sketch/0.1.0\"}"
        }),
    };
    let result = decode_segmented_tool_call(
        &call,
        "plan.propose_segment",
        PlannerRoundPhase::ProposeSegment,
        None,
    )
    .expect("guide.get stringified object schema call");
    let content = match result {
        DecodedSegmentedToolCall::ToolMessage { content, .. } => content,
        _ => panic!("guide.get must return tool message"),
    };
    let value: Value = serde_json::from_str(content.as_str()).expect("json payload");
    assert_eq!(value.pointer("/kind"), Some(&json!("schema")));
    assert_eq!(
        value.pointer("/error/code"),
        Some(&json!("schema_not_found"))
    );
}

#[test]
fn guide_get_tool_schema_prefers_canonical_string_request_shape() {
    let schema = segmented_planner_tools()
        .into_iter()
        .find(|tool| tool.name == "guide.get")
        .map(|tool| tool.input_schema)
        .expect("guide.get schema");
    assert_eq!(
        schema.pointer("/oneOf/0/properties/schema/type"),
        Some(&json!("string"))
    );
    assert_eq!(
        schema.pointer("/oneOf/0/properties/full/type"),
        Some(&json!("boolean"))
    );
    assert_eq!(
        schema.pointer("/oneOf/1/properties/topic/enum/0"),
        Some(&json!("cel"))
    );
    assert_eq!(
        schema.pointer("/oneOf/1/properties/topic/enum/1"),
        Some(&json!("valueref"))
    );
    assert_eq!(
        schema.pointer("/oneOf/1/properties/topic/enum/2"),
        Some(&json!("typing"))
    );
    assert_eq!(
        schema.pointer("/oneOf/1/properties/topic/enum/3"),
        Some(&json!("failure"))
    );
    assert!(
        schema.pointer("/oneOf/1/properties/topic/enum/4").is_none(),
        "guide topic enum should have exactly 4 entries"
    );
}

#[test]
fn catalog_discover_tool_schema_exposes_optional_filters() {
    let schema = segmented_planner_tools()
        .into_iter()
        .find(|tool| tool.name == "catalog.discover")
        .map(|tool| tool.input_schema)
        .expect("catalog.discover schema");
    assert_eq!(
        schema.pointer("/properties/chain/type"),
        Some(&json!("string"))
    );
    assert_eq!(
        schema.pointer("/properties/protocol/type"),
        Some(&json!("string"))
    );
    assert_eq!(
        schema.pointer("/properties/query/type"),
        Some(&json!("string"))
    );
}

#[test]
fn guide_get_cache_key_uses_canonical_string_shapes_only() {
    let schema_from_canonical = super::super::tools::cache::tool_cache_key(
        "guide.get",
        &json!({"schema":"ais-plan-sketch/0.1.0"}),
    )
    .expect("canonical schema cache key");
    let schema_from_object = super::super::tools::cache::tool_cache_key(
        "guide.get",
        &json!({"schema":{"id":"ais-plan-sketch/0.1.0"}}),
    )
    .expect("object schema cache key");
    assert_ne!(schema_from_canonical, schema_from_object);

    let topic_from_canonical =
        super::super::tools::cache::tool_cache_key("guide.get", &json!({"topic":"cel"}))
            .expect("canonical topic cache key");
    let topic_from_object =
        super::super::tools::cache::tool_cache_key("guide.get", &json!({"topic":{"name":"cel"}}))
            .expect("object topic cache key");
    assert_ne!(topic_from_canonical, topic_from_object);
}

#[test]
fn tool_arg_normalization_for_validation_is_strictly_whitelisted() {
    let normalized = normalize_tool_args_for_validation(
        "guide.get",
        &json!({"schema":"ais-plan-sketch/0.1.0","full":"TRUE","extra":"keep"}),
    );
    assert!(normalized.changed());
    assert_eq!(
        normalized.arguments,
        json!({"schema":"ais-plan-sketch/0.1.0","full":true,"extra":"keep"})
    );
    assert_eq!(normalized.normalized_fields, vec!["full:string->bool"]);

    let already_valid = normalize_tool_args_for_validation(
        "guide.get",
        &json!({"schema":"ais-plan-sketch/0.1.0","full":true}),
    );
    assert!(!already_valid.changed());
    assert_eq!(
        already_valid.arguments,
        json!({"schema":"ais-plan-sketch/0.1.0","full":true})
    );

    let unrelated = normalize_tool_args_for_validation(
        "catalog.discover",
        &json!({"query":"erc20","full":"true"}),
    );
    assert!(!unrelated.changed());
    assert_eq!(unrelated.arguments, json!({"query":"erc20","full":"true"}));
}

#[test]
fn catalog_search_cache_key_normalizes_synonyms_and_token_order() {
    let first = super::super::tools::cache::tool_cache_key(
        "catalog.discover",
        &json!({"kind":"query","query":"erc20 balance"}),
    )
    .expect("first cache key");
    let second = super::super::tools::cache::tool_cache_key(
        "catalog.discover",
        &json!({"kind":"query","query":"token   balance"}),
    )
    .expect("second cache key");
    let third = super::super::tools::cache::tool_cache_key(
        "catalog.discover",
        &json!({"kind":"query","query":"balance token"}),
    )
    .expect("third cache key");
    assert_eq!(first, second);
    assert_eq!(second, third);
}

#[test]
fn resolve_missing_facts_cache_key_normalizes_missing_refs() {
    let first = super::super::tools::cache::tool_cache_key(
        "catalog.resolve_missing_facts",
        &json!({
            "missing_refs": ["runtime.inputs.token.decimals", "inputs.owner"],
            "limit_per_ref": 3
        }),
    )
    .expect("first cache key");
    let second = super::super::tools::cache::tool_cache_key(
        "catalog.resolve_missing_facts",
        &json!({
            "missing_refs": ["inputs.owner", "token.decimals"],
            "limit_per_ref": 3
        }),
    )
    .expect("second cache key");
    assert_eq!(first, second);
}

#[test]
fn resolve_missing_facts_cache_key_keeps_facts_and_nodes_namespaces() {
    let first = super::super::tools::cache::tool_cache_key(
        "catalog.resolve_missing_facts",
        &json!({
            "missing_refs": ["fact:quote.price", "nodes.q_balance.outputs.balance"],
            "limit_per_ref": 3
        }),
    )
    .expect("first cache key");
    let second = super::super::tools::cache::tool_cache_key(
        "catalog.resolve_missing_facts",
        &json!({
            "missing_refs": ["facts.quote.price", "runtime.nodes.q_balance.outputs.balance"],
            "limit_per_ref": 3
        }),
    )
    .expect("second cache key");
    assert_eq!(first, second);
}

#[test]
fn requires_successful_check_only_for_segment_finalize_with_context() {
    assert!(!requires_successful_check_before_finalize(
        PlannerRoundPhase::Begin,
        None
    ));
    assert!(!requires_successful_check_before_finalize(
        PlannerRoundPhase::ProposeSegment,
        None
    ));
    let context = SegmentCheckContext {
        intent: "i".to_string(),
        session_id: "s".to_string(),
        cursor: "0".to_string(),
        pack_snapshot_hash: "a".repeat(64),
        chain_scope: vec!["eip155:1".to_string()],
        volatile_facts_policy: crate::policy::VolatileFactsPolicy::default(),
        known_input_refs: vec![],
        grounding_fact_keys: vec![],
        current_todo_scope: None,
    };
    assert!(requires_successful_check_before_finalize(
        PlannerRoundPhase::ProposeSegment,
        Some(&context)
    ));
    assert!(requires_successful_check_before_finalize(
        PlannerRoundPhase::ReviseSegment,
        Some(&context)
    ));
    assert!(!requires_successful_check_before_finalize(
        PlannerRoundPhase::ProposeTodos,
        Some(&context)
    ));
}

#[test]
fn unavailable_draft_extracts_missing_input_questions_from_error_details() {
    let draft = parse_segment_draft(SegmentToolArgs {
        status: "unavailable".to_string(),
        done: false,
        segment: None,
        summary: None,
        cursor_next: None,
        issues: PlannerIssueList::default(),
        error: Some(PlannerToolError {
            reason_code: "missing_required_input".to_string(),
            message: Some("missing owner".to_string()),
            details: Some(PlannerErrorDetails::Raw(json!({
                "questions": [
                    {
                        "id": "owner",
                        "question": "who is the owner",
                        "source": "details",
                        "options": [
                            {"label": "wallet-1", "value": "0xabc", "confidence": 90}
                        ]
                    }
                ],
                "decisions": [
                    {
                        "kind": "run_producer",
                        "target": "inputs.owner",
                        "query_ref": "wallet@0.0.1/defaultOwner"
                    }
                ],
                "recovery_exhaustion": {
                    "unresolved_refs": ["inputs.owner"],
                    "reasons": ["no_query_candidates", "no_binding_candidates"],
                    "attempt_trace_id": "missing_resolution:grounding:todo_1:user_input:2"
                }
            }))),
        }),
        questions: PlannerQuestionList::default(),
    })
    .expect("parse unavailable draft");
    match draft {
        SegmentDraft::Unavailable {
            reason_code,
            questions,
            error_details,
            ..
        } => {
            assert_eq!(reason_code, "missing_required_input");
            assert_eq!(questions.len(), 1);
            assert_eq!(questions[0].pointer("/id"), Some(&json!("owner")));
            assert_eq!(questions[0].pointer("/source"), Some(&json!("details")));
            assert_eq!(
                questions[0].pointer("/options/0/label"),
                Some(&json!("wallet-1"))
            );
            assert_eq!(
                questions[0].pointer("/options/0/confidence"),
                Some(&json!(90))
            );
            assert_eq!(
                error_details
                    .as_ref()
                    .and_then(|details| details.pointer("/decisions/0/query_ref"))
                    .and_then(Value::as_str),
                Some("wallet@0.0.1/defaultOwner")
            );
        }
        _ => panic!("draft must be unavailable"),
    }
}

#[test]
fn unavailable_draft_rejects_params_question_id_for_missing_required_input() {
    let error = parse_segment_draft(SegmentToolArgs {
        status: "unavailable".to_string(),
        done: false,
        segment: None,
        summary: None,
        cursor_next: None,
        issues: PlannerIssueList::default(),
        error: Some(PlannerToolError {
            reason_code: "missing_required_input".to_string(),
            message: Some("missing token address".to_string()),
            details: Some(PlannerErrorDetails::Raw(json!({
                "questions": [
                    {
                        "id": "params.token.address",
                        "question": "token?"
                    }
                ],
                "recovery_exhaustion": {
                    "unresolved_refs": ["inputs.token.address"],
                    "reasons": ["no_query_candidates"],
                    "attempt_trace_id": "missing_resolution:grounding:todo_1:user_input:1"
                }
            }))),
        }),
        questions: PlannerQuestionList::default(),
    })
    .expect_err("params.* question id must be rejected");
    assert!(error
        .to_string()
        .contains("question.id must be canonical source ref"));
}

#[test]
fn unavailable_draft_rejects_params_unresolved_refs_for_missing_required_input() {
    let error = parse_segment_draft(SegmentToolArgs {
        status: "unavailable".to_string(),
        done: false,
        segment: None,
        summary: None,
        cursor_next: None,
        issues: PlannerIssueList::default(),
        error: Some(PlannerToolError {
            reason_code: "missing_required_input".to_string(),
            message: Some("missing token address".to_string()),
            details: Some(PlannerErrorDetails::Raw(json!({
                "questions": [
                    {
                        "id": "token.address",
                        "question": "token?"
                    }
                ],
                "recovery_exhaustion": {
                    "unresolved_refs": ["params.token.address"],
                    "reasons": ["no_query_candidates"],
                    "attempt_trace_id": "missing_resolution:grounding:todo_1:user_input:1"
                }
            }))),
        }),
        questions: PlannerQuestionList::default(),
    })
    .expect_err("params.* unresolved ref must be rejected");
    assert!(error
        .to_string()
        .contains("recovery_exhaustion.unresolved_refs must use source refs only"));
}

#[test]
fn decode_segment_tool_args_preserves_issue_question_and_error_details_json() {
    let raw_issues = vec![json!({
        "reason_code": "missing_required_input",
        "reference": "inputs.owner",
        "message": "owner is required",
        "extra": {"severity": "high"}
    })];
    let raw_questions = vec![json!({
        "id": "owner",
        "question": "Who is the owner?",
        "source": "top_level",
        "options": [{"label": "wallet-1", "value": "0xabc", "confidence": 80}]
    })];
    let raw_details = json!({
        "questions": [{
            "id": "owner",
            "question": "Who is the owner?",
            "options": [{"label": "wallet-1", "value": "0xabc"}]
        }],
        "meta": {"source": "planner"}
    });

    let args = decode_segment_tool_args(
        json!({
            "status": "unavailable",
            "done": false,
            "issues": raw_issues,
            "questions": raw_questions,
            "error": {
                "reason_code": "missing_required_input",
                "message": "need owner",
                "details": raw_details
            }
        }),
        "plan.propose_segment",
    )
    .expect("decode segment args");
    let SegmentToolArgs {
        issues,
        questions,
        error,
        ..
    } = args;
    assert_eq!(
        issues.into_values(),
        vec![json!({
            "reason_code": "missing_required_input",
            "reference": "inputs.owner",
            "message": "owner is required",
            "extra": {"severity": "high"}
        })]
    );
    assert_eq!(
        questions.into_values(),
        vec![json!({
            "id": "owner",
            "question": "Who is the owner?",
            "source": "top_level",
            "options": [{"label": "wallet-1", "value": "0xabc", "confidence": 80}]
        })]
    );
    let details = error
        .and_then(|entry| entry.details)
        .expect("error details must exist");
    assert_eq!(
        details.to_value().pointer("/meta/source"),
        Some(&json!("planner"))
    );
}

#[test]
fn decode_segment_tool_args_accepts_stringified_error_object() {
    let args = decode_segment_tool_args(
        json!({
            "status": "unavailable",
            "done": false,
            "error": "{\"reason_code\":\"missing_required_input\",\"message\":\"need owner\",\"details\":{\"questions\":[{\"id\":\"inputs.owner\",\"question\":\"owner?\"}],\"recovery_exhaustion\":{\"unresolved_refs\":[\"inputs.owner\"],\"reasons\":[\"no_query_candidates\"],\"attempt_trace_id\":\"missing_resolution:todo_2:user_input:1\"}}}"
        }),
        "plan.revise_segment",
    )
    .expect("decode segment args");
    let error = args.error.expect("error object");
    assert_eq!(error.reason_code, "missing_required_input");
    let details = error.details.expect("error details");
    assert_eq!(
        details
            .to_value()
            .pointer("/recovery_exhaustion/attempt_trace_id"),
        Some(&json!("missing_resolution:todo_2:user_input:1"))
    );
}

#[test]
fn todo_unavailable_prefers_top_level_questions_over_error_details() {
    let draft = parse_todo_draft(
        decode_todo_tool_args(
            json!({
                "status": "unavailable",
                "summary": "need owner",
                "issues": [{"reason_code":"missing_required_input"}],
                "questions": [{
                    "id": "owner",
                    "question": "top-level owner?",
                    "options": [{"label":"wallet-2","value":"0xdef"}]
                }],
                "error": {
                    "reason_code": "missing_required_input",
                    "details": {
                        "questions": [{
                            "id": "owner_from_details",
                            "question": "details owner?",
                            "options": [{"label":"wallet-1","value":"0xabc"}]
                        }],
                        "recovery_exhaustion": {
                            "unresolved_refs": ["inputs.owner"],
                            "reasons": ["no_query_candidates"],
                            "attempt_trace_id": "missing_resolution:todo:todo_1:user_input:1"
                        }
                    }
                }
            }),
            "plan.propose_todos",
        )
        .expect("decode todo args"),
    )
    .expect("parse todo draft");

    match draft {
        TodoDraft::Unavailable {
            reason_code,
            issues,
            questions,
            error_details,
            ..
        } => {
            assert_eq!(reason_code, "missing_required_input");
            assert_eq!(issues.len(), 1);
            assert_eq!(questions.len(), 1);
            assert_eq!(questions[0].pointer("/id"), Some(&json!("owner")));
            assert_eq!(
                questions[0].pointer("/options/0/label"),
                Some(&json!("wallet-2"))
            );
            assert_eq!(
                error_details
                    .as_ref()
                    .and_then(|details| details.pointer("/questions/0/id"))
                    .and_then(Value::as_str),
                Some("owner")
            );
        }
        _ => panic!("draft must be unavailable"),
    }
}

#[test]
fn render_segment_prompt_uses_detect_free_valueref_and_contracts() {
    let prompt = render_segment_prompt(
        "plan.propose_segment",
        &SegmentPlanningRequest {
            intent: "transfer token".to_string(),
            session: SegmentPlanningSession {
                session_id: "s".to_string(),
                snapshot_hash: "h".to_string(),
                cursor: "0".to_string(),
                max_rounds: 6,
                max_segments: 8,
            },
            state_summary: None,
            typed_summary: None,
            previous_error: None,
            last_segment: None,
        },
    );
    let value: Value = serde_json::from_str(prompt.as_str()).expect("prompt json");
    // High-frequency contracts moved to system prompt contracts_summary
    assert!(
        value.get("segment_contract").is_none(),
        "segment_contract should be in system prompt, not user prompt"
    );
    assert!(
        value.get("value_ref_contract").is_none(),
        "value_ref_contract should be in system prompt, not user prompt"
    );
    assert!(
        value.get("asset_param_contract").is_none(),
        "asset_param_contract should be in system prompt, not user prompt"
    );
    assert!(
        value.get("write_gate_contract").is_none(),
        "write_gate_contract should be in system prompt, not user prompt"
    );
    // Low-frequency contracts moved to guide.get topics
    assert!(
        value.get("schema_lookup_contract").is_none(),
        "schema_lookup_contract moved to guide.get(topic=typing)"
    );
    assert!(
        value.get("tool_call_typing_contract").is_none(),
        "tool_call_typing_contract moved to guide.get(topic=typing)"
    );
    assert!(
        value.get("failure_contract").is_none(),
        "failure_contract moved to guide.get(topic=failure)"
    );
    assert!(
        value.get("input_ref_semantic_contract").is_none(),
        "input_ref_semantic_contract moved to guide.get(topic=failure)"
    );
    assert!(
        value.get("depends_on_contract").is_none(),
        "depends_on_contract merged into system prompt segment contract"
    );
    // Contracts that remain in user prompt
    let repair_rules = value
        .pointer("/repair_instructions/rules")
        .and_then(Value::as_array)
        .expect("repair rules");
    assert!(repair_rules.iter().any(|rule| {
        rule.as_str()
            .map(|s| s.contains("unknown_input_ref"))
            .unwrap_or(false)
    }));
    assert_eq!(
        value.pointer("/self_check_before_tool_or_finalize/checklist/2"),
        Some(&json!(
            "JSON types exactly match schema (bool/number are not quoted strings)."
        ))
    );
    assert!(value.get("check_segment_contract").is_some());
    assert!(value.get("todo_contract").is_some());
    assert!(
        value.get("adjudicate_recovery_contract").is_none(),
        "prompt payload should not include legacy adjudicate_recovery_contract block"
    );
    assert!(
        !prompt.contains("seg_1/"),
        "prompt must not encourage cross-segment depends_on references"
    );

    // Verify guide.get returns the low-frequency contracts
    let typing_payload = schema_guide_payload("typing");
    assert!(typing_payload.is_some());
    let typing = typing_payload.unwrap();
    assert!(typing.get("tool_call_typing_contract").is_some());
    assert!(typing.get("schema_lookup_contract").is_some());

    let failure_payload = schema_guide_payload("failure");
    assert!(failure_payload.is_some());
    let failure = failure_payload.unwrap();
    assert!(failure.get("failure_contract").is_some());
    assert!(failure.get("input_ref_semantic_contract").is_some());
}

#[test]
fn render_grounding_prompt_includes_actionable_not_ready_examples() {
    let prompt = render_grounding_prompt_with_patch(
        &IntentGroundingRequest {
            intent: "transfer token".to_string(),
            session: SegmentPlanningSession {
                session_id: "s".to_string(),
                snapshot_hash: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    .to_string(),
                cursor: "0".to_string(),
                max_rounds: 6,
                max_segments: 8,
            },
            state_summary: None,
            typed_summary: None,
        },
        None,
    );
    let value: Value = serde_json::from_str(prompt.as_str()).expect("prompt json");
    assert_eq!(
        value.pointer("/grounding_contract/actionability_examples/good/0/ready_for_todos"),
        Some(&json!(false))
    );
    assert_eq!(
        value.pointer("/grounding_contract/actionability_examples/good/1/missing_refs/0"),
        Some(&json!("inputs.token.decimals"))
    );
    assert_eq!(
        value.pointer("/grounding_contract/actionability_examples/bad/1/questions"),
        Some(&json!([]))
    );
    let grounding_rules = value
        .pointer("/grounding_contract/rules")
        .and_then(Value::as_array)
        .expect("grounding rules");
    assert!(grounding_rules.iter().any(|rule| {
        rule.as_str()
            .map(|text| text.contains("token_policy_signal"))
            .unwrap_or(false)
    }));
}

#[test]
fn prompt_renderers_build_prompt_compact_at_render_time() {
    let state_summary = json!({
        "full_only": "must_not_enter_prompt",
        "todo_state": {
            "current_todo": {
                "title": "bridge"
            }
        },
        "input_registry": {
            "counts": {
                "missing": 2
            }
        },
        "reusable_outputs": {
            "summary": {
                "reusable_refs": 4
            }
        },
        "previous_error": {
            "reason_code": "missing_required_input"
        },
        "context_budget": {
            "pressure_mode": "critical",
            "pack_trace": [{"block_id":"tool_memory_projection","action":"compress"}],
            "pack_overflow_reason": "budget_exceeded_no_further_actions",
            "pack_diagnostics": {
                "packed_blocks_total": 4
            }
        }
    });
    let session = SegmentPlanningSession {
        session_id: "s".to_string(),
        snapshot_hash: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            .to_string(),
        cursor: "0".to_string(),
        max_rounds: 6,
        max_segments: 8,
    };

    let todos_prompt = render_todos_prompt_with_patch(
        &TodoPlanningRequest {
            intent: "transfer".to_string(),
            session: session.clone(),
            state_summary: Some(state_summary.clone()),
            typed_summary: None,
        },
        None,
    );
    let todos_payload: Value = serde_json::from_str(&todos_prompt).expect("todos prompt");
    assert_eq!(
        todos_payload.pointer("/state_summary/schema"),
        Some(&json!("ais-agent-state-summary-prompt-compact/0.0.1"))
    );
    assert!(
        todos_payload.pointer("/state_summary/full_only").is_none(),
        "todos prompt must use compact state_summary"
    );
    assert!(
        todos_payload
            .pointer("/state_summary/context_budget/pack_trace")
            .is_none(),
        "todos prompt must not include pack_trace"
    );
    assert_eq!(
        todos_payload.pointer("/state_summary/context_budget/pack_overflow_reason"),
        Some(&json!("budget_exceeded_no_further_actions"))
    );
    assert!(todos_payload
        .pointer("/state_summary/summary_text")
        .and_then(Value::as_str)
        .is_some());

    let grounding_prompt = render_grounding_prompt_with_patch(
        &IntentGroundingRequest {
            intent: "transfer".to_string(),
            session: session.clone(),
            state_summary: Some(state_summary.clone()),
            typed_summary: None,
        },
        None,
    );
    let grounding_payload: Value =
        serde_json::from_str(&grounding_prompt).expect("grounding prompt");
    assert_eq!(
        grounding_payload.pointer("/state_summary/schema"),
        Some(&json!("ais-agent-state-summary-prompt-compact/0.0.1"))
    );
    assert!(
        grounding_payload
            .pointer("/state_summary/context_budget/pack_trace")
            .is_none(),
        "grounding prompt must not include pack_trace"
    );

    let segment_prompt = render_segment_prompt(
        "plan.propose_segment",
        &SegmentPlanningRequest {
            intent: "transfer".to_string(),
            session,
            state_summary: Some(state_summary),
            typed_summary: None,
            previous_error: None,
            last_segment: None,
        },
    );
    let segment_payload: Value = serde_json::from_str(&segment_prompt).expect("segment prompt");
    assert_eq!(
        segment_payload.pointer("/state_summary/schema"),
        Some(&json!("ais-agent-state-summary-prompt-compact/0.0.1"))
    );
    assert!(
        segment_payload
            .pointer("/state_summary/context_budget/pack_trace")
            .is_none(),
        "segment prompt must not include pack_trace"
    );
}

#[test]
fn decode_plan_sketch_segment_arg_reports_missing_candidate_ref_with_step_context() {
    let error = decode_plan_sketch_segment_arg(&json!({
        "segment_id":"seg_1",
        "cursor_in":"0",
        "cursor_out":"1",
        "done":false,
        "steps":[
            {"id":"q_balance","kind":"query","inputs":{}},
            {"id":"a_guard","kind":"assert","inputs":{}},
            {"id":"a_tx","kind":"action","candidate_ref":"erc20@0.0.2/transfer","inputs":{}}
        ]
    }))
    .expect_err("missing candidate_ref must fail");
    let message = error.to_string();
    assert!(message.contains("missing required `candidate_ref`"));
    assert!(message.contains("q_balance(query)"));
    assert!(!message.contains("a_guard(assert)"));
}

#[test]
fn render_segment_prompt_with_patch_overrides_nested_fields() {
    let request = SegmentPlanningRequest {
        intent: "transfer token".to_string(),
        session: SegmentPlanningSession {
            session_id: "s".to_string(),
            snapshot_hash: "h".to_string(),
            cursor: "0".to_string(),
            max_rounds: 6,
            max_segments: 8,
        },
        state_summary: None,
        typed_summary: None,
        previous_error: None,
        last_segment: None,
    };
    let patch = json!({
        "segment_contract": {
            "notes": "patched-note"
        },
        "custom_hint": "x"
    });
    let prompt = render_segment_prompt_with_patch("plan.propose_segment", &request, Some(&patch));
    let value: Value = serde_json::from_str(prompt.as_str()).expect("prompt json");
    assert_eq!(
        value.pointer("/segment_contract/notes"),
        Some(&json!("patched-note"))
    );
    assert_eq!(value.pointer("/custom_hint"), Some(&json!("x")));
}

#[test]
fn system_prompt_builder_emits_stable_version_and_hash() {
    let builder = SegmentedPromptContextBuilder::default();
    let rendered_a = builder.render(PlannerRoundPhase::ProposeSegment, None);
    let rendered_b = builder.render(PlannerRoundPhase::ProposeSegment, None);
    assert_eq!(rendered_a.version, SEGMENTED_PROMPT_VERSION);
    assert_eq!(rendered_a.hash, rendered_b.hash);
    assert!(rendered_a
        .prompt
        .contains("Prompt-Version: aisrs-segmented-planner-v3"));
    assert!(rendered_a.prompt.contains("Prompt-Hash: "));
    assert!(rendered_a.prompt.contains("Emit schema-typed JSON only"));
    assert!(rendered_a.prompt.contains("digest-first"));
    assert!(rendered_a
        .prompt
        .contains("Post-write invalidation is real"));
    assert!(rendered_a
        .prompt
        .contains("same-segment scheduling/gate reachability only"));
    assert!(rendered_a
        .prompt
        .contains("follow-up writes must query again"));
    assert!(rendered_a.prompt.contains("semantic_hints"));
    assert!(rendered_a.prompt.contains("token_policy_signal"));
    assert!(rendered_a
        .prompt
        .contains("query, calculated, policy, ctx, and contracts"));
}

#[test]
fn usage_tracker_records_estimated_tokens() {
    let request = CompleteWithToolsRequest {
        messages: vec![LlmMessage {
            role: MessageRole::User,
            content: Some("hello".to_string()),
            tool_name: None,
            tool_call_id: None,
            tool_calls: vec![],
        }],
        tools: vec![],
    };
    let response = ais_llm::CompleteWithToolsResponse {
        assistant_content: Some("world".to_string()),
        tool_calls: vec![],
    };
    let mut tracker = PlannerLlmUsageTracker::default().with_context_limit_tokens(Some(1000));
    let usage = tracker.record_estimated(&request, &response);
    assert!(usage.input_tokens > 0);
    assert!(usage.output_tokens > 0);
    assert!(usage.total_tokens >= usage.input_tokens);
    assert!(usage.estimated);
    assert_eq!(usage.context_limit_tokens, Some(1000));
    assert_eq!(usage.context_soft_limit_tokens, Some(900));
    assert_eq!(
        usage.context_remaining_tokens,
        Some(900_u64.saturating_sub(usage.input_tokens))
    );
    let value = tracker.to_value();
    assert_eq!(value.pointer("/calls"), Some(&json!(1)));
    assert_eq!(
        value.pointer("/source"),
        Some(&json!("tiktoken(o200k_base)"))
    );
    assert_eq!(value.pointer("/context_limit_tokens"), Some(&json!(1000)));
    assert_eq!(
        value.pointer("/context_soft_limit_tokens"),
        Some(&json!(900))
    );
    assert_eq!(
        value.pointer("/context_window_input_tokens"),
        Some(&json!(usage.input_tokens))
    );
    assert_eq!(
        value.pointer("/context_window_total_tokens"),
        Some(&json!(usage.total_tokens))
    );
    assert_eq!(
        value.pointer("/context_remaining_tokens"),
        Some(&json!(900_u64.saturating_sub(usage.input_tokens)))
    );
}

#[test]
fn extract_round_context_signal_reads_pressure_and_compression_flags() {
    let signal = extract_round_context_signal(
        json!({
            "state_summary": {
                "context_budget": {
                    "pressure_mode": "critical",
                    "pack_diagnostics": {
                        "compressed_blocks_total": 1,
                        "packed_blocks_evicted": 0
                    }
                }
            }
        })
        .to_string()
        .as_str(),
    );
    assert_eq!(signal.pressure_mode.as_deref(), Some("critical"));
    assert!(signal.compressed);
    assert!(!signal.adjudicate_mode);
}

#[test]
fn planner_diagnostics_track_reusable_inventory_and_redundant_query_rejections() {
    let mut tracker = super::PlannerDiagnosticsTracker::default();
    tracker.observe_reusable_inventory_summary(
        None,
        Some(&json!({
            "reusable_outputs": {
                "summary": {
                    "total_refs": 4,
                    "reusable_refs": 3,
                    "fresh_volatile_refs": 1,
                    "stale_volatile_refs": 1
                }
            }
        })),
    );
    tracker.observe_check_segment_redundant_query_rejections(
        json!({
            "ok": false,
            "issues": [
                {"reason_code":"redundant_query_step","step_id":"q_balance"},
                {"reason_code":"todo_scope_violation","step_id":"a1"}
            ]
        })
        .to_string()
        .as_str(),
    );

    let value = tracker.to_value();
    assert_eq!(
        value.pointer("/reusable_inventory/total_refs"),
        Some(&json!(4))
    );
    assert_eq!(
        value.pointer("/reusable_inventory/reusable_refs"),
        Some(&json!(3))
    );
    assert_eq!(
        value.pointer("/redundant_query_rejections/total"),
        Some(&json!(1))
    );
    assert_eq!(
        value.pointer("/redundant_query_rejections/steps_total"),
        Some(&json!(1))
    );
}

#[test]
fn planner_diagnostics_reusable_inventory_prefers_typed_summary_over_raw_projection() {
    let mut tracker = super::PlannerDiagnosticsTracker::default();
    let typed = super::StateSummary {
        completed_segments: 0,
        completed_nodes: 0,
        plan_epoch: 0,
        paused_reason: None,
        done: false,
        previous_error: None,
        input_store: Some(json!({
            "facts": {
                "token": {"address":"0xabc"}
            },
            "meta": {
                "token.address": {
                    "source":"user",
                    "source_kind":"user",
                    "source_priority": 100,
                    "stability":"stable"
                }
            }
        })),
        runtime_facts: Some(json!({
            "facts": {
                "quote": {"price":"10"}
            },
            "meta": {
                "quote.price": {
                    "source":"query",
                    "source_priority": 50,
                    "stability":"volatile"
                }
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
        input_registry: json!({"known_refs":["inputs.token.address"]}),
        node_output_refs: json!({"known_refs":[]}),
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
    let raw_summary = json!({
        "reusable_outputs": {
            "summary": {
                "total_refs": 99,
                "reusable_refs": 88,
                "fresh_volatile_refs": 77,
                "stale_volatile_refs": 66
            }
        }
    });

    tracker.observe_reusable_inventory_summary(Some(&typed), Some(&raw_summary));

    let value = tracker.to_value();
    assert_ne!(
        value.pointer("/reusable_inventory/total_refs"),
        Some(&json!(99))
    );
    assert_ne!(
        value.pointer("/reusable_inventory/reusable_refs"),
        Some(&json!(88))
    );
}

#[test]
fn extract_round_context_signal_detects_host_binding_adjudicate_mode() {
    let signal = extract_round_context_signal(
        json!({
            "previous_error": {
                "autofill": {
                    "mode": "host_binding_adjudicate_round"
                }
            }
        })
        .to_string()
        .as_str(),
    );
    assert!(signal.adjudicate_mode);
}

#[test]
fn extract_round_context_signal_detects_discovery_basis_from_memory_projection() {
    let signal = extract_round_context_signal(
        json!({
            "state_summary": {
                "tool_memory_projection": {
                    "recent": {
                        "list_inventory": [
                            {"filters":{"chain":"eip155:1"}}
                        ]
                    }
                }
            }
        })
        .to_string()
        .as_str(),
    );
    assert!(!signal.compressed);
}

#[test]
fn extract_round_context_signal_detects_discovery_basis_from_candidate_refs() {
    let signal = extract_round_context_signal(
        json!({
            "state_summary": {
                "tool_memory_projection": {
                    "recent": {
                        "candidate_detail": [
                            {"signatures":[{"ref":"erc20@0.0.2/balance-of"}]}
                        ]
                    }
                }
            }
        })
        .to_string()
        .as_str(),
    );
    assert!(!signal.compressed);
}

#[test]
fn diagnostics_tracker_reports_duplicate_and_empty_search_streak_metrics() {
    let context = large_catalog_candidate_context(2, 2);
    let provider = ScriptedLlmProvider::from_responses(vec![
        Ok(CompleteWithToolsResponse {
            assistant_content: Some("search-1".to_string()),
            tool_calls: vec![ToolCall {
                id: "tool-search-1".to_string(),
                name: "catalog.discover".to_string(),
                arguments: json!({"query":"unknown-thing","kind":"query"}),
            }],
        }),
        Ok(CompleteWithToolsResponse {
            assistant_content: Some("search-2".to_string()),
            tool_calls: vec![ToolCall {
                id: "tool-search-2".to_string(),
                name: "catalog.discover".to_string(),
                arguments: json!({"query":"unknown-thing","kind":"query"}),
            }],
        }),
        Ok(CompleteWithToolsResponse {
            assistant_content: Some("finalize".to_string()),
            tool_calls: vec![ToolCall {
                id: "tool-final".to_string(),
                name: "plan.propose_todos".to_string(),
                arguments: json!({
                    "status":"proposed",
                    "summary":"done",
                    "todos":[{"title":"t1"}]
                }),
            }],
        }),
    ]);

    let mut planner =
        LlmSegmentedIntentPlanner::new(provider).with_candidate_context(Some(context));
    let draft = planner.propose_todos(TodoPlanningRequest {
        intent: "plan todos".to_string(),
        session: SegmentPlanningSession {
            session_id: "sess-1".to_string(),
            snapshot_hash: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                .to_string(),
            cursor: "0".to_string(),
            max_rounds: 8,
            max_segments: 8,
        },
        typed_summary: None,
        state_summary: Some(json!({
            "context_budget": {
                "pressure_mode": "light",
                "pack_diagnostics": {
                    "compressed_blocks_total": 0,
                    "packed_blocks_evicted": 0
                }
            }
        })),
    });
    assert!(matches!(draft, Ok(TodoDraft::Proposed { .. })));

    let usage = planner.llm_usage_value();
    assert_eq!(
        usage.pointer("/diagnostics/tool_calls_total"),
        Some(&json!(3))
    );
    assert_eq!(
        usage.pointer("/diagnostics/tool_call_count_by_tool/catalog.discover"),
        Some(&json!(2))
    );
    assert_eq!(
        usage.pointer("/diagnostics/tool_calls_duplicate"),
        Some(&json!(1))
    );
    assert_eq!(
        usage.pointer("/diagnostics/empty_search_streak_max"),
        Some(&json!(2))
    );
    assert_eq!(
        usage.pointer("/diagnostics/memory_hit_rate_by_tool/catalog.discover/hits"),
        Some(&json!(1))
    );
    assert_eq!(
        usage.pointer("/diagnostics/phase_round_count/propose_todos"),
        Some(&json!(3))
    );
    assert_eq!(
        usage.pointer("/diagnostics/discovery_tool_call_ratio_bps"),
        Some(&json!(6666))
    );
}

#[test]
fn catalog_search_loop_guard_emits_hint_once_per_streak() {
    let mut guard = CatalogSearchLoopGuard::default();
    assert!(!guard.observe_empty(Some("query|q".to_string())));
    assert!(guard.observe_empty(Some("query|q".to_string())));
    assert!(!guard.observe_empty(Some("query|q".to_string())));
    assert_eq!(guard.max_streak(), 3);
    guard.observe_non_empty();
    assert!(!guard.observe_empty(Some("query|q".to_string())));
    assert!(guard.observe_empty(Some("query|q".to_string())));
}

#[test]
fn begin_session_resets_usage_and_diagnostics_trackers() {
    let begin_payload = || {
        Ok(CompleteWithToolsResponse {
            assistant_content: Some("begin".to_string()),
            tool_calls: vec![ToolCall {
                id: "tool-begin".to_string(),
                name: "plan.begin".to_string(),
                arguments: json!({
                    "session_id":"sess",
                    "snapshot_hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    "cursor":"0",
                    "limits":{"max_rounds":4,"max_segments":3}
                }),
            }],
        })
    };
    let provider = ScriptedLlmProvider::from_responses(vec![begin_payload(), begin_payload()]);
    let mut planner =
        LlmSegmentedIntentPlanner::new(provider).with_context_limit_tokens(Some(1000));

    planner
        .begin_session(SegmentBeginRequest {
            intent: "a".to_string(),
            snapshot_hash: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                .to_string(),
            pack_snapshot_hash: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                .to_string(),
            catalog_hash: "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
                .to_string(),
            chain_scope: vec!["eip155:1".to_string()],
        })
        .expect("first begin");
    let first_usage = planner.llm_usage_value();
    assert_eq!(first_usage.pointer("/calls"), Some(&json!(1)));

    planner
        .begin_session(SegmentBeginRequest {
            intent: "b".to_string(),
            snapshot_hash: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                .to_string(),
            pack_snapshot_hash: "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"
                .to_string(),
            catalog_hash: "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"
                .to_string(),
            chain_scope: vec!["eip155:1".to_string()],
        })
        .expect("second begin");
    let second_usage = planner.llm_usage_value();
    assert_eq!(second_usage.pointer("/calls"), Some(&json!(1)));
    assert_eq!(
        second_usage.pointer("/diagnostics/tool_calls_total"),
        Some(&json!(1))
    );
}

#[test]
fn begin_phase_rejects_discovery_tools() {
    let calls = vec![ToolCall {
        id: "tool-1".to_string(),
        name: "catalog.discover".to_string(),
        arguments: json!({}),
    }];
    let error = validate_tool_calls_for_phase(&calls, PlannerRoundPhase::Begin)
        .expect_err("begin phase should reject discovery tools");
    assert!(error.to_string().contains("not allowed"));
}

#[test]
fn begin_phase_rejects_abort_intent() {
    let calls = vec![ToolCall {
        id: "tool-1".to_string(),
        name: "plan.abort_intent".to_string(),
        arguments: json!({
            "reason_code":"recovery_exhausted",
            "summary":"cannot continue with current intent",
            "evidence":{"attempted_recovery":["runtime.query.resolve"]}
        }),
    }];
    let error = validate_tool_calls_for_phase(&calls, PlannerRoundPhase::Begin)
        .expect_err("begin phase should reject abort tool");
    assert!(error.to_string().contains("not allowed"));
}

#[test]
fn propose_phase_rejects_revise_tool() {
    let calls = vec![ToolCall {
        id: "tool-1".to_string(),
        name: "plan.revise_segment".to_string(),
        arguments: json!({
            "status":"invalid",
            "done":false,
            "error":{"reason_code":"x"}
        }),
    }];
    let error = validate_tool_calls_for_phase(&calls, PlannerRoundPhase::ProposeSegment)
        .expect_err("propose phase should reject revise tool");
    assert!(error.to_string().contains("not allowed"));
}

#[test]
fn todo_phase_rejects_segment_finalize_tool() {
    let calls = vec![ToolCall {
        id: "tool-1".to_string(),
        name: "plan.propose_segment".to_string(),
        arguments: json!({
            "status":"invalid",
            "done":false,
            "error":{"reason_code":"x"}
        }),
    }];
    let error = validate_tool_calls_for_phase(&calls, PlannerRoundPhase::ProposeTodos)
        .expect_err("todo phase should reject segment finalize tool");
    assert!(error.to_string().contains("not allowed"));
}

#[test]
fn todo_phase_allows_discovery_then_finalize() {
    let calls = vec![
        ToolCall {
            id: "tool-1".to_string(),
            name: "catalog.discover".to_string(),
            arguments: json!({}),
        },
        ToolCall {
            id: "tool-2".to_string(),
            name: "plan.propose_todos".to_string(),
            arguments: json!({
                "status":"proposed",
                "todos":[{"title":"t1"}]
            }),
        },
    ];
    validate_tool_calls_for_phase(&calls, PlannerRoundPhase::ProposeTodos)
        .expect("todo phase should allow discovery + finalize");
}

#[test]
fn phase_policy_allows_catalog_discover_without_discovery_basis() {
    let calls = vec![ToolCall {
        id: "tool-1".to_string(),
        name: "catalog.discover".to_string(),
        arguments: json!({"query":"balance"}),
    }];
    validate_tool_calls_for_phase_with_context(
        &calls,
        PlannerRoundPhase::ProposeSegment,
        ToolCallValidationContext::default(),
    )
    .expect("catalog.discover should be allowed without discovery basis gate");
}

#[test]
fn phase_policy_allows_catalog_discover_with_in_round_discovery() {
    let calls = vec![
        ToolCall {
            id: "tool-1".to_string(),
            name: "catalog.discover".to_string(),
            arguments: json!({"chain":"eip155:1"}),
        },
        ToolCall {
            id: "tool-2".to_string(),
            name: "catalog.discover".to_string(),
            arguments: json!({"query":"transfer"}),
        },
    ];
    validate_tool_calls_for_phase_with_context(
        &calls,
        PlannerRoundPhase::ProposeSegment,
        ToolCallValidationContext::default(),
    )
    .expect("in-round catalog.discover should be allowed");
}

#[test]
fn revise_phase_rejects_propose_tool() {
    let calls = vec![ToolCall {
        id: "tool-1".to_string(),
        name: "plan.propose_segment".to_string(),
        arguments: json!({
            "status":"invalid",
            "done":false,
            "error":{"reason_code":"x"}
        }),
    }];
    let error = validate_tool_calls_for_phase(&calls, PlannerRoundPhase::ReviseSegment)
        .expect_err("revise phase should reject propose tool");
    assert!(error.to_string().contains("not allowed"));
}

#[test]
fn finalize_tool_must_be_last_in_round() {
    let calls = vec![
        ToolCall {
            id: "tool-1".to_string(),
            name: "plan.propose_segment".to_string(),
            arguments: json!({
                "status":"invalid",
                "done":false,
                "error":{"reason_code":"x"}
            }),
        },
        ToolCall {
            id: "tool-2".to_string(),
            name: "get_candidate_detail".to_string(),
            arguments: json!({
                "refs":["demo@0.0.1/action-1"]
            }),
        },
    ];
    let error = validate_tool_calls_for_phase(&calls, PlannerRoundPhase::ProposeSegment)
        .expect_err("finalize tool should be last");
    assert!(error.to_string().contains("must be the last tool call"));
}

#[test]
fn finalize_tool_at_most_once_per_round() {
    let calls = vec![
        ToolCall {
            id: "tool-1".to_string(),
            name: "plan.propose_segment".to_string(),
            arguments: json!({
                "status":"invalid",
                "done":false,
                "error":{"reason_code":"x"}
            }),
        },
        ToolCall {
            id: "tool-2".to_string(),
            name: "plan.propose_segment".to_string(),
            arguments: json!({
                "status":"invalid",
                "done":false,
                "error":{"reason_code":"x"}
            }),
        },
    ];
    let error = validate_tool_calls_for_phase(&calls, PlannerRoundPhase::ProposeSegment)
        .expect_err("finalize tool should appear at most once");
    assert!(error.to_string().contains("at most one finalize tool"));
}

#[test]
fn abort_tool_must_be_last_in_round() {
    let calls = vec![
        ToolCall {
            id: "tool-1".to_string(),
            name: "plan.abort_intent".to_string(),
            arguments: json!({
                "reason_code":"recovery_exhausted",
                "summary":"cannot continue with current intent",
                "evidence":{"attempted_recovery":["runtime.query.resolve"]}
            }),
        },
        ToolCall {
            id: "tool-2".to_string(),
            name: "guide.get".to_string(),
            arguments: json!({"topic":"failure"}),
        },
    ];
    let error = validate_tool_calls_for_phase(&calls, PlannerRoundPhase::ProposeSegment)
        .expect_err("abort tool should be last");
    assert!(error
        .to_string()
        .contains("terminal tool `plan.abort_intent` must be the last"));
}

#[test]
fn finalize_and_abort_cannot_coexist_in_same_round() {
    let calls = vec![
        ToolCall {
            id: "tool-1".to_string(),
            name: "plan.propose_segment".to_string(),
            arguments: json!({
                "status":"invalid",
                "done":false,
                "error":{"reason_code":"x"}
            }),
        },
        ToolCall {
            id: "tool-2".to_string(),
            name: "plan.abort_intent".to_string(),
            arguments: json!({
                "reason_code":"recovery_exhausted",
                "summary":"cannot continue with current intent",
                "evidence":{"attempted_recovery":["runtime.query.resolve"]}
            }),
        },
    ];
    let error = validate_tool_calls_for_phase(&calls, PlannerRoundPhase::ProposeSegment)
        .expect_err("finalize + abort must not coexist");
    assert!(error.to_string().contains("cannot emit both"));
}

#[test]
fn abort_intent_tool_schema_requires_reason_summary_and_evidence() {
    let schema = segmented_planner_tools()
        .into_iter()
        .find(|tool| tool.name == "plan.abort_intent")
        .map(|tool| tool.input_schema)
        .expect("plan.abort_intent schema");
    assert_eq!(
        schema.pointer("/properties/reason_code/type"),
        Some(&json!("string"))
    );
    assert_eq!(
        schema.pointer("/properties/summary/minLength"),
        Some(&json!(10))
    );
    assert_eq!(
        schema.pointer("/properties/evidence/required/0"),
        Some(&json!("attempted_recovery"))
    );
}

#[test]
fn decode_abort_intent_requires_attempted_recovery() {
    let abort_call = ToolCall {
        id: "tool-abort".to_string(),
        name: "plan.abort_intent".to_string(),
        arguments: json!({
            "reason_code":"recovery_exhausted",
            "summary":"cannot continue with current intent",
            "evidence":{"attempted_recovery":[]}
        }),
    };
    let error = decode_segmented_tool_call(
        &abort_call,
        "plan.propose_segment",
        PlannerRoundPhase::ProposeSegment,
        None,
    )
    .expect_err("empty attempted_recovery should be rejected");
    assert!(error
        .to_string()
        .contains("requires non-empty evidence.attempted_recovery"));
}

#[test]
fn decode_abort_intent_rejects_when_recovery_candidates_exist() {
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
    let abort_call = ToolCall {
        id: "tool-abort".to_string(),
        name: "plan.abort_intent".to_string(),
        arguments: json!({
            "reason_code":"recovery_exhausted",
            "summary":"cannot continue with current intent",
            "evidence":{
              "attempted_recovery":["runtime.query.resolve"],
              "missing_refs":["inputs.token.decimals"]
            }
        }),
    };
    let typed_summary = super::StateSummary {
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
        input_registry: json!({"known_refs":[]}),
        node_output_refs: json!({"known_refs":[]}),
        reusable_outputs: None,
        tool_memory_projection: None,
        intent_slots: None,
        intent_context: None,
        capability_view: None,
        capability_ready: None,
        side_effect_lifecycle: None,
        todo_state: None,
        recovery_diagnostics: Some(json!({
            "available_attempt_keys": ["runtime.query.resolve"]
        })),
    };
    let error = decode_segmented_tool_call_with_memory_and_summary(
        &abort_call,
        "plan.propose_segment",
        PlannerRoundPhase::ProposeSegment,
        Some(&context),
        None,
        None,
        None,
        None,
        None,
        Some(&typed_summary),
    )
    .expect_err("recoverable refs should reject abort");
    assert!(error
        .to_string()
        .contains("still have recoverable query candidates"));
}

#[test]
fn decode_abort_intent_rejects_without_host_recovery_history() {
    let abort_call = ToolCall {
        id: "tool-abort".to_string(),
        name: "plan.abort_intent".to_string(),
        arguments: json!({
            "reason_code":"recovery_exhausted",
            "summary":"cannot continue with current intent",
            "evidence":{
              "attempted_recovery":["runtime.query.resolve"],
              "missing_refs":["inputs.token.decimals"]
            }
        }),
    };
    let error = decode_segmented_tool_call_with_memory_and_summary(
        &abort_call,
        "plan.propose_segment",
        PlannerRoundPhase::ProposeSegment,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .expect_err("abort should be rejected without host recovery history");
    assert!(error
        .to_string()
        .contains("host recovery history is unavailable"));
}

#[test]
fn decode_abort_intent_rejects_unknown_attempted_recovery_entries() {
    let abort_call = ToolCall {
        id: "tool-abort".to_string(),
        name: "plan.abort_intent".to_string(),
        arguments: json!({
            "reason_code":"recovery_exhausted",
            "summary":"cannot continue with current intent",
            "evidence":{
              "attempted_recovery":["unknown.recovery.path"],
              "missing_refs":["inputs.token.decimals"]
            }
        }),
    };
    let typed_summary = super::StateSummary {
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
        input_registry: json!({"known_refs":[]}),
        node_output_refs: json!({"known_refs":[]}),
        reusable_outputs: None,
        tool_memory_projection: None,
        intent_slots: None,
        intent_context: None,
        capability_view: None,
        capability_ready: None,
        side_effect_lifecycle: None,
        todo_state: None,
        recovery_diagnostics: Some(json!({
            "available_attempt_keys": ["runtime.query.resolve"]
        })),
    };
    let error = decode_segmented_tool_call_with_memory_and_summary(
        &abort_call,
        "plan.propose_segment",
        PlannerRoundPhase::ProposeSegment,
        None,
        None,
        None,
        None,
        None,
        None,
        Some(&typed_summary),
    )
    .expect_err("unknown attempted_recovery entries should be rejected");
    assert!(error.to_string().contains("unknown entry"));
}

#[test]
fn decode_abort_intent_accepts_when_history_matches_and_no_recoverable_candidates() {
    let abort_call = ToolCall {
        id: "tool-abort".to_string(),
        name: "plan.abort_intent".to_string(),
        arguments: json!({
            "reason_code":"recovery_exhausted",
            "summary":"cannot continue with current intent",
            "evidence":{
              "attempted_recovery":["runtime.query.resolve"],
              "missing_refs":["inputs.unresolvable.ref"]
            },
            "user_fix_hint":"please provide a valid token contract"
        }),
    };
    let typed_summary = super::StateSummary {
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
        input_registry: json!({"known_refs":[]}),
        node_output_refs: json!({"known_refs":[]}),
        reusable_outputs: None,
        tool_memory_projection: None,
        intent_slots: None,
        intent_context: None,
        capability_view: None,
        capability_ready: None,
        side_effect_lifecycle: None,
        todo_state: None,
        recovery_diagnostics: Some(json!({
            "available_attempt_keys": ["runtime.query.resolve"]
        })),
    };
    let decoded = decode_segmented_tool_call_with_memory_and_summary(
        &abort_call,
        "plan.propose_segment",
        PlannerRoundPhase::ProposeSegment,
        Some(&CandidateContext::default()),
        None,
        None,
        None,
        None,
        None,
        Some(&typed_summary),
    )
    .expect("abort should be accepted with matched host history and no recoverable refs");
    assert!(matches!(
        decoded,
        DecodedSegmentedToolCall::Final(PlannerToolOutput::AbortIntent(_))
    ));
}

#[test]
fn plan_check_segment_uses_typed_summary_store_projections_for_write_gate_validation() {
    let mut context = CandidateContext::default();
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let protocol_path = format!("{manifest_dir}/../../fixtures/runner-local/intent-native-erc20-transfer/workspace/erc20.ais.yaml");
    let protocol_text = std::fs::read_to_string(protocol_path).expect("erc20 protocol fixture");
    let protocol: ais_sdk::ProtocolDocument =
        serde_yaml::from_str(protocol_text.as_str()).expect("protocol yaml");
    context.protocols.push(protocol);

    context.executable_candidates.queries.push(json!({
        "ref":"erc20@0.0.2/balance-of",
        "kind":"query"
    }));
    context.executable_candidates.actions.push(json!({
        "ref":"erc20@0.0.2/transfer",
        "kind":"action"
    }));

    context.detail_by_ref.insert(
        "erc20@0.0.2/balance-of".to_string(),
        json!({
            "ref":"erc20@0.0.2/balance-of",
            "kind":"query",
            "returns":[{"name":"balance","type":"uint256"}]
        }),
    );
    context.detail_by_ref.insert(
        "erc20@0.0.2/transfer".to_string(),
        json!({
            "ref":"erc20@0.0.2/transfer",
            "kind":"action",
            "risk_tags":["transfer"],
            "params":[
                {"name":"token","type":"asset","required":true},
                {"name":"to","type":"address","required":true},
                {"name":"amount","type":"uint256","required":true}
            ]
        }),
    );

    let check_call = ToolCall {
        id: "tool-check".to_string(),
        name: "plan.check_segment".to_string(),
        arguments: json!({
            "segment": {
                "segment_id":"seg-transfer",
                "cursor_in":"0",
                "cursor_out":"1",
                "done":false,
                "summary":"transfer erc20 using inputs.token",
                "steps":[
                    {"id":"q_balance","kind":"query","candidate_ref":"erc20@0.0.2/balance-of","inputs":{"owner":{"lit":"0xabc"},"token":{"lit":"0xdef"}}},
                    {"id":"g_assert","kind":"assert","depends_on":["q_balance"],"inputs":{"condition":{"lit":true}}},
                    {"id":"a_transfer","kind":"action","candidate_ref":"erc20@0.0.2/transfer","depends_on":["g_assert"],"inputs":{"token":{"ref":"inputs.token"},"to":{"lit":"0xabc"},"amount":{"lit":"1000"}}}
                ]
            }
        }),
    };

    let mut input_store = crate::agent::input_store::InputStore::default();
    input_store.upsert_user(
        "inputs.token",
        json!({
            "address": {"lit":"0x8464135c8F25Da09e49BC8782676a84730C318bC"},
            "decimals": "6"
        }),
        "test.prefilled.token",
    );

    let typed_summary = super::StateSummary {
        completed_segments: 0,
        completed_nodes: 0,
        plan_epoch: 0,
        paused_reason: None,
        done: false,
        previous_error: None,
        input_store: Some(input_store.to_projected_planning_value()),
        runtime_facts: None,
        input_binding: crate::agent::state_summary::InputBindingContract {
            schema: "ais-agent-input-binding-contract/0.0.1",
            bindable_namespace: "inputs",
            bindable_refs_source: "state_summary.input_store",
            bindable_refs_projection: "state_summary.input_registry.known_refs",
            known_refs_only: true,
            facts_bindable: false,
        },
        input_registry: json!({"known_refs":["inputs.token"]}),
        node_output_refs: json!({"known_refs":[]}),
        reusable_outputs: None,
        tool_memory_projection: None,
        intent_slots: None,
        intent_context: None,
        capability_view: None,
        capability_ready: None,
        side_effect_lifecycle: None,
        todo_state: None,
        recovery_diagnostics: Some(json!({
            "available_attempt_keys": []
        })),
    };

    let check_context = SegmentCheckContext {
        intent: "x".to_string(),
        session_id: "sess".to_string(),
        cursor: "0".to_string(),
        pack_snapshot_hash: "packhash".to_string(),
        chain_scope: vec!["eip155:1".to_string()],
        volatile_facts_policy: crate::policy::VolatileFactsPolicy::default(),
        known_input_refs: vec!["inputs.token".to_string()],
        grounding_fact_keys: vec![],
        current_todo_scope: None,
    };

    let decoded = decode_segmented_tool_call_with_memory_and_summary(
        &check_call,
        "plan.propose_segment",
        PlannerRoundPhase::ProposeSegment,
        Some(&context),
        Some(&check_context),
        None,
        None,
        None,
        None,
        Some(&typed_summary),
    )
    .expect("plan.check_segment should succeed using typed_summary store projections");

    let DecodedSegmentedToolCall::ToolMessage { content, .. } = decoded else {
        panic!("expected ToolMessage, got {decoded:?}");
    };
    let payload: Value = serde_json::from_str(content.as_str()).expect("payload json");
    if payload.get("ok").and_then(Value::as_bool) != Some(true) {
        panic!("expected ok=true, got payload={payload}");
    }
}

#[test]
fn begin_prompt_includes_snapshot_hash_for_plan_begin_echo() {
    let payload = render_begin_prompt_with_patch(
        &SegmentBeginRequest {
            intent: "x".to_string(),
            snapshot_hash: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                .to_string(),
            pack_snapshot_hash: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                .to_string(),
            catalog_hash: "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
                .to_string(),
            chain_scope: vec!["eip155:1".to_string()],
        },
        None,
    );
    let value: Value = serde_json::from_str(payload.as_str()).expect("json");
    assert_eq!(
        value.pointer("/snapshot_hash"),
        Some(&json!(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        ))
    );
    assert_eq!(
        value.pointer("/begin_contract/snapshot_hash_rule"),
        Some(&json!("must echo the provided snapshot_hash exactly"))
    );
}

#[test]
fn repeated_plan_check_failure_payload_is_structured() {
    let content = json!({
            "ok": false,
            "reason_code": "write_gate_missing",
            "issues": [
                {
                    "reason_code": "missing_gate_data_backing",
                    "family_reason_code": "missing_query_assert_branch_chain",
                    "step_id": "a_transfer",
                    "message": "write action must depend on assert/branch gate backed by query ancestry or explicit node outputs"
                }
            ]
        })
        .to_string();
    let payload =
        repeated_plan_check_failure_payload(content.as_str(), 3, 3, "plan.revise_segment");
    assert_eq!(
        payload.pointer("/error/code"),
        Some(&json!("repeated_plan_check_failure"))
    );
    assert_eq!(
        payload.pointer("/error/reason_code"),
        Some(&json!("write_gate_missing"))
    );
    assert_eq!(payload.pointer("/error/streak"), Some(&json!(3)));
    assert_eq!(
        payload.pointer("/error/step_ids/0"),
        Some(&json!("a_transfer"))
    );
}

#[test]
fn repeated_plan_check_failure_payload_keeps_unknown_input_ref_candidates() {
    let content = json!({
        "ok": false,
        "reason_code": "compile_error",
        "issues": [
            {
                "reference": "unknown_input_ref",
                "reason_code": "unknown_input_ref",
                "path": "steps[0].inputs.token.ref",
                "suggested_ref": "inputs.token.address",
                "candidates": ["inputs.token.address", "inputs.token.decimals", "inputs.owner"],
                "message": "unknown input ref"
            }
        ]
    })
    .to_string();
    let payload =
        repeated_plan_check_failure_payload(content.as_str(), 3, 3, "plan.revise_segment");
    assert_eq!(
        payload.pointer("/error/issues/0/reference"),
        Some(&json!("unknown_input_ref"))
    );
    assert_eq!(
        payload.pointer("/error/issues/0/suggested_ref"),
        Some(&json!("inputs.token.address"))
    );
    assert_eq!(
        payload.pointer("/error/issues/0/candidates/0"),
        Some(&json!("inputs.token.address"))
    );
}

#[test]
fn plan_check_failure_loop_guard_triggers_on_same_signature() {
    let content = json!({
        "ok": false,
        "reason_code": "write_gate_missing",
        "issues": [
            {
                "reason_code": "missing_gate_data_backing",
                "family_reason_code": "missing_query_assert_branch_chain",
                "step_id": "a_transfer",
                "message": "missing"
            }
        ]
    })
    .to_string();
    let signature = plan_check_failure_signature(content.as_str());
    let mut guard = PlanCheckFailureLoopGuard::default();
    assert!(!guard.observe(signature.clone()));
    assert!(!guard.observe(signature.clone()));
    assert!(guard.observe(signature));
}

#[test]
fn pre_finalize_segment_mismatch_payload_contains_signatures() {
    let payload = pre_finalize_segment_mismatch_payload(
        "plan.propose_segment",
        Some("checked_sig"),
        Some("final_sig"),
    );
    assert_eq!(
        payload.pointer("/error/code"),
        Some(&json!("pre_finalize_segment_mismatch"))
    );
    assert_eq!(
        payload.pointer("/error/checked_segment_signature"),
        Some(&json!("checked_sig"))
    );
    assert_eq!(
        payload.pointer("/error/finalized_segment_signature"),
        Some(&json!("final_sig"))
    );
}

#[test]
fn finalize_schema_repair_payload_classifies_missing_status() {
    let repair = finalize_schema_repair_payload(
        &RunnerError::Llm("invalid plan.propose_segment args: missing field `status`".to_string()),
        "plan.propose_segment",
        2,
        1,
        2,
    )
    .expect("missing status payload");
    assert_eq!(
        repair.payload.pointer("/error/reason_code"),
        Some(&json!("schema_missing_required_field"))
    );
    assert_eq!(
        repair.payload.pointer("/error/sub_reason_code"),
        Some(&json!("missing_status"))
    );
    assert_eq!(repair.sub_reason_code, "missing_status");
    assert_eq!(
        repair.payload.pointer("/error/repair_attempt"),
        Some(&json!(1))
    );
}

#[test]
fn finalize_schema_repair_payload_classifies_invalid_boolean_type() {
    let repair = finalize_schema_repair_payload(
        &RunnerError::Llm(
            "invalid plan.propose_segment args: invalid type: string \"false\", expected a boolean"
                .to_string(),
        ),
        "plan.propose_segment",
        3,
        2,
        2,
    )
    .expect("invalid boolean type payload");
    assert_eq!(repair.sub_reason_code, "invalid_boolean_type");
    assert_eq!(
        repair.payload.pointer("/error/reason_code"),
        Some(&json!("schema_invalid_type"))
    );
    assert_eq!(
        repair.payload.pointer("/error/sub_reason_code"),
        Some(&json!("invalid_boolean_type"))
    );
    assert_eq!(
        repair.payload.pointer("/error/typing_examples/bad/0/done"),
        Some(&json!("false"))
    );
}

#[test]
fn finalize_schema_repair_payload_classifies_non_actionable_not_ready_grounding() {
    let repair = finalize_schema_repair_payload(
            &RunnerError::Llm(
                "invalid plan.ground_intent args: status=proposed with ready_for_todos=false requires non-empty `questions` or `missing_refs`"
                    .to_string(),
            ),
            "plan.ground_intent",
            2,
            1,
            2,
        )
        .expect("non-actionable grounding repair payload");
    assert_eq!(repair.sub_reason_code, "grounding_not_ready_non_actionable");
    assert_eq!(
        repair.payload.pointer("/error/reason_code"),
        Some(&json!("schema_missing_required_field"))
    );
    assert_eq!(
        repair
            .payload
            .pointer("/error/required_any_of/1/missing_refs"),
        Some(&json!("non-empty array"))
    );
}

#[test]
fn non_finalize_tool_args_repair_payload_classifies_missing_segment() {
    let repair = non_finalize_tool_args_repair_payload(
        &RunnerError::Llm("invalid plan.check_segment args: missing field `segment`".to_string()),
        "plan.check_segment",
        3,
        1,
        2,
    )
    .expect("missing segment payload");
    assert_eq!(repair.sub_reason_code, "missing_segment");
    assert_eq!(
        repair.payload.pointer("/error/reason_code"),
        Some(&json!("schema_missing_required_field"))
    );
    assert_eq!(
        repair.payload.pointer("/error/shape/bad/raw"),
        Some(&json!("{\"segment\": {...}}"))
    );
}

#[test]
fn no_toolcall_repair_payload_contains_phase_finalize_and_allowed_tools() {
    let tools = segmented_planner_tools_for_phase(PlannerRoundPhase::ProposeSegment);
    let payload = no_toolcall_repair_payload(
        PlannerRoundPhase::ProposeSegment,
        "plan.propose_segment",
        1,
        1,
        2,
        &tools,
    );
    assert_eq!(
        payload.pointer("/error/reason_code"),
        Some(&json!("no_tool_calls"))
    );
    assert_eq!(
        payload.pointer("/error/phase"),
        Some(&json!("propose_segment"))
    );
    assert_eq!(
        payload.pointer("/error/finalize_tool"),
        Some(&json!("plan.propose_segment"))
    );
    let allowed = payload
        .pointer("/error/allowed_tools")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert!(
        allowed
            .iter()
            .any(|tool| tool.as_str() == Some("plan.propose_segment")),
        "allowed_tools={allowed:?}"
    );
}

#[test]
fn adjudicate_finalize_guard_payload_requires_finalize() {
    let payload =
        adjudicate_finalize_guard_payload("plan.revise_segment", 2, "empty_catalog_search_streak");
    assert_eq!(
        payload.pointer("/loop_guard/kind"),
        Some(&json!("adjudicate_budget_guard"))
    );
    assert_eq!(
        payload.pointer("/loop_guard/max_rounds"),
        Some(&json!(ADJUDICATE_MAX_TOOL_ROUNDS))
    );
    assert_eq!(
        payload.pointer("/loop_guard/contract"),
        Some(&json!(
            "Call `plan.revise_segment` exactly once as the last tool call in this response."
        ))
    );
}

#[test]
fn planner_llm_transcript_writes_full_request_and_response() {
    let provider = ScriptedLlmProvider::from_responses(vec![Ok(CompleteWithToolsResponse {
        assistant_content: Some("final".to_string()),
        tool_calls: vec![ToolCall {
            id: "tool-final".to_string(),
            name: "plan.propose_todos".to_string(),
            arguments: json!({
                "status":"proposed",
                "summary":"done",
                "todos":[{"title":"t1"}]
            }),
        }],
    })]);
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("ais-runner-llm-transcript-{stamp}.md"));
    let mut planner =
        LlmSegmentedIntentPlanner::new(provider).with_llm_transcript(Some(path.clone()), false);
    let _ = planner
        .propose_todos(TodoPlanningRequest {
            intent: "plan todos".to_string(),
            session: SegmentPlanningSession {
                session_id: "sess-1".to_string(),
                snapshot_hash: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    .to_string(),
                cursor: "0".to_string(),
                max_rounds: 8,
                max_segments: 8,
            },
            state_summary: Some(json!({})),
            typed_summary: None,
        })
        .expect("planner run");
    let text = fs::read_to_string(&path).expect("transcript text");
    assert!(text.contains("### Request"));
    assert!(text.contains("### Response"));
    assert!(text.contains("\"plan.propose_todos\""));
    assert!(text.contains("\"todos\""));
    let _ = fs::remove_file(path);
}

#[test]
fn planner_llm_transcript_append_mode_keeps_existing_content() {
    let provider = ScriptedLlmProvider::from_responses(vec![Ok(CompleteWithToolsResponse {
        assistant_content: Some("final".to_string()),
        tool_calls: vec![ToolCall {
            id: "tool-final".to_string(),
            name: "plan.propose_todos".to_string(),
            arguments: json!({
                "status":"proposed",
                "summary":"done",
                "todos":[{"title":"t1"}]
            }),
        }],
    })]);
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("ais-runner-llm-transcript-append-{stamp}.md"));
    fs::write(&path, "# seed\n").expect("seed");
    let mut planner =
        LlmSegmentedIntentPlanner::new(provider).with_llm_transcript(Some(path.clone()), true);
    let _ = planner
        .propose_todos(TodoPlanningRequest {
            intent: "plan todos".to_string(),
            session: SegmentPlanningSession {
                session_id: "sess-1".to_string(),
                snapshot_hash: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    .to_string(),
                cursor: "0".to_string(),
                max_rounds: 8,
                max_segments: 8,
            },
            state_summary: Some(json!({})),
            typed_summary: None,
        })
        .expect("planner run");
    let text = fs::read_to_string(&path).expect("transcript text");
    assert!(text.starts_with("# seed\n"));
    assert!(text.contains("### Request"));
    let _ = fs::remove_file(path);
}
