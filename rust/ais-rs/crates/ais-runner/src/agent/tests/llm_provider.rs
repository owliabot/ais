#[test]
fn load_llm_provider_returns_none_when_not_configured() {
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
    let config = RunnerConfig {
        schema: "ais-runner/0.0.1".to_string(),
        engine: RunnerEngineConfig::default(),
        llm: None,
        chains: BTreeMap::new(),
        plugins: RunnerPluginsConfig::default(),
    };
    let provider = super::load_llm_provider(&command, &config).expect("must return none");
    assert!(provider.is_none());
}


#[test]
fn load_llm_provider_rejects_unknown_provider() {
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
    let config = RunnerConfig {
        schema: "ais-runner/0.0.1".to_string(),
        engine: RunnerEngineConfig::default(),
        llm: Some(RunnerLlmConfig {
            provider: "unknown".to_string(),
            model: "gpt-x".to_string(),
            api_key: "key".to_string(),
            api_base: None,
            fallback: vec![],
            max_retries_per_provider: None,
            rotation: RunnerLlmRotationMode::StickyPrimary,
            controller_prompts_dir: None,
            operator_templates_dir: None,
            planner_context_token_budget: None,
            max_tool_rounds: None,
            context_limit_tokens: None,
        }),
        chains: BTreeMap::new(),
        plugins: RunnerPluginsConfig::default(),
    };
    let result = super::load_llm_provider(&command, &config);
    let error = result.err().expect("must reject");
    assert!(matches!(error, RunnerError::Llm(_)));
}


#[test]
fn load_llm_provider_accepts_provider_chain_config() {
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
    let config = RunnerConfig {
        schema: "ais-runner/0.0.1".to_string(),
        engine: RunnerEngineConfig::default(),
        llm: Some(RunnerLlmConfig {
            provider: "openrouter".to_string(),
            model: "openai/gpt-4.1-mini".to_string(),
            api_key: "key-primary".to_string(),
            api_base: None,
            fallback: vec![crate::config::RunnerLlmEndpointConfig {
                provider: "groq".to_string(),
                model: "llama-3.3-70b-versatile".to_string(),
                api_key: "key-fallback".to_string(),
                api_base: None,
            }],
            max_retries_per_provider: Some(2),
            rotation: RunnerLlmRotationMode::RoundRobin,
            controller_prompts_dir: None,
            operator_templates_dir: None,
            planner_context_token_budget: None,
            max_tool_rounds: None,
            context_limit_tokens: None,
        }),
        chains: BTreeMap::new(),
        plugins: RunnerPluginsConfig::default(),
    };
    let result = super::load_llm_provider(&command, &config).expect("must build");
    assert!(result.is_some());
}

