use super::super::context::budget_policy::ToolMemoryBudgetPolicy;
use super::super::todos::TodoSpec;
use super::*;
use ais_engine::{EngineEvent, EngineEventType, Executor, ExecutorOutput, RouterExecutor};
use ais_llm::{CompleteWithToolsResponse, LlmProviderError, ScriptedLlmProvider, ToolCall};
use serde_json::{json, Map};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn test_agent_command() -> AgentCommand {
    AgentCommand {
        plan: None,
        intent: Some("check balances and transfer".to_string()),
        intent_file: None,
        workspace: None,
        config: PathBuf::from("runner.local.yaml"),
        pack: None,
        runtime: None,
        events_jsonl: None,
        trace: None,
        agent_trace_jsonl: None,
        checkpoint: None,
        profile: AgentProfile::Standard,
        llm_script_jsonl: None,
        verbose: false,
        verbose_llm: false,
        approvals_mode: None,
        max_iterations: None,
        max_planner_rounds: None,
        max_tool_rounds: None,
        max_index_candidates: None,
        planner_context_token_budget: None,
        llm_transcript_path: None,
        llm_transcript_append: false,
        format: OutputFormat::Json,
    }
}

fn test_segmented_context() -> SegmentedAgentContext {
    SegmentedAgentContext::new(
        "check balances and transfer".to_string(),
        intent_segmented::SegmentPlanningSession {
            session_id: "sess-1".to_string(),
            snapshot_hash: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                .to_string(),
            cursor: "cursor-0".to_string(),
            max_rounds: 4,
            max_segments: 4,
        },
        InputStore::default(),
        super::super::runtime_facts_store::RuntimeFactsStore::default(),
        TodoBoard::bootstrap("check balances and transfer"),
        4,
        4,
        ToolMemoryBudgetPolicy::TOOL_MEMORY_PROJECTION_DEFAULT_TOKENS,
        checkpoint_ext::AgentCheckpointExtensions::default(),
    )
}

struct QueryAutofillSuccessExecutor;

impl Executor for QueryAutofillSuccessExecutor {
    fn execute(&self, _node: &Value, _runtime: &mut Value) -> Result<ExecutorOutput, String> {
        Ok(ExecutorOutput {
            result: json!({
                "decimals": 18
            }),
            writes: Map::new(),
            side_effects: Vec::new(),
        })
    }
}

struct QueryAutofillEmptyExecutor;

impl Executor for QueryAutofillEmptyExecutor {
    fn execute(&self, _node: &Value, _runtime: &mut Value) -> Result<ExecutorOutput, String> {
        Ok(ExecutorOutput {
            result: json!({}),
            writes: Map::new(),
            side_effects: Vec::new(),
        })
    }
}

fn native_erc20_fixture_command_for_candidates() -> AgentCommand {
    let fixture_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/runner-local/intent-native-erc20-transfer");
    let workspace_dir = fixture_root.join("workspace");
    let pack_path = workspace_dir.join("safe-defi.ais-pack.yaml");
    let config_path = fixture_root.join("config/runner.local.yaml");
    let intent_file = fixture_root.join("intent/intent.txt");
    AgentCommand {
        plan: None,
        intent: None,
        intent_file: Some(intent_file),
        workspace: Some(workspace_dir),
        config: config_path,
        pack: Some(pack_path),
        runtime: None,
        events_jsonl: None,
        trace: None,
        agent_trace_jsonl: None,
        checkpoint: None,
        profile: AgentProfile::DemoScripted,
        llm_script_jsonl: None,
        verbose: false,
        verbose_llm: false,
        approvals_mode: Some(crate::cli::ApprovalsMode::Safe),
        max_iterations: Some(16),
        max_planner_rounds: Some(6),
        max_tool_rounds: None,
        max_index_candidates: Some(64),
        planner_context_token_budget: None,
        llm_transcript_path: None,
        llm_transcript_append: false,
        format: OutputFormat::Json,
    }
}

fn conditional_transfer_segment() -> PlanSketchSegment {
    serde_json::from_value(json!({
        "segment_id":"seg-conditional-transfer",
        "cursor_in":"c2",
        "cursor_out":"c3",
        "done":false,
        "steps":[
            {
                "id":"q_native_balance",
                "kind":"query",
                "candidate_ref":"evm-native-utils@0.0.1/native-balance",
                "inputs":{"addr":{"ref":"inputs.owner"}}
            },
            {
                "id":"q_token_balance",
                "kind":"query",
                "candidate_ref":"erc20@0.0.2/balance-of",
                "inputs":{
                    "owner":{"ref":"inputs.owner"},
                    "token":{"object":{
                        "address":{"lit":"0x8464135c8F25Da09e49BC8782676a84730C318bC"},
                        "chain_ref":{"lit":"eip155:31338"}
                    }}
                }
            },
            {
                "id":"check_balance_condition",
                "kind":"assert",
                "depends_on":["q_native_balance","q_token_balance"],
                "inputs":{},
                "when":{"cel":"nodes.q_native_balance.outputs.balance > to_atomic(100, 18) && nodes.q_token_balance.outputs.balance > to_atomic(100, inputs.token.decimals)"}
            },
            {
                "id":"a_native_transfer",
                "kind":"action",
                "candidate_ref":"evm-native-utils@0.0.1/native-transfer",
                "depends_on":["check_balance_condition"],
                "inputs":{
                    "amount":{"lit":5000000000000000000u64},
                    "to":{"ref":"inputs.recipient"}
                }
            },
            {
                "id":"a_token_transfer",
                "kind":"action",
                "candidate_ref":"erc20@0.0.2/transfer",
                "depends_on":["check_balance_condition"],
                "inputs":{
                    "amount":{"lit":10000000},
                    "to":{"ref":"inputs.recipient"},
                    "token":{"object":{
                        "address":{"lit":"0x8464135c8F25Da09e49BC8782676a84730C318bC"},
                        "chain_ref":{"lit":"eip155:31338"}
                    }}
                }
            }
        ],
        "extensions":{}
    }))
    .expect("segment")
}

fn conditional_transfer_planned_segment() -> PlannedSegment {
    PlannedSegment {
        todo_id: "todo.transfer".to_string(),
        summary: Some("conditional transfer".to_string()),
        segment: conditional_transfer_segment(),
        cursor_next: "c3".to_string(),
        done: false,
        issues: vec![],
    }
}

fn scripted_grounding_unavailable_missing_input_response(
    question_id: &str,
    question_text: &str,
    message: &str,
) -> CompleteWithToolsResponse {
    CompleteWithToolsResponse {
        assistant_content: Some("grounding unavailable".to_string()),
        tool_calls: vec![ToolCall {
            id: "tool-ground-unavailable".to_string(),
            name: "plan.ground_intent".to_string(),
            arguments: json!({
                "status":"unavailable",
                "issues":[],
                "error": {
                    "reason_code":"missing_required_input",
                    "message": message,
                    "details": {
                        "questions":[
                            {
                                "id": question_id,
                                "question": question_text,
                                "required": true,
                                "options": []
                            }
                        ],
                        "recovery_exhaustion": {
                            "unresolved_refs": [question_id],
                            "reasons": ["no_query_candidates"],
                            "attempt_trace_id": "missing_resolution:grounding:grounding:need_user_input"
                        }
                    }
                }
            }),
        }],
    }
}

#[test]
fn compile_guard_matches_store_aware_segment_compile_for_bound_decimals() {
    let command = native_erc20_fixture_command_for_candidates();
    let pack = crate::policy::load_pack_document(command.pack.as_ref().expect("fixture pack path"))
        .expect("pack");
    let candidate_context = super::super::candidates::build_candidate_context_for_agent(
        &command,
        Some(&pack),
        super::super::candidates::DEFAULT_MAX_INDEX_CANDIDATES,
    )
    .expect("candidate context build")
    .expect("workspace candidates");
    let mut context = test_segmented_context();
    let state = EngineRunnerState::default();

    let _ = super::upsert_store_value_with_source(
        context.input_store_mut(),
        "owner",
        json!("0x70997970c51812dc3a010c7d01b50e0d17dc79c8"),
        InputValueLayer::Seed,
        "user",
        100,
        "test.owner",
    );
    let _ = super::upsert_store_value_with_source(
        context.input_store_mut(),
        "recipient",
        json!("0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"),
        InputValueLayer::Seed,
        "user",
        100,
        "test.recipient",
    );
    let _ = super::upsert_store_value_with_source(
        context.input_store_mut(),
        "token.decimals",
        json!(6),
        InputValueLayer::Seed,
        "user",
        100,
        "test.token.decimals",
    );
    context.refresh_state_summary(&state, false);

    let mut planned = conditional_transfer_planned_segment();
    let chain_scope = ["eip155:31338".to_string()];
    let guard_plan = compile_guard(
        &mut planned,
        &context,
        &candidate_context,
        None,
        &chain_scope,
        crate::policy::VolatileFactsPolicy::default(),
    )
    .expect("compile_guard should accept store-backed decimals");

    let known_refs = super::super::known_input_refs_from_typed_summary(context.typed_summary());
    let compile_plan = super::super::compile_segment_plan_with_snapshot_hash(
        context.intent(),
        context.session().session_id.as_str(),
        context.session().cursor.as_str(),
        &planned.segment,
        &candidate_context,
        context.session().snapshot_hash.as_str(),
        &chain_scope,
        known_refs.as_slice(),
        Some(context.runtime_facts_store()),
        Some(context.input_store()),
    )
    .expect("direct compile should match compile_guard");

    let guard_node_ids = guard_plan
        .nodes
        .iter()
        .filter_map(|node| node.get("id").and_then(Value::as_str))
        .collect::<Vec<_>>();
    let compile_node_ids = compile_plan
        .nodes
        .iter()
        .filter_map(|node| node.get("id").and_then(Value::as_str))
        .collect::<Vec<_>>();
    assert_eq!(guard_node_ids, compile_node_ids);
}

#[test]
fn compile_guard_and_segment_compile_fail_identically_when_decimals_are_absent() {
    let command = native_erc20_fixture_command_for_candidates();
    let pack = crate::policy::load_pack_document(command.pack.as_ref().expect("fixture pack path"))
        .expect("pack");
    let candidate_context = super::super::candidates::build_candidate_context_for_agent(
        &command,
        Some(&pack),
        super::super::candidates::DEFAULT_MAX_INDEX_CANDIDATES,
    )
    .expect("candidate context build")
    .expect("workspace candidates");
    let mut context = test_segmented_context();
    let state = EngineRunnerState::default();

    let _ = super::upsert_store_value_with_source(
        context.input_store_mut(),
        "owner",
        json!("0x70997970c51812dc3a010c7d01b50e0d17dc79c8"),
        InputValueLayer::Seed,
        "user",
        100,
        "test.owner",
    );
    let _ = super::upsert_store_value_with_source(
        context.input_store_mut(),
        "recipient",
        json!("0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"),
        InputValueLayer::Seed,
        "user",
        100,
        "test.recipient",
    );
    context.refresh_state_summary(&state, false);

    let chain_scope = ["eip155:31338".to_string()];
    let mut planned = conditional_transfer_planned_segment();
    let guard_error = compile_guard(
        &mut planned,
        &context,
        &candidate_context,
        None,
        &chain_scope,
        crate::policy::VolatileFactsPolicy::default(),
    )
    .expect_err("compile_guard should reject missing decimals");
    let known_refs = super::super::known_input_refs_from_typed_summary(context.typed_summary());
    let compile_error = super::super::compile_segment_plan_with_snapshot_hash(
        context.intent(),
        context.session().session_id.as_str(),
        context.session().cursor.as_str(),
        &planned.segment,
        &candidate_context,
        context.session().snapshot_hash.as_str(),
        &chain_scope,
        known_refs.as_slice(),
        Some(context.runtime_facts_store()),
        Some(context.input_store()),
    )
    .expect_err("direct compile should reject missing decimals too");

    assert_eq!(
        guard_error.pointer("/reason_code"),
        compile_error.pointer("/reason_code")
    );
    assert_eq!(
        guard_error.pointer("/issues/0/reason_code"),
        compile_error.pointer("/issues/0/reason_code")
    );
    assert_eq!(
        guard_error.pointer("/issues/0/required_fact"),
        compile_error.pointer("/issues/0/required_fact")
    );
}

#[test]
fn precheck_missing_input_refs_for_current_todo_uses_required_facts_and_state_summary() {
    let mut context = test_segmented_context();
    let intent = context.intent.clone();
    context.todo_board_mut().replace_from_specs(
        intent.as_str(),
        &[TodoSpec {
            title: "todo".to_string(),
            required_facts: vec![
                "inputs.owner".to_string(),
                "token.decimals".to_string(),
                "native_balance".to_string(),
            ],
            produced_facts: vec![],
            acceptance: vec![],
        }],
    );
    *context.packed_summary_mut() = Some(json!({
        "input_registry": {
            "known_refs": ["inputs.owner"]
        }
    }));
    *context.typed_summary_mut() = Some(super::super::state_summary::StateSummary {
        completed_segments: 0,
        completed_nodes: 0,
        plan_epoch: 0,
        paused_reason: None,
        done: false,
        previous_error: None,
        input_store: None,
        runtime_facts: None,
        input_binding: super::super::state_summary::InputBindingContract {
            schema: "ais-agent-input-binding-contract/0.0.1",
            bindable_namespace: "inputs",
            bindable_refs_source: "state_summary.input_store",
            bindable_refs_projection: "state_summary.input_registry.known_refs",
            known_refs_only: true,
            facts_bindable: false,
        },
        input_registry: json!({"known_refs": ["inputs.owner"]}),
        node_output_refs: json!({"known_refs": []}),
        reusable_outputs: None,
        tool_memory_projection: None,
        intent_slots: None,
        intent_context: None,
        capability_view: None,
        capability_ready: None,
        side_effect_lifecycle: None,
        todo_state: None,
        recovery_diagnostics: None,
    });

    let refs = super::missing_resolution::precheck_missing_input_refs_for_current_todo(
        &context,
        context.typed_summary(),
    );
    assert_eq!(
        refs,
        vec![
            "inputs.native_balance".to_string(),
            "inputs.token.decimals".to_string()
        ]
    );
}

#[test]
fn input_ref_guard_accepts_legal_known_ref() {
    let segment: PlanSketchSegment = serde_json::from_value(json!({
        "segment_id":"seg_ok",
        "cursor_in":"0",
        "cursor_out":"1",
        "done":false,
        "steps":[
            {
                "id":"q1",
                "kind":"query",
                "candidate_ref":"demo@0.0.2/quote",
                "inputs":{"owner":{"ref":"inputs.owner"}}
            }
        ]
    }))
    .expect("segment");

    let canonical =
        super::super::canonicalize_segment_input_refs(&segment, &["inputs.owner".to_string()], &[])
            .expect("guard should accept known ref");
    let canonical_value = serde_json::to_value(canonical).expect("segment value");
    assert_eq!(
        canonical_value.pointer("/steps/0/inputs/owner/ref"),
        Some(&json!("inputs.owner"))
    );
}

#[test]
fn input_ref_guard_canonicalizes_safe_alias_ref() {
    let segment: PlanSketchSegment = serde_json::from_value(json!({
        "segment_id":"seg_alias",
        "cursor_in":"0",
        "cursor_out":"1",
        "done":false,
        "steps":[
            {
                "id":"q1",
                "kind":"query",
                "candidate_ref":"demo@0.0.2/quote",
                "inputs":{"token":{"ref":"runtime.inputs.fact:token.value"}}
            }
        ]
    }))
    .expect("segment");

    let canonical = super::super::canonicalize_segment_input_refs(
        &segment,
        &["inputs.fact.token".to_string()],
        &[],
    )
    .expect("guard should canonicalize alias");
    let canonical_value = serde_json::to_value(canonical).expect("segment value");
    assert_eq!(
        canonical_value.pointer("/steps/0/inputs/token/ref"),
        Some(&json!("inputs.fact.token"))
    );
}

#[test]
fn input_ref_guard_rejects_unknown_ref_with_ranked_candidates() {
    let segment: PlanSketchSegment = serde_json::from_value(json!({
        "segment_id":"seg_bad",
        "cursor_in":"0",
        "cursor_out":"1",
        "done":false,
        "steps":[
            {
                "id":"q1",
                "kind":"query",
                "candidate_ref":"demo@0.0.2/quote",
                "inputs":{"token":{"ref":"inputs.fact:tokn"}}
            }
        ]
    }))
    .expect("segment");

    let error = super::super::canonicalize_segment_input_refs(
        &segment,
        &["inputs.fact.token".to_string(), "inputs.owner".to_string()],
        &[],
    )
    .expect_err("guard should reject unknown ref before compile");
    assert_eq!(
        error.pointer("/reason_code").and_then(Value::as_str),
        Some("compile_error")
    );
    assert_eq!(
        error.pointer("/issues/0/reference"),
        Some(&json!("unknown_input_ref"))
    );
    assert_eq!(
        error.pointer("/issues/0/path"),
        Some(&json!("steps[0].inputs.token.ref"))
    );
    assert_eq!(
        error.pointer("/issues/0/candidates/0"),
        Some(&json!("inputs.fact.token"))
    );
}

#[test]
fn input_ref_guard_prefers_grounded_token_alias_candidates() {
    let segment: PlanSketchSegment = serde_json::from_value(json!({
        "segment_id":"seg_bad_alias",
        "cursor_in":"0",
        "cursor_out":"1",
        "done":false,
        "steps":[
            {
                "id":"q1",
                "kind":"query",
                "candidate_ref":"demo@0.0.2/quote",
                "inputs":{"token":{"ref":"inputs.fact:token"}}
            }
        ]
    }))
    .expect("segment");

    let error = super::super::canonicalize_segment_input_refs(
        &segment,
        &[
            "inputs.owner".to_string(),
            "inputs.token.decimals".to_string(),
            "inputs.tst_token_address".to_string(),
        ],
        &["token".to_string()],
    )
    .expect_err("guard should reject unknown ref before compile");
    assert_eq!(
        error.pointer("/issues/0/suggested_ref"),
        Some(&json!("inputs.tst_token_address"))
    );
    assert_eq!(
        error.pointer("/issues/0/candidates/0"),
        Some(&json!("inputs.tst_token_address"))
    );
}

#[test]
fn input_ref_guard_does_not_suggest_irrelevant_semantic_refs() {
    let segment: PlanSketchSegment = serde_json::from_value(json!({
        "segment_id":"seg_bad_irrelevant",
        "cursor_in":"0",
        "cursor_out":"1",
        "done":false,
        "steps":[
            {
                "id":"q1",
                "kind":"query",
                "candidate_ref":"demo@0.0.2/quote",
                "inputs":{"token":{"ref":"inputs.fact:token"}}
            }
        ]
    }))
    .expect("segment");

    let error = super::super::canonicalize_segment_input_refs(
        &segment,
        &["inputs.owner".to_string(), "inputs.amount".to_string()],
        &["token".to_string()],
    )
    .expect_err("guard should reject unknown ref before compile");
    let candidates = error
        .pointer("/issues/0/candidates")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert!(candidates.is_empty(), "candidates={candidates:?}");
}

#[test]
fn tool_memory_projection_budget_uses_remaining_ratio() {
    let planner_usage = json!({
        "context_soft_limit_tokens": 100_000,
        "context_remaining_tokens": 90_000
    });
    let budget = resolve_tool_memory_projection_token_budget(Some(&planner_usage), None);
    assert_eq!(budget, 40_000);

    let planner_usage = json!({
        "context_soft_limit_tokens": 100_000,
        "context_remaining_tokens": 10_000
    });
    let budget = resolve_tool_memory_projection_token_budget(Some(&planner_usage), None);
    assert_eq!(budget, 20_000);
}

#[test]
fn tool_memory_projection_budget_falls_back_to_runtime_and_absolute_remaining() {
    let runtime_usage = json!({
        "context_remaining_tokens": 14_000
    });
    let budget = resolve_tool_memory_projection_token_budget(None, Some(&runtime_usage));
    assert!(budget > ToolMemoryBudgetPolicy::TOOL_MEMORY_PROJECTION_MIN_TOKENS);
    assert!(budget < ToolMemoryBudgetPolicy::TOOL_MEMORY_PROJECTION_MAX_TOKENS);

    let budget = resolve_tool_memory_projection_token_budget(None, None);
    assert_eq!(
        budget,
        ToolMemoryBudgetPolicy::TOOL_MEMORY_PROJECTION_DEFAULT_TOKENS
    );
}

#[test]
fn planning_memory_store_budget_defaults_without_usage() {
    let budget = resolve_planning_memory_store_budget(None, None);
    assert_eq!(budget.max_entries, 48);
    assert_eq!(budget.max_entry_chars, 8_000);
    assert_eq!(budget.max_total_chars, 120_000);
}

#[test]
fn planning_memory_store_budget_tightens_under_critical_pressure() {
    let runtime_usage = json!({
        "context_soft_limit_tokens": 100_000,
        "context_remaining_tokens": 2_000
    });
    let budget = resolve_planning_memory_store_budget(None, Some(&runtime_usage));
    assert!(budget.max_entries <= 16);
    assert!(budget.max_entry_chars <= 3_000);
    assert!(budget.max_total_chars <= 40_000);
}

#[test]
fn refresh_tool_memory_projection_keeps_planning_memory_entries() {
    let mut planner = LlmSegmentedIntentPlanner::new(ScriptedLlmProvider::from_responses(vec![]));
    let snapshot = super::super::planning_memory::PlanningMemorySnapshot {
            snapshot_hash: "snap-prune".to_string(),
            tool_cache: vec![
                super::super::planning_memory::PlanningMemoryCacheEntry {
                    key: "catalog.discover:1".to_string(),
                    content: json!({
                        "protocols":[{"protocol":"erc20","actions":[{"ref":"a"}],"queries":[{"ref":"q"}],"chains":["eip155:*"]}]
                    })
                    .to_string(),
                },
                super::super::planning_memory::PlanningMemoryCacheEntry {
                    key: "catalog.discover:2".to_string(),
                    content: json!({
                        "protocols":[{"protocol":"uniswap","actions":[{"ref":"b"}],"queries":[{"ref":"q2"}],"chains":["eip155:*"]}]
                    })
                    .to_string(),
                },
                super::super::planning_memory::PlanningMemoryCacheEntry {
                    key: "catalog.discover:1".to_string(),
                    content: json!({
                        "query":"transfer",
                        "returned_matches":0,
                        "results":[]
                    })
                    .to_string(),
                },
                super::super::planning_memory::PlanningMemoryCacheEntry {
                    key: "catalog.discover:2".to_string(),
                    content: json!({
                        "query":"transfer",
                        "returned_matches":4,
                        "results":[
                            {"ref":"proto@0.0.1/action-1","kind":"action"},
                            {"ref":"proto@0.0.2/action-2","kind":"action"}
                        ]
                    })
                    .to_string(),
                },
                super::super::planning_memory::PlanningMemoryCacheEntry {
                    key: "catalog.discover:3".to_string(),
                    content: json!({
                        "query":"swap",
                        "returned_matches":1,
                        "results":[
                            {"ref":"proto@0.0.1/action-s","kind":"action"}
                        ]
                    })
                    .to_string(),
                },
                super::super::planning_memory::PlanningMemoryCacheEntry {
                    key: "get_candidate_detail:1".to_string(),
                    content: json!({
                        "details":[
                            {"ref":"proto@0.0.1/action-1","kind":"action","params":[{"name":"owner","required":true}],"execution_chains":["eip155:*"]}
                        ]
                    })
                    .to_string(),
                },
                super::super::planning_memory::PlanningMemoryCacheEntry {
                    key: "get_candidate_detail:2".to_string(),
                    content: json!({
                        "details":[
                            {"ref":"proto@0.0.1/action-2","kind":"action","params":[{"name":"to","required":true}],"execution_chains":["eip155:1"]}
                        ]
                    })
                    .to_string(),
                },
                super::super::planning_memory::PlanningMemoryCacheEntry {
                    key: "guide.get:1".to_string(),
                    content: json!({"kind":"topic","topic":{"topic":"cel","summary":"first"}}).to_string(),
                },
                super::super::planning_memory::PlanningMemoryCacheEntry {
                    key: "guide.get:2".to_string(),
                    content: json!({"kind":"topic","topic":{"topic":"valueref","summary":"second"}}).to_string(),
                },
            ],
        };
    assert!(planner.restore_planning_memory_from_checkpoint(Some(
        &serde_json::to_value(&snapshot).expect("snapshot value")
    )));

    let before = serde_json::from_value::<super::super::planning_memory::PlanningMemorySnapshot>(
        planner
            .planning_memory_checkpoint_value()
            .expect("checkpoint"),
    )
    .expect("snapshot decode")
    .tool_cache
    .len();

    let mut context = test_segmented_context();
    *context.packed_summary_mut() = Some(json!({
        "context_budget": {
            "pressure_mode": "critical"
        }
    }));

    refresh_tool_memory_projection(&mut context, &mut planner, &EngineRunnerState::default());

    let after = serde_json::from_value::<super::super::planning_memory::PlanningMemorySnapshot>(
        planner
            .planning_memory_checkpoint_value()
            .expect("checkpoint"),
    )
    .expect("snapshot decode")
    .tool_cache
    .len();
    assert_eq!(after, before);
    let diagnostics = planner.llm_usage_value();
    assert!(diagnostics
        .pointer("/diagnostics/memory_prune_runs")
        .is_none());
    assert!(diagnostics
        .pointer("/diagnostics/memory_pruned_by_tool")
        .is_none());
    assert!(
        diagnostics
            .pointer("/diagnostics/memory_projection_budget_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            > 0
    );
    let projection = context
        .tool_memory_projection()
        .clone()
        .expect("projection");
    assert!(
        projection.get("recent").is_some() || projection.get("cached_refs").is_some(),
        "expected projection to include either full/summary recent entries or skeleton cached_refs; projection={projection:?}"
    );
    assert!(projection.get("estimated_tokens").is_some());
    assert!(projection.get("token_budget").is_some());
}

#[test]
fn refresh_tool_memory_projection_records_empty_projection_estimate() {
    let mut planner = LlmSegmentedIntentPlanner::new(ScriptedLlmProvider::from_responses(vec![]));
    let snapshot = super::super::planning_memory::PlanningMemorySnapshot {
        snapshot_hash: "snap-empty-pressure".to_string(),
        tool_cache: vec![
            super::super::planning_memory::PlanningMemoryCacheEntry {
                key: "guide.get:legacy".to_string(),
                content: "{unparseable}".to_string(),
            },
            super::super::planning_memory::PlanningMemoryCacheEntry {
                key: "catalog.discover:legacy".to_string(),
                content: "{unparseable}".to_string(),
            },
        ],
    };
    assert!(planner.restore_planning_memory_from_checkpoint(Some(
        &serde_json::to_value(&snapshot).expect("snapshot value")
    )));

    let mut context = test_segmented_context();
    *context.packed_summary_mut() = Some(json!({
        "context_budget": {
            "pressure_mode": "critical"
        }
    }));

    refresh_tool_memory_projection(&mut context, &mut planner, &EngineRunnerState::default());
    let usage = planner.llm_usage_value();
    assert!(usage
        .pointer("/diagnostics/memory_projection_empty_due_to_pressure_total")
        .is_none());
    assert_eq!(
        usage.pointer("/diagnostics/memory_projection_estimated_tokens"),
        Some(&json!(0))
    );
    assert!(context.tool_memory_projection().is_none());
}

#[test]
fn apply_segment_stores_projects_query_and_action_outputs() {
    let segment: PlanSketchSegment = serde_json::from_value(json!({
        "segment_id":"seg_1",
        "cursor_in":"0",
        "cursor_out":"1",
        "done":false,
        "steps":[
            {
                "id":"q_balance",
                "kind":"query",
                "candidate_ref":"demo@0.0.2/quote",
                "inputs":{},
                "stores":{"balance":"facts.balance"}
            },
            {
                "id":"a_transfer",
                "kind":"action",
                "candidate_ref":"demo@0.0.2/swap",
                "inputs":{},
                "stores":{"tx_hash":"tx.hash","confirmed":"tx.confirmed"}
            }
        ]
    }))
    .expect("segment");
    let state = EngineRunnerState {
        runtime: json!({
            "nodes": {
                "seg_1/q_balance": {"outputs":{"balance":"100"}},
                "seg_1/a_transfer": {"outputs":{"outputs":{"tx_hash":"0xabc","confirmed":true}}}
            }
        }),
        ..EngineRunnerState::default()
    };
    let mut input_store = InputStore::default();
    let mut runtime_facts_store = super::super::runtime_facts_store::RuntimeFactsStore::default();

    super::phase_machine::segment_exec::apply_segment_stores_from_runtime_with_runtime_facts(
        &segment,
        &state,
        &mut runtime_facts_store,
        &mut input_store,
        false,
    );

    assert!(input_store.get("facts.balance").is_none());
    assert_eq!(
        runtime_facts_store
            .get("facts.balance")
            .and_then(|entry| entry.value.as_str()),
        Some("100")
    );
    assert_eq!(
        input_store
            .get("inputs.tx.hash")
            .and_then(|entry| entry.value.as_str()),
        Some("0xabc")
    );
    assert_eq!(
        input_store
            .get("inputs.tx.confirmed")
            .and_then(|entry| entry.value.as_bool()),
        Some(true)
    );
}

#[test]
fn bind_segment_todo_id_writes_segment_extension() {
    let mut segment: PlanSketchSegment = serde_json::from_value(json!({
        "segment_id":"seg_1",
        "cursor_in":"0",
        "cursor_out":"1",
        "done":false,
        "steps":[
            {"id":"q1","kind":"query","candidate_ref":"demo@0.0.2/quote","inputs":{}}
        ]
    }))
    .expect("segment");
    bind_segment_todo_id(&mut segment, "todo_1");
    assert_eq!(segment.extensions.get("todo_id"), Some(&json!("todo_1")));
}

#[test]
fn annotate_events_with_todo_adds_agent_extension() {
    let segment: PlanSketchSegment = serde_json::from_value(json!({
        "segment_id":"seg_1",
        "cursor_in":"0",
        "cursor_out":"1",
        "done":false,
        "steps":[
            {"id":"q1","kind":"query","candidate_ref":"demo@0.0.2/quote","inputs":{}}
        ]
    }))
    .expect("segment");
    let mut node_event = EngineEvent::new(EngineEventType::NodeReady);
    node_event.node_id = Some("seg_1/q1".to_string());
    let events = vec![
        EngineEventRecord::new("run-1", 1, "1970-01-01T00:00:00Z", node_event),
        EngineEventRecord::new(
            "run-1",
            2,
            "1970-01-01T00:00:00Z",
            EngineEvent::new(EngineEventType::PlanReplaced),
        ),
    ];

    let annotated = super::phase_machine::segment_exec::annotate_events_with_todo(
        events.as_slice(),
        &segment,
        "todo_1",
    );
    let ext0 = Value::Object(annotated[0].event.extensions.clone());
    let ext1 = Value::Object(annotated[1].event.extensions.clone());
    assert_eq!(ext0.pointer("/agent/todo_id"), Some(&json!("todo_1")));
    assert_eq!(ext0.pointer("/agent/segment_id"), Some(&json!("seg_1")));
    assert_eq!(ext0.pointer("/agent/step_id"), Some(&json!("q1")));
    assert_eq!(ext1.pointer("/agent/todo_id"), Some(&json!("todo_1")));
}

#[test]
fn build_todo_receipt_collects_completed_nodes_and_tx_hashes() {
    let segment: PlanSketchSegment = serde_json::from_value(json!({
        "segment_id":"seg_1",
        "cursor_in":"0",
        "cursor_out":"1",
        "done":false,
        "steps":[
            {"id":"q1","kind":"query","candidate_ref":"demo@0.0.2/quote","inputs":{}},
            {"id":"a1","kind":"action","candidate_ref":"demo@0.0.2/swap","inputs":{}}
        ]
    }))
    .expect("segment");
    let planned = PlannedSegment {
        todo_id: "todo_1".to_string(),
        summary: None,
        segment,
        cursor_next: "1".to_string(),
        done: false,
        issues: Vec::new(),
    };
    let state = EngineRunnerState {
        completed_node_ids: vec!["seg_1/q1".to_string()],
        paused_reason: Some("need_user_confirm:seg_1/a1".to_string()),
        runtime: json!({
            "nodes":{
                "seg_1/a1":{"outputs":{"tx_hash":"0xruntime_should_be_ignored"}}
            }
        }),
        ..EngineRunnerState::default()
    };
    let mut side_effect_event = EngineEvent::new(EngineEventType::SideEffectObserved);
    side_effect_event.data.insert(
        "record".to_string(),
        json!({
            "schema":"ais-side-effect-record/0.1.0",
            "idempotency_key":"tx:seg_1/a1:0xabc",
            "node_id":"seg_1/a1",
            "effect_type":"tx",
            "chain":"eip155:1",
            "execution_type":"evm_send",
            "tx_hash":"0xabc",
            "status":"sent",
            "observed_at":"1970-01-01T00:00:00Z"
        }),
    );
    let events = vec![
        EngineEventRecord::new("run-1", 2, "1970-01-01T00:00:00Z", side_effect_event),
        EngineEventRecord::new(
            "run-1",
            3,
            "1970-01-01T00:00:00Z",
            EngineEvent::new(EngineEventType::NeedUserConfirm),
        ),
    ];
    let mut checkpoint_ledger = RunnerCheckpointLedger::default();
    checkpoint_ledger.absorb_events(events.as_slice());
    let receipt = super::receipt_view::build_segment_todo_receipt(
        planned.todo_id.as_str(),
        &planned.segment,
        EngineRunStatus::Paused,
        &state,
        events.as_slice(),
        Some(&checkpoint_ledger),
    );
    assert_eq!(receipt.todo_id, "todo_1");
    assert_eq!(receipt.segment_id, "seg_1");
    assert_eq!(receipt.status, "paused");
    assert_eq!(receipt.completed_node_ids, vec!["seg_1/q1".to_string()]);
    assert_eq!(receipt.tx_hashes, vec!["0xabc".to_string()]);
    assert_eq!(
        state.runtime.pointer("/nodes/seg_1~1a1/outputs/tx_hash"),
        Some(&json!("0xruntime_should_be_ignored"))
    );
    assert_eq!(
        checkpoint_ledger.side_effects()[0].tx_hash,
        Some("0xabc".to_string())
    );
    assert_eq!(
        receipt.event_types,
        vec![
            "need_user_confirm".to_string(),
            "side_effect_observed".to_string()
        ]
    );
    assert_eq!(receipt.event_count, 2);
}

#[test]
fn build_todo_receipt_collects_ledger_tx_hashes_for_native_and_erc20_writes() {
    let segment: PlanSketchSegment = serde_json::from_value(json!({
            "segment_id":"seg_1",
            "cursor_in":"0",
            "cursor_out":"1",
            "done":false,
            "steps":[
                {"id":"native_send","kind":"action","candidate_ref":"demo@0.0.2/native_send","inputs":{}},
                {"id":"erc20_send","kind":"action","candidate_ref":"demo@0.0.2/erc20_send","inputs":{}}
            ]
        }))
        .expect("segment");
    let planned = PlannedSegment {
        todo_id: "todo_1".to_string(),
        summary: None,
        segment,
        cursor_next: "1".to_string(),
        done: false,
        issues: Vec::new(),
    };
    let state = EngineRunnerState {
        completed_node_ids: vec![
            "seg_1/native_send".to_string(),
            "seg_1/erc20_send".to_string(),
        ],
        runtime: json!({
            "nodes":{
                "seg_1/native_send":{"outputs":{"tx_hash":"0xruntime_native"}},
                "seg_1/erc20_send":{"outputs":{"tx_hash":"0xruntime_erc20"}}
            }
        }),
        ..EngineRunnerState::default()
    };
    let mut native_event = EngineEvent::new(EngineEventType::SideEffectObserved);
    native_event.data.insert(
        "record".to_string(),
        json!({
            "schema":"ais-side-effect-record/0.1.0",
            "idempotency_key":"tx:seg_1/native_send:0xnative",
            "node_id":"seg_1/native_send",
            "effect_type":"tx",
            "chain":"eip155:1",
            "execution_type":"evm_send",
            "tx_hash":"0xnative",
            "status":"sent",
            "observed_at":"1970-01-01T00:00:00Z"
        }),
    );
    let mut erc20_event = EngineEvent::new(EngineEventType::SideEffectObserved);
    erc20_event.data.insert(
        "record".to_string(),
        json!({
            "schema":"ais-side-effect-record/0.1.0",
            "idempotency_key":"tx:seg_1/erc20_send:0xerc20",
            "node_id":"seg_1/erc20_send",
            "effect_type":"tx",
            "chain":"eip155:1",
            "execution_type":"erc20_send",
            "tx_hash":"0xerc20",
            "status":"sent",
            "observed_at":"1970-01-01T00:00:01Z"
        }),
    );
    let events = vec![
        EngineEventRecord::new("run-1", 3, "1970-01-01T00:00:00Z", native_event),
        EngineEventRecord::new("run-1", 4, "1970-01-01T00:00:01Z", erc20_event),
    ];
    let mut checkpoint_ledger = RunnerCheckpointLedger::default();
    checkpoint_ledger.absorb_events(events.as_slice());

    let receipt = super::receipt_view::build_segment_todo_receipt(
        planned.todo_id.as_str(),
        &planned.segment,
        EngineRunStatus::Completed,
        &state,
        events.as_slice(),
        Some(&checkpoint_ledger),
    );
    assert_eq!(
        receipt.tx_hashes,
        vec!["0xerc20".to_string(), "0xnative".to_string()]
    );
    assert_eq!(
        state
            .runtime
            .pointer("/nodes/seg_1~1native_send/outputs/tx_hash"),
        Some(&json!("0xruntime_native"))
    );
    assert_eq!(
        state
            .runtime
            .pointer("/nodes/seg_1~1erc20_send/outputs/tx_hash"),
        Some(&json!("0xruntime_erc20"))
    );
}

#[test]
fn sync_todo_progress_receipt_tx_hashes_from_ledger_updates_runtime_on_restore() {
    let mut state = EngineRunnerState {
        runtime: json!({
            "agent":{
                "todo_progress":{
                    "current_todo":{
                        "id":"todo_1",
                        "receipt":{
                            "todo_id":"todo_1",
                            "node_ids":["seg_1/native_send","seg_1/erc20_send"],
                            "tx_hashes":["0xstale"]
                        }
                    },
                    "todos":[
                        {
                            "id":"todo_1",
                            "receipt":{
                                "todo_id":"todo_1",
                                "node_ids":["seg_1/native_send","seg_1/erc20_send"],
                                "tx_hashes":["0xstale"]
                            }
                        }
                    ]
                }
            },
            "nodes":{
                "seg_1/native_send":{"outputs":{"tx_hash":"0xruntime_native"}},
                "seg_1/erc20_send":{"outputs":{"tx_hash":"0xruntime_erc20"}}
            }
        }),
        ..EngineRunnerState::default()
    };
    let mut native_event = EngineEvent::new(EngineEventType::SideEffectObserved);
    native_event.data.insert(
        "record".to_string(),
        json!({
            "schema":"ais-side-effect-record/0.1.0",
            "idempotency_key":"tx:seg_1/native_send:0xnative",
            "node_id":"seg_1/native_send",
            "effect_type":"tx",
            "chain":"eip155:1",
            "execution_type":"evm_send",
            "tx_hash":"0xnative",
            "status":"sent",
            "observed_at":"1970-01-01T00:00:00Z"
        }),
    );
    let mut erc20_event = EngineEvent::new(EngineEventType::SideEffectObserved);
    erc20_event.data.insert(
        "record".to_string(),
        json!({
            "schema":"ais-side-effect-record/0.1.0",
            "idempotency_key":"tx:seg_1/erc20_send:0xerc20",
            "node_id":"seg_1/erc20_send",
            "effect_type":"tx",
            "chain":"eip155:1",
            "execution_type":"erc20_send",
            "tx_hash":"0xerc20",
            "status":"sent",
            "observed_at":"1970-01-01T00:00:01Z"
        }),
    );
    let mut checkpoint_ledger = RunnerCheckpointLedger::default();
    checkpoint_ledger.absorb_events(&[
        EngineEventRecord::new("run-1", 3, "1970-01-01T00:00:00Z", native_event),
        EngineEventRecord::new("run-1", 4, "1970-01-01T00:00:01Z", erc20_event),
    ]);

    super::receipt_view::project_todo_progress_receipts_from_ledger(
        &mut state.runtime,
        &checkpoint_ledger,
    );

    assert_eq!(
        state
            .runtime
            .pointer("/agent/todo_progress/current_todo/receipt/tx_hashes"),
        Some(&json!(["0xerc20", "0xnative"]))
    );
    assert_eq!(
        state
            .runtime
            .pointer("/agent/todo_progress/todos/0/receipt/tx_hashes"),
        Some(&json!(["0xerc20", "0xnative"]))
    );
    assert_eq!(
        state
            .runtime
            .pointer("/nodes/seg_1~1native_send/outputs/tx_hash"),
        Some(&json!("0xruntime_native"))
    );
    assert_eq!(
        state
            .runtime
            .pointer("/nodes/seg_1~1erc20_send/outputs/tx_hash"),
        Some(&json!("0xruntime_erc20"))
    );
}

#[test]
fn completion_gate_does_not_treat_intent_context_as_acceptance_source() {
    let mut board = TodoBoard::bootstrap("native + erc20 conditional transfer");
    board.replace_from_specs(
        "native + erc20 conditional transfer",
        &[
            TodoSpec {
                title: "Transfer native".to_string(),
                required_facts: vec![],
                produced_facts: vec!["facts.native.transfer_done".to_string()],
                acceptance: vec!["facts.native.transfer_done".to_string()],
            },
            TodoSpec {
                title: "Transfer erc20".to_string(),
                required_facts: vec![],
                produced_facts: vec!["facts.erc20.transfer_done".to_string()],
                acceptance: vec!["facts.erc20.transfer_done".to_string()],
            },
        ],
    );
    board.mark_current_done();

    let typed_summary = super::super::state_summary::StateSummary {
        completed_segments: 0,
        completed_nodes: 0,
        plan_epoch: 0,
        paused_reason: None,
        done: false,
        previous_error: None,
        input_store: None,
        runtime_facts: None,
        input_binding: super::super::state_summary::InputBindingContract {
            schema: "ais-agent-input-binding-contract/0.0.1",
            bindable_namespace: "inputs",
            bindable_refs_source: "state_summary.input_store",
            bindable_refs_projection: "state_summary.input_registry.known_refs",
            known_refs_only: true,
            facts_bindable: false,
        },
        input_registry: json!({"known_refs": []}),
        node_output_refs: json!({"known_refs": []}),
        reusable_outputs: None,
        tool_memory_projection: None,
        intent_slots: None,
        intent_context: Some(json!({
            "facts": {
                "facts": {
                    "native": {"transfer_done": true},
                    "erc20": {"transfer_done": true}
                }
            }
        })),
        capability_view: None,
        capability_ready: None,
        side_effect_lifecycle: None,
        todo_state: None,
        recovery_diagnostics: None,
    };

    let done = advance_todo_after_execute_completion(&mut board, Some(&typed_summary), false);
    let runtime = board.to_runtime_value();
    assert!(!done);
    assert!(board.current().is_some());
    assert_eq!(runtime.pointer("/next_seq"), Some(&json!(4)));
    assert_eq!(
        runtime.pointer("/todos/2/title"),
        Some(&json!("Continue intent segment 3"))
    );
}

#[test]
fn completion_gate_opens_follow_up_when_acceptance_is_not_satisfied() {
    let mut board = TodoBoard::bootstrap("native + erc20 conditional transfer");
    board.replace_from_specs(
        "native + erc20 conditional transfer",
        &[
            TodoSpec {
                title: "Transfer native".to_string(),
                required_facts: vec![],
                produced_facts: vec!["facts.native.transfer_done".to_string()],
                acceptance: vec!["facts.native.transfer_done".to_string()],
            },
            TodoSpec {
                title: "Transfer erc20".to_string(),
                required_facts: vec![],
                produced_facts: vec!["facts.erc20.transfer_done".to_string()],
                acceptance: vec!["facts.erc20.transfer_done".to_string()],
            },
        ],
    );
    board.mark_current_done();

    let typed_summary = super::super::state_summary::StateSummary {
        completed_segments: 0,
        completed_nodes: 0,
        plan_epoch: 0,
        paused_reason: None,
        done: false,
        previous_error: None,
        input_store: None,
        runtime_facts: None,
        input_binding: super::super::state_summary::InputBindingContract {
            schema: "ais-agent-input-binding-contract/0.0.1",
            bindable_namespace: "inputs",
            bindable_refs_source: "state_summary.input_store",
            bindable_refs_projection: "state_summary.input_registry.known_refs",
            known_refs_only: true,
            facts_bindable: false,
        },
        input_registry: json!({"known_refs": []}),
        node_output_refs: json!({"known_refs": []}),
        reusable_outputs: None,
        tool_memory_projection: None,
        intent_slots: None,
        intent_context: Some(json!({
            "facts": {
                "facts": {
                    "native": {"transfer_done": true}
                }
            }
        })),
        capability_view: None,
        capability_ready: None,
        side_effect_lifecycle: None,
        todo_state: None,
        recovery_diagnostics: None,
    };

    let done = advance_todo_after_execute_completion(&mut board, Some(&typed_summary), false);
    let runtime = board.to_runtime_value();
    assert!(!done);
    assert_eq!(runtime.pointer("/current_todo/id"), Some(&json!("todo_3")));
    assert_eq!(
        runtime.pointer("/current_todo/title"),
        Some(&json!("Continue intent segment 3"))
    );
}

#[test]
fn missing_required_input_payload_from_pause_maps_need_user_input_event() {
    let state = EngineRunnerState {
        paused_reason: Some("need_user_input:seg_1/q_owner".to_string()),
        ..EngineRunnerState::default()
    };
    let mut event = EngineEvent::new(EngineEventType::NeedUserInput);
    event.node_id = Some("seg_1/q_owner".to_string());
    event.data = serde_json::Map::from_iter([
        ("reason_code".to_string(), json!("missing_required_input")),
        (
            "reason".to_string(),
            json!("missing_inputs_or_runtime_refs"),
        ),
        (
            "details".to_string(),
            json!({
                "missing_refs":["inputs.owner","params.owner"],
                "suggested_paths":["inputs.owner","params.owner"],
                "questions":[{"id":"owner","question":"Provide owner","required":true,"options":[]}],
                "issues":[{"reason_code":"missing_required_input"}]
            }),
        ),
    ]);
    let record = EngineEventRecord::new("run-1", 4, "1970-01-01T00:00:00Z", event);

    let payload =
        missing_required_input_payload_from_pause(&state, std::slice::from_ref(&record), 2)
            .expect("missing payload");
    assert_eq!(
        payload.get("reason_code").and_then(Value::as_str),
        Some("missing_required_input")
    );
    assert_eq!(
        payload.pointer("/missing_refs/0"),
        Some(&json!("inputs.owner"))
    );
    assert_eq!(
        payload.pointer("/suggested_paths/0"),
        Some(&json!("inputs.owner"))
    );
    assert_eq!(
        payload.pointer("/questions/0/id"),
        Some(&json!("inputs.owner"))
    );
}

#[test]
fn apply_intent_grounding_normalizes_prefixed_keys_and_wrapped_values() {
    let mut state = EngineRunnerState::default();
    let mut input_store = InputStore::default();
    let resolved_inputs = std::collections::BTreeMap::from([
        (
            "inputs.owner".to_string(),
            json!({"confidence": 100, "value": "0xabc"}),
        ),
        (
            "inputs.token".to_string(),
            json!({"address": "0xtoken", "chain_id": "eip155:1"}),
        ),
    ]);
    let confidence = std::collections::BTreeMap::from([("inputs.owner".to_string(), 95u8)]);

    let summary = apply_intent_grounding(
        &mut state,
        &mut input_store,
        &resolved_inputs,
        &std::collections::BTreeMap::new(),
        &confidence,
        "check balance and transfer",
    );

    assert_eq!(
        state.runtime.pointer("/inputs/owner"),
        Some(&json!("0xabc"))
    );
    assert_eq!(
        state.runtime.pointer("/inputs/token/address"),
        Some(&json!("0xtoken"))
    );
    assert!(state.runtime.pointer("/inputs/inputs/owner").is_none());
    assert!(input_store.get("inputs.inputs.owner").is_none());
    assert_eq!(
        input_store
            .get("inputs.owner")
            .and_then(|entry| entry.value.as_str()),
        Some("0xabc")
    );
    assert_eq!(
        input_store.get_projected("inputs.token").and_then(|entry| {
            entry
                .value
                .pointer("/address")
                .and_then(Value::as_str)
                .map(str::to_string)
        }),
        Some("0xtoken".to_string())
    );
    assert!(summary.applied.iter().any(|item| item == "inputs.owner:95"));
}

#[test]
fn apply_intent_grounding_accepts_confidence_from_wrapped_value() {
    let mut state = EngineRunnerState::default();
    let mut input_store = InputStore::default();
    let resolved_inputs = std::collections::BTreeMap::from([(
        "inputs.recipient".to_string(),
        json!({"confidence": 99, "value": "0xdef"}),
    )]);

    let summary = apply_intent_grounding(
        &mut state,
        &mut input_store,
        &resolved_inputs,
        &std::collections::BTreeMap::new(),
        &std::collections::BTreeMap::new(),
        "transfer when balance > 10",
    );

    assert_eq!(
        state.runtime.pointer("/inputs/recipient"),
        Some(&json!("0xdef"))
    );
    assert!(summary
        .applied
        .iter()
        .any(|item| item == "inputs.recipient:99"));
}

#[test]
fn apply_intent_grounding_skips_hybrid_fact_like_input_slots() {
    let mut state = EngineRunnerState::default();
    let mut input_store = InputStore::default();
    let resolved_inputs = std::collections::BTreeMap::from([
        ("inputs.owner".to_string(), json!("0xabc")),
        ("fact:token".to_string(), json!("USDC")),
    ]);

    let summary = apply_intent_grounding(
        &mut state,
        &mut input_store,
        &resolved_inputs,
        &std::collections::BTreeMap::new(),
        &std::collections::BTreeMap::new(),
        "transfer",
    );

    assert_eq!(
        state.runtime.pointer("/inputs/owner"),
        Some(&json!("0xabc"))
    );
    assert!(state.runtime.pointer("/inputs/fact:token").is_none());
    assert!(input_store.get("inputs.fact:token").is_none());
    assert!(summary
        .skipped_low_confidence
        .iter()
        .any(|item| item == "inputs.fact:token:invalid_input_slot"));
}

#[test]
fn apply_intent_grounding_rule_extracts_balance_threshold_for_native_erc20_intent_fixture() {
    let mut state = EngineRunnerState::default();
    let mut input_store = InputStore::default();
    let intent_facts = std::collections::BTreeMap::from([(
        "condition".to_string(),
        json!("native_balance > 100 AND tst_balance > 100"),
    )]);
    let intent_text = fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/runner-local/intent-native-erc20-transfer/intent/intent.txt"),
    )
    .expect("native+erc20 intent fixture");

    let summary = apply_intent_grounding(
        &mut state,
        &mut input_store,
        &std::collections::BTreeMap::new(),
        &intent_facts,
        &std::collections::BTreeMap::new(),
        intent_text.as_str(),
    );

    assert_eq!(
        state.runtime.pointer("/inputs/balance_threshold"),
        Some(&json!(100))
    );
    assert_eq!(
        input_store
            .get("inputs.balance_threshold")
            .and_then(|entry| entry.meta.provenance.as_deref()),
        Some("rule_extracted.balance_threshold")
    );
    assert!(summary
        .deterministic_applied
        .iter()
        .any(|item| item == "inputs.balance_threshold:100:rule_extracted"));
}

#[test]
fn apply_intent_grounding_rule_skips_non_match_without_balance_comparator() {
    let mut state = EngineRunnerState::default();
    let mut input_store = InputStore::default();
    let intent_facts = std::collections::BTreeMap::from([(
        "condition".to_string(),
        json!("nodes.q_balance.outputs.balance != null"),
    )]);

    let summary = apply_intent_grounding(
        &mut state,
        &mut input_store,
        &std::collections::BTreeMap::new(),
        &intent_facts,
        &std::collections::BTreeMap::new(),
        "check balance if available",
    );

    assert!(state.runtime.pointer("/inputs/balance_threshold").is_none());
    assert!(summary.deterministic_applied.is_empty());
    assert!(summary
        .deterministic_skipped
        .iter()
        .any(|item| item == "inputs.balance_threshold:no_high_confidence_match"));
}

#[test]
fn apply_intent_grounding_rule_conflict_prefers_rule_extracted_threshold() {
    let mut state = EngineRunnerState::default();
    let mut input_store = InputStore::default();
    let resolved_inputs = std::collections::BTreeMap::from([(
        "inputs.balance_threshold".to_string(),
        json!({"confidence": 100, "value": 88}),
    )]);
    let intent_facts = std::collections::BTreeMap::from([(
        "condition".to_string(),
        json!("native_balance > 100 AND tst_balance > 100"),
    )]);

    let summary = apply_intent_grounding(
        &mut state,
        &mut input_store,
        &resolved_inputs,
        &intent_facts,
        &std::collections::BTreeMap::new(),
        "native + erc20 conditional transfer",
    );

    assert_eq!(
        state.runtime.pointer("/inputs/balance_threshold"),
        Some(&json!(100))
    );
    assert!(summary.deterministic_conflicts.iter().any(|item| {
        item.contains("inputs.balance_threshold") && item.contains("policy=rule_extracted_over_llm")
    }));
}

#[test]
fn intent_grounding_ready_for_todos_accepts_legacy_false_without_questions() {
    let state = EngineRunnerState {
        runtime: json!({
            "agent": {
                "intent_grounding": {
                    "ready_for_todos": false,
                    "questions": [],
                    "resolved_inputs": {"owner":"0xabc"}
                }
            }
        }),
        ..EngineRunnerState::default()
    };
    assert!(intent_grounding_ready_for_todos(&state));
}

#[test]
fn intent_grounding_ready_for_todos_respects_false_with_questions() {
    let state = EngineRunnerState {
        runtime: json!({
            "agent": {
                "intent_grounding": {
                    "ready_for_todos": false,
                    "questions": [{"id":"owner","question":"owner?"}],
                    "resolved_inputs": {"owner":"0xabc"}
                }
            }
        }),
        ..EngineRunnerState::default()
    };
    assert!(!intent_grounding_ready_for_todos(&state));
}

#[test]
fn grounding_planner_call_failed_falls_back_instead_of_hard_fail() {
    let command = test_agent_command();
    let mut state = EngineRunnerState::default();
    let mut context = test_segmented_context();
    let _ = super::upsert_store_value_with_source(
        context.input_store_mut(),
        "token.address",
        json!("0x8464135c8F25Da09e49BC8782676a84730C318bC"),
        InputValueLayer::Derived,
        "test_seed",
        90,
        "test.seed.token.address",
    );
    context.refresh_state_summary(&state, false);
    let provider = ScriptedLlmProvider::from_responses(vec![Err(LlmProviderError::CallFailed {
        reason: "grounding transport unavailable".to_string(),
    })]);
    let mut planner = LlmSegmentedIntentPlanner::new(provider);
    let candidate_context = CandidateContext::default();

    let ready = bootstrap_intent_grounding_if_needed(
        &command,
        &mut planner,
        &mut state,
        &mut context,
        &candidate_context,
        None,
        false,
    )
    .expect("grounding planner-call failure should fallback");
    assert!(ready);
    assert_eq!(
        state.runtime.pointer("/agent/intent_grounding/status"),
        Some(&json!("fallback"))
    );
    assert_eq!(
        state
            .runtime
            .pointer("/agent/intent_grounding/ready_for_todos"),
        Some(&json!(true))
    );
    assert_eq!(
        state.runtime.pointer("/agent/intent_grounding/reason_code"),
        Some(&json!("planner_call_failed"))
    );
    assert_eq!(
        state
            .runtime
            .pointer("/agent/intent_grounding/input_binding/bindable_refs_source"),
        Some(&json!("state_summary.input_store"))
    );
    assert_eq!(
        state
            .runtime
            .pointer("/agent/intent_grounding/input_binding/bindable_refs_projection"),
        Some(&json!("state_summary.input_registry.known_refs"))
    );
}

#[test]
fn grounding_planner_call_failed_keeps_todo_bootstrap_recoverable() {
    let command = test_agent_command();
    let mut state = EngineRunnerState {
        runtime: json!({
            "agent": {
                "capability_view": {
                    "ready": true
                }
            }
        }),
        ..EngineRunnerState::default()
    };
    let mut context = test_segmented_context();
    context.refresh_state_summary(&state, false);
    let provider = ScriptedLlmProvider::from_responses(vec![
        Err(LlmProviderError::CallFailed {
            reason: "grounding transport unavailable".to_string(),
        }),
        Ok(CompleteWithToolsResponse {
            assistant_content: Some("todo draft".to_string()),
            tool_calls: vec![ToolCall {
                id: "tool-todos".to_string(),
                name: "plan.propose_todos".to_string(),
                arguments: json!({
                    "status":"proposed",
                    "todos":[
                        {"title":"Prepare transfer"}
                    ]
                }),
            }],
        }),
    ]);
    let mut planner = LlmSegmentedIntentPlanner::new(provider);
    let candidate_context = CandidateContext::default();

    let ready = bootstrap_intent_grounding_if_needed(
        &command,
        &mut planner,
        &mut state,
        &mut context,
        &candidate_context,
        None,
        false,
    )
    .expect("grounding planner-call failure should fallback");
    assert!(ready);
    bootstrap_todos_if_needed(
        &command,
        &mut planner,
        &mut state,
        &mut context,
        &candidate_context,
        None,
        false,
    )
    .expect("todo bootstrap should remain recoverable");

    assert_eq!(
        state
            .runtime
            .pointer("/agent/todo_progress/current_todo/title"),
        Some(&json!("Prepare transfer"))
    );
    assert_eq!(
        state
            .runtime
            .pointer("/agent/todo_progress/current_todo/status"),
        Some(&json!("todo"))
    );
    assert!(context.previous_error.is_none());
}

#[test]
fn planning_failure_checkpoint_save_error_preserves_primary_error() {
    let mut command = test_agent_command();
    let mut checkpoint_dir = std::env::temp_dir();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time must be monotonic")
        .as_nanos();
    checkpoint_dir.push(format!(
        "ais-runner-orchestrator-checkpoint-dir-{}-{nanos}",
        std::process::id()
    ));
    fs::create_dir_all(&checkpoint_dir).expect("create checkpoint directory");
    command.checkpoint = Some(checkpoint_dir.clone());

    let active_plan = super::empty_plan_document();
    let active_plan_hash = super::hash_plan(&active_plan).expect("hash active plan");
    let mut checkpoint_ledger = RunnerCheckpointLedger::default();
    let checkpoint_extensions = checkpoint_ext::AgentCheckpointExtensions::default();
    let input_store = InputStore::default();
    let runtime_facts_store = super::super::runtime_facts_store::RuntimeFactsStore::default();
    let planning_error = RunnerError::Llm("primary planner failure".to_string());

    let mut probe_state = EngineRunnerState::default();
    let mut audit_attempt = crate::audit_contract::AuditStreamAttempt::fresh();
    let probe_error = super::checkpoint_flow::record_planning_failure_event_and_checkpoint(
        &command,
        "run-probe",
        &active_plan_hash,
        &active_plan,
        &mut probe_state,
        &mut checkpoint_ledger,
        None,
        &input_store,
        &runtime_facts_store,
        &checkpoint_extensions,
        &planning_error,
        1,
        &mut audit_attempt,
    )
    .expect_err("checkpoint write should fail when path is a directory");
    assert!(matches!(probe_error, RunnerError::CheckpointSave { .. }));

    let mut state = EngineRunnerState::default();
    let ckpt = super::checkpoint_flow::CheckpointGuard {
        command: &command,
        run_id: "run-main",
        active_plan_hash: &active_plan_hash,
        active_plan: &active_plan,
    };
    let returned = record_planning_failure_preserving_primary_error(
        &ckpt,
        &mut state,
        &mut checkpoint_ledger,
        None,
        &input_store,
        &runtime_facts_store,
        &checkpoint_extensions,
        1,
        planning_error,
        &mut audit_attempt,
    );

    assert_eq!(state.next_seq, 1);
    match returned {
        RunnerError::Llm(message) => assert_eq!(message, "primary planner failure"),
        other => panic!("expected primary llm error, got {other:?}"),
    }

    let _ = fs::remove_dir_all(&checkpoint_dir);
}

#[test]
fn compile_error_missing_required_input_payload_extracts_inputs_refs() {
    let payload = compile_error_missing_required_input_payload(
        &json!({
            "reason_code":"compile_error",
            "message":"segment compile failed",
            "issues":[
                {
                    "reference":"unknown_input_ref",
                    "message":"unknown input ref `inputs.owner.value`; suggested_ref=inputs.owner"
                }
            ]
        }),
        2,
    )
    .expect("payload must exist");
    assert_eq!(
        payload.get("reason_code").and_then(Value::as_str),
        Some("missing_required_input")
    );
    assert_eq!(
        payload.pointer("/missing_refs/0"),
        Some(&json!("inputs.owner"))
    );
    assert_eq!(
        payload.pointer("/suggested_paths/0"),
        Some(&json!("inputs.owner"))
    );
    assert_eq!(payload.pointer("/questions/0/id"), Some(&json!("owner")));
    assert_eq!(payload.get("round").and_then(Value::as_u64), Some(2));
}

#[test]
fn compile_error_missing_required_input_payload_uses_unknown_ref_candidates() {
    let payload = compile_error_missing_required_input_payload(
        &json!({
            "reason_code":"compile_error",
            "message":"segment compile failed",
            "issues":[
                {
                    "reference":"unknown_input_ref",
                    "raw_ref":"inputs.fact:token",
                    "normalized_ref":"inputs.fact.token",
                    "candidates":["inputs.tst_token_address","inputs.token.decimals"]
                }
            ]
        }),
        2,
    )
    .expect("payload must exist");
    assert_eq!(
        payload.pointer("/missing_refs/0"),
        Some(&json!("inputs.token.decimals"))
    );
    assert_eq!(
        payload.pointer("/missing_refs/1"),
        Some(&json!("inputs.tst_token_address"))
    );
}

#[test]
fn compile_error_missing_required_input_payload_ignores_non_inputs_unknown_refs() {
    let payload = compile_error_missing_required_input_payload(
        &json!({
            "reason_code":"compile_error",
            "issues":[
                {
                    "reference":"unknown_input_ref",
                    "message":"unknown input ref `nodes.q_balance.outputs.balance`"
                }
            ]
        }),
        1,
    );
    assert!(payload.is_none());
}

#[test]
fn compile_error_missing_required_input_payload_ignores_non_input_namespace_required_fact() {
    let payload = compile_error_missing_required_input_payload(
        &json!({
            "reason_code":"write_gate_missing",
            "issues":[
                {
                    "kind":"write_gate_missing",
                    "reason_code":"missing_required_fact",
                    "required_fact":"params.owner"
                }
            ]
        }),
        1,
    );
    assert!(payload.is_none());
}

#[test]
fn compile_error_missing_required_input_payload_extracts_write_gate_required_fact() {
    let payload = compile_error_missing_required_input_payload(
            &json!({
                "reason_code":"write_gate_missing",
                "message":"segment write preconditions are not satisfied",
                "issues":[
                    {
                        "kind":"write_gate_missing",
                        "reason_code":"missing_token_decimals",
                        "required_fact":"token.decimals",
                        "message":"token decimals unavailable; add decimals query (e.g. erc20/decimals) or return missing_required_input"
                    }
                ]
            }),
            4,
        )
        .expect("payload must exist");
    assert_eq!(
        payload.get("reason_code").and_then(Value::as_str),
        Some("missing_required_input")
    );
    assert_eq!(
        payload.pointer("/missing_refs/0"),
        Some(&json!("inputs.token.decimals"))
    );
    assert_eq!(
        payload.pointer("/questions/0/id"),
        Some(&json!("token.decimals"))
    );
    assert_eq!(payload.get("round").and_then(Value::as_u64), Some(4));
}

#[test]
fn compile_error_missing_required_input_payload_accepts_generic_required_fact() {
    let payload = compile_error_missing_required_input_payload(
        &json!({
            "reason_code":"write_gate_missing",
            "issues":[
                {
                    "kind":"write_gate_missing",
                    "reason_code":"missing_required_fact",
                    "required_fact":"quote.slippage_bps",
                    "message":"required_fact=`quote.slippage_bps`"
                }
            ]
        }),
        2,
    )
    .expect("payload must exist");
    assert_eq!(
        payload.pointer("/missing_refs/0"),
        Some(&json!("inputs.quote.slippage_bps"))
    );
}

#[test]
fn compile_error_missing_required_input_payload_keeps_object_ref_generic() {
    let payload = compile_error_missing_required_input_payload(
        &json!({
            "reason_code":"compile_error",
            "message":"segment compile failed",
            "issues":[
                {
                    "reference":"unknown_input_ref",
                    "message":"unknown input ref `inputs.token`; suggested_ref=inputs.token"
                }
            ]
        }),
        3,
    )
    .expect("payload must exist");

    assert_eq!(
        payload.pointer("/missing_refs"),
        Some(&json!(["inputs.token"]))
    );
    assert_eq!(
        payload.pointer("/questions"),
        Some(&json!([{
            "id":"token",
            "question":"Please provide `token`",
            "required":true,
            "options":[]
        }]))
    );
    assert_eq!(payload.get("round").and_then(Value::as_u64), Some(3));
}

#[test]
fn missing_required_input_refs_keep_object_ref_generic() {
    let payload = json!({
        "missing_refs": ["inputs.owner", "inputs.token"]
    });
    let refs = super::super::missing_resolution::missing_required_input_refs(&payload);
    assert_eq!(
        refs,
        vec!["inputs.owner".to_string(), "inputs.token".to_string()]
    );
}

#[test]
fn detect_grounding_non_actionable_pause_requires_empty_questions_and_missing_refs() {
    let mut state = EngineRunnerState {
        paused_reason: Some("missing_required_input".to_string()),
        runtime: json!({
            "agent": {
                "missing_required_input": {
                    "message": "intent grounding missing inputs",
                    "questions": [],
                    "missing_refs": [],
                    "issues": [{"reason_code":"missing_required_input"}]
                }
            }
        }),
        ..EngineRunnerState::default()
    };
    let detected = detect_grounding_non_actionable_pause(&state).expect("must detect");
    assert_eq!(detected.message, "intent grounding missing inputs");
    assert_eq!(detected.issues.len(), 1);

    state.runtime = json!({
        "agent": {
            "missing_required_input": {
                "message": "actionable",
                "questions": [{"id":"token.decimals","question":"token decimals?"}],
                "missing_refs": []
            }
        }
    });
    assert!(detect_grounding_non_actionable_pause(&state).is_none());
}

#[test]
fn terminal_grounding_non_actionable_fallback_creates_actionable_payload() {
    let mut state = EngineRunnerState {
        paused_reason: Some("missing_required_input".to_string()),
        runtime: json!({
            "agent": {
                "missing_required_input": {
                    "message": "intent grounding missing inputs",
                    "questions": [],
                    "missing_refs": []
                }
            }
        }),
        ..EngineRunnerState::default()
    };
    let mut context = test_segmented_context();
    context.refresh_state_summary(&state, false);

    apply_grounding_non_actionable_terminal_fallback(
        &mut state,
        &mut context,
        &GroundingNonActionablePause {
            message: "intent grounding missing inputs".to_string(),
            issues: vec![],
        },
    );

    assert_eq!(
        state.paused_reason.as_deref(),
        Some("missing_required_input")
    );
    assert_eq!(
        state
            .runtime
            .pointer("/agent/missing_required_input/questions/0/id"),
        Some(&json!("intent.clarification"))
    );
    assert_eq!(
        state
            .runtime
            .pointer("/agent/missing_required_input/missing_refs/0"),
        Some(&json!("inputs.intent.clarification"))
    );
    assert_eq!(
        state.runtime.pointer("/agent/intent_grounding/status"),
        Some(&json!("unavailable"))
    );
    assert_eq!(
        state.runtime.pointer("/agent/intent_grounding/reason_code"),
        Some(&json!(GROUNDING_NON_ACTIONABLE_REASON_CODE))
    );
}

#[test]
fn grounding_non_actionable_retry_is_bounded() {
    assert_eq!(
        grounding_non_actionable_action(0),
        GroundingNonActionableAction::Retry
    );
    assert_eq!(
        grounding_non_actionable_action(GROUNDING_NON_ACTIONABLE_REPAIR_RETRY_LIMIT),
        GroundingNonActionableAction::TerminalFallback
    );
    assert_eq!(
        grounding_non_actionable_action(
            GROUNDING_NON_ACTIONABLE_REPAIR_RETRY_LIMIT.saturating_add(1)
        ),
        GroundingNonActionableAction::TerminalFallback
    );
}

#[test]
fn seed_grounding_non_actionable_repair_context_clears_pause_and_runtime_marker() {
    let mut state = EngineRunnerState {
        paused_reason: Some("missing_required_input".to_string()),
        runtime: json!({
            "agent": {
                "missing_required_input": {
                    "message": "intent grounding missing inputs",
                    "questions": [],
                    "missing_refs": []
                },
                "intent_grounding": {
                    "status": "proposed",
                    "ready_for_todos": false
                }
            }
        }),
        ..EngineRunnerState::default()
    };
    let mut context = test_segmented_context();
    context.refresh_state_summary(&state, false);

    seed_grounding_non_actionable_repair_context(
        &mut state,
        &mut context,
        &GroundingNonActionablePause {
            message: "intent grounding missing inputs".to_string(),
            issues: vec![],
        },
    );

    assert!(state.paused_reason.is_none());
    assert!(state
        .runtime
        .pointer("/agent/missing_required_input")
        .is_none());
    assert!(state.runtime.pointer("/agent/intent_grounding").is_none());
    assert_eq!(
        context
            .previous_error
            .as_ref()
            .and_then(|value| value.get("reason_code"))
            .and_then(Value::as_str),
        Some("grounding.grounding_non_actionable_pause")
    );
}

#[test]
fn selected_query_refs_from_missing_resolution_picks_first_candidate_per_ref() {
    let refs =
        super::super::missing_resolution::selected_query_refs_from_missing_resolution(&json!({
            "resolved": [
                {
                    "missing_ref": "inputs.token.decimals",
                    "query_candidates": [
                        {"query_ref":"erc20@0.0.2/decimals","score":120},
                        {"query_ref":"erc20@0.0.2/balanceOf","score":40}
                    ]
                },
                {
                    "missing_ref": "inputs.token.address",
                    "query_candidates": [
                        {"query_ref":"erc20@0.0.2/token","score":100}
                    ]
                }
            ]
        }));
    assert_eq!(
        refs,
        vec![
            "erc20@0.0.2/decimals".to_string(),
            "erc20@0.0.2/token".to_string()
        ]
    );
}

#[test]
fn missing_input_query_autofill_round_schedules_from_question_options() {
    let command = test_agent_command();
    let mut state = EngineRunnerState::default();
    let mut context = test_segmented_context();
    context.refresh_state_summary(&state, false);
    let mut candidate_context = CandidateContext::default();
    candidate_context.executable_candidates.queries.push(json!({
        "ref":"erc20@0.0.2/decimals",
        "kind":"query"
    }));
    candidate_context.detail_by_ref.insert(
        "erc20@0.0.2/decimals".to_string(),
        json!({
            "kind":"query",
            "returns":[{"name":"decimals"}]
        }),
    );
    let missing_payload = json!({
        "questions":[
            {
                "id":"inputs.token.decimals",
                "question":"What is token decimals?",
                "required":true,
                "options":[{"label":"Query decimals","value":"query"}]
            }
        ]
    });

    let first = super::super::missing_resolution::missing_resolution_recover_missing_refs(
        &command,
        &mut state,
        &mut context,
        &missing_payload,
        &candidate_context,
        None,
        "grounding",
        false,
        "grounding",
    )
    .should_retry_round();
    assert!(first);
    assert_eq!(
        context
            .previous_error
            .as_ref()
            .and_then(|value| value
                .pointer("/autofill/query_candidate_pool/0/query_candidates/0/query_ref")),
        Some(&json!("erc20@0.0.2/decimals"))
    );
    assert_eq!(
        state.runtime.pointer("/agent/missing_ref_refill/status"),
        Some(&json!("adjudicate_scheduled"))
    );

    let second = super::super::missing_resolution::missing_resolution_recover_missing_refs(
        &command,
        &mut state,
        &mut context,
        &missing_payload,
        &candidate_context,
        None,
        "grounding",
        false,
        "grounding",
    )
    .should_retry_round();
    assert!(!second);
    assert_eq!(
        state
            .runtime
            .pointer("/agent/missing_input_autofill/reason"),
        Some(&json!("router_unavailable"))
    );
    assert_eq!(
        state
            .runtime
            .pointer("/agent/missing_ref_termination/reason"),
        Some(&json!("router_unavailable"))
    );
}

#[test]
fn missing_input_query_autofill_round_applies_static_refill_before_query() {
    let command = test_agent_command();
    let mut state = EngineRunnerState::default();
    let mut context = test_segmented_context();
    let _ = super::super::upsert_store_value_with_source(
        context.input_store_mut(),
        "native_amount",
        json!(5),
        super::super::input_store::InputValueLayer::Derived,
        "seed",
        25,
        "test.native_amount",
    );
    context.refresh_state_summary(&state, false);
    let candidate_context = CandidateContext::default();
    let missing_payload = json!({
        "questions":[
            {
                "id":"inputs.native_transfer_amount",
                "question":"Provide transfer amount",
                "required":true,
                "options":[]
            }
        ]
    });

    let outcome = super::super::missing_resolution::missing_resolution_recover_missing_refs(
        &command,
        &mut state,
        &mut context,
        &missing_payload,
        &candidate_context,
        None,
        "grounding",
        false,
        "grounding",
    );
    assert!(outcome.should_retry_round());
    assert_eq!(
        context
            .previous_error
            .as_ref()
            .and_then(|value| value.pointer("/autofill/mode")),
        Some(&json!("host_static_refill_round"))
    );
    assert_eq!(
        state.runtime.pointer("/inputs/native_transfer_amount"),
        Some(&json!(5))
    );
    assert_eq!(
        state.runtime.pointer("/agent/missing_ref_refill/attempt"),
        Some(&json!("static_intent_config"))
    );
}

#[test]
fn compile_autofill_round_semantically_refills_token_address_from_input_store() {
    let command = test_agent_command();
    let mut state = EngineRunnerState::default();
    let mut context = test_segmented_context();
    let _ = super::upsert_store_value_with_source(
        context.input_store_mut(),
        "erc20_token_address",
        json!("0x8464135c8F25Da09e49BC8782676a84730C318bC"),
        InputValueLayer::Derived,
        "test_seed",
        80,
        "test.seed.erc20_token_address",
    );
    context.refresh_state_summary(&state, false);
    let candidate_context = CandidateContext::default();
    let missing_payload = json!({
        "missing_refs":["inputs.token.address"]
    });
    let compile_payload = json!({
        "reason_code":"unknown_input_ref",
        "issues":[{"reference":"unknown_input_ref","required_fact":"token.address"}]
    });

    let scheduled = try_schedule_compile_autofill_round(
        &command,
        &mut state,
        &mut context,
        &compile_payload,
        &missing_payload,
        &candidate_context,
        "todo-token-address",
        false,
    );
    assert!(scheduled);
    assert_eq!(
        context
            .previous_error
            .as_ref()
            .and_then(|value| value.pointer("/autofill/mode")),
        Some(&json!("host_static_refill_round"))
    );
    assert_eq!(
        state.runtime.pointer("/inputs/token/address"),
        Some(&json!("0x8464135c8F25Da09e49BC8782676a84730C318bC"))
    );
    assert_eq!(
        state.runtime.pointer("/agent/missing_ref_refill/status"),
        Some(&json!("resolved"))
    );
    assert_eq!(
        state.runtime.pointer("/agent/missing_ref_refill/attempt"),
        Some(&json!("static_intent_config"))
    );
}

#[test]
fn compile_autofill_round_skips_ambiguous_address_binding() {
    let command = test_agent_command();
    let mut state = EngineRunnerState::default();
    let mut context = test_segmented_context();
    let _ = super::upsert_store_value_with_source(
        context.input_store_mut(),
        "token_in_address",
        json!("0x1111111111111111111111111111111111111111"),
        InputValueLayer::Derived,
        "test_seed",
        80,
        "test.seed.token_in_address",
    );
    let _ = super::upsert_store_value_with_source(
        context.input_store_mut(),
        "token_out_address",
        json!("0x2222222222222222222222222222222222222222"),
        InputValueLayer::Derived,
        "test_seed",
        80,
        "test.seed.token_out_address",
    );
    context.refresh_state_summary(&state, false);
    let candidate_context = CandidateContext::default();
    let missing_payload = json!({
        "missing_refs":["inputs.token.address"]
    });
    let compile_payload = json!({
        "reason_code":"unknown_input_ref",
        "issues":[{"reference":"unknown_input_ref","required_fact":"token.address"}]
    });

    let scheduled = try_schedule_compile_autofill_round(
        &command,
        &mut state,
        &mut context,
        &compile_payload,
        &missing_payload,
        &candidate_context,
        "todo-ambiguous-address",
        false,
    );
    assert!(scheduled);
    assert_eq!(state.runtime.pointer("/inputs/token/address"), None);
    assert_eq!(
        context
            .previous_error
            .as_ref()
            .and_then(|value| value.pointer("/autofill/mode")),
        Some(&json!("host_binding_adjudicate_round"))
    );
    assert_eq!(
        context
            .previous_error
            .as_ref()
            .and_then(|value| value.pointer("/autofill/ambiguous_bindings/0/missing_ref")),
        Some(&json!("inputs.token.address"))
    );
    assert_eq!(
        state.runtime.pointer("/agent/missing_ref_refill/status"),
        Some(&json!("adjudicate_scheduled"))
    );
    assert_eq!(
        state.runtime.pointer("/agent/missing_ref_refill/attempt"),
        Some(&json!("llm_binding_adjudicate"))
    );
}

#[test]
fn missing_input_autofill_schedules_llm_adjudicate_for_ambiguous_binding() {
    let command = test_agent_command();
    let mut state = EngineRunnerState::default();
    let mut context = test_segmented_context();
    let _ = super::upsert_store_value_with_source(
        context.input_store_mut(),
        "token_in_address",
        json!("0x1111111111111111111111111111111111111111"),
        InputValueLayer::Derived,
        "test_seed",
        80,
        "test.seed.token_in_address",
    );
    let _ = super::upsert_store_value_with_source(
        context.input_store_mut(),
        "token_out_address",
        json!("0x2222222222222222222222222222222222222222"),
        InputValueLayer::Derived,
        "test_seed",
        80,
        "test.seed.token_out_address",
    );
    context.refresh_state_summary(&state, false);
    let candidate_context = CandidateContext::default();
    let missing_payload = json!({
        "missing_refs":["inputs.token.address"],
        "questions":[
            {"id":"inputs.token.address","question":"Provide token address"}
        ]
    });

    let scheduled = super::super::missing_resolution::missing_resolution_recover_missing_refs(
        &command,
        &mut state,
        &mut context,
        &missing_payload,
        &candidate_context,
        None,
        "grounding",
        false,
        "grounding",
    )
    .should_retry_round();
    assert!(scheduled);
    assert_eq!(
        context
            .previous_error
            .as_ref()
            .and_then(|value| value.pointer("/autofill/mode")),
        Some(&json!("host_binding_adjudicate_round"))
    );
    assert_eq!(
        state.runtime.pointer("/agent/missing_ref_refill/attempt"),
        Some(&json!("llm_binding_adjudicate"))
    );
}

#[test]
fn grounding_query_recoverable_decimals_prefers_host_query_over_user_prompt() {
    let command = native_erc20_fixture_command_for_candidates();
    let pack_path = command.pack.clone().expect("pack");
    let pack = load_pack_document(pack_path.as_path()).expect("pack");
    let candidate_context = build_candidate_context_for_agent(&command, Some(&pack), 128)
        .expect("candidate context")
        .expect("workspace candidates");

    let mut state = EngineRunnerState::default();
    let mut context = test_segmented_context();
    let _ = super::upsert_store_value_with_source(
        context.input_store_mut(),
        "token.address",
        json!("0x8464135c8F25Da09e49BC8782676a84730C318bC"),
        InputValueLayer::Derived,
        "test_seed",
        90,
        "test.seed.token.address",
    );
    context.refresh_state_summary(&state, false);

    let mut readonly_router = RouterExecutor::new();
    readonly_router.register(
        "evm_read",
        "eip155:1",
        Box::new(QueryAutofillSuccessExecutor),
    );
    readonly_router.register(
        "evm_read",
        "eip155:31338",
        Box::new(QueryAutofillSuccessExecutor),
    );

    let missing_payload = json!({
        "reason_code":"missing_required_input",
        "questions":[
            {"id":"token.decimals","question":"token decimals?"}
        ],
        "missing_refs":["inputs.token.decimals"]
    });
    let outcome = super::super::missing_resolution::missing_resolution_recover_missing_refs(
        &command,
        &mut state,
        &mut context,
        &missing_payload,
        &candidate_context,
        Some(&readonly_router),
        "grounding",
        false,
        "grounding",
    );
    assert!(
        matches!(
            outcome,
            super::super::missing_resolution::MissingResolutionOutcome::Recovered
                | super::super::missing_resolution::MissingResolutionOutcome::RetryScheduled
        ),
        "expected machine-first recovery path, got {outcome:?}"
    );
    assert!(
        !matches!(
            outcome,
            super::super::missing_resolution::MissingResolutionOutcome::NeedUserInput { .. }
        ),
        "query-recoverable grounding should not directly ask user"
    );
    assert!(
        context
            .previous_error
            .as_ref()
            .and_then(|value| value.pointer("/autofill/mode"))
            .and_then(Value::as_str)
            .is_some(),
        "expected autofill envelope to be set for follow-up round"
    );
}

#[test]
fn missing_resolution_recovers_stale_input_question_from_true_input_store_value() {
    let command = test_agent_command();
    let candidate_context = CandidateContext::default();
    let mut state = EngineRunnerState::default();
    let mut context = test_segmented_context();
    let _ = super::upsert_seed_input_value(
        &mut state.runtime,
        "token.decimals",
        json!(18),
        "test.seed.token.decimals",
    );
    let _ = super::upsert_store_value_with_source(
        context.input_store_mut(),
        "token.decimals",
        json!(18),
        InputValueLayer::Seed,
        "user",
        100,
        "test.seed.token.decimals",
    );
    context.refresh_state_summary(&state, false);

    let payload = json!({
        "reason_code":"missing_required_input",
        "questions":[
            {"id":"inputs.token.decimals","question":"Provide token decimals"}
        ]
    });

    let outcome = super::super::missing_resolution::missing_resolution_recover_missing_refs(
        &command,
        &mut state,
        &mut context,
        &payload,
        &candidate_context,
        None,
        "grounding",
        false,
        "grounding",
    );

    assert!(matches!(
        outcome,
        super::super::missing_resolution::MissingResolutionOutcome::Recovered
    ));
}

#[test]
fn missing_resolution_recovers_stale_input_question_from_input_store_value() {
    let command = test_agent_command();
    let candidate_context = CandidateContext::default();
    let mut state = EngineRunnerState::default();
    let mut input_store = InputStore::default();
    let _ = input_store.upsert(
        "inputs.token.decimals",
        json!(18),
        super::super::input_store::InputValueMeta {
            source: "query".to_string(),
            source_priority: 90,
            provenance: Some("test.input_store.token.decimals".to_string()),
            confidence: None,
            layer: InputValueLayer::Observed,
            stability: super::super::input_store::InputValueStability::Stable,
            observed_at_ms: Some(123),
        },
    );
    let mut context = SegmentedAgentContext::new(
        "check balances and transfer".to_string(),
        intent_segmented::SegmentPlanningSession {
            session_id: "sess-1".to_string(),
            snapshot_hash: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                .to_string(),
            cursor: "cursor-0".to_string(),
            max_rounds: 4,
            max_segments: 4,
        },
        input_store,
        super::super::runtime_facts_store::RuntimeFactsStore::default(),
        TodoBoard::bootstrap("check balances and transfer"),
        4,
        4,
        ToolMemoryBudgetPolicy::TOOL_MEMORY_PROJECTION_DEFAULT_TOKENS,
        checkpoint_ext::AgentCheckpointExtensions::default(),
    );
    context.refresh_state_summary(&state, false);

    let payload = json!({
        "reason_code":"missing_required_input",
        "questions":[
            {"id":"inputs.token.decimals","question":"Provide token decimals"}
        ]
    });

    let outcome = super::super::missing_resolution::missing_resolution_recover_missing_refs(
        &command,
        &mut state,
        &mut context,
        &payload,
        &candidate_context,
        None,
        "grounding",
        false,
        "grounding",
    );

    assert!(matches!(
        outcome,
        super::super::missing_resolution::MissingResolutionOutcome::Recovered
    ));
}

#[test]
fn unavailable_recovery_backflow_retries_when_question_ref_is_already_satisfied() {
    let command = native_erc20_fixture_command_for_candidates();
    let pack_path = command.pack.clone().expect("pack");
    let pack = load_pack_document(pack_path.as_path()).expect("pack");
    let candidate_context = build_candidate_context_for_agent(&command, Some(&pack), 128)
        .expect("candidate context")
        .expect("workspace candidates");
    let mut state = EngineRunnerState::default();
    let mut context = test_segmented_context();
    let _ = super::upsert_seed_input_value(
        &mut state.runtime,
        "token.decimals",
        json!(18),
        "test.seed.token.decimals",
    );
    let _ = super::upsert_store_value_with_source(
        context.input_store_mut(),
        "token.decimals",
        json!(18),
        InputValueLayer::Seed,
        "user",
        100,
        "test.seed.token.decimals",
    );
    context.refresh_state_summary(&state, false);

    let payload = json!({
        "reason_code":"missing_required_input",
        "message":"Need token decimals before todo planning",
        "questions":[
            {"id":"inputs.token.decimals","question":"Provide token decimals","required":true,"options":[]}
        ],
        "issues":[]
    });

    let outcome = super::super::phase_machine::pause::recover_missing_required_input_payload(
        &command,
        &mut state,
        &mut context,
        &candidate_context,
        None,
        &payload,
        "grounding",
        false,
        "grounding",
        false,
        true,
    )
    .expect("recovery backflow");

    assert!(matches!(
        outcome,
        super::super::phase_machine::pause::MissingRequiredInputRecoveryBackflow::Retry {
            state_changed: true,
            answers: None,
        }
    ));
}

#[test]
fn host_query_autofill_exhausts_then_stops_with_retry_limited() {
    let command = native_erc20_fixture_command_for_candidates();
    let pack_path = command.pack.clone().expect("pack");
    let pack = load_pack_document(pack_path.as_path()).expect("pack");
    let candidate_context = build_candidate_context_for_agent(&command, Some(&pack), 128)
        .expect("candidate context")
        .expect("workspace candidates");

    let mut state = EngineRunnerState::default();
    let mut context = test_segmented_context();
    let _ = super::upsert_store_value_with_source(
        context.input_store_mut(),
        "token.address",
        json!("0x8464135c8F25Da09e49BC8782676a84730C318bC"),
        InputValueLayer::Derived,
        "test_seed",
        90,
        "test.seed.token.address",
    );
    context.refresh_state_summary(&state, false);

    let mut readonly_router = RouterExecutor::new();
    readonly_router.register("evm_read", "eip155:1", Box::new(QueryAutofillEmptyExecutor));
    readonly_router.register(
        "evm_read",
        "eip155:31338",
        Box::new(QueryAutofillEmptyExecutor),
    );

    let missing_payload = json!({
        "reason_code":"missing_required_input",
        "questions":[
            {"id":"token.decimals","question":"token decimals?"}
        ],
        "missing_refs":["inputs.token.decimals"]
    });

    let first = super::super::missing_resolution::missing_resolution_recover_missing_refs(
        &command,
        &mut state,
        &mut context,
        &missing_payload,
        &candidate_context,
        Some(&readonly_router),
        "grounding",
        false,
        "grounding",
    );
    assert_eq!(
        first,
        super::super::missing_resolution::MissingResolutionOutcome::RetryScheduled
    );
    assert_eq!(state.runtime.pointer("/inputs/token/decimals"), None);
    assert!(
        context
            .previous_error
            .as_ref()
            .and_then(|value| value.pointer("/autofill/mode"))
            .and_then(Value::as_str)
            .is_some(),
        "first round should schedule machine recovery follow-up"
    );

    let second = super::super::missing_resolution::missing_resolution_recover_missing_refs(
        &command,
        &mut state,
        &mut context,
        &missing_payload,
        &candidate_context,
        Some(&readonly_router),
        "grounding",
        false,
        "grounding",
    );
    assert!(
        matches!(
            second,
            super::super::missing_resolution::MissingResolutionOutcome::ExhaustedUnavailable { .. }
                | super::super::missing_resolution::MissingResolutionOutcome::NeedUserInput { .. }
        ),
        "second round should stop retrying and surface unresolved state, got {second:?}"
    );
}

#[test]
fn missing_resolution_policy_partial_keeps_valid_subset_instead_of_full_reject() {
    let command = test_agent_command();
    let mut state = EngineRunnerState::default();
    let mut context = test_segmented_context();
    context.refresh_state_summary(&state, false);
    let candidate_context = CandidateContext::default();
    let missing_payload = json!({
        "reason_code":"missing_required_input",
        "missing_refs":["inputs.token.decimals", "inputs.owner"],
        "questions":[
            {"id":"inputs.token.decimals","question":"token decimals?"},
            {"id":"inputs.owner","question":"owner?"}
        ],
        "decisions":[
            {
                "kind":"run_producer",
                "target":"inputs.token.decimals",
                "query_ref":""
            },
            {
                "kind":"run_producer",
                "target":"inputs.owner",
                "query_ref":"wallet@0.0.1/defaultOwner"
            }
        ]
    });

    let outcome = super::super::missing_resolution::missing_resolution_recover_missing_refs(
        &command,
        &mut state,
        &mut context,
        &missing_payload,
        &candidate_context,
        None,
        "grounding",
        false,
        "grounding",
    );

    match &outcome {
        super::super::missing_resolution::MissingResolutionOutcome::RetryScheduled => {}
        super::super::missing_resolution::MissingResolutionOutcome::ExhaustedUnavailable {
            reason,
            ..
        } if reason == "router_unavailable" => {}
        _ => panic!("policy should keep valid subset and continue machine flow, got {outcome:?}"),
    }
    assert_eq!(
        state
            .runtime
            .pointer("/agent/missing_ref_policy_validation/status"),
        Some(&json!("partial"))
    );
    assert_eq!(
        state
            .runtime
            .pointer("/agent/missing_ref_policy_validation/accepted_decisions")
            .and_then(Value::as_array)
            .map(|items| items.len()),
        Some(1)
    );
    assert_eq!(
        state
            .runtime
            .pointer("/agent/missing_ref_policy_validation/rejected_decisions")
            .and_then(Value::as_array)
            .map(|items| items.len()),
        Some(1)
    );
    assert!(state
        .runtime
        .pointer("/agent/missing_ref_policy_validation/rejected_decisions/0/issues")
        .and_then(Value::as_array)
        .is_some_and(|items| items
            .iter()
            .any(|item| item.get("code") == Some(&json!("run_producer_query_ref_empty")))));
}

#[test]
fn missing_resolution_policy_validation_failed_emits_termination_telemetry() {
    let command = test_agent_command();
    let mut state = EngineRunnerState::default();
    let mut context = test_segmented_context();
    context.refresh_state_summary(&state, false);
    let candidate_context = CandidateContext::default();
    let missing_payload = json!({
        "reason_code":"missing_required_input",
        "missing_refs":["inputs.token.decimals"],
        "questions":[
            {"id":"inputs.token.decimals","question":"token decimals?"}
        ],
        "decisions":[
            {
                "kind":"run_producer",
                "target":"inputs.token.decimals",
                "query_ref":""
            }
        ]
    });

    let outcome = super::super::missing_resolution::missing_resolution_recover_missing_refs(
        &command,
        &mut state,
        &mut context,
        &missing_payload,
        &candidate_context,
        None,
        "grounding",
        false,
        "grounding",
    );
    assert!(
        matches!(
            outcome,
            super::super::missing_resolution::MissingResolutionOutcome::NeedUserInput {
                ref reason,
                ..
            } if reason == "policy_validation_failed"
        ),
        "expected policy_validation_failed fallback, got {outcome:?}"
    );
    assert_eq!(
        state
            .runtime
            .pointer("/agent/missing_ref_policy_validation/status"),
        Some(&json!("rejected"))
    );
    assert_eq!(
        state
            .runtime
            .pointer("/agent/missing_ref_termination/reason"),
        Some(&json!("policy_validation_failed"))
    );
    assert_eq!(
        state
            .runtime
            .pointer("/agent/missing_ref_termination/phase_hint"),
        Some(&json!("grounding"))
    );
}

#[test]
fn missing_resolution_policy_abort_emits_termination_telemetry() {
    let command = test_agent_command();
    let mut state = EngineRunnerState::default();
    let mut context = test_segmented_context();
    context.refresh_state_summary(&state, false);
    let candidate_context = CandidateContext::default();
    let missing_payload = json!({
        "reason_code":"missing_required_input",
        "missing_refs":["inputs.owner"],
        "questions":[
            {"id":"inputs.owner","question":"owner?"}
        ],
        "decisions":[
            {
                "kind":"abort",
                "reason":"user_required_for_owner"
            }
        ]
    });

    let outcome = super::super::missing_resolution::missing_resolution_recover_missing_refs(
        &command,
        &mut state,
        &mut context,
        &missing_payload,
        &candidate_context,
        None,
        "grounding",
        false,
        "grounding",
    );
    assert!(
        matches!(
            outcome,
            super::super::missing_resolution::MissingResolutionOutcome::NeedUserInput {
                ref reason,
                ..
            } if reason == "policy_abort:user_required_for_owner"
        ),
        "expected policy_abort path, got {outcome:?}"
    );
    assert_eq!(
        state
            .runtime
            .pointer("/agent/missing_ref_policy_validation/status"),
        Some(&json!("accepted"))
    );
    assert_eq!(
        state
            .runtime
            .pointer("/agent/missing_ref_termination/reason"),
        Some(&json!("policy_abort:user_required_for_owner"))
    );
}

#[test]
fn planner_no_toolcall_repair_keeps_previous_error_autofill_context_sticky() {
    let provider = ScriptedLlmProvider::from_responses(vec![
        Ok(CompleteWithToolsResponse {
            assistant_content: Some("empty tool call batch".to_string()),
            tool_calls: vec![],
        }),
        Ok(CompleteWithToolsResponse {
            assistant_content: Some("finalize revise".to_string()),
            tool_calls: vec![ToolCall {
                id: "tool-revise-final".to_string(),
                name: "plan.revise_segment".to_string(),
                arguments: json!({
                    "status":"unavailable",
                    "done":false,
                    "error":{
                        "reason_code":"missing_required_input",
                        "message":"still missing recipient",
                        "details":{
                            "questions":[{"id":"recipient","question":"recipient?"}],
                            "recovery_exhaustion":{
                                "unresolved_refs":["recipient"],
                                "reasons":["host_recovery_exhausted"],
                                "attempt_trace_id":"trace-toolcall-repair-1"
                            }
                        }
                    }
                }),
            }],
        }),
    ]);
    let mut planner = super::super::intent_segmented::LlmSegmentedIntentPlanner::new(provider)
        .with_candidate_context(Some(CandidateContext::default()));
    let state = EngineRunnerState::default();
    let mut context = test_segmented_context();
    context.refresh_state_summary(&state, false);
    context.set_previous_error_and_refresh(
        &state,
        false,
        json!({
            "reason_code":"missing_required_input",
            "autofill":{
                "mode":"host_binding_adjudicate_round",
                "selected_query_refs":["erc20@0.0.2/decimals"],
                "scope_id":"todo_1"
            }
        }),
    );

    let _ =
        super::super::phase_machine::segment_plan::plan_round(&mut planner, &state, &mut context)
            .expect("plan round should recover after planner-output repair");

    assert_eq!(
        context
            .previous_error
            .as_ref()
            .and_then(|value| value.pointer("/autofill/mode")),
        Some(&json!("host_binding_adjudicate_round"))
    );
    assert_eq!(
        context
            .previous_error
            .as_ref()
            .and_then(|value| value.pointer("/autofill/selected_query_refs/0")),
        Some(&json!("erc20@0.0.2/decimals"))
    );
    assert_eq!(
        context
            .previous_error
            .as_ref()
            .and_then(|value| value.pointer("/reason_code"))
            .and_then(Value::as_str),
        Some("missing_required_input")
    );
}

#[test]
fn grounding_abort_intent_accepts_and_short_circuits_to_stopped() {
    let command = test_agent_command();
    let mut state = EngineRunnerState::default();
    let mut context = test_segmented_context();
    context.set_previous_error_and_refresh(
        &state,
        false,
        json!({
            "reason_code":"missing_required_input",
            "autofill_history":{
                "attempt_keys":["runtime.query.resolve"]
            }
        }),
    );
    let provider = ScriptedLlmProvider::from_responses(vec![Ok(CompleteWithToolsResponse {
        assistant_content: Some("abort intent".to_string()),
        tool_calls: vec![ToolCall {
            id: "tool-abort".to_string(),
            name: "plan.abort_intent".to_string(),
            arguments: json!({
                "reason_code":"recovery_exhausted",
                "summary":"unable to complete intent with current evidence",
                "evidence":{
                    "attempted_recovery":["runtime.query.resolve"],
                    "missing_refs":["inputs.token.decimals"]
                }
            }),
        }],
    })]);
    let mut planner = super::super::intent_segmented::LlmSegmentedIntentPlanner::new(provider)
        .with_candidate_context(Some(CandidateContext::default()));

    let ready = super::super::phase_machine::grounding::bootstrap_intent_grounding_if_needed(
        &command,
        &mut planner,
        &mut state,
        &mut context,
        &CandidateContext::default(),
        None,
        false,
    )
    .expect("grounding bootstrap should succeed");
    assert!(!ready, "abort should stop grounding progression");
    assert_eq!(context.final_status(), EngineRunStatus::Stopped);
    assert_eq!(
        state.runtime.pointer("/agent/abort_intent/accepted"),
        Some(&json!(true))
    );
}

#[test]
fn grounding_abort_intent_reject_falls_back_to_planner_call_failed() {
    let command = test_agent_command();
    let mut state = EngineRunnerState::default();
    let mut context = test_segmented_context();
    context.set_previous_error_and_refresh(
        &state,
        false,
        json!({
            "reason_code":"missing_required_input",
            "autofill_history":{
                "attempt_keys":["runtime.query.resolve"]
            }
        }),
    );
    let provider = ScriptedLlmProvider::from_responses(vec![Ok(CompleteWithToolsResponse {
        assistant_content: Some("abort intent".to_string()),
        tool_calls: vec![ToolCall {
            id: "tool-abort".to_string(),
            name: "plan.abort_intent".to_string(),
            arguments: json!({
                "reason_code":"recovery_exhausted",
                "summary":"unable to complete intent with current evidence",
                "evidence":{
                    "attempted_recovery":["unknown.recovery.key"],
                    "missing_refs":["inputs.token.decimals"]
                }
            }),
        }],
    })]);
    let mut planner = super::super::intent_segmented::LlmSegmentedIntentPlanner::new(provider)
        .with_candidate_context(Some(CandidateContext::default()));

    let ready = super::super::phase_machine::grounding::bootstrap_intent_grounding_if_needed(
        &command,
        &mut planner,
        &mut state,
        &mut context,
        &CandidateContext::default(),
        None,
        false,
    )
    .expect("grounding bootstrap should fallback");
    assert!(
        ready,
        "rejected abort should fallback to ready-for-todos path"
    );
    assert_ne!(context.final_status(), EngineRunStatus::Stopped);
    assert_eq!(
        state.runtime.pointer("/agent/intent_grounding/reason_code"),
        Some(&json!("planner_call_failed"))
    );
}

#[test]
fn grounding_ignores_stale_missing_refs_when_input_store_already_has_value() {
    let command = test_agent_command();
    let mut state = EngineRunnerState {
        runtime: json!({
            "inputs": {
                "token": {
                    "decimals": 18
                }
            }
        }),
        ..EngineRunnerState::default()
    };
    let mut context = test_segmented_context();
    let _ = super::upsert_store_value_with_source(
        context.input_store_mut(),
        "token.decimals",
        json!(18),
        super::InputValueLayer::Seed,
        "user",
        100,
        "user.prompt.token.decimals",
    );
    context.refresh_state_summary(&state, false);
    let provider = ScriptedLlmProvider::from_responses(vec![Ok(CompleteWithToolsResponse {
        assistant_content: Some("stale missing refs".to_string()),
        tool_calls: vec![ToolCall {
            id: "tool-ground".to_string(),
            name: "plan.ground_intent".to_string(),
            arguments: json!({
                "status":"proposed",
                "ready_for_todos":false,
                "resolved_inputs":{
                    "owner":"0x70997970c51812dc3a010c7d01b50e0d17dc79c8",
                    "recipient":"0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
                    "chain":"eip155:31338",
                    "token.address":"0x8464135c8F25Da09e49BC8782676a84730C318bC",
                    "native_amount":5,
                    "token_amount":10
                },
                "intent_facts":{
                    "owner":"0x70997970c51812dc3a010c7d01b50e0d17dc79c8",
                    "recipient":"0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
                    "chain":"eip155:31338",
                    "token.address":"0x8464135c8F25Da09e49BC8782676a84730C318bC",
                    "native_amount":5,
                    "token_amount":10
                },
                "confidence":{
                    "owner":100,
                    "recipient":100,
                    "chain":100,
                    "token.address":100,
                    "native_amount":100,
                    "token_amount":100
                },
                "missing_refs":["inputs.token.decimals"]
            }),
        }],
    })]);
    let mut planner = LlmSegmentedIntentPlanner::new(provider);

    let ready = bootstrap_intent_grounding_if_needed(
        &command,
        &mut planner,
        &mut state,
        &mut context,
        &CandidateContext::default(),
        None,
        false,
    )
    .expect("stale missing refs should be filtered");
    assert!(ready);
    assert_eq!(state.paused_reason, None);
}

#[test]
fn scripted_grounding_unavailable_helper_decodes_with_single_planner_call() {
    let command = native_erc20_fixture_command_for_candidates();
    let pack_path = command.pack.clone().expect("pack");
    let pack = load_pack_document(pack_path.as_path()).expect("pack");
    let candidate_context = build_candidate_context_for_agent(&command, Some(&pack), 128)
        .expect("candidate context")
        .expect("workspace candidates");
    let provider = ScriptedLlmProvider::from_responses(vec![Ok(
        scripted_grounding_unavailable_missing_input_response(
            "inputs.token.decimals",
            "Provide token decimals",
            "Need token decimals before todo planning",
        ),
    )]);
    let mut planner =
        LlmSegmentedIntentPlanner::new(provider).with_candidate_context(Some(candidate_context));
    let mut context = test_segmented_context();
    let state = EngineRunnerState::default();
    context.refresh_state_summary(&state, false);

    let draft = planner
        .ground_intent(IntentGroundingRequest {
            intent: context.intent().to_string(),
            session: context.session().clone(),
            state_summary: context.packed_summary().clone(),
            typed_summary: None,
        })
        .expect("unavailable draft should decode");

    match draft {
        IntentGroundingDraft::Unavailable {
            reason_code,
            questions,
            ..
        } => {
            assert_eq!(reason_code, "missing_required_input");
            assert_eq!(questions.len(), 1);
            assert_eq!(
                questions[0].get("id").and_then(Value::as_str),
                Some("inputs.token.decimals")
            );
        }
        other => panic!("expected unavailable draft, got {other:?}"),
    }
}

#[test]
fn grounding_post_recovery_fast_path_skips_second_planner_call_after_query_autofill() {
    let command = native_erc20_fixture_command_for_candidates();
    let pack_path = command.pack.clone().expect("pack");
    let pack = load_pack_document(pack_path.as_path()).expect("pack");
    let candidate_context = build_candidate_context_for_agent(&command, Some(&pack), 128)
        .expect("candidate context")
        .expect("workspace candidates");

    let mut state = EngineRunnerState::default();
    let mut context = test_segmented_context();
    context.refresh_state_summary(&state, false);

    let provider = ScriptedLlmProvider::from_responses(vec![Ok(CompleteWithToolsResponse {
        assistant_content: Some("grounding draft".to_string()),
        tool_calls: vec![ToolCall {
            id: "tool-ground".to_string(),
            name: "plan.ground_intent".to_string(),
            arguments: json!({
                "status":"proposed",
                "summary":"Need token decimals before todo planning",
                "ready_for_todos":false,
                "resolved_inputs":{
                    "owner":"0x70997970c51812dc3a010c7d01b50e0d17dc79c8",
                    "recipient":"0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
                    "chain":"eip155:31338",
                    "token.address":"0x8464135c8F25Da09e49BC8782676a84730C318bC",
                    "native_amount":5,
                    "token_amount":10
                },
                "intent_facts":{
                    "owner":"0x70997970c51812dc3a010c7d01b50e0d17dc79c8",
                    "recipient":"0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
                    "chain":"eip155:31338",
                    "token.address":"0x8464135c8F25Da09e49BC8782676a84730C318bC",
                    "native_amount":5,
                    "token_amount":10
                },
                "confidence":{
                    "owner":100,
                    "recipient":100,
                    "chain":100,
                    "token.address":100,
                    "native_amount":100,
                    "token_amount":100
                },
                "missing_refs":["inputs.token.decimals"],
                "questions":[]
            }),
        }],
    })]);
    let mut planner = LlmSegmentedIntentPlanner::new(provider)
        .with_candidate_context(Some(candidate_context.clone()));

    let mut readonly_router = RouterExecutor::new();
    readonly_router.register(
        "evm_read",
        "eip155:31338",
        Box::new(QueryAutofillSuccessExecutor),
    );

    let ready = bootstrap_intent_grounding_if_needed(
        &command,
        &mut planner,
        &mut state,
        &mut context,
        &candidate_context,
        Some(&readonly_router),
        false,
    )
    .expect("grounding fast path should complete with a single planner response");

    assert!(ready, "post-recovery fast path should mark grounding ready");
    assert_eq!(
        state.runtime.pointer("/inputs/token/decimals"),
        Some(&json!(18))
    );
    assert_eq!(
        state
            .runtime
            .pointer("/agent/intent_grounding/ready_for_todos"),
        Some(&json!(true))
    );
    assert_eq!(
        state
            .runtime
            .pointer("/agent/intent_grounding/resolution_state"),
        Some(&json!("ready"))
    );
}

#[test]
fn grounding_unavailable_post_recovery_fast_path_skips_second_planner_call() {
    let command = native_erc20_fixture_command_for_candidates();
    let pack_path = command.pack.clone().expect("pack");
    let pack = load_pack_document(pack_path.as_path()).expect("pack");
    let candidate_context = build_candidate_context_for_agent(&command, Some(&pack), 128)
        .expect("candidate context")
        .expect("workspace candidates");

    let mut state = EngineRunnerState::default();
    let mut context = test_segmented_context();
    let _ = super::upsert_seed_input_value(
        &mut state.runtime,
        "token.decimals",
        json!(18),
        "test.seed.token.decimals",
    );
    let _ = super::upsert_store_value_with_source(
        context.input_store_mut(),
        "token.decimals",
        json!(18),
        InputValueLayer::Seed,
        "user",
        100,
        "test.seed.token.decimals",
    );
    context.refresh_state_summary(&state, false);

    let provider = ScriptedLlmProvider::from_responses(vec![Ok(
        scripted_grounding_unavailable_missing_input_response(
            "inputs.token.decimals",
            "Provide token decimals",
            "Need token decimals before todo planning",
        ),
    )]);
    let mut planner = LlmSegmentedIntentPlanner::new(provider)
        .with_candidate_context(Some(candidate_context.clone()));

    let ready = bootstrap_intent_grounding_if_needed(
        &command,
        &mut planner,
        &mut state,
        &mut context,
        &candidate_context,
        None,
        false,
    )
    .expect("unavailable fast path should complete with a single planner response");

    assert!(ready, "stale unavailable payload should collapse to ready");
    assert_eq!(
        state.runtime.pointer("/agent/intent_grounding/status"),
        Some(&json!("unavailable_recovered"))
    );
    assert_eq!(
        state
            .runtime
            .pointer("/agent/intent_grounding/ready_for_todos"),
        Some(&json!(true))
    );
    assert_eq!(
        state
            .runtime
            .pointer("/agent/intent_grounding/resolution_state"),
        Some(&json!("ready"))
    );
}

#[test]
fn compile_autofill_round_is_bounded_per_todo() {
    let command = test_agent_command();
    let mut state = EngineRunnerState::default();
    let mut context = test_segmented_context();
    context.refresh_state_summary(&state, false);
    let mut candidate_context = CandidateContext::default();
    candidate_context.executable_candidates.queries.push(json!({
        "ref":"erc20@0.0.2/decimals",
        "kind":"query"
    }));
    candidate_context.detail_by_ref.insert(
        "erc20@0.0.2/decimals".to_string(),
        json!({
            "kind":"query",
            "returns":[{"name":"decimals"}]
        }),
    );
    let missing_payload = json!({
        "missing_refs":["inputs.token.decimals"]
    });
    let compile_payload = json!({
        "reason_code":"write_gate_missing",
        "issues":[{"kind":"write_gate_missing","required_fact":"token.decimals"}]
    });

    let first = try_schedule_compile_autofill_round(
        &command,
        &mut state,
        &mut context,
        &compile_payload,
        &missing_payload,
        &candidate_context,
        "todo-1",
        false,
    );
    assert!(first);
    assert_eq!(
        context
            .previous_error
            .as_ref()
            .and_then(|value| value.pointer("/autofill/selected_query_refs/0")),
        Some(&json!("erc20@0.0.2/decimals"))
    );
    assert_eq!(
        state.runtime.pointer("/agent/compile_autofill/status"),
        Some(&json!("scheduled"))
    );

    let second = try_schedule_compile_autofill_round(
        &command,
        &mut state,
        &mut context,
        &compile_payload,
        &missing_payload,
        &candidate_context,
        "todo-1",
        false,
    );
    assert!(!second);
    assert_eq!(
        state.runtime.pointer("/agent/compile_autofill/reason"),
        Some(&json!("retry_limited"))
    );
}

#[test]
fn compile_autofill_round_falls_back_when_no_query_candidates() {
    let command = test_agent_command();
    let mut state = EngineRunnerState::default();
    let mut context = test_segmented_context();
    context.refresh_state_summary(&state, false);
    let candidate_context = CandidateContext::default();
    let missing_payload = json!({
        "missing_refs":["inputs.unresolvable_field"]
    });
    let compile_payload = json!({
        "reason_code":"write_gate_missing",
        "issues":[{"kind":"write_gate_missing","required_fact":"unresolvable_field"}]
    });

    let scheduled = try_schedule_compile_autofill_round(
        &command,
        &mut state,
        &mut context,
        &compile_payload,
        &missing_payload,
        &candidate_context,
        "todo-x",
        false,
    );
    assert!(scheduled);
    assert_eq!(
        context
            .previous_error
            .as_ref()
            .and_then(|value| value.pointer("/autofill/mode")),
        Some(&json!("host_binding_adjudicate_round"))
    );
    assert_eq!(
        state.runtime.pointer("/agent/missing_ref_refill/status"),
        Some(&json!("adjudicate_scheduled"))
    );
}
