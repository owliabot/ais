#[test]
fn render_agent_output_includes_llm_usage_line_when_available() {
    let command = AgentCommand {
        plan: None,
        intent: Some("transfer".to_string()),
        intent_file: None,
        config: PathBuf::from("runner.yaml"),
        runtime: None,
        workspace: None,
        pack: None,
        profile: AgentProfile::DemoScripted,
        llm_script_jsonl: None,
        events_jsonl: None,
        trace: None,
        agent_trace_jsonl: None,
        checkpoint: None,
        approvals_mode: None,
        max_iterations: None,
        max_planner_rounds: None,
        max_tool_rounds: None,
        max_index_candidates: None,
        planner_context_token_budget: None,
        llm_transcript_path: None,
        llm_transcript_append: false,
        verbose: false,
        verbose_llm: false,
        format: OutputFormat::Text,
    };
    let state = EngineRunnerState {
        runtime: json!({
            "agent": {
                "llm_usage": {
                    "calls": 3,
                    "input_tokens": 120,
                    "output_tokens": 40,
                    "total_tokens": 160,
                    "estimated_calls": 3,
                    "source": "tiktoken(o200k_base)",
                    "context_limit_tokens": 8192,
                    "context_soft_limit_tokens": 7372,
                    "context_remaining_tokens": 7212,
                    "diagnostics": {
                        "duplicate_tool_call_ratio_bps": 1200,
                        "discovery_tool_call_ratio_bps": 6400,
                        "empty_search_streak_max": 2
                    }
                }
            }
        }),
        ..EngineRunnerState::default()
    };
    let output =
        super::render_agent_output(&command, &state, EngineRunStatus::Completed, 5, 12, false)
            .expect("render output");
    assert!(output.contains("llm_usage: calls=3"));
    assert!(output.contains("total_tokens=160"));
    assert!(output.contains("source=tiktoken(o200k_base)"));
    assert!(output.contains("context_limit_tokens=8192"));
    assert!(output.contains("context_soft_limit_tokens=7372"));
    assert!(output.contains("context_remaining_tokens=7212"));
    assert!(output.contains("duplicate_tool_call_ratio_bps=1200"));
    assert!(output.contains("discovery_tool_call_ratio_bps=6400"));
    assert!(output.contains("empty_search_streak_max=2"));
}

#[test]
fn render_agent_output_downgrades_paused_without_reason_to_completed() {
    let command = AgentCommand {
        plan: None,
        intent: Some("transfer".to_string()),
        intent_file: None,
        config: PathBuf::from("runner.yaml"),
        runtime: None,
        workspace: None,
        pack: None,
        profile: AgentProfile::DemoScripted,
        llm_script_jsonl: None,
        events_jsonl: None,
        trace: None,
        agent_trace_jsonl: None,
        checkpoint: None,
        approvals_mode: None,
        max_iterations: None,
        max_planner_rounds: None,
        max_tool_rounds: None,
        max_index_candidates: None,
        planner_context_token_budget: None,
        llm_transcript_path: None,
        llm_transcript_append: false,
        verbose: false,
        verbose_llm: false,
        format: OutputFormat::Json,
    };
    let state = EngineRunnerState::default();

    let output =
        super::render_agent_output(&command, &state, EngineRunStatus::Paused, 1, 0, false)
            .expect("render output");
    let parsed: Value = serde_json::from_str(output.as_str()).expect("json");

    assert_eq!(parsed.get("status").and_then(Value::as_str), Some("completed"));
    assert!(parsed.get("paused_reason").is_some());
    assert!(parsed.get("paused_reason").unwrap().is_null());
}

#[test]
fn render_agent_output_infers_missing_input_pause_reason_from_runtime_payload() {
    let command = AgentCommand {
        plan: None,
        intent: Some("transfer".to_string()),
        intent_file: None,
        config: PathBuf::from("runner.yaml"),
        runtime: None,
        workspace: None,
        pack: None,
        profile: AgentProfile::DemoScripted,
        llm_script_jsonl: None,
        events_jsonl: None,
        trace: None,
        agent_trace_jsonl: None,
        checkpoint: None,
        approvals_mode: None,
        max_iterations: None,
        max_planner_rounds: None,
        max_tool_rounds: None,
        max_index_candidates: None,
        planner_context_token_budget: None,
        llm_transcript_path: None,
        llm_transcript_append: false,
        verbose: false,
        verbose_llm: false,
        format: OutputFormat::Json,
    };
    let state = EngineRunnerState {
        runtime: json!({
            "agent": {
                "missing_required_input": {
                    "reason_code": "missing_required_input",
                    "consumed": false,
                    "questions": [{"id":"inputs.token.decimals","question":"Provide decimals","required":true}]
                }
            }
        }),
        ..EngineRunnerState::default()
    };

    let output =
        super::render_agent_output(&command, &state, EngineRunStatus::Paused, 1, 0, false)
            .expect("render output");
    let parsed: Value = serde_json::from_str(output.as_str()).expect("json");

    assert_eq!(parsed.get("status").and_then(Value::as_str), Some("paused"));
    assert_eq!(
        parsed.get("paused_reason").and_then(Value::as_str),
        Some("missing_required_input")
    );
}


#[test]
fn planner_context_budget_resolution_prefers_cli_then_config_then_default() {
    let mut command = AgentCommand {
        plan: None,
        intent: Some("transfer".to_string()),
        intent_file: None,
        config: PathBuf::from("runner.yaml"),
        runtime: None,
        workspace: None,
        pack: None,
        profile: AgentProfile::DemoScripted,
        llm_script_jsonl: None,
        events_jsonl: None,
        trace: None,
        agent_trace_jsonl: None,
        checkpoint: None,
        approvals_mode: None,
        max_iterations: None,
        max_planner_rounds: None,
        max_tool_rounds: None,
        max_index_candidates: None,
        planner_context_token_budget: None,
        llm_transcript_path: None,
        llm_transcript_append: false,
        verbose: false,
        verbose_llm: false,
        format: OutputFormat::Text,
    };
    let mut config = RunnerConfig {
        schema: "ais-runner/0.0.1".to_string(),
        engine: RunnerEngineConfig::default(),
        llm: Some(RunnerLlmConfig {
            provider: "openrouter".to_string(),
            model: "openai/gpt-4.1-mini".to_string(),
            api_key: "key".to_string(),
            api_base: None,
            fallback: vec![],
            max_retries_per_provider: None,
            rotation: RunnerLlmRotationMode::StickyPrimary,
            controller_prompts_dir: None,
            operator_templates_dir: None,
            planner_context_token_budget: Some(7000),
            max_tool_rounds: None,
            context_limit_tokens: None,
        }),
        chains: BTreeMap::new(),
        plugins: RunnerPluginsConfig::default(),
    };

    assert_eq!(
        super::resolve_planner_context_token_budget(&command, &config),
        7000
    );
    command.planner_context_token_budget = Some(9000);
    assert_eq!(
        super::resolve_planner_context_token_budget(&command, &config),
        9000
    );
    command.planner_context_token_budget = None;
    config.llm = None;
    assert_eq!(
        super::resolve_planner_context_token_budget(&command, &config),
        super::context_view::DEFAULT_PLANNER_CONTEXT_TOKEN_BUDGET
    );
}


#[test]
fn segmented_max_tool_rounds_resolution_prefers_cli_then_config_then_default() {
    let mut command = AgentCommand {
        plan: None,
        intent: Some("transfer".to_string()),
        intent_file: None,
        config: PathBuf::from("runner.yaml"),
        runtime: None,
        workspace: None,
        pack: None,
        profile: AgentProfile::DemoScripted,
        llm_script_jsonl: None,
        events_jsonl: None,
        trace: None,
        agent_trace_jsonl: None,
        checkpoint: None,
        approvals_mode: None,
        max_iterations: None,
        max_planner_rounds: None,
        max_tool_rounds: None,
        max_index_candidates: None,
        planner_context_token_budget: None,
        llm_transcript_path: None,
        llm_transcript_append: false,
        verbose: false,
        verbose_llm: false,
        format: OutputFormat::Text,
    };
    let mut config = RunnerConfig {
        schema: "ais-runner/0.0.1".to_string(),
        engine: RunnerEngineConfig::default(),
        llm: Some(RunnerLlmConfig {
            provider: "openrouter".to_string(),
            model: "openai/gpt-4.1-mini".to_string(),
            api_key: "key".to_string(),
            api_base: None,
            fallback: vec![],
            max_retries_per_provider: None,
            rotation: RunnerLlmRotationMode::StickyPrimary,
            controller_prompts_dir: None,
            operator_templates_dir: None,
            planner_context_token_budget: None,
            max_tool_rounds: Some(18),
            context_limit_tokens: None,
        }),
        chains: BTreeMap::new(),
        plugins: RunnerPluginsConfig::default(),
    };

    assert_eq!(
        super::resolve_segmented_max_tool_rounds(&command, &config),
        18
    );
    command.max_tool_rounds = Some(30);
    assert_eq!(
        super::resolve_segmented_max_tool_rounds(&command, &config),
        30
    );
    command.max_tool_rounds = None;
    config.llm = None;
    assert_eq!(
        super::resolve_segmented_max_tool_rounds(&command, &config),
        super::intent_segmented::DEFAULT_SEGMENTED_MAX_TOOL_ROUNDS
    );
}
