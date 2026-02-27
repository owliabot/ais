use super::brain::{DecisionPolicy, LlmBrain};
use super::intent_segmented::SegmentedIntentPlanner;
use super::r#loop::{run_agent_loop, AgentLoopConfig, CommandBuilder};
use super::summary::PauseKind;
use crate::cli::{AgentCommand, AgentProfile, OutputFormat};
use crate::config::{
    ChainConfig, RunnerConfig, RunnerEngineConfig, RunnerLlmConfig, RunnerLlmRotationMode,
    RunnerPluginsConfig, SignerConfig,
};
use crate::error::RunnerError;
use ais_engine::{
    create_checkpoint_document, run_plan_once, save_checkpoint_to_path, CheckpointEngineState,
    DefaultSolver, EngineCommandEnvelope, EngineRunStatus, EngineRunnerOptions, EngineRunnerState,
    Executor, ExecutorOutput, RouterExecutor,
};
use ais_llm::{CompleteWithToolsResponse, ScriptedLlmProvider, ToolCall};
use ais_sdk::PlanDocument;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

struct TestExecutor;

impl Executor for TestExecutor {
    fn execute(&self, _node: &Value, _runtime: &mut Value) -> Result<ExecutorOutput, String> {
        Ok(ExecutorOutput {
            result: json!({"ok": true}),
            writes: Map::new(),
            side_effects: Vec::new(),
        })
    }
}

struct SegmentedFixtureExecutor;

impl Executor for SegmentedFixtureExecutor {
    fn execute(&self, node: &Value, _runtime: &mut Value) -> Result<ExecutorOutput, String> {
        let node_id = node
            .as_object()
            .and_then(|object| object.get("id"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        let result = if node_id.ends_with("q_native_balance") {
            json!({
                "execution_type":"offchain_apy_query",
                "outputs":{"balance":"200"}
            })
        } else if node_id.ends_with("q_token_balance") {
            json!({
                "execution_type":"offchain_apy_query",
                "outputs":{"balance":"260"}
            })
        } else {
            json!({
                "execution_type":"offchain_apy_query",
                "outputs":{"tx_hash":"0xsegmented_transfer"}
            })
        };
        Ok(ExecutorOutput {
            result,
            writes: Map::new(),
            side_effects: Vec::new(),
        })
    }
}

struct SegmentedUntilRetryExecutor;

impl Executor for SegmentedUntilRetryExecutor {
    fn execute(&self, node: &Value, runtime: &mut Value) -> Result<ExecutorOutput, String> {
        let node_id = node
            .as_object()
            .and_then(|object| object.get("id"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        let result = if node_id.ends_with("q_native_balance") {
            json!({
                "execution_type":"offchain_apy_query",
                "outputs":{"balance":"200"}
            })
        } else if node_id.ends_with("q_token_balance") {
            json!({
                "execution_type":"offchain_apy_query",
                "outputs":{"balance":"260"}
            })
        } else {
            let escaped = node_id.replace('~', "~0").replace('/', "~1");
            let seen_before = runtime
                .pointer(format!("/nodes/{escaped}/outputs/outputs/confirmed").as_str())
                .is_some();
            json!({
                "execution_type":"offchain_apy_query",
                "outputs":{
                    "tx_hash":"0xsegmented_transfer_retry",
                    "confirmed": seen_before
                }
            })
        };
        Ok(ExecutorOutput {
            result,
            writes: Map::new(),
            side_effects: Vec::new(),
        })
    }
}

struct ApproveOnceBrain {
    approved: bool,
}

impl DecisionPolicy for ApproveOnceBrain {
    fn decide(
        &mut self,
        summary: &super::summary::PauseSummary,
        commands: &mut CommandBuilder,
    ) -> Result<Vec<EngineCommandEnvelope>, RunnerError> {
        assert_eq!(summary.kind, PauseKind::NeedUserConfirm);
        let node_id = summary.node_id.as_deref().expect("node_id must exist");
        assert!(
            !self.approved,
            "brain must only be invoked once for this plan"
        );
        self.approved = true;
        Ok(vec![commands.user_confirm(node_id, "approve")])
    }
}

struct PanicBrain;

impl DecisionPolicy for PanicBrain {
    fn decide(
        &mut self,
        _summary: &super::summary::PauseSummary,
        _commands: &mut CommandBuilder,
    ) -> Result<Vec<EngineCommandEnvelope>, RunnerError> {
        panic!("brain must not be invoked for this test");
    }
}

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
        format: OutputFormat::Text,
    };

    let error = super::execute_agent(&command).expect_err("must require script");
    assert!(matches!(error, RunnerError::AgentProfile(_)));
}

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
            prompts_dir: None,
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
            prompts_dir: None,
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
        format: OutputFormat::Text,
    };
    let error = super::execute_agent(&command).expect_err("must reject without llm provider");
    assert!(matches!(error, RunnerError::Llm(_)));
    assert!(error
        .to_string()
        .contains("requires configured llm provider"));
}

#[test]
fn execute_agent_segmented_missing_required_input_pauses_instead_of_failing() {
    let fixture_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/runner-local/intent-segmented-offchain-transfer");
    let workspace_dir = fixture_root.join("workspace");
    let pack_path = workspace_dir.join("safe-defi.ais-pack.yaml");
    let config_path = fixture_root.join("config/runner.local.yaml");
    let intent_file = fixture_root.join("intent/intent.txt");
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
                                        "id":"token_decimals",
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
    let llm_script_path =
        write_temp_file("agent-segmented-missing-input-script", llm_script.as_str());

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
        format: OutputFormat::Json,
    };

    let output = super::execute_agent(&command).expect("missing input should pause, not fail");
    let parsed: Value = serde_json::from_str(output.as_str()).expect("json output");
    assert_eq!(parsed.get("status").and_then(Value::as_str), Some("paused"));
    assert_eq!(
        parsed.get("paused_reason").and_then(Value::as_str),
        Some("missing_required_input")
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
    let mut session = planner
        .begin_session(super::intent_segmented::SegmentBeginRequest {
            intent: intent.clone(),
            pack_snapshot_hash: super::derive_pack_snapshot_hash(Some(&pack)).expect("pack hash"),
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
        Some("threshold_risk_level_unknown")
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
    let mut session = planner
        .begin_session(super::intent_segmented::SegmentBeginRequest {
            intent: intent.clone(),
            pack_snapshot_hash: super::derive_pack_snapshot_hash(Some(&pack)).expect("pack hash"),
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
    let mut session = planner
        .begin_session(super::intent_segmented::SegmentBeginRequest {
            intent: intent.clone(),
            pack_snapshot_hash: super::derive_pack_snapshot_hash(Some(&pack)).expect("pack hash"),
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
    assert!(first_invalid_error.to_string().contains(
        "proposed segment draft `segment` must be a JSON object (stringified JSON is not allowed)"
    ));
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
fn load_or_init_state_rejects_legacy_checkpoint_node_ids() {
    let checkpoint_path = write_temp_file("legacy-checkpoint", "");
    let checkpoint = create_checkpoint_document(
        "run-legacy",
        "legacy-plan-hash",
        CheckpointEngineState {
            completed_node_ids: vec!["seg_1/q1".to_string()],
            ..CheckpointEngineState::default()
        },
        Some(json!({})),
        Some(json!({
            "schema":"ais-plan/0.0.3",
            "nodes":[{"id":"seg_1/q1"}]
        })),
        None,
    );
    save_checkpoint_to_path(&checkpoint_path, &checkpoint).expect("save checkpoint");

    let command = AgentCommand {
        plan: Some(PathBuf::from("ignored.plan.json")),
        intent: None,
        intent_file: None,
        workspace: None,
        config: PathBuf::from("ignored.config.yaml"),
        pack: None,
        runtime: None,
        events_jsonl: None,
        trace: None,
        checkpoint: Some(checkpoint_path.clone()),
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
        format: OutputFormat::Text,
    };

    let error = super::load_or_init_state(&command, "current-plan-hash", json!({}))
        .expect_err("legacy checkpoint must be rejected");
    assert!(matches!(error, RunnerError::CheckpointLoad { .. }));
    assert!(error
        .to_string()
        .contains("legacy checkpoint is not supported"));
}

#[test]
fn load_or_init_state_dedupes_checkpoint_plan_snapshot_nodes() {
    let checkpoint_path = write_temp_file("dedupe-checkpoint", "");
    let checkpoint = create_checkpoint_document(
        "run-dedupe",
        "checkpoint-plan-hash",
        CheckpointEngineState::default(),
        Some(json!({})),
        Some(json!({
            "schema":"ais-plan/0.0.3",
            "nodes":[
                {"id":"seg_1__q1","chain":"eip155:1","execution":{"type":"evm_read"},"writes":[{"path":"nodes.seg_1__q1.outputs","mode":"set"}]},
                {"id":"seg_1__q1","chain":"eip155:1","execution":{"type":"evm_read"},"writes":[{"path":"nodes.seg_1__q1.outputs","mode":"set"}]},
                {"id":"seg_1__q2","chain":"eip155:1","execution":{"type":"evm_read"},"writes":[{"path":"nodes.seg_1__q2.outputs","mode":"set"}]}
            ]
        })),
        None,
    );
    save_checkpoint_to_path(&checkpoint_path, &checkpoint).expect("save checkpoint");

    let command = AgentCommand {
        plan: Some(PathBuf::from("ignored.plan.json")),
        intent: None,
        intent_file: None,
        workspace: None,
        config: PathBuf::from("ignored.config.yaml"),
        pack: None,
        runtime: None,
        events_jsonl: None,
        trace: None,
        checkpoint: Some(checkpoint_path.clone()),
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
        format: OutputFormat::Text,
    };

    let (_, resumed, checkpoint_plan, checkpoint_hash, _, _) =
        super::load_or_init_state(&command, "current-plan-hash", json!({})).expect("load state");
    assert!(resumed);
    assert_eq!(checkpoint_hash.as_deref(), Some("checkpoint-plan-hash"));
    let checkpoint_plan = checkpoint_plan.expect("checkpoint plan");
    assert_eq!(checkpoint_plan.nodes.len(), 2);
    let ids = checkpoint_plan
        .nodes
        .iter()
        .filter_map(|node| node.get("id"))
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    assert_eq!(ids, vec!["seg_1__q1", "seg_1__q2"]);
}

#[test]
fn initial_fact_store_derives_owner_from_evm_signer() {
    let mut chains = BTreeMap::new();
    chains.insert(
        "eip155:31338".to_string(),
        ChainConfig {
            rpc_url: "https://rpc.example".to_string(),
            timeout_ms: None,
            wait_for_receipt: None,
            receipt_poll: None,
            commitment: None,
            wait_for_confirmation: None,
            confirmation_poll: None,
            signer: Some(SignerConfig::EvmPrivateKey {
                private_key: "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d"
                    .to_string(),
            }),
        },
    );
    let config = RunnerConfig {
        schema: "ais-runner/0.0.1".to_string(),
        engine: RunnerEngineConfig::default(),
        llm: None,
        chains,
        plugins: RunnerPluginsConfig::default(),
    };

    let fact_store =
        super::build_initial_fact_store(&json!({}), &config, &["eip155:31338".to_string()])
            .expect("fact store");
    let owner = fact_store.get("owner").expect("owner fact");
    assert_eq!(
        owner.value.as_str(),
        Some("0x70997970c51812dc3a010c7d01b50e0d17dc79c8")
    );
    assert_eq!(owner.source, super::facts::FactSource::ConfigDerived);
    assert_eq!(owner.provenance, "runner_config.chains.eip155:31338.signer");
    assert_eq!(
        fact_store
            .get("owner_by_chain.eip155:31338")
            .and_then(|entry| entry.value.as_str()),
        Some("0x70997970c51812dc3a010c7d01b50e0d17dc79c8")
    );
}

#[test]
fn initial_fact_store_uses_runtime_owner_when_signer_missing() {
    let mut chains = BTreeMap::new();
    chains.insert(
        "eip155:1".to_string(),
        ChainConfig {
            rpc_url: "https://rpc.example".to_string(),
            timeout_ms: None,
            wait_for_receipt: None,
            receipt_poll: None,
            commitment: None,
            wait_for_confirmation: None,
            confirmation_poll: None,
            signer: None,
        },
    );
    let config = RunnerConfig {
        schema: "ais-runner/0.0.1".to_string(),
        engine: RunnerEngineConfig::default(),
        llm: None,
        chains,
        plugins: RunnerPluginsConfig::default(),
    };

    let runtime = json!({
        "inputs": {
            "wallet": "0x1111111111111111111111111111111111111111"
        }
    });
    let fact_store = super::build_initial_fact_store(&runtime, &config, &["eip155:1".to_string()])
        .expect("fact store");
    let owner = fact_store.get("owner").expect("owner fact");
    assert_eq!(
        owner.value.as_str(),
        Some("0x1111111111111111111111111111111111111111")
    );
    assert_eq!(owner.source, super::facts::FactSource::RuntimeProvided);
    assert_eq!(owner.provenance, "runtime.inputs.wallet");
}

#[test]
fn initial_fact_store_prefers_signer_over_runtime_owner() {
    let mut chains = BTreeMap::new();
    chains.insert(
        "eip155:31338".to_string(),
        ChainConfig {
            rpc_url: "https://rpc.example".to_string(),
            timeout_ms: None,
            wait_for_receipt: None,
            receipt_poll: None,
            commitment: None,
            wait_for_confirmation: None,
            confirmation_poll: None,
            signer: Some(SignerConfig::EvmPrivateKey {
                private_key: "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d"
                    .to_string(),
            }),
        },
    );
    let config = RunnerConfig {
        schema: "ais-runner/0.0.1".to_string(),
        engine: RunnerEngineConfig::default(),
        llm: None,
        chains,
        plugins: RunnerPluginsConfig::default(),
    };

    let runtime = json!({
        "inputs": {
            "owner": "0x1111111111111111111111111111111111111111"
        }
    });
    let fact_store =
        super::build_initial_fact_store(&runtime, &config, &["eip155:31338".to_string()])
            .expect("fact store");
    let owner = fact_store.get("owner").expect("owner fact");
    assert_eq!(
        owner.value.as_str(),
        Some("0x70997970c51812dc3a010c7d01b50e0d17dc79c8")
    );
    assert_eq!(owner.source, super::facts::FactSource::ConfigDerived);
    assert_eq!(owner.provenance, "runner_config.chains.eip155:31338.signer");
}

#[test]
fn initial_fact_store_seeds_runtime_inputs_as_canonical_refs() {
    let config = RunnerConfig {
        schema: "ais-runner/0.0.1".to_string(),
        engine: RunnerEngineConfig::default(),
        llm: None,
        chains: BTreeMap::new(),
        plugins: RunnerPluginsConfig::default(),
    };
    let runtime = json!({
        "inputs": {
            "owner": "0x1111111111111111111111111111111111111111",
            "token": {
                "address": "0x2222222222222222222222222222222222222222",
                "decimals": 6
            },
            "amount": "1.5"
        }
    });
    let fact_store = super::build_initial_fact_store(&runtime, &config, &["eip155:1".to_string()])
        .expect("fact store");
    assert_eq!(
        fact_store
            .get("owner")
            .and_then(|entry| entry.value.as_str()),
        Some("0x1111111111111111111111111111111111111111")
    );
    assert_eq!(
        fact_store
            .get("inputs.owner")
            .and_then(|entry| entry.value.as_str()),
        Some("0x1111111111111111111111111111111111111111")
    );
    assert_eq!(
        fact_store
            .get("token.address")
            .and_then(|entry| entry.value.as_str()),
        Some("0x2222222222222222222222222222222222222222")
    );
    assert_eq!(
        fact_store
            .get("inputs.token.address")
            .and_then(|entry| entry.value.as_str()),
        Some("0x2222222222222222222222222222222222222222")
    );
    assert_eq!(
        fact_store
            .get("inputs.token.decimals")
            .and_then(|entry| entry.value.as_i64()),
        Some(6)
    );
    assert_eq!(
        fact_store
            .get("inputs.amount")
            .and_then(|entry| entry.value.as_str()),
        Some("1.5")
    );
}

#[test]
fn state_summary_includes_fact_store_payload() {
    let mut fact_store = super::facts::FactStore::default();
    fact_store.upsert(
        "owner",
        json!("0x2222222222222222222222222222222222222222"),
        super::facts::FactLayer::Seed,
        super::facts::FactSource::UserInput,
        "user.prompt",
    );
    let state = EngineRunnerState::default();
    let summary = super::build_state_summary(&state, 0, false, None, Some(&fact_store));
    assert_eq!(
        summary.pointer("/fact_store/facts/owner"),
        Some(&json!("0x2222222222222222222222222222222222222222"))
    );
    assert_eq!(
        summary.pointer("/fact_store/meta/owner/source"),
        Some(&json!("user_input"))
    );
}

#[test]
fn state_summary_includes_todo_state_payload_from_runtime() {
    let state = EngineRunnerState {
        runtime: json!({
            "agent": {
                "todo_progress": {
                    "current_todo": {"id":"todo_1","status":"in_progress"},
                    "progress": {"todo":0,"in_progress":1,"done":0,"blocked":0,"total":1}
                }
            }
        }),
        ..EngineRunnerState::default()
    };
    let summary = super::build_state_summary(&state, 0, false, None, None);
    assert_eq!(
        summary.pointer("/todo_state/current_todo/id"),
        Some(&json!("todo_1"))
    );
    assert_eq!(
        summary.pointer("/todo_state/progress/total"),
        Some(&json!(1))
    );
}

#[test]
fn record_todo_progress_tracks_follow_up_todo_after_completion() {
    let mut runtime = json!({});
    let mut board = super::todos::TodoBoard::bootstrap("transfer 1 token");
    board.mark_current_in_progress(Some("query balances"), "seg_1");
    super::record_todo_progress(&mut runtime, &board);
    assert_eq!(
        runtime.pointer("/agent/todo_progress/current_todo/id"),
        Some(&json!("todo_1"))
    );
    assert_eq!(
        runtime.pointer("/agent/todo_progress/current_todo/status"),
        Some(&json!("in_progress"))
    );

    board.mark_current_done();
    board.open_follow_up_todo();
    super::record_todo_progress(&mut runtime, &board);
    assert_eq!(
        runtime.pointer("/agent/todo_progress/current_todo/id"),
        Some(&json!("todo_2"))
    );
    assert_eq!(
        runtime.pointer("/agent/todo_progress/progress/done"),
        Some(&json!(1))
    );
}

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

    let error = super::validate_segment_write_gates(&segment, &candidate_context, None)
        .expect_err("transfer without gate chain must fail");
    assert_eq!(
        error.get("reason_code").and_then(Value::as_str),
        Some("write_gate_missing")
    );
    assert!(error
        .to_string()
        .contains("missing_query_assert_branch_chain"));
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

    let fact_store = super::facts::FactStore::default();
    let error =
        super::validate_segment_write_gates(&segment, &candidate_context, Some(&fact_store))
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

    let mut fact_store = super::facts::FactStore::default();
    fact_store.upsert_with_observed_at(
        "wallet.balance.native",
        json!("100"),
        super::facts::FactLayer::Observed,
        super::facts::FactSource::QueryObserved,
        "query:native-balance",
        Some(1),
    );
    let error =
        super::validate_segment_write_gates(&segment, &candidate_context, Some(&fact_store))
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

    let fact_store = super::facts::FactStore::default();
    super::validate_segment_write_gates(&segment, &candidate_context, Some(&fact_store))
        .expect("same segment refresh query should satisfy volatile freshness check");
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

    super::validate_segment_write_gates(&segment, &candidate_context, None)
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

    let error = super::validate_segment_write_gates(&segment, &candidate_context, None)
        .expect_err("explicit write_gate required must enforce gate chain");
    assert_eq!(
        error.get("reason_code").and_then(Value::as_str),
        Some("write_gate_missing")
    );
    assert!(error
        .to_string()
        .contains("missing_query_assert_branch_chain"));
}

#[test]
fn checkpoint_extensions_roundtrip_restores_fact_store_todo_and_intent_facts() {
    let mut store = super::facts::FactStore::default();
    store.upsert(
        "owner",
        json!("0x1111111111111111111111111111111111111111"),
        super::facts::FactLayer::Seed,
        super::facts::FactSource::RuntimeProvided,
        "runtime.inputs.owner",
    );
    let runtime = json!({
        "agent": {
            "intent_grounding": {
                "intent_facts": {
                    "recipient": "0x2222222222222222222222222222222222222222",
                    "amount": "1"
                }
            },
            "todo_progress": {
                "schema": "ais-agent-todo-progress/0.0.1",
                "current_todo": {"id":"todo_1","status":"in_progress"},
                "todos": [],
                "progress": {"todo":0,"in_progress":1,"done":0,"blocked":0,"total":1},
                "next_seq": 2
            }
        }
    });

    let decoded = super::checkpoint_ext::AgentCheckpointExtensions::decode(None);
    let intent_facts = runtime
        .pointer("/agent/intent_grounding/intent_facts")
        .and_then(Value::as_object)
        .map(|facts| {
            facts
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect::<std::collections::BTreeMap<String, Value>>()
        });
    let extensions = decoded.encode_updated(
        None,
        &store,
        runtime.pointer("/agent/todo_progress"),
        intent_facts.as_ref(),
    );
    assert!(extensions.get("fact_store").is_some());
    assert!(extensions.get("todo_progress").is_some());
    assert!(extensions.get("intent_facts").is_some());

    let mut restored_runtime = json!({});
    let restored_extensions =
        super::decode_agent_checkpoint_extensions(&mut restored_runtime, Some(&extensions), false);
    let restored_store = restored_extensions
        .fact_store()
        .cloned()
        .expect("fact store restored");
    assert_eq!(
        restored_store
            .get("owner")
            .and_then(|entry| entry.value.as_str()),
        Some("0x1111111111111111111111111111111111111111")
    );
    assert_eq!(
        restored_runtime.pointer("/agent/todo_progress/current_todo/id"),
        Some(&json!("todo_1"))
    );
    assert_eq!(
        restored_runtime.pointer("/agent/intent_grounding/intent_facts/recipient"),
        Some(&json!("0x2222222222222222222222222222222222222222"))
    );
}

#[test]
fn missing_required_input_payload_roundtrip_records_questions() {
    let payload = super::missing_required_input_payload(
        Some("missing token decimals"),
        &[json!({
            "id": "token_decimals",
            "question": "token decimals?",
            "options": [{"label":"18","value":18}]
        })],
        &[json!({"kind":"schema_error","reason_code":"missing_input","message":"x"})],
        2,
    );
    assert_eq!(
        payload.pointer("/reason_code"),
        Some(&json!("missing_required_input"))
    );
    assert_eq!(
        payload.pointer("/questions/0/id"),
        Some(&json!("token_decimals"))
    );
    assert_eq!(payload.pointer("/round"), Some(&json!(2)));

    let mut runtime = json!({});
    super::record_missing_required_input(&mut runtime, &payload);
    assert_eq!(
        runtime.pointer("/agent/missing_required_input/reason_code"),
        Some(&json!("missing_required_input"))
    );
    assert_eq!(
        runtime.pointer("/agent/missing_required_input/questions/0/id"),
        Some(&json!("token_decimals"))
    );
}

#[test]
fn apply_missing_input_answers_backfills_runtime_and_fact_store() {
    let mut state = EngineRunnerState {
        runtime: json!({}),
        ..EngineRunnerState::default()
    };
    let mut store = super::facts::FactStore::default();
    let answers = Map::from_iter([
        ("owner".to_string(), json!("0xabc")),
        ("token.decimals".to_string(), json!(18)),
    ]);
    super::apply_missing_input_answers(&mut state, &mut store, &answers);

    assert_eq!(
        state.runtime.pointer("/inputs/owner"),
        Some(&json!("0xabc"))
    );
    assert_eq!(
        state.runtime.pointer("/inputs/token/decimals"),
        Some(&json!(18))
    );
    assert_eq!(
        store.get("owner").and_then(|entry| entry.value.as_str()),
        Some("0xabc")
    );
    assert_eq!(
        store
            .get("wallet.default")
            .and_then(|entry| entry.value.as_str()),
        Some("0xabc")
    );
    assert_eq!(
        store
            .get("inputs.token.decimals")
            .and_then(|entry| entry.value.as_i64()),
        Some(18)
    );
}

#[test]
fn parse_user_supplied_answer_value_prefers_json_literal() {
    assert_eq!(super::parse_user_supplied_answer_value("18"), json!(18));
    assert_eq!(
        super::parse_user_supplied_answer_value("{\"a\":1}"),
        json!({"a":1})
    );
    assert_eq!(
        super::parse_user_supplied_answer_value("0xabc"),
        json!("0xabc")
    );
}

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
        checkpoint: None,
        approvals_mode: None,
        max_iterations: None,
        max_planner_rounds: None,
        max_tool_rounds: None,
        max_index_candidates: None,
        planner_context_token_budget: None,
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
                    "source": "estimated(chars_div_4)",
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
    assert!(output.contains("source=estimated(chars_div_4)"));
    assert!(output.contains("context_limit_tokens=8192"));
    assert!(output.contains("context_soft_limit_tokens=7372"));
    assert!(output.contains("context_remaining_tokens=7212"));
    assert!(output.contains("duplicate_tool_call_ratio_bps=1200"));
    assert!(output.contains("discovery_tool_call_ratio_bps=6400"));
    assert!(output.contains("empty_search_streak_max=2"));
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
        checkpoint: None,
        approvals_mode: None,
        max_iterations: None,
        max_planner_rounds: None,
        max_tool_rounds: None,
        max_index_candidates: None,
        planner_context_token_budget: None,
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
            prompts_dir: None,
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
        checkpoint: None,
        approvals_mode: None,
        max_iterations: None,
        max_planner_rounds: None,
        max_tool_rounds: None,
        max_index_candidates: None,
        planner_context_token_budget: None,
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
            prompts_dir: None,
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

fn write_temp_file(prefix: &str, content: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time must be monotonic")
        .as_nanos();
    path.push(format!(
        "ais-runner-agent-{prefix}-{}-{nanos}.tmp",
        std::process::id()
    ));
    fs::write(&path, content).expect("write temp file");
    path
}
