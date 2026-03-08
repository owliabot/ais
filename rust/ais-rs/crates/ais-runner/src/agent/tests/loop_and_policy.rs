#[test]
fn agent_loop_can_auto_continue_after_need_user_confirm() {
    let plan = PlanDocument {
        schema: "ais-plan/0.0.3".to_string(),
        meta: None,
        nodes: vec![json!({
            "id": "swap-1",
            "chain": "test",
            "execution": {
                "type": "test_exec",
                "method": "swap_exact_tokens_for_tokens"
            },
            "extensions": {
                "risk_level": 5,
                "risk_tags": ["swap"]
            },
            "params": {}
        })],
        extensions: Map::new(),
    };

    let mut router = RouterExecutor::new();
    router.register("test-exec", "test", Box::new(TestExecutor));

    let mut state = EngineRunnerState {
        runtime: json!({}),
        ..EngineRunnerState::default()
    };
    let mut engine_options = EngineRunnerOptions::default();
    engine_options.policy.thresholds.max_risk_level = Some(0);
    let loop_config = AgentLoopConfig { max_iterations: 8 };
    let mut brain = ApproveOnceBrain { approved: false };
    let mut builder = CommandBuilder::new("run-test");

    let result = run_agent_loop(
        "run-test",
        &plan,
        &mut state,
        &router,
        &DefaultSolver,
        &engine_options,
        &loop_config,
        &mut builder,
        &mut brain,
        |_state, _events| Ok(()),
    )
    .expect("agent loop must complete");

    assert_eq!(result.status, ais_engine::EngineRunStatus::Completed);
    assert!(brain.approved);
    assert_eq!(state.completed_node_ids, vec!["swap-1".to_string()]);
}


#[test]
fn agent_loop_returns_when_hard_blocked() {
    let plan = PlanDocument {
        schema: "ais-plan/0.0.3".to_string(),
        meta: None,
        nodes: vec![json!({
            "id": "swap-1",
            "chain": "test",
            "execution": {
                "type": "test_exec",
                "method": "swap_exact_tokens_for_tokens"
            },
            "bindings": {
                "params": {
                    "spend_amount": { "lit": "1" },
                    "slippage_bps": { "lit": 100 }
                }
            }
        })],
        extensions: Map::new(),
    };

    let mut router = RouterExecutor::new();
    router.register("test-exec", "test", Box::new(TestExecutor));

    let mut state = EngineRunnerState {
        runtime: json!({}),
        ..EngineRunnerState::default()
    };
    let mut engine_options = EngineRunnerOptions::default();
    engine_options.policy.strict_allowlist = true;
    let loop_config = AgentLoopConfig { max_iterations: 8 };
    let mut brain = PanicBrain;
    let mut builder = CommandBuilder::new("run-test");

    let result = run_agent_loop(
        "run-test",
        &plan,
        &mut state,
        &router,
        &DefaultSolver,
        &engine_options,
        &loop_config,
        &mut builder,
        &mut brain,
        |_state, _events| Ok(()),
    )
    .expect("agent loop must return");

    assert_eq!(result.status, ais_engine::EngineRunStatus::Paused);
    assert_eq!(state.paused_reason.as_deref(), Some("hard_block:swap-1"));
}


#[test]
fn llm_brain_can_auto_approve_need_user_confirm() {
    let plan = PlanDocument {
        schema: "ais-plan/0.0.3".to_string(),
        meta: None,
        nodes: vec![json!({
            "id": "swap-1",
            "chain": "test",
            "execution": {
                "type": "test_exec",
                "method": "swap_exact_tokens_for_tokens"
            },
            "extensions": {
                "risk_level": 5,
                "risk_tags": ["swap"]
            },
            "params": {}
        })],
        extensions: Map::new(),
    };

    let mut router = RouterExecutor::new();
    router.register("test-exec", "test", Box::new(TestExecutor));

    let mut state = EngineRunnerState {
        runtime: json!({}),
        ..EngineRunnerState::default()
    };
    let mut engine_options = EngineRunnerOptions::default();
    engine_options.policy.thresholds.max_risk_level = Some(0);
    let loop_config = AgentLoopConfig { max_iterations: 8 };
    let provider = ScriptedLlmProvider::from_responses(vec![Ok(CompleteWithToolsResponse {
        assistant_content: Some("approve this".to_string()),
        tool_calls: vec![ToolCall {
            id: "tool-1".to_string(),
            name: "confirm".to_string(),
            arguments: json!({"decision":"approve"}),
        }],
    })]);
    let mut brain = LlmBrain::new(provider);
    let mut builder = CommandBuilder::new("run-test");

    let result = run_agent_loop(
        "run-test",
        &plan,
        &mut state,
        &router,
        &DefaultSolver,
        &engine_options,
        &loop_config,
        &mut builder,
        &mut brain,
        |_state, _events| Ok(()),
    )
    .expect("agent loop must complete");

    assert_eq!(result.status, ais_engine::EngineRunStatus::Completed);
    assert_eq!(state.completed_node_ids, vec!["swap-1".to_string()]);
}


#[test]
fn engine_stays_paused_when_user_denies_confirmation() {
    let plan = PlanDocument {
        schema: "ais-plan/0.0.3".to_string(),
        meta: None,
        nodes: vec![json!({
            "id": "transfer-1",
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
    };

    let mut router = RouterExecutor::new();
    router.register("test-exec", "test", Box::new(TestExecutor));
    let mut state = EngineRunnerState {
        runtime: json!({}),
        ..EngineRunnerState::default()
    };
    let mut engine_options = EngineRunnerOptions::default();
    engine_options.policy.thresholds.max_risk_level = Some(0);
    let first = run_plan_once(
        "run-test",
        &plan,
        &mut state,
        &router,
        &DefaultSolver,
        &[],
        &engine_options,
    );
    assert_eq!(first.status, ais_engine::EngineRunStatus::Paused);
    assert_eq!(
        state.paused_reason.as_deref(),
        Some("need_user_confirm:transfer-1")
    );

    let mut builder = CommandBuilder::new("run-test");
    let second = run_plan_once(
        "run-test",
        &plan,
        &mut state,
        &router,
        &DefaultSolver,
        &[builder.user_confirm("transfer-1", "deny")],
        &engine_options,
    );
    assert_eq!(second.status, ais_engine::EngineRunStatus::Paused);
    assert!(!state.completed_node_ids.contains(&"transfer-1".to_string()));
}


#[test]
fn llm_brain_returns_error_on_unknown_tool() {
    let mut brain = LlmBrain::new(ScriptedLlmProvider::from_responses(vec![Ok(
        CompleteWithToolsResponse {
            assistant_content: Some("bad".to_string()),
            tool_calls: vec![ToolCall {
                id: "tool-1".to_string(),
                name: "unknown_tool".to_string(),
                arguments: json!({}),
            }],
        },
    )]));
    let mut builder = CommandBuilder::new("run-test");
    let summary = super::summary::PauseSummary {
        raw_reason: Some("need_user_confirm:swap-1".to_string()),
        kind: PauseKind::NeedUserConfirm,
        node_id: Some("swap-1".to_string()),
        need_user_confirm: None,
        last_error_reason: None,
    };

    let error = brain
        .decide(&summary, &mut builder)
        .expect_err("must reject");
    assert!(error.to_string().contains("unsupported tool"));
}


#[test]
fn execute_agent_rejects_demo_script_for_standard_profile() {
    let command = AgentCommand {
        plan: Some("missing.plan.json".into()),
        intent: None,
        intent_file: None,
        workspace: None,
        config: "missing.config.yaml".into(),
        pack: None,
        runtime: None,
        events_jsonl: None,
        trace: None,
        agent_trace_jsonl: None,
        checkpoint: None,
        profile: AgentProfile::Standard,
        llm_script_jsonl: Some("demo.jsonl".into()),
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

    let error = super::execute_agent(&command).expect_err("must reject profile/script mismatch");
    assert!(matches!(error, RunnerError::AgentProfile(_)));
}


#[test]
fn execute_agent_requires_script_for_demo_scripted_profile() {
    let command = AgentCommand {
        plan: Some("missing.plan.json".into()),
        intent: None,
        intent_file: None,
        workspace: None,
        config: "missing.config.yaml".into(),
        pack: None,
        runtime: None,
        events_jsonl: None,
        trace: None,
        agent_trace_jsonl: None,
        checkpoint: None,
        profile: AgentProfile::DemoScripted,
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

    let error = super::execute_agent(&command).expect_err("must require script");
    assert!(matches!(error, RunnerError::AgentProfile(_)));
}

#[test]
fn command_builder_resumes_after_seen_command_ids() {
    let mut builder = CommandBuilder::new("run-fixed");
    let seen = vec![
        "run-fixed-cmd-000001".to_string(),
        "run-fixed-cmd-000003".to_string(),
        "other-run-cmd-000099".to_string(),
    ];
    let max_seen = builder.set_next_index_from_seen_ids(&seen);
    assert_eq!(max_seen, 3);
    assert_eq!(
        builder.cancel().command.id,
        "run-fixed-cmd-000004".to_string()
    );
}
