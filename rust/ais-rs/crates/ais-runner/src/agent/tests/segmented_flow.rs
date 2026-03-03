#[test]
fn execute_agent_intent_mode_requires_workspace_candidates() {
    let config_path = write_temp_file(
        "agent-intent-no-llm-config",
        r#"
schema: ais-runner/0.0.1
chains:
  eip155:1:
    rpc_url: https://rpc.evm.example
"#,
    );
    let command = AgentCommand {
        plan: None,
        intent: Some("check balance and transfer".to_string()),
        intent_file: None,
        workspace: None,
        config: config_path,
        pack: None,
        runtime: None,
        events_jsonl: None,
        trace: None,
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
        format: OutputFormat::Text,
    };
    let error = super::execute_agent(&command).expect_err("must reject without workspace");
    assert!(matches!(error, RunnerError::Llm(_)));
    assert!(error.to_string().contains("requires `--workspace`"));
}

#[test]
fn execute_agent_intent_mode_requires_llm_provider_with_workspace() {
    let fixture_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/runner-local/intent-segmented-offchain-transfer");
    let workspace_dir = fixture_root.join("workspace");
    let pack_path = workspace_dir.join("safe-defi.ais-pack.yaml");
    let config_path = fixture_root.join("config/runner.local.yaml");
    let intent_file = fixture_root.join("intent/intent.txt");
    let command = AgentCommand {
        plan: None,
        intent: None,
        intent_file: Some(intent_file),
        workspace: Some(workspace_dir),
        config: config_path,
        pack: Some(pack_path),
        runtime: None,
        events_jsonl: None,
        trace: None,
        checkpoint: None,
        profile: AgentProfile::Standard,
        llm_script_jsonl: None,
        verbose: false,
        verbose_llm: false,
        approvals_mode: Some(crate::cli::ApprovalsMode::Safe),
        max_iterations: Some(8),
        max_planner_rounds: Some(2),
        max_tool_rounds: None,
        max_index_candidates: None,
        planner_context_token_budget: None,
        llm_transcript_path: None,
        llm_transcript_append: false,
        format: OutputFormat::Text,
    };
    let error = super::execute_agent(&command).expect_err("must reject without llm provider");
    assert!(matches!(error, RunnerError::Llm(_)));
    assert!(error
        .to_string()
        .contains("requires configured llm provider"));
}

fn segmented_fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/runner-local/intent-segmented-offchain-transfer")
}

fn build_segmented_demo_command(
    llm_script_jsonl: PathBuf,
    checkpoint: Option<PathBuf>,
    events_jsonl: Option<PathBuf>,
) -> AgentCommand {
    let fixture_root = segmented_fixture_root();
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
        events_jsonl: events_jsonl.map(|path| path.display().to_string()),
        trace: None,
        checkpoint,
        profile: AgentProfile::DemoScripted,
        llm_script_jsonl: Some(llm_script_jsonl),
        verbose: false,
        verbose_llm: false,
        approvals_mode: Some(crate::cli::ApprovalsMode::Safe),
        max_iterations: Some(24),
        max_planner_rounds: Some(8),
        max_tool_rounds: None,
        max_index_candidates: None,
        planner_context_token_budget: None,
        llm_transcript_path: None,
        llm_transcript_append: false,
        format: OutputFormat::Json,
    }
}

fn segmented_native_erc20_fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/runner-local/intent-native-erc20-transfer")
}

fn build_segmented_native_erc20_command(llm_script_jsonl: PathBuf) -> AgentCommand {
    let fixture_root = segmented_native_erc20_fixture_root();
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
        checkpoint: None,
        profile: AgentProfile::DemoScripted,
        llm_script_jsonl: Some(llm_script_jsonl),
        verbose: false,
        verbose_llm: false,
        approvals_mode: Some(crate::cli::ApprovalsMode::Safe),
        max_iterations: Some(24),
        max_planner_rounds: Some(8),
        max_tool_rounds: None,
        max_index_candidates: None,
        planner_context_token_budget: None,
        llm_transcript_path: None,
        llm_transcript_append: false,
        format: OutputFormat::Json,
    }
}

fn parse_agent_output_json(command: &AgentCommand) -> Value {
    let output = super::execute_agent(command).expect("agent execution");
    serde_json::from_str(output.as_str()).expect("json output")
}

fn write_compile_write_gate_missing_then_revise_missing_input_script() -> PathBuf {
    let llm_script = [
        serde_json::to_string(&json!({
            "assistant_content":"begin",
            "tool_calls":[{
                "id":"tool-begin",
                "name":"plan.begin",
                "arguments":{
                    "session_id":"sess-compile-autofill",
                    "snapshot_hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    "cursor":"cursor-0",
                    "limits":{"max_rounds":8,"max_segments":4}
                }
            }]
        }))
        .expect("script line 1"),
        serde_json::to_string(&json!({
            "assistant_content":"ground",
            "tool_calls":[{
                "id":"tool-ground",
                "name":"plan.ground_intent",
                "arguments":{
                    "status":"proposed",
                    "ready_for_todos": true,
                    "resolved_inputs": {"owner":"0x70997970C51812dc3A010C7d01b50e0d17dc79C8"}
                }
            }]
        }))
        .expect("script line 2"),
        serde_json::to_string(&json!({
            "assistant_content":"todos",
            "tool_calls":[{
                "id":"tool-todos",
                "name":"plan.propose_todos",
                "arguments":{
                    "status":"proposed",
                    "todos":[{"title":"transfer erc20"}]
                }
            }]
        }))
        .expect("script line 3"),
        serde_json::to_string(&json!({
            "assistant_content":"first segment misses write-gate facts",
            "tool_calls":[
                {
                    "id":"tool-check",
                    "name":"plan.check_segment",
                    "arguments":{
                        "segment":{
                            "segment_id":"seg-transfer",
                            "cursor_in":"cursor-0",
                            "cursor_out":"cursor-1",
                            "done":false,
                            "summary":"unsafe transfer draft",
                            "steps":[
                                {
                                    "id":"q_balance",
                                    "kind":"query",
                                    "candidate_ref":"erc20@0.0.2/balance-of",
                                    "inputs":{
                                        "token":{"object":{"address":{"lit":"0x8464135c8F25Da09e49BC8782676a84730C318bC"},"chain_ref":{"lit":"eip155:31338"}}},
                                        "owner":{"lit":"0x70997970C51812dc3A010C7d01b50e0d17dc79C8"}
                                    }
                                },
                                {
                                    "id":"assert_balance",
                                    "kind":"assert",
                                    "depends_on":["q_balance"],
                                    "inputs":{"condition":{"cel":"nodes.q_balance.outputs.balance != null"}}
                                },
                                {
                                    "id":"a_transfer_erc20",
                                    "kind":"action",
                                    "candidate_ref":"erc20@0.0.2/transfer",
                                    "depends_on":["assert_balance"],
                                    "inputs":{
                                        "token":{"object":{"address":{"lit":"0x8464135c8F25Da09e49BC8782676a84730C318bC"},"chain_ref":{"lit":"eip155:31338"}}},
                                        "to":{"lit":"0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"},
                                        "amount":{"lit":"10000000000000000000"}
                                    }
                                }
                            ],
                            "extensions":{}
                        }
                    }
                },
                {
                    "id":"tool-propose",
                    "name":"plan.propose_segment",
                    "arguments":{
                        "status":"proposed",
                        "done":false,
                        "cursor_next":"cursor-1",
                        "summary":"transfer token without decimals query",
                        "segment":{
                            "segment_id":"seg-transfer",
                            "cursor_in":"cursor-0",
                            "cursor_out":"cursor-1",
                            "done":false,
                            "summary":"unsafe transfer draft",
                            "steps":[
                                {
                                    "id":"q_balance",
                                    "kind":"query",
                                    "candidate_ref":"erc20@0.0.2/balance-of",
                                    "inputs":{
                                        "token":{"object":{"address":{"lit":"0x8464135c8F25Da09e49BC8782676a84730C318bC"},"chain_ref":{"lit":"eip155:31338"}}},
                                        "owner":{"lit":"0x70997970C51812dc3A010C7d01b50e0d17dc79C8"}
                                    }
                                },
                                {
                                    "id":"assert_balance",
                                    "kind":"assert",
                                    "depends_on":["q_balance"],
                                    "inputs":{"condition":{"cel":"nodes.q_balance.outputs.balance != null"}}
                                },
                                {
                                    "id":"a_transfer_erc20",
                                    "kind":"action",
                                    "candidate_ref":"erc20@0.0.2/transfer",
                                    "depends_on":["assert_balance"],
                                    "inputs":{
                                        "token":{"object":{"address":{"lit":"0x8464135c8F25Da09e49BC8782676a84730C318bC"},"chain_ref":{"lit":"eip155:31338"}}},
                                        "to":{"lit":"0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"},
                                        "amount":{"lit":"10000000000000000000"}
                                    }
                                }
                            ],
                            "extensions":{}
                        },
                        "issues":[]
                    }
                }
            ]
        }))
        .expect("script line 4"),
        serde_json::to_string(&json!({
            "assistant_content":"revise after host autofill hint",
            "tool_calls":[{
                "id":"tool-revise",
                "name":"plan.revise_segment",
                "arguments":{
                    "status":"unavailable",
                    "done":false,
                    "error":{
                        "reason_code":"missing_required_input",
                        "message":"still missing recipient profile",
                        "details":{
                            "questions":[
                                {"id":"recipient","question":"recipient profile?"}
                            ]
                        }
                    }
                }
            }]
        }))
        .expect("script line 5"),
    ]
    .join("\n");
    write_temp_file(
        "agent-segmented-compile-autofill-resolvable-script",
        llm_script.as_str(),
    )
}

fn write_compile_write_gate_missing_retry_bounded_script() -> PathBuf {
    let llm_script = [
        serde_json::to_string(&json!({
            "assistant_content":"begin",
            "tool_calls":[{
                "id":"tool-begin",
                "name":"plan.begin",
                "arguments":{
                    "session_id":"sess-compile-bounded",
                    "snapshot_hash":"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
                    "cursor":"cursor-0",
                    "limits":{"max_rounds":8,"max_segments":4}
                }
            }]
        }))
        .expect("script line 1"),
        serde_json::to_string(&json!({
            "assistant_content":"ground",
            "tool_calls":[{
                "id":"tool-ground",
                "name":"plan.ground_intent",
                "arguments":{
                    "status":"proposed",
                    "ready_for_todos": true,
                    "resolved_inputs": {"owner":"0x70997970C51812dc3A010C7d01b50e0d17dc79C8"}
                }
            }]
        }))
        .expect("script line 2"),
        serde_json::to_string(&json!({
            "assistant_content":"todos",
            "tool_calls":[{
                "id":"tool-todos",
                "name":"plan.propose_todos",
                "arguments":{
                    "status":"proposed",
                    "todos":[{"title":"transfer erc20"}]
                }
            }]
        }))
        .expect("script line 3"),
        serde_json::to_string(&json!({
            "assistant_content":"propose still missing decimals gate",
            "tool_calls":[
                {
                    "id":"tool-check",
                    "name":"plan.check_segment",
                    "arguments":{
                        "segment":{
                            "segment_id":"seg-transfer",
                            "cursor_in":"cursor-0",
                            "cursor_out":"cursor-1",
                            "done":false,
                            "summary":"missing decimals gate",
                            "steps":[
                                {
                                    "id":"q_balance",
                                    "kind":"query",
                                    "candidate_ref":"erc20@0.0.2/balance-of",
                                    "inputs":{
                                        "token":{"object":{"address":{"lit":"0x8464135c8F25Da09e49BC8782676a84730C318bC"},"chain_ref":{"lit":"eip155:31338"}}},
                                        "owner":{"lit":"0x70997970C51812dc3A010C7d01b50e0d17dc79C8"}
                                    }
                                },
                                {
                                    "id":"assert_balance",
                                    "kind":"assert",
                                    "depends_on":["q_balance"],
                                    "inputs":{"condition":{"cel":"nodes.q_balance.outputs.balance != null"}}
                                },
                                {
                                    "id":"a_transfer_erc20",
                                    "kind":"action",
                                    "candidate_ref":"erc20@0.0.2/transfer",
                                    "depends_on":["assert_balance"],
                                    "inputs":{
                                        "token":{"object":{"address":{"lit":"0x8464135c8F25Da09e49BC8782676a84730C318bC"},"chain_ref":{"lit":"eip155:31338"}}},
                                        "to":{"lit":"0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"},
                                        "amount":{"lit":"10000000000000000000"}
                                    }
                                }
                            ],
                            "extensions":{}
                        }
                    }
                },
                {
                    "id":"tool-propose",
                    "name":"plan.propose_segment",
                    "arguments":{
                        "status":"proposed",
                        "done":false,
                        "cursor_next":"cursor-1",
                        "summary":"bad transfer draft",
                        "segment":{
                            "segment_id":"seg-transfer",
                            "cursor_in":"cursor-0",
                            "cursor_out":"cursor-1",
                            "done":false,
                            "summary":"missing decimals gate",
                            "steps":[
                                {
                                    "id":"q_balance",
                                    "kind":"query",
                                    "candidate_ref":"erc20@0.0.2/balance-of",
                                    "inputs":{
                                        "token":{"object":{"address":{"lit":"0x8464135c8F25Da09e49BC8782676a84730C318bC"},"chain_ref":{"lit":"eip155:31338"}}},
                                        "owner":{"lit":"0x70997970C51812dc3A010C7d01b50e0d17dc79C8"}
                                    }
                                },
                                {
                                    "id":"assert_balance",
                                    "kind":"assert",
                                    "depends_on":["q_balance"],
                                    "inputs":{"condition":{"cel":"nodes.q_balance.outputs.balance != null"}}
                                },
                                {
                                    "id":"a_transfer_erc20",
                                    "kind":"action",
                                    "candidate_ref":"erc20@0.0.2/transfer",
                                    "depends_on":["assert_balance"],
                                    "inputs":{
                                        "token":{"object":{"address":{"lit":"0x8464135c8F25Da09e49BC8782676a84730C318bC"},"chain_ref":{"lit":"eip155:31338"}}},
                                        "to":{"lit":"0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"},
                                        "amount":{"lit":"10000000000000000000"}
                                    }
                                }
                            ],
                            "extensions":{}
                        },
                        "issues":[]
                    }
                }
            ]
        }))
        .expect("script line 4"),
        serde_json::to_string(&json!({
            "assistant_content":"revise but still unresolved",
            "tool_calls":[
                {
                    "id":"tool-check-revise",
                    "name":"plan.check_segment",
                    "arguments":{
                        "segment":{
                            "segment_id":"seg-transfer",
                            "cursor_in":"cursor-1",
                            "cursor_out":"cursor-2",
                            "done":false,
                            "summary":"still missing decimals gate",
                            "steps":[
                                {
                                    "id":"q_balance_retry",
                                    "kind":"query",
                                    "candidate_ref":"erc20@0.0.2/balance-of",
                                    "inputs":{
                                        "token":{"object":{"address":{"lit":"0x8464135c8F25Da09e49BC8782676a84730C318bC"},"chain_ref":{"lit":"eip155:31338"}}},
                                        "owner":{"lit":"0x70997970C51812dc3A010C7d01b50e0d17dc79C8"}
                                    }
                                },
                                {
                                    "id":"assert_balance_retry",
                                    "kind":"assert",
                                    "depends_on":["q_balance_retry"],
                                    "inputs":{"condition":{"cel":"nodes.q_balance_retry.outputs.balance != null"}}
                                },
                                {
                                    "id":"a_transfer_erc20_retry",
                                    "kind":"action",
                                    "candidate_ref":"erc20@0.0.2/transfer",
                                    "depends_on":["assert_balance_retry"],
                                    "inputs":{
                                        "token":{"object":{"address":{"lit":"0x8464135c8F25Da09e49BC8782676a84730C318bC"},"chain_ref":{"lit":"eip155:31338"}}},
                                        "to":{"lit":"0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"},
                                        "amount":{"lit":"10000000000000000000"}
                                    }
                                }
                            ],
                            "extensions":{}
                        }
                    }
                },
                {
                    "id":"tool-revise",
                    "name":"plan.revise_segment",
                    "arguments":{
                        "status":"proposed",
                        "done":false,
                        "cursor_next":"cursor-2",
                        "summary":"still missing gate facts",
                        "segment":{
                            "segment_id":"seg-transfer",
                            "cursor_in":"cursor-1",
                            "cursor_out":"cursor-2",
                            "done":false,
                            "summary":"still missing decimals gate",
                            "steps":[
                                {
                                    "id":"q_balance_retry",
                                    "kind":"query",
                                    "candidate_ref":"erc20@0.0.2/balance-of",
                                    "inputs":{
                                        "token":{"object":{"address":{"lit":"0x8464135c8F25Da09e49BC8782676a84730C318bC"},"chain_ref":{"lit":"eip155:31338"}}},
                                        "owner":{"lit":"0x70997970C51812dc3A010C7d01b50e0d17dc79C8"}
                                    }
                                },
                                {
                                    "id":"assert_balance_retry",
                                    "kind":"assert",
                                    "depends_on":["q_balance_retry"],
                                    "inputs":{"condition":{"cel":"nodes.q_balance_retry.outputs.balance != null"}}
                                },
                                {
                                    "id":"a_transfer_erc20_retry",
                                    "kind":"action",
                                    "candidate_ref":"erc20@0.0.2/transfer",
                                    "depends_on":["assert_balance_retry"],
                                    "inputs":{
                                        "token":{"object":{"address":{"lit":"0x8464135c8F25Da09e49BC8782676a84730C318bC"},"chain_ref":{"lit":"eip155:31338"}}},
                                        "to":{"lit":"0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"},
                                        "amount":{"lit":"10000000000000000000"}
                                    }
                                }
                            ],
                            "extensions":{}
                        },
                        "issues":[]
                    }
                }
            ]
        }))
        .expect("script line 5"),
    ]
    .join("\n");
    write_temp_file(
        "agent-segmented-compile-autofill-bounded-script",
        llm_script.as_str(),
    )
}

fn write_missing_required_input_script() -> PathBuf {
    let llm_script = [serde_json::to_string(&json!({
            "assistant_content":"begin",
            "tool_calls":[
                {
                    "id":"tool-begin",
                    "name":"plan.begin",
                    "arguments":{
                        "session_id":"sess-1",
                        "snapshot_hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                        "cursor":"cursor-0",
                        "limits":{"max_rounds":8,"max_segments":4}
                    }
                }
            ]
        }))
        .expect("script line 1"),
        serde_json::to_string(&json!({
            "assistant_content":"ground intent",
            "tool_calls":[
                {
                    "id":"tool-ground",
                    "name":"plan.ground_intent",
                    "arguments":{
                        "status":"proposed",
                        "ready_for_todos":true,
                        "resolved_inputs":{"owner":"0x1111111111111111111111111111111111111111"}
                    }
                }
            ]
        }))
        .expect("script line 2"),
        serde_json::to_string(&json!({
            "assistant_content":"need more input",
            "tool_calls":[
                {
                    "id":"tool-todos",
                    "name":"plan.propose_todos",
                    "arguments":{
                        "status":"proposed",
                        "todos":[
                            {"title":"prepare transfer"}
                        ]
                    }
                }
            ]
        }))
        .expect("script line 3"),
        serde_json::to_string(&json!({
            "assistant_content":"need more input",
            "tool_calls":[
                {
                    "id":"tool-propose",
                    "name":"plan.propose_segment",
                    "arguments":{
                        "status":"unavailable",
                        "done":false,
                        "error":{
                            "reason_code":"missing_required_input",
                            "message":"missing token decimals",
                            "details":{
                                "questions":[
                                    {
                                        "id":"token.decimals",
                                        "question":"token decimals?",
                                        "options":[{"label":"18","value":18}]
                                    }
                                ]
                            }
                        }
                    }
                }
            ]
        }))
        .expect("script line 4")]
    .join("\n");
    write_temp_file("agent-segmented-missing-input-script", llm_script.as_str())
}

fn write_missing_required_object_input_script() -> PathBuf {
    let llm_script = [serde_json::to_string(&json!({
            "assistant_content":"begin",
            "tool_calls":[
                {
                    "id":"tool-begin",
                    "name":"plan.begin",
                    "arguments":{
                        "session_id":"sess-object-1",
                        "snapshot_hash":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                        "cursor":"cursor-0",
                        "limits":{"max_rounds":8,"max_segments":4}
                    }
                }
            ]
        }))
        .expect("script line 1"),
        serde_json::to_string(&json!({
            "assistant_content":"ground intent",
            "tool_calls":[
                {
                    "id":"tool-ground",
                    "name":"plan.ground_intent",
                    "arguments":{
                        "status":"proposed",
                        "ready_for_todos":true,
                        "resolved_inputs":{"owner":"0x1111111111111111111111111111111111111111"}
                    }
                }
            ]
        }))
        .expect("script line 2"),
        serde_json::to_string(&json!({
            "assistant_content":"need more input",
            "tool_calls":[
                {
                    "id":"tool-todos",
                    "name":"plan.propose_todos",
                    "arguments":{
                        "status":"proposed",
                        "todos":[
                            {"title":"prepare transfer"}
                        ]
                    }
                }
            ]
        }))
        .expect("script line 3"),
        serde_json::to_string(&json!({
            "assistant_content":"need more input",
            "tool_calls":[
                {
                    "id":"tool-propose",
                    "name":"plan.propose_segment",
                    "arguments":{
                        "status":"unavailable",
                        "done":false,
                        "error":{
                            "reason_code":"missing_required_input",
                            "message":"missing recipient profile",
                            "details":{
                                "questions":[
                                    {
                                        "id":"recipient.profile",
                                        "question":"recipient profile?",
                                        "options":[{
                                            "label":"ops wallet",
                                            "value":{"address":"0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266","chain_ref":"eip155:31338"}
                                        }]
                                    }
                                ]
                            }
                        }
                    }
                }
            ]
        }))
        .expect("script line 4")]
    .join("\n");
    write_temp_file(
        "agent-segmented-missing-object-input-script",
        llm_script.as_str(),
    )
}


struct FailingSegmentExecutor;

impl Executor for FailingSegmentExecutor {
    fn execute(&self, _node: &Value, _runtime: &mut Value) -> Result<ExecutorOutput, String> {
        Err("rpc timeout".to_string())
    }
}

fn plan_requiring_runtime_input() -> PlanDocument {
    PlanDocument {
        schema: "ais-plan/0.0.3".to_string(),
        meta: None,
        nodes: vec![json!({
            "id":"swap-1",
            "chain":"test",
            "execution":{"type":"test_exec","method":"swap"},
            "bindings":{
                "params":{
                    "spend_amount":{"ref":"inputs.amount"}
                }
            },
            "params":{
                "amount":{"ref":"params.spend_amount"}
            }
        })],
        extensions: Map::new(),
    }
}

fn plan_requiring_user_confirm(node_id: &str) -> PlanDocument {
    PlanDocument {
        schema: "ais-plan/0.0.3".to_string(),
        meta: None,
        nodes: vec![json!({
            "id": node_id,
            "chain": "test",
            "execution": {
                "type": "test_exec",
                "method": "transfer"
            },
            "extensions": {
                "risk_level": 4,
                "risk_tags": ["transfer"]
            },
            "params": {}
        })],
        extensions: Map::new(),
    }
}

#[test]
fn execute_agent_segmented_missing_required_input_pauses_instead_of_failing() {
    let fixture_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/runner-local/intent-segmented-offchain-transfer");
    let workspace_dir = fixture_root.join("workspace");
    let pack_path = workspace_dir.join("safe-defi.ais-pack.yaml");
    let config_path = fixture_root.join("config/runner.local.yaml");
    let intent_file = fixture_root.join("intent/intent.txt");
    let llm_script_path = write_missing_required_input_script();

    let command = AgentCommand {
        plan: None,
        intent: None,
        intent_file: Some(intent_file),
        workspace: Some(workspace_dir),
        config: config_path,
        pack: Some(pack_path),
        runtime: None,
        events_jsonl: None,
        trace: None,
        checkpoint: None,
        profile: AgentProfile::DemoScripted,
        llm_script_jsonl: Some(llm_script_path),
        verbose: false,
        verbose_llm: false,
        approvals_mode: Some(crate::cli::ApprovalsMode::Safe),
        max_iterations: Some(16),
        max_planner_rounds: Some(4),
        max_tool_rounds: None,
        max_index_candidates: None,
        planner_context_token_budget: None,
        llm_transcript_path: None,
        llm_transcript_append: false,
        format: OutputFormat::Json,
    };

    let output = super::execute_agent(&command).expect("missing input should pause, not fail");
    let parsed: Value = serde_json::from_str(output.as_str()).expect("json output");
    assert_eq!(parsed.get("status").and_then(Value::as_str), Some("paused"));
    assert_eq!(
        parsed.get("paused_reason").and_then(Value::as_str),
        Some("missing_required_input")
    );
    assert!(
        parsed
            .pointer("/llm_usage/calls")
            .and_then(Value::as_u64)
            .is_some_and(|calls| calls >= 4),
        "missing-input pause flow should include begin/ground/todos/propose calls: {parsed}"
    );
}

#[test]
fn execute_agent_segmented_missing_required_object_input_keeps_same_pause_contract() {
    let checkpoint_path = write_temp_file("agent-segmented-object-input-checkpoint", "{}");
    let _ = fs::remove_file(&checkpoint_path);
    let command = build_segmented_demo_command(
        write_missing_required_object_input_script(),
        Some(checkpoint_path.clone()),
        None,
    );
    let parsed = parse_agent_output_json(&command);
    assert_eq!(parsed.get("status").and_then(Value::as_str), Some("paused"));
    assert_eq!(
        parsed.get("paused_reason").and_then(Value::as_str),
        Some("missing_required_input")
    );
    assert!(
        parsed
            .pointer("/llm_usage/calls")
            .and_then(Value::as_u64)
            .is_some_and(|calls| calls >= 4),
        "missing-input pause flow should include begin/ground/todos/propose calls: {parsed}"
    );

    let checkpoint_text = fs::read_to_string(checkpoint_path).expect("checkpoint file");
    let checkpoint: Value = serde_json::from_str(checkpoint_text.as_str()).expect("checkpoint");
    assert_eq!(
        checkpoint.pointer("/runtime_snapshot/agent/missing_required_input/reason_code"),
        Some(&json!("missing_required_input"))
    );
    assert_eq!(
        checkpoint.pointer("/runtime_snapshot/agent/missing_required_input/questions/0/id"),
        Some(&json!("recipient.profile"))
    );
    assert_eq!(
        checkpoint.pointer("/runtime_snapshot/agent/missing_required_input/questions/0/question"),
        Some(&json!("recipient profile?"))
    );
}

#[test]
fn compile_write_gate_missing_runs_host_autofill_revise_before_missing_input_pause() {
    let command = build_segmented_native_erc20_command(
        write_compile_write_gate_missing_then_revise_missing_input_script(),
    );
    let parsed = parse_agent_output_json(&command);
    assert_eq!(parsed.get("status").and_then(Value::as_str), Some("paused"));
    assert_eq!(
        parsed.get("paused_reason").and_then(Value::as_str),
        Some("missing_required_input")
    );
    assert_eq!(
        parsed.pointer("/llm_usage/diagnostics/phase_round_count/ground_intent"),
        Some(&json!(1))
    );
    assert_eq!(
        parsed.pointer("/llm_usage/diagnostics/phase_round_count/revise_segment"),
        Some(&json!(1))
    );
}

#[test]
fn compile_autofill_retry_is_bounded_to_single_revise_round() {
    let command = build_segmented_native_erc20_command(
        write_compile_write_gate_missing_retry_bounded_script(),
    );
    let parsed = parse_agent_output_json(&command);
    assert_eq!(parsed.get("status").and_then(Value::as_str), Some("paused"));
    assert_eq!(
        parsed.get("paused_reason").and_then(Value::as_str),
        Some("missing_required_input")
    );
    assert_eq!(
        parsed.pointer("/llm_usage/diagnostics/phase_round_count/ground_intent"),
        Some(&json!(1))
    );
    assert_eq!(
        parsed.pointer("/llm_usage/diagnostics/phase_round_count/revise_segment"),
        Some(&json!(1))
    );
}

#[test]
fn execute_agent_segmented_intent_fixture_queries_then_pauses_for_confirm() {
    let fixture_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/runner-local/intent-segmented-offchain-transfer");
    let workspace_dir = fixture_root.join("workspace");
    let pack_path = workspace_dir.join("safe-defi.ais-pack.yaml");
    let config_path = fixture_root.join("config/runner.local.yaml");
    let intent_file = fixture_root.join("intent/intent.txt");
    let llm_template_path = fixture_root.join("llm/segmented.success.template.jsonl");
    let llm_template = fs::read_to_string(llm_template_path).expect("llm template");
    let llm_script = llm_template.replace("__MOCK_BASE_URL__", "http://offline.local");
    let llm_script_path = write_temp_file("agent-segmented-e2e-script", llm_script.as_str());
    let seed_command = AgentCommand {
        plan: None,
        intent: None,
        intent_file: Some(intent_file.clone()),
        workspace: Some(workspace_dir.clone()),
        config: config_path,
        pack: Some(pack_path.clone()),
        runtime: None,
        events_jsonl: None,
        trace: None,
        checkpoint: None,
        profile: AgentProfile::DemoScripted,
        llm_script_jsonl: Some(llm_script_path),
        verbose: false,
        verbose_llm: false,
        approvals_mode: Some(crate::cli::ApprovalsMode::Safe),
        max_iterations: Some(24),
        max_planner_rounds: Some(6),
        max_tool_rounds: None,
        max_index_candidates: None,
        planner_context_token_budget: None,
        llm_transcript_path: None,
        llm_transcript_append: false,
        format: OutputFormat::Json,
    };

    let pack = crate::policy::load_pack_document(&pack_path).expect("pack");
    let candidate_context = super::candidates::build_candidate_context_for_agent(
        &seed_command,
        Some(&pack),
        super::candidates::DEFAULT_MAX_INDEX_CANDIDATES,
    )
    .expect("candidate context")
    .expect("workspace candidates");
    let llm_provider = super::load_scripted_llm_provider(
        seed_command
            .llm_script_jsonl
            .as_ref()
            .expect("script path")
            .as_path(),
    )
    .expect("script provider");
    let mut planner = super::intent_segmented::LlmSegmentedIntentPlanner::new(llm_provider)
        .with_candidate_context(Some(candidate_context.clone()))
        .with_verbose_llm(false);
    let intent = fs::read_to_string(intent_file)
        .expect("intent file")
        .trim()
        .to_string();
    let chain_scope = super::derive_chain_scope(&candidate_context);
    let pack_snapshot_hash = super::derive_pack_snapshot_hash(Some(&pack)).expect("pack hash");
    let snapshot_hash = super::derive_planning_snapshot_hash(
        pack_snapshot_hash.as_str(),
        candidate_context
            .executable_candidates
            .catalog_hash
            .as_str(),
        chain_scope.as_slice(),
        Some(crate::cli::ApprovalsMode::Safe),
    )
    .expect("snapshot hash");
    let mut session = planner
        .begin_session(super::intent_segmented::SegmentBeginRequest {
            intent: intent.clone(),
            snapshot_hash,
            pack_snapshot_hash,
            catalog_hash: candidate_context.executable_candidates.catalog_hash.clone(),
            chain_scope: chain_scope.clone(),
        })
        .expect("begin session");
    let mut router = RouterExecutor::new();
    router.register(
        "offchain_apy_query",
        "eip155:31338",
        Box::new(SegmentedFixtureExecutor),
    );
    let mut state = EngineRunnerState::default();
    let mut active_plan = super::empty_plan_document();

    let draft_first = planner
        .propose_segment(super::intent_segmented::SegmentPlanningRequest {
            intent: intent.clone(),
            session: session.clone(),
            state_summary: None,
            previous_error: None,
            last_segment: None,
        })
        .expect("first segment");
    let (first_segment, first_cursor_next, first_done) = match draft_first {
        super::intent_segmented::SegmentDraft::Proposed {
            segment,
            cursor_next,
            done,
            ..
        } => (segment, cursor_next, done),
        _ => panic!("first segment must be proposed"),
    };
    assert!(!first_done);
    let segment_plan = super::compile_segment_plan(
        intent.as_str(),
        &session,
        &first_segment,
        &candidate_context,
        Some(&pack),
        chain_scope.as_slice(),
    )
    .expect("compile first segment");
    active_plan = super::merge_segment_plan(&active_plan, &segment_plan).expect("merge first");
    let mut options_segment1 = EngineRunnerOptions::default();
    options_segment1.policy = crate::policy::policy_from_pack(&pack).expect("policy");
    options_segment1.policy.thresholds.max_risk_level = None;
    let first_run = run_plan_once(
        "run-segmented-e2e",
        &active_plan,
        &mut state,
        &router,
        &DefaultSolver,
        &[],
        &options_segment1,
    );
    assert_eq!(first_run.status, EngineRunStatus::Completed);
    assert_eq!(
        state
            .runtime
            .pointer("/nodes/seg_balance__q_native_balance/outputs/balance")
            .and_then(Value::as_str),
        Some("200")
    );
    assert_eq!(
        state
            .runtime
            .pointer("/nodes/seg_balance__q_token_balance/outputs/balance")
            .and_then(Value::as_str),
        Some("260")
    );
    session.cursor = first_cursor_next;

    let draft_second = planner
        .propose_segment(super::intent_segmented::SegmentPlanningRequest {
            intent: intent.clone(),
            session: session.clone(),
            state_summary: None,
            previous_error: None,
            last_segment: Some(first_segment),
        })
        .expect("second segment");
    let (second_segment, second_done) = match draft_second {
        super::intent_segmented::SegmentDraft::Proposed { segment, done, .. } => (segment, done),
        _ => panic!("second segment must be proposed"),
    };
    assert!(second_done);
    let second_segment_plan = super::compile_segment_plan(
        intent.as_str(),
        &session,
        &second_segment,
        &candidate_context,
        Some(&pack),
        chain_scope.as_slice(),
    )
    .expect("compile second segment");
    active_plan =
        super::merge_segment_plan(&active_plan, &second_segment_plan).expect("merge second");
    let mut options_segment2 = EngineRunnerOptions::default();
    options_segment2.policy = crate::policy::policy_from_pack(&pack).expect("policy");
    let second_run = run_plan_once(
        "run-segmented-e2e",
        &active_plan,
        &mut state,
        &router,
        &DefaultSolver,
        &[],
        &options_segment2,
    );
    assert_eq!(second_run.status, EngineRunStatus::Paused);
    assert_eq!(
        state.paused_reason.as_deref(),
        Some("need_user_confirm:seg_transfer__a_transfer_native_5")
    );
    let need_confirm = second_run
        .events
        .iter()
        .find(|record| {
            record.event.event_type == ais_engine::EngineEventType::NeedUserConfirm
                && record.event.node_id.as_deref() == Some("seg_transfer__a_transfer_native_5")
        })
        .expect("need_user_confirm event for transfer segment");
    assert_eq!(
        need_confirm
            .event
            .data
            .get("reason_code")
            .and_then(Value::as_str),
        Some("threshold_risk_level_exceeded")
    );
    assert!(second_run.events.iter().any(|record| {
        record.event.event_type == ais_engine::EngineEventType::NeedUserConfirm
            && record.event.node_id.as_deref() == Some("seg_transfer__a_transfer_native_5")
    }));
}

#[test]
fn segmented_intent_fixture_revise_with_until_retry_then_complete() {
    let fixture_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/runner-local/intent-segmented-offchain-transfer");
    let workspace_dir = fixture_root.join("workspace");
    let pack_path = workspace_dir.join("safe-defi.ais-pack.yaml");
    let intent_file = fixture_root.join("intent/intent.txt");
    let llm_template_path = fixture_root.join("llm/segmented.until-retry.repair.template.jsonl");
    let llm_template = fs::read_to_string(llm_template_path).expect("llm template");
    let llm_script = llm_template.replace("__MOCK_BASE_URL__", "http://offline.local");
    let llm_script_path =
        write_temp_file("agent-segmented-revise-retry-script", llm_script.as_str());

    let seed_command = AgentCommand {
        plan: None,
        intent: None,
        intent_file: Some(intent_file.clone()),
        workspace: Some(workspace_dir.clone()),
        config: fixture_root.join("config/runner.local.yaml"),
        pack: Some(pack_path.clone()),
        runtime: None,
        events_jsonl: None,
        trace: None,
        checkpoint: None,
        profile: AgentProfile::DemoScripted,
        llm_script_jsonl: Some(llm_script_path),
        verbose: false,
        verbose_llm: false,
        approvals_mode: Some(crate::cli::ApprovalsMode::Safe),
        max_iterations: Some(24),
        max_planner_rounds: Some(8),
        max_tool_rounds: None,
        max_index_candidates: None,
        planner_context_token_budget: None,
        llm_transcript_path: None,
        llm_transcript_append: false,
        format: OutputFormat::Json,
    };

    let pack = crate::policy::load_pack_document(&pack_path).expect("pack");
    let candidate_context = super::candidates::build_candidate_context_for_agent(
        &seed_command,
        Some(&pack),
        super::candidates::DEFAULT_MAX_INDEX_CANDIDATES,
    )
    .expect("candidate context")
    .expect("workspace candidates");
    let llm_provider = super::load_scripted_llm_provider(
        seed_command
            .llm_script_jsonl
            .as_ref()
            .expect("script path")
            .as_path(),
    )
    .expect("script provider");
    let mut planner = super::intent_segmented::LlmSegmentedIntentPlanner::new(llm_provider)
        .with_candidate_context(Some(candidate_context.clone()))
        .with_verbose_llm(false);
    let intent = fs::read_to_string(intent_file)
        .expect("intent file")
        .trim()
        .to_string();
    let chain_scope = super::derive_chain_scope(&candidate_context);
    let pack_snapshot_hash = super::derive_pack_snapshot_hash(Some(&pack)).expect("pack hash");
    let snapshot_hash = super::derive_planning_snapshot_hash(
        pack_snapshot_hash.as_str(),
        candidate_context
            .executable_candidates
            .catalog_hash
            .as_str(),
        chain_scope.as_slice(),
        Some(crate::cli::ApprovalsMode::Safe),
    )
    .expect("snapshot hash");
    let mut session = planner
        .begin_session(super::intent_segmented::SegmentBeginRequest {
            intent: intent.clone(),
            snapshot_hash,
            pack_snapshot_hash,
            catalog_hash: candidate_context.executable_candidates.catalog_hash.clone(),
            chain_scope: chain_scope.clone(),
        })
        .expect("begin session");
    let mut router = RouterExecutor::new();
    router.register(
        "offchain_apy_query",
        "eip155:31338",
        Box::new(SegmentedUntilRetryExecutor),
    );
    let mut state = EngineRunnerState::default();
    let mut active_plan = super::empty_plan_document();

    let draft_first = planner
        .propose_segment(super::intent_segmented::SegmentPlanningRequest {
            intent: intent.clone(),
            session: session.clone(),
            state_summary: None,
            previous_error: None,
            last_segment: None,
        })
        .expect("first segment");
    let (first_segment, first_cursor_next, first_done) = match draft_first {
        super::intent_segmented::SegmentDraft::Proposed {
            segment,
            cursor_next,
            done,
            ..
        } => (segment, cursor_next, done),
        _ => panic!("first segment must be proposed"),
    };
    assert!(!first_done);
    let first_plan = super::compile_segment_plan(
        intent.as_str(),
        &session,
        &first_segment,
        &candidate_context,
        Some(&pack),
        chain_scope.as_slice(),
    )
    .expect("compile first segment");
    active_plan = super::merge_segment_plan(&active_plan, &first_plan).expect("merge first");

    let first_run = run_plan_once(
        "run-segmented-revise-retry",
        &active_plan,
        &mut state,
        &router,
        &DefaultSolver,
        &[],
        &EngineRunnerOptions::default(),
    );
    assert_eq!(first_run.status, EngineRunStatus::Completed);
    session.cursor = first_cursor_next;

    let draft_second = planner
        .propose_segment(super::intent_segmented::SegmentPlanningRequest {
            intent: intent.clone(),
            session: session.clone(),
            state_summary: None,
            previous_error: None,
            last_segment: Some(first_segment.clone()),
        })
        .expect("second segment");
    let (second_segment, second_cursor_next, second_done) = match draft_second {
        super::intent_segmented::SegmentDraft::Proposed {
            segment,
            cursor_next,
            done,
            ..
        } => (segment, cursor_next, done),
        _ => panic!("second segment must be proposed"),
    };
    assert!(second_done);
    assert!(second_segment
        .steps
        .iter()
        .find(|step| step.id == "a_transfer_native_5")
        .and_then(|step| step.retry.as_ref())
        .is_some());
    let second_plan = super::compile_segment_plan(
        intent.as_str(),
        &session,
        &second_segment,
        &candidate_context,
        Some(&pack),
        chain_scope.as_slice(),
    )
    .expect("compile second segment");
    let transfer_node = second_plan
        .nodes
        .iter()
        .find(|node| {
            node.get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| id == "seg_transfer__a_transfer_native_5")
        })
        .expect("transfer node");
    assert_eq!(
        transfer_node
            .pointer("/retry/interval_ms")
            .and_then(Value::as_u64),
        Some(1000)
    );
    assert_eq!(
        transfer_node
            .pointer("/retry/max_attempts")
            .and_then(Value::as_u64),
        Some(3)
    );
    assert_eq!(
        transfer_node.pointer("/timeout_ms").and_then(Value::as_u64),
        Some(5000)
    );

    active_plan = super::merge_segment_plan(&active_plan, &second_plan).expect("merge second");
    state
        .approved_node_ids
        .push("seg_transfer__a_transfer_native_5".to_string());

    let retry_run = run_plan_once(
        "run-segmented-revise-retry",
        &active_plan,
        &mut state,
        &router,
        &DefaultSolver,
        &[],
        &EngineRunnerOptions::default(),
    );
    assert_eq!(retry_run.status, EngineRunStatus::Paused);
    assert!(
        retry_run.events.iter().any(|record| {
            record.event.event_type == ais_engine::EngineEventType::NodeWaiting
                && record.event.node_id.as_deref() == Some("seg_transfer__a_transfer_native_5")
                && record.event.data.get("reason") == Some(&json!("until_retry"))
        }),
        "paused_reason={:?}, events={:?}",
        state.paused_reason,
        retry_run.events
    );
    assert!(state
        .pending_retries
        .get("seg_transfer__a_transfer_native_5")
        .is_some());

    let complete_run = run_plan_once(
        "run-segmented-revise-retry",
        &active_plan,
        &mut state,
        &router,
        &DefaultSolver,
        &[],
        &EngineRunnerOptions::default(),
    );
    assert_eq!(complete_run.status, EngineRunStatus::Completed);
    assert!(state.pending_retries.is_empty());
    assert_eq!(
        state
            .runtime
            .pointer("/nodes/seg_transfer__a_transfer_native_5/outputs/outputs/confirmed")
            .and_then(Value::as_bool),
        Some(true)
    );
    session.cursor = second_cursor_next;
}

#[test]
fn segmented_intent_fixture_repairs_format_then_compiles_assert_branch_segment() {
    let fixture_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/runner-local/intent-segmented-offchain-transfer");
    let workspace_dir = fixture_root.join("workspace");
    let pack_path = workspace_dir.join("safe-defi.ais-pack.yaml");
    let intent_file = fixture_root.join("intent/intent.txt");
    let llm_template_path = fixture_root.join("llm/segmented.format-repair.template.jsonl");
    let llm_template = fs::read_to_string(llm_template_path).expect("llm template");
    let llm_script = llm_template.replace("__MOCK_BASE_URL__", "http://offline.local");
    let llm_script_path =
        write_temp_file("agent-segmented-format-repair-script", llm_script.as_str());

    let seed_command = AgentCommand {
        plan: None,
        intent: None,
        intent_file: Some(intent_file.clone()),
        workspace: Some(workspace_dir.clone()),
        config: fixture_root.join("config/runner.local.yaml"),
        pack: Some(pack_path.clone()),
        runtime: None,
        events_jsonl: None,
        trace: None,
        checkpoint: None,
        profile: AgentProfile::DemoScripted,
        llm_script_jsonl: Some(llm_script_path),
        verbose: false,
        verbose_llm: false,
        approvals_mode: Some(crate::cli::ApprovalsMode::Safe),
        max_iterations: Some(24),
        max_planner_rounds: Some(8),
        max_tool_rounds: None,
        max_index_candidates: None,
        planner_context_token_budget: None,
        llm_transcript_path: None,
        llm_transcript_append: false,
        format: OutputFormat::Json,
    };

    let pack = crate::policy::load_pack_document(&pack_path).expect("pack");
    let candidate_context = super::candidates::build_candidate_context_for_agent(
        &seed_command,
        Some(&pack),
        super::candidates::DEFAULT_MAX_INDEX_CANDIDATES,
    )
    .expect("candidate context")
    .expect("workspace candidates");
    let llm_provider = super::load_scripted_llm_provider(
        seed_command
            .llm_script_jsonl
            .as_ref()
            .expect("script path")
            .as_path(),
    )
    .expect("script provider");
    let mut planner = super::intent_segmented::LlmSegmentedIntentPlanner::new(llm_provider)
        .with_candidate_context(Some(candidate_context.clone()))
        .with_verbose_llm(false);
    let intent = fs::read_to_string(intent_file)
        .expect("intent file")
        .trim()
        .to_string();
    let chain_scope = super::derive_chain_scope(&candidate_context);
    let pack_snapshot_hash = super::derive_pack_snapshot_hash(Some(&pack)).expect("pack hash");
    let snapshot_hash = super::derive_planning_snapshot_hash(
        pack_snapshot_hash.as_str(),
        candidate_context
            .executable_candidates
            .catalog_hash
            .as_str(),
        chain_scope.as_slice(),
        Some(crate::cli::ApprovalsMode::Safe),
    )
    .expect("snapshot hash");
    let mut session = planner
        .begin_session(super::intent_segmented::SegmentBeginRequest {
            intent: intent.clone(),
            snapshot_hash,
            pack_snapshot_hash,
            catalog_hash: candidate_context.executable_candidates.catalog_hash.clone(),
            chain_scope: chain_scope.clone(),
        })
        .expect("begin session");

    let first_invalid_error = planner
        .propose_segment(super::intent_segmented::SegmentPlanningRequest {
            intent: intent.clone(),
            session: session.clone(),
            state_summary: None,
            previous_error: None,
            last_segment: None,
        })
        .expect_err("malformed segment string must fail");
    assert!(first_invalid_error
        .to_string()
        .contains("proposed segment draft `segment` string must be valid JSON object text"));
    let first_failed_finalize = planner
        .take_last_failed_finalize()
        .expect("failed finalize payload captured");
    assert_eq!(
        first_failed_finalize.get("tool").and_then(Value::as_str),
        Some("plan.propose_segment")
    );
    assert_eq!(
        first_failed_finalize.pointer("/arguments/status"),
        Some(&json!("proposed"))
    );
    let first_invalid_payload = super::segmented_planner_output_error_payload(
        &first_invalid_error,
        "plan.propose_segment",
        1,
        1,
        Some(first_failed_finalize),
    );
    let draft_first = planner
        .revise_segment(super::intent_segmented::SegmentPlanningRequest {
            intent: intent.clone(),
            session: session.clone(),
            state_summary: Some(json!({
                "completed_segments": 0,
                "previous_error": first_invalid_payload
            })),
            previous_error: Some(json!({
                "phase": "planning",
                "reason_code": "planner_invalid_tool_output"
            })),
            last_segment: None,
        })
        .expect("first segment revised");
    let (first_segment, first_cursor_next, first_done) = match draft_first {
        super::intent_segmented::SegmentDraft::Proposed {
            segment,
            cursor_next,
            done,
            ..
        } => (segment, cursor_next, done),
        _ => panic!("first segment must be proposed"),
    };
    assert!(!first_done);
    let first_plan = super::compile_segment_plan(
        intent.as_str(),
        &session,
        &first_segment,
        &candidate_context,
        Some(&pack),
        chain_scope.as_slice(),
    )
    .expect("compile first repaired segment");
    assert_eq!(first_plan.nodes.len(), 1);
    session.cursor = first_cursor_next;

    let draft_second = planner
        .propose_segment(super::intent_segmented::SegmentPlanningRequest {
            intent: intent.clone(),
            session: session.clone(),
            state_summary: None,
            previous_error: None,
            last_segment: Some(first_segment.clone()),
        })
        .expect("second segment");
    let (second_segment, second_cursor_next, second_done) = match draft_second {
        super::intent_segmented::SegmentDraft::Proposed {
            segment,
            cursor_next,
            done,
            ..
        } => (segment, cursor_next, done),
        _ => panic!("second segment must be proposed"),
    };
    assert!(second_done);
    let second_plan = super::compile_segment_plan(
        intent.as_str(),
        &session,
        &second_segment,
        &candidate_context,
        Some(&pack),
        chain_scope.as_slice(),
    )
    .expect("compile second segment");
    assert_eq!(second_plan.nodes.len(), 2);

    let query_node = second_plan
        .nodes
        .iter()
        .find(|node| {
            node.get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| id == "seg_transfer__q_native_balance")
        })
        .expect("query node");
    assert_eq!(
        query_node.pointer("/kind").and_then(Value::as_str),
        Some("query_ref")
    );

    let transfer_node = second_plan
        .nodes
        .iter()
        .find(|node| {
            node.get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| id == "seg_transfer__a_transfer_native_5")
        })
        .expect("transfer node");
    assert_eq!(
        transfer_node.pointer("/kind").and_then(Value::as_str),
        Some("action_ref")
    );
    assert_eq!(
        transfer_node.pointer("/deps/0").and_then(Value::as_str),
        Some("seg_transfer__q_native_balance")
    );
    assert_eq!(
        transfer_node
            .pointer("/condition/cel")
            .and_then(Value::as_str),
        Some("nodes.seg_transfer__q_native_balance.outputs.balance != null")
    );
    session.cursor = second_cursor_next;
}

#[test]
fn planner_invalid_output_error_is_retryable() {
    let error = RunnerError::Llm(
        "proposed segment draft `segment` must be a JSON object (stringified JSON is not allowed)"
            .to_string(),
    );
    assert!(super::should_retry_segmented_planner_output(&error));
}

#[test]
fn planner_decode_non_object_error_is_retryable_and_mapped() {
    let error = RunnerError::Llm(
        "proposed segment draft `segment` must decode to a JSON object".to_string(),
    );
    assert!(super::should_retry_segmented_planner_output(&error));
    let payload =
        super::segmented_planner_output_error_payload(&error, "plan.revise_segment", 2, 1, None);
    assert_eq!(
        payload.get("sub_reason_code").and_then(Value::as_str),
        Some("segment_not_json")
    );
}

#[test]
fn planner_missing_error_payload_is_retryable() {
    let error = RunnerError::Llm("invalid segment draft requires `error`".to_string());
    assert!(super::should_retry_segmented_planner_output(&error));
    let payload =
        super::segmented_planner_output_error_payload(&error, "plan.revise_segment", 2, 1, None);
    assert_eq!(
        payload.get("sub_reason_code").and_then(Value::as_str),
        Some("missing_error")
    );
}

#[test]
fn planner_no_tool_calls_is_retryable() {
    let error = RunnerError::Llm("segmented planner provider returned no tool calls".to_string());
    assert!(super::should_retry_segmented_planner_output(&error));
}

#[test]
fn planner_unrelated_error_is_not_retryable() {
    let llm_error = RunnerError::Llm("network timeout".to_string());
    assert!(!super::should_retry_segmented_planner_output(&llm_error));
    let io_error = RunnerError::WorkflowCompile("x".to_string());
    assert!(!super::should_retry_segmented_planner_output(&io_error));
}

#[test]
fn execution_pause_retry_table_is_prefix_driven() {
    assert!(super::should_attempt_intent_repair(Some(
        "executor_error:swap failed"
    )));
    assert!(super::should_attempt_intent_repair(Some("assert_failed:x")));
    assert!(super::should_attempt_intent_repair(Some(
        "condition_failed:x"
    )));
    assert!(!super::should_attempt_intent_repair(Some(
        "need_user_confirm:node-1"
    )));
}

#[test]
fn resolve_pause_backflow_preserves_missing_input_and_confirm_split() {
    let mut missing_input_router = RouterExecutor::new();
    missing_input_router.register("test-exec", "test", Box::new(TestExecutor));
    let mut missing_input_state = EngineRunnerState::default();
    let missing_input_run = run_plan_once(
        "run-resolve-pause-missing-input",
        &plan_requiring_runtime_input(),
        &mut missing_input_state,
        &missing_input_router,
        &DefaultSolver,
        &[],
        &EngineRunnerOptions::default(),
    );
    assert_eq!(missing_input_run.status, EngineRunStatus::Paused);
    assert_eq!(
        missing_input_state.paused_reason.as_deref(),
        Some("need_user_input:swap-1")
    );
    let mut input_store = super::InputStore::default();
    let missing_input_backflow = super::phase_machine::pause::resolve_execution_pause_backflow(
        &mut missing_input_state,
        &mut input_store,
        missing_input_run.events.as_slice(),
        1,
    )
    .expect("missing input backflow");
    assert!(matches!(
        missing_input_backflow,
        super::phase_machine::pause::ResolvePauseBackflow::MissingRequiredInputPaused
    ));
    assert_eq!(
        missing_input_state.paused_reason.as_deref(),
        Some("missing_required_input")
    );

    let mut confirm_router = RouterExecutor::new();
    confirm_router.register("test-exec", "test", Box::new(TestExecutor));
    let mut confirm_state = EngineRunnerState::default();
    let mut confirm_options = EngineRunnerOptions::default();
    confirm_options.policy.thresholds.max_risk_level = Some(0);
    let confirm_run = run_plan_once(
        "run-resolve-pause-confirm",
        &plan_requiring_user_confirm("transfer-1"),
        &mut confirm_state,
        &confirm_router,
        &DefaultSolver,
        &[],
        &confirm_options,
    );
    assert_eq!(confirm_run.status, EngineRunStatus::Paused);
    assert_eq!(
        confirm_state.paused_reason.as_deref(),
        Some("need_user_confirm:transfer-1")
    );
    let confirm_backflow = super::phase_machine::pause::resolve_execution_pause_backflow(
        &mut confirm_state,
        &mut input_store,
        confirm_run.events.as_slice(),
        1,
    )
    .expect("confirm backflow");
    match confirm_backflow {
        super::phase_machine::pause::ResolvePauseBackflow::PauseTerminal { blocked_reason } => {
            assert_eq!(blocked_reason, "need_user_confirm:transfer-1");
        }
        other => panic!("expected terminal need_user_confirm pause, got {other:?}"),
    }
}

#[test]
fn resolve_pause_backflow_condition_failed_keeps_structured_missing_input_signal_for_repair() {
    let mut state = EngineRunnerState {
        paused_reason: Some("condition_failed:transfer-1".to_string()),
        ..EngineRunnerState::default()
    };
    let mut input_store = super::InputStore::default();
    let mut last_error = ais_engine::EngineEvent::new(ais_engine::EngineEventType::Error);
    last_error.node_id = Some("transfer-1".to_string());
    last_error.data = Map::from_iter([
        ("reason_code".to_string(), json!("condition_failed")),
        (
            "reason".to_string(),
            json!("missing_inputs_or_runtime_refs"),
        ),
        (
            "details".to_string(),
            json!({
                "missing_refs":["inputs.token.decimals"],
                "suggested_paths":["inputs.token.decimals"]
            }),
        ),
    ]);
    let last_error_record =
        ais_engine::EngineEventRecord::new("run-1", 11, "1970-01-01T00:00:01Z", last_error);

    let backflow = super::phase_machine::pause::resolve_execution_pause_backflow(
        &mut state,
        &mut input_store,
        &[last_error_record],
        3,
    )
    .expect("condition_failed backflow");
    let previous_error = match backflow {
        super::phase_machine::pause::ResolvePauseBackflow::RepairScheduled { previous_error } => {
            previous_error
        }
        other => panic!("expected repair schedule for condition_failed pause, got {other:?}"),
    };
    assert_eq!(
        previous_error.get("reason_code").and_then(Value::as_str),
        Some("execution_paused")
    );
    assert_eq!(
        previous_error.get("sub_reason_code").and_then(Value::as_str),
        Some("condition_failed")
    );
    assert_eq!(
        previous_error.pointer("/last_error/data/reason"),
        Some(&json!("missing_inputs_or_runtime_refs"))
    );
}

#[test]
fn execution_pause_payload_uses_compatible_reason_subreason_codes() {
    let mut first_error = ais_engine::EngineEvent::new(ais_engine::EngineEventType::Error);
    first_error.node_id = Some("seg_transfer__a_transfer_native_5".to_string());
    first_error.data = Map::from_iter([
        ("reason_code".to_string(), json!("executor_error")),
        ("message".to_string(), json!("rpc timeout")),
    ]);
    let first_error_record =
        ais_engine::EngineEventRecord::new("run-1", 10, "1970-01-01T00:00:00Z", first_error);

    let mut last_error = ais_engine::EngineEvent::new(ais_engine::EngineEventType::Error);
    last_error.node_id = Some("seg_transfer__a_transfer_native_5".to_string());
    last_error.data = Map::from_iter([
        ("reason_code".to_string(), json!("executor_error")),
        ("message".to_string(), json!("rpc timeout retry")),
    ]);
    let last_error_record =
        ais_engine::EngineEventRecord::new("run-1", 11, "1970-01-01T00:00:01Z", last_error);

    let retryable_payload = super::intent_execution_error_payload(
        Some("executor_error:rpc_timeout"),
        &[first_error_record, last_error_record],
        6,
    );
    assert_eq!(
        retryable_payload.get("phase").and_then(Value::as_str),
        Some("execution")
    );
    assert_eq!(
        retryable_payload.get("reason_code").and_then(Value::as_str),
        Some("execution_paused")
    );
    assert_eq!(
        retryable_payload
            .get("sub_reason_code")
            .and_then(Value::as_str),
        Some("executor_error")
    );
    assert_eq!(
        retryable_payload.get("retryable").and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        retryable_payload.pointer("/last_error/data/message"),
        Some(&json!("rpc timeout retry"))
    );

    let non_retryable_payload =
        super::intent_execution_error_payload(Some("need_user_confirm:node-1"), &[], 6);
    assert_eq!(
        non_retryable_payload
            .get("reason_code")
            .and_then(Value::as_str),
        Some("execution_paused")
    );
    assert_eq!(
        non_retryable_payload
            .get("sub_reason_code")
            .and_then(Value::as_str),
        Some("non_retryable_pause")
    );
    assert_eq!(
        non_retryable_payload
            .get("retryable")
            .and_then(Value::as_bool),
        Some(false)
    );
}

#[test]
fn planner_error_payload_subreason_enum_is_backward_compatible() {
    let cases = [
        ("invalid plan.propose_segment args", "invalid_tool_args"),
        (
            "proposed segment draft `segment` string must be valid JSON object text",
            "segment_not_json",
        ),
        (
            "proposed segment draft `segment` must decode to a JSON object",
            "segment_not_json",
        ),
        (
            "segmented planner provider returned no tool calls",
            "no_tool_calls",
        ),
        (
            "proposed segment draft requires `segment`",
            "missing_segment",
        ),
        ("invalid segment draft requires `error`", "missing_error"),
        ("invalid segment draft status", "invalid_status"),
        ("unmapped planner failure", "unknown"),
    ];
    for (message, expected_sub_reason) in cases {
        let payload = super::segmented_planner_output_error_payload(
            &RunnerError::Llm(message.to_string()),
            "plan.revise_segment",
            8,
            1,
            None,
        );
        assert_eq!(
            payload.get("reason_code").and_then(Value::as_str),
            Some("planner_invalid_tool_output")
        );
        assert_eq!(
            payload.get("sub_reason_code").and_then(Value::as_str),
            Some(expected_sub_reason)
        );
        assert_eq!(
            payload.get("phase_reason_code").and_then(Value::as_str),
            Some(format!("planning.{expected_sub_reason}").as_str())
        );
    }
}

#[test]
fn compile_error_state_payload_normalizes_phase_reason_and_round() {
    let payload = super::compile_error_state_payload(
        &json!({
            "reason_code":"write_gate_missing",
            "message":"segment write preconditions are not satisfied",
            "issues":[{"reason_code":"missing_query_assert_branch_chain"}]
        }),
        3,
    );
    assert_eq!(
        payload.get("phase").and_then(Value::as_str),
        Some("compile")
    );
    assert_eq!(
        payload.get("reason_code").and_then(Value::as_str),
        Some("write_gate_missing")
    );
    assert_eq!(payload.get("round").and_then(Value::as_u64), Some(3));
    assert_eq!(
        payload.pointer("/issues/0/reason_code"),
        Some(&json!("missing_query_assert_branch_chain"))
    );
}

#[test]
fn compile_error_state_payload_classifies_unknown_input_ref_issue() {
    let payload = super::compile_error_state_payload(
        &json!({
            "reason_code":"compile_error",
            "message":"segment compile failed",
            "issues":[{"reference":"unknown_input_ref","path":"steps[0].inputs.amount"}]
        }),
        2,
    );
    assert_eq!(
        payload.get("phase").and_then(Value::as_str),
        Some("compile")
    );
    assert_eq!(
        payload.get("sub_reason_code").and_then(Value::as_str),
        Some("unknown_input_ref")
    );
    assert_eq!(
        payload.get("phase_reason_code").and_then(Value::as_str),
        Some("compile.unknown_input_ref")
    );
    assert_eq!(payload.get("round").and_then(Value::as_u64), Some(2));
}

#[test]
fn compile_segment_plan_fails_when_when_cel_references_unknown_input_slot() {
    let fixture_root = segmented_native_erc20_fixture_root();
    let pack_path = fixture_root.join("workspace/safe-defi.ais-pack.yaml");
    let pack = crate::policy::load_pack_document(&pack_path).expect("pack");
    let command = build_segmented_native_erc20_command(write_temp_file(
        "agent-segmented-unknown-input-when-dummy-script",
        "{}",
    ));
    let candidate_context = super::candidates::build_candidate_context_for_agent(
        &command,
        Some(&pack),
        super::candidates::DEFAULT_MAX_INDEX_CANDIDATES,
    )
    .expect("candidate context")
    .expect("workspace candidates");
    let mut input_store = super::InputStore::default();
    input_store.upsert_user(
        "inputs.owner",
        json!("0x1111111111111111111111111111111111111111"),
        "test.known_inputs.owner",
    );
    let segment: ais_sdk::documents::PlanSketchSegment = serde_json::from_value(json!({
        "segment_id":"seg-unknown-input-when",
        "cursor_in":"c0",
        "cursor_out":"c1",
        "done":false,
        "steps":[
            {
                "id":"q_balance",
                "kind":"query",
                "candidate_ref":"erc20@0.0.2/balance-of",
                "inputs":{
                    "token":{"object":{"address":{"lit":"0x8464135c8F25Da09e49BC8782676a84730C318bC"},"chain_ref":{"lit":"eip155:31338"}}},
                    "owner":{"ref":"inputs.native_threshold"}
                },
                "when":{"cel":"inputs.native_threshold > 0"}
            }
        ],
        "extensions":{}
    }))
    .expect("segment");
    let error = super::compile_segment_plan_with_snapshot_hash_and_facts(
        "check unknown input in when",
        "s-1",
        "c0",
        &segment,
        &candidate_context,
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        &["eip155:1".to_string()],
        Some(&input_store,
        ),
    )
    .expect_err("unknown inputs.native_threshold in when.cel must fail compile check");
    let issues = error
        .get("issues")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert!(
        !issues.is_empty(),
        "compile failure should include issues for unknown when.cel input: {error}"
    );
    assert!(
        issues.iter().any(|issue| {
            let lowered = issue.to_string().to_lowercase();
            lowered.contains("unknown_input")
                || lowered.contains("unknown input")
                || lowered.contains("invalid_member")
                || lowered.contains("invalid member")
        }),
        "issue should classify unknown input/invalid member access for when.cel: {error}"
    );
    assert!(
        issues
            .iter()
            .any(|issue| issue.to_string().contains("inputs.native_threshold")),
        "unknown slot should be preserved in compile issue payload: {error}"
    );
}

#[test]
fn todo_phase_error_payload_namespaces_reason_and_round() {
    let payload = super::todo_phase_error_payload(
        "missing_required_input",
        Some("token decimals missing"),
        &[json!({"reason_code":"missing_required_input","message":"token decimals missing"})],
        &[json!({"id":"token.decimals","question":"token decimals?"})],
        5,
    );
    assert_eq!(
        payload.get("phase").and_then(Value::as_str),
        Some("todo_planning")
    );
    assert_eq!(
        payload.get("reason_code").and_then(Value::as_str),
        Some("todo.missing_required_input")
    );
    assert_eq!(
        payload.get("base_reason_code").and_then(Value::as_str),
        Some("missing_required_input")
    );
    assert_eq!(payload.get("round").and_then(Value::as_u64), Some(5));
    assert_eq!(
        payload.pointer("/questions/0/id"),
        Some(&json!("token.decimals"))
    );
}

#[test]
fn planner_invalid_output_payload_has_stable_reason_code() {
    let error = RunnerError::Llm("invalid plan.propose_segment args".to_string());
    let payload =
        super::segmented_planner_output_error_payload(&error, "plan.propose_segment", 3, 1, None);
    assert_eq!(
        payload.get("reason_code").and_then(Value::as_str),
        Some("planner_invalid_tool_output")
    );
    assert_eq!(
        payload.get("sub_reason_code").and_then(Value::as_str),
        Some("invalid_tool_args")
    );
    assert_eq!(payload.get("round").and_then(Value::as_u64), Some(3));
    assert_eq!(payload.get("retry").and_then(Value::as_u64), Some(1));
}

#[test]
fn planner_invalid_output_payload_includes_last_failed_finalize_when_present() {
    let error = RunnerError::Llm("invalid plan.revise_segment args".to_string());
    let last_failed_finalize = json!({
        "tool": "plan.revise_segment",
        "tool_call_id": "call_x",
        "arguments": {
            "status": "proposed",
            "segment": {"segment_id":"seg_1"}
        }
    });
    let payload = super::segmented_planner_output_error_payload(
        &error,
        "plan.revise_segment",
        2,
        1,
        Some(last_failed_finalize),
    );
    assert_eq!(
        payload.pointer("/last_failed_finalize/tool"),
        Some(&json!("plan.revise_segment"))
    );
    assert_eq!(
        payload.pointer("/last_failed_finalize/arguments/segment/segment_id"),
        Some(&json!("seg_1"))
    );
}

#[test]
fn planner_missing_candidate_ref_payload_has_targeted_sub_reason_and_hint() {
    let error = RunnerError::Llm(
        "proposed segment draft `segment` is invalid: steps missing required `candidate_ref`: a_guard(assert)"
            .to_string(),
    );
    let payload =
        super::segmented_planner_output_error_payload(&error, "plan.revise_segment", 4, 1, None);
    assert_eq!(
        payload.get("sub_reason_code").and_then(Value::as_str),
        Some("missing_candidate_ref")
    );
    assert_eq!(
        payload.pointer("/hint/required_step_fields/2"),
        Some(&json!("inputs"))
    );
}

#[test]
fn planner_invalid_status_payload_has_expected_hint_contract() {
    let error = RunnerError::Llm("invalid segment draft status".to_string());
    let payload =
        super::segmented_planner_output_error_payload(&error, "plan.revise_segment", 6, 2, None);
    assert_eq!(
        payload.get("reason_code").and_then(Value::as_str),
        Some("planner_invalid_tool_output")
    );
    assert_eq!(
        payload.get("sub_reason_code").and_then(Value::as_str),
        Some("invalid_status")
    );
    assert_eq!(
        payload.get("phase_reason_code").and_then(Value::as_str),
        Some("planning.invalid_status")
    );
    assert_eq!(
        payload.pointer("/hint/allowed"),
        Some(&json!(["proposed", "invalid", "unavailable"]))
    );
}

#[test]
fn segment_planning_phase_migration_keeps_planner_error_contract_parity() {
    let error = RunnerError::Llm("invalid plan.revise_segment args".to_string());
    let revise_payload =
        super::segmented_planner_output_error_payload(&error, "plan.revise_segment", 7, 2, None);
    let propose_payload =
        super::segmented_planner_output_error_payload(&error, "plan.propose_segment", 7, 2, None);

    let assert_payload = |payload: &Value, expected_finalize_tool: &str| {
        assert_eq!(
            payload.get("phase").and_then(Value::as_str),
            Some("planning")
        );
        assert_eq!(
            payload.get("reason_code").and_then(Value::as_str),
            Some("planner_invalid_tool_output")
        );
        assert_eq!(
            payload.get("sub_reason_code").and_then(Value::as_str),
            Some("invalid_tool_args")
        );
        assert_eq!(
            payload.get("phase_reason_code").and_then(Value::as_str),
            Some("planning.invalid_tool_args")
        );
        assert_eq!(
            payload
                .get("expected_finalize_tool")
                .and_then(Value::as_str),
            Some(expected_finalize_tool)
        );
        assert_eq!(payload.get("round").and_then(Value::as_u64), Some(7));
        assert_eq!(payload.get("retry").and_then(Value::as_u64), Some(2));
        assert_eq!(payload.pointer("/repair_order/0"), Some(&json!("shape")));
        assert_eq!(payload.pointer("/repair_order/1"), Some(&json!("ref")));
        assert_eq!(payload.pointer("/repair_order/2"), Some(&json!("slot")));
        assert_eq!(payload.pointer("/repair_order/3"), Some(&json!("semantic")));
    };

    assert_payload(&revise_payload, "plan.revise_segment");
    assert_payload(&propose_payload, "plan.propose_segment");
}

#[test]
fn phase_machine_plan_segment_transition_contract_stays_stable() {
    let command = build_segmented_demo_command(write_missing_required_input_script(), None, None);
    let parsed = parse_agent_output_json(&command);
    assert_eq!(parsed.get("status").and_then(Value::as_str), Some("paused"));
    assert_eq!(
        parsed.get("paused_reason").and_then(Value::as_str),
        Some("missing_required_input")
    );
    assert!(
        parsed
            .pointer("/llm_usage/calls")
            .and_then(Value::as_u64)
            .is_some_and(|calls| calls >= 4),
        "plan-segment pause flow should emit planner calls: {parsed}"
    );
}

#[test]
fn execute_segment_phase_migration_keeps_pause_and_fail_contract_parity() {
    let mut confirm_router = RouterExecutor::new();
    confirm_router.register("test-exec", "test", Box::new(TestExecutor));
    let mut confirm_state = EngineRunnerState::default();
    let mut confirm_options = EngineRunnerOptions::default();
    confirm_options.policy.thresholds.max_risk_level = Some(0);
    let confirm_run = run_plan_once(
        "run-execute-segment-pause",
        &plan_requiring_user_confirm("transfer-1"),
        &mut confirm_state,
        &confirm_router,
        &DefaultSolver,
        &[],
        &confirm_options,
    );
    assert_eq!(confirm_run.status, EngineRunStatus::Paused);
    assert_eq!(
        confirm_state.paused_reason.as_deref(),
        Some("need_user_confirm:transfer-1")
    );
    assert!(confirm_run.events.iter().any(|record| {
        record.event.event_type == ais_engine::EngineEventType::NeedUserConfirm
            && record.event.node_id.as_deref() == Some("transfer-1")
    }));

    let mut fail_router = RouterExecutor::new();
    fail_router.register("test-exec", "test", Box::new(FailingSegmentExecutor));
    let mut fail_state = EngineRunnerState::default();
    let fail_run = run_plan_once(
        "run-execute-segment-repair",
        &PlanDocument {
            schema: "ais-plan/0.0.3".to_string(),
            meta: None,
            nodes: vec![json!({
                "id": "transfer-fail-1",
                "chain": "test",
                "execution": {
                    "type": "test_exec",
                    "method": "transfer"
                }
            })],
            extensions: Map::new(),
        },
        &mut fail_state,
        &fail_router,
        &DefaultSolver,
        &[],
        &EngineRunnerOptions::default(),
    );
    assert_eq!(fail_run.status, EngineRunStatus::Paused);
    assert!(
        fail_state
            .paused_reason
            .as_deref()
            .is_some_and(|reason| reason.starts_with("executor_error:")),
        "paused_reason={:?}",
        fail_state.paused_reason
    );
    let mut input_store = super::InputStore::default();
    let repair_backflow = super::phase_machine::pause::resolve_execution_pause_backflow(
        &mut fail_state,
        &mut input_store,
        fail_run.events.as_slice(),
        2,
    )
    .expect("resolve executor_error backflow");
    let repair_payload = match repair_backflow {
        super::phase_machine::pause::ResolvePauseBackflow::RepairScheduled { previous_error } => {
            previous_error
        }
        other => panic!("expected repair schedule for executor_error, got {other:?}"),
    };
    assert_eq!(
        repair_payload.get("reason_code").and_then(Value::as_str),
        Some("execution_paused")
    );
    assert!(
        repair_payload
            .get("sub_reason_code")
            .and_then(Value::as_str)
            .is_some_and(|value| value == "executor_error"),
        "payload={repair_payload}"
    );
}

#[test]
fn merge_segment_plan_tracks_count_in_meta_extensions() {
    let base = super::empty_plan_document();
    let segment = PlanDocument {
        schema: "ais-plan/0.0.3".to_string(),
        meta: None,
        nodes: vec![],
        extensions: Map::new(),
    };

    let merged_once = super::merge_segment_plan(&base, &segment).expect("merge once");
    let merged_twice = super::merge_segment_plan(&merged_once, &segment).expect("merge twice");
    let meta = merged_twice
        .meta
        .as_ref()
        .and_then(Value::as_object)
        .expect("meta object");
    assert!(meta.get("segment_count").is_none());
    assert_eq!(
        meta.get("extensions")
            .and_then(Value::as_object)
            .and_then(|extensions| extensions.get("segment_count"))
            .and_then(Value::as_u64),
        Some(2)
    );
}

#[test]
fn merge_segment_plan_replaces_nodes_for_same_segment_id() {
    let base = PlanDocument {
        schema: "ais-plan/0.0.3".to_string(),
        meta: Some(json!({})),
        nodes: vec![
            json!({
                "id":"seg_1__q1",
                "extensions":{"plan_sketch":{"segment_id":"seg_1","step_id":"q1"}}
            }),
            json!({
                "id":"seg_2__old_q",
                "extensions":{"plan_sketch":{"segment_id":"seg_2","step_id":"old_q"}}
            }),
            json!({
                "id":"seg_2__old_a",
                "extensions":{"plan_sketch":{"segment_id":"seg_2","step_id":"old_a"}}
            }),
        ],
        extensions: Map::new(),
    };
    let replacement = PlanDocument {
        schema: "ais-plan/0.0.3".to_string(),
        meta: None,
        nodes: vec![
            json!({
                "id":"seg_2__new_q",
                "extensions":{"plan_sketch":{"segment_id":"seg_2","step_id":"new_q"}}
            }),
            json!({
                "id":"seg_2__new_a",
                "extensions":{"plan_sketch":{"segment_id":"seg_2","step_id":"new_a"}}
            }),
        ],
        extensions: Map::new(),
    };

    let merged = super::merge_segment_plan(&base, &replacement).expect("merge");
    let node_ids = merged
        .nodes
        .iter()
        .filter_map(|node| node.get("id"))
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    assert_eq!(node_ids, vec!["seg_1__q1", "seg_2__new_q", "seg_2__new_a"]);
}

#[test]
fn normalize_segment_asset_inputs_uses_existing_input_ref_without_inventing_token_address() {
    let segment: ais_sdk::documents::PlanSketchSegment = serde_json::from_value(json!({
        "segment_id":"seg_asset",
        "cursor_in":"0",
        "cursor_out":"1",
        "done":false,
        "steps":[
            {
                "id":"a1",
                "kind":"action",
                "candidate_ref":"erc20@0.0.2/transfer",
                "inputs":{
                    "token":{"ref":"inputs.token.tst"},
                    "amount":{"lit":"1"},
                    "recipient":{"lit":"0x1111111111111111111111111111111111111111"}
                }
            }
        ]
    }))
    .expect("segment");
    let mut candidate_context = super::CandidateContext::default();
    candidate_context.detail_by_ref.insert(
        "erc20@0.0.2/transfer".to_string(),
        json!({
            "kind":"action",
            "params":[
                {"name":"token","type":"asset","required":true},
                {"name":"amount","type":"amount","required":true},
                {"name":"recipient","type":"address","required":true}
            ]
        }),
    );

    let normalized = super::normalize_segment_asset_inputs_for_compile(
        &segment,
        &candidate_context,
        Some("eip155:1"),
        &["inputs.token.tst".to_string(), "inputs.chain_id".to_string()],
    );
    let value = serde_json::to_value(normalized).expect("normalized segment");
    assert_eq!(
        value.pointer("/steps/0/inputs/token/object/address/ref"),
        Some(&json!("inputs.token.tst"))
    );
    assert_eq!(
        value.pointer("/steps/0/inputs/token/object/chain_ref/ref"),
        Some(&json!("inputs.chain_id"))
    );
    assert!(
        !value.to_string().contains("inputs.token.address"),
        "normalized segment must not invent token.address: {value}"
    );
}

#[test]
fn validate_segment_todo_scope_blocks_action_when_current_todo_is_query_only() {
    let segment: ais_sdk::documents::PlanSketchSegment = serde_json::from_value(json!({
        "segment_id":"seg_scope",
        "cursor_in":"0",
        "cursor_out":"1",
        "done":false,
        "steps":[
            {
                "id":"a1",
                "kind":"action",
                "candidate_ref":"demo@0.0.1/transfer",
                "inputs":{}
            }
        ]
    }))
    .expect("segment");
    let mut candidate_context = super::CandidateContext::default();
    candidate_context.detail_by_ref.insert(
        "demo@0.0.1/transfer".to_string(),
        json!({"kind":"action"}),
    );
    let current_todo = json!({
        "id":"todo_1",
        "title":"Query balances only",
        "execution_scope":"query_only",
    });

    let error = super::validate_segment_todo_scope(
        &segment,
        &candidate_context,
        Some(&current_todo),
    )
    .expect_err("query_only todo should block action step");
    assert_eq!(
        error.pointer("/reason_code").and_then(Value::as_str),
        Some("todo_scope_violation")
    );
    assert_eq!(
        error.pointer("/issues/0/step_id").and_then(Value::as_str),
        Some("a1")
    );
}
