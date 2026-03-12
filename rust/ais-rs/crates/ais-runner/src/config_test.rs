use super::{
    build_router_executor, build_router_executor_for_plan, load_runner_config, RunnerConfigError,
};
use ais_sdk::PlanDocument;
use serde_json::{json, Map, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn load_runner_config_parses_yaml_and_registers_exact_chain_routes() {
    let path = write_temp_file(
        "runner-config-ok",
        r#"
schema: ais-runner/0.0.1
engine:
  max_concurrency: 8
  per_chain:
    eip155:1:
      max_read_concurrency: 8
      max_write_concurrency: 1
chains:
  eip155:1:
    rpc_url: https://rpc.evm.example
    timeout_ms: 12000
    wait_for_receipt: true
    receipt_poll:
      interval_ms: 500
      max_attempts: 10
    signer:
      type: evm_private_key
      private_key: 0x1111111111111111111111111111111111111111111111111111111111111111
  solana:mainnet:
    rpc_url: https://rpc.solana.example
    commitment: finalized
    timeout_ms: 15000
    wait_for_confirmation: true
    confirmation_poll:
      interval_ms: 500
      max_attempts: 20
    signer:
      type: solana_private_key
      private_key: dev-local-key
"#,
    );

    let config = load_runner_config(path.as_path()).expect("config must load");
    let router = build_router_executor(&config).expect("router must build");
    assert_eq!(router.registrations().len(), 3);
    assert!(router
        .registrations()
        .iter()
        .any(|reg| reg.chain == "eip155:1"));
    assert!(router
        .registrations()
        .iter()
        .any(|reg| reg.chain == "solana:mainnet"));
    assert!(router.can_route("eip155:1", "evm_call"));
    assert!(router.can_route("eip155:1", "evm_rpc"));
    assert!(!router.can_route("eip155:1", "sui_tx"));
}

#[test]
fn load_runner_config_with_offchain_plugin_registers_handler() {
    let path = write_temp_file(
        "runner-config-offchain-plugin",
        r#"
schema: ais-runner/0.0.1
chains:
  eip155:1:
    rpc_url: https://rpc.evm.example
plugins:
  execution:
    offchain_apy_query:
      enabled: true
      chains: ["eip155:1"]
      allowed_domains: ["api.example.com"]
      timeout_ms: 3000
      max_retries: 2
      retry_backoff_ms: 100
"#,
    );
    let config = load_runner_config(path.as_path()).expect("config must load");
    let router = build_router_executor(&config).expect("router must build");
    assert!(router.can_route("eip155:1", "offchain_apy_query"));
}

#[test]
fn load_runner_config_allows_external_chain_for_plugin_routes() {
    let path = write_temp_file(
        "runner-config-external-chain-plugin",
        r#"
schema: ais-runner/0.0.1
chains:
  sui:mainnet:
    rpc_url: https://rpc.sui.example
plugins:
  execution:
    offchain_apy_query:
      enabled: true
      chains: ["sui:mainnet"]
      allowed_domains: ["api.example.com"]
"#,
    );
    let config = load_runner_config(path.as_path()).expect("config must load");
    let router = build_router_executor(&config).expect("router must build");
    assert!(router.can_route("sui:mainnet", "offchain_apy_query"));
}

#[test]
fn build_router_executor_for_plan_reports_missing_chain_as_issue() {
    let config = load_runner_config(
        write_temp_file(
            "runner-config-missing",
            r#"
schema: ais-runner/0.0.1
chains:
  eip155:1:
    rpc_url: https://rpc.evm.example
"#,
        )
        .as_path(),
    )
    .expect("config must load");
    let plan = PlanDocument {
        schema: "ais-plan/0.0.3".to_string(),
        meta: Some(json!({})),
        nodes: vec![
            json!({"id":"n1","chain":"eip155:1","execution":{"type":"evm_rpc"}}),
            json!({"id":"n2","chain":"solana:mainnet","execution":{"type":"solana_read"}}),
        ],
        extensions: Map::<String, Value>::new(),
    };

    let issues = match build_router_executor_for_plan(&plan, &config) {
        Ok(_) => panic!("must fail"),
        Err(issues) => issues,
    };
    assert_eq!(issues.len(), 1);
    assert_eq!(
        issues[0].reference.as_deref(),
        Some("runner.config.chain_missing")
    );
    assert_eq!(issues[0].field_path.to_string(), "$.nodes[1].chain");
}

#[test]
fn build_router_executor_for_plan_reports_unregistered_execution_type() {
    let config = load_runner_config(
        write_temp_file(
            "runner-config-unregistered-type",
            r#"
schema: ais-runner/0.0.1
chains:
  eip155:1:
    rpc_url: https://rpc.evm.example
"#,
        )
        .as_path(),
    )
    .expect("config must load");
    let plan = PlanDocument {
        schema: "ais-plan/0.0.3".to_string(),
        meta: Some(json!({})),
        nodes: vec![json!({
            "id":"n1",
            "chain":"eip155:1",
            "execution":{"type":"sui_tx"}
        })],
        extensions: Map::<String, Value>::new(),
    };
    let issues = match build_router_executor_for_plan(&plan, &config) {
        Ok(_) => panic!("must fail"),
        Err(issues) => issues,
    };
    assert_eq!(issues.len(), 1);
    assert_eq!(
        issues[0].reference.as_deref(),
        Some("runner.config.execution_type_unregistered")
    );
    assert_eq!(
        issues[0].field_path.to_string(),
        "$.nodes[0].execution.type"
    );
}

#[test]
fn load_runner_config_rejects_signer_type_mismatch() {
    let path = write_temp_file(
        "runner-config-type-mismatch",
        r#"
schema: ais-runner/0.0.1
chains:
  eip155:1:
    rpc_url: https://rpc.evm.example
    signer:
      type: solana_private_key
      private_key: not-evm
"#,
    );

    let error = load_runner_config(path.as_path()).expect_err("must reject");
    match error {
        RunnerConfigError::Validation(issues) => {
            assert_eq!(issues.len(), 1);
            assert_eq!(
                issues[0].reference.as_deref(),
                Some("runner.config.signer.type_mismatch")
            );
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn load_runner_config_rejects_invalid_offchain_plugin_settings() {
    let path = write_temp_file(
        "runner-config-offchain-invalid",
        r#"
schema: ais-runner/0.0.1
chains:
  eip155:1:
    rpc_url: https://rpc.evm.example
plugins:
  execution:
    offchain_apy_query:
      enabled: true
      chains: []
      allowed_domains: []
      timeout_ms: 0
      retry_backoff_ms: 0
"#,
    );
    let error = load_runner_config(path.as_path()).expect_err("must reject");
    match error {
        RunnerConfigError::Validation(issues) => {
            let refs = issues
                .iter()
                .filter_map(|issue| issue.reference.as_deref())
                .collect::<Vec<_>>();
            assert!(refs.contains(&"runner.config.offchain_apy_query.chains.non_empty"));
            assert!(refs.contains(&"runner.config.offchain_apy_query.allowed_domains.non_empty"));
            assert!(refs.contains(&"runner.config.offchain_apy_query.timeout"));
            assert!(refs.contains(&"runner.config.offchain_apy_query.retry_backoff"));
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn load_runner_config_expands_env_placeholders() {
    let env_key = format!("AIS_RUNNER_TEST_RPC_{}", std::process::id());
    let env_value = "https://rpc.env.example";
    unsafe {
        std::env::set_var(env_key.as_str(), env_value);
    }

    let path = write_temp_file(
        "runner-config-env",
        format!(
            r#"
schema: ais-runner/0.0.1
chains:
  eip155:1:
    rpc_url: ${{{env_key}}}
"#
        )
        .as_str(),
    );

    let config = load_runner_config(path.as_path()).expect("config must load");
    assert_eq!(
        config
            .chains
            .get("eip155:1")
            .expect("chain")
            .rpc_url
            .as_str(),
        env_value
    );
}

#[test]
fn load_runner_config_expands_env_placeholders_for_llm_and_signer_secrets() {
    let api_key_env = format!("AIS_RUNNER_TEST_OPENROUTER_KEY_{}", std::process::id());
    let private_key_env = format!("AIS_RUNNER_TEST_EVM_KEY_{}", std::process::id());
    let api_key = "test-openrouter-key";
    let private_key = "0x1111111111111111111111111111111111111111111111111111111111111111";
    unsafe {
        std::env::set_var(api_key_env.as_str(), api_key);
        std::env::set_var(private_key_env.as_str(), private_key);
    }

    let path = write_temp_file(
        "runner-config-secret-env",
        format!(
            r#"
schema: ais-runner/0.0.1
llm:
  provider: openrouter
  model: openai/gpt-4.1-mini
  api_key: ${{{api_key_env}}}
chains:
  eip155:1:
    rpc_url: https://rpc.evm.example
    signer:
      type: evm_private_key
      private_key: ${{{private_key_env}}}
"#
        )
        .as_str(),
    );

    let config = load_runner_config(path.as_path()).expect("config must load");
    let llm = config.llm.as_ref().expect("llm config");
    assert_eq!(llm.api_key, api_key);
    let signer = &config
        .chains
        .get("eip155:1")
        .expect("chain")
        .signer
        .as_ref()
        .expect("signer");
    match signer {
        super::SignerConfig::EvmPrivateKey { private_key: value } => {
            assert_eq!(value, private_key);
        }
        other => panic!("unexpected signer: {other:?}"),
    }
}

#[test]
fn load_runner_config_reports_missing_secret_env_placeholders_as_validation_issues() {
    let api_key_env = format!(
        "AIS_RUNNER_TEST_MISSING_OPENROUTER_KEY_{}",
        std::process::id()
    );
    let private_key_env = format!("AIS_RUNNER_TEST_MISSING_EVM_KEY_{}", std::process::id());
    unsafe {
        std::env::remove_var(api_key_env.as_str());
        std::env::remove_var(private_key_env.as_str());
    }

    let path = write_temp_file(
        "runner-config-missing-secret-env",
        format!(
            r#"
schema: ais-runner/0.0.1
llm:
  provider: openrouter
  model: openai/gpt-4.1-mini
  api_key: ${{{api_key_env}}}
chains:
  eip155:1:
    rpc_url: https://rpc.evm.example
    signer:
      type: evm_private_key
      private_key: ${{{private_key_env}}}
"#
        )
        .as_str(),
    );

    let error = load_runner_config(path.as_path()).expect_err("must reject");
    match error {
        RunnerConfigError::Validation(issues) => {
            let refs = issues
                .iter()
                .filter_map(|issue| issue.reference.as_deref())
                .collect::<Vec<_>>();
            assert!(refs.contains(&"runner.config.env_placeholder.missing"));
            assert!(issues
                .iter()
                .any(|issue| issue.field_path.to_string() == "$.llm.api_key"));
            assert!(issues.iter().any(|issue| {
                issue.field_path.to_string().contains("private_key")
                    && issue
                        .message
                        .contains(format!("${{{private_key_env}}}").as_str())
            }));
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn load_runner_config_parses_llm_config() {
    let path = write_temp_file(
        "runner-config-llm",
        r#"
schema: ais-runner/0.0.1
llm:
  provider: openrouter
  model: openai/gpt-4.1-mini
  api_key: test-key
  controller_prompts_dir: ./prompts
  operator_templates_dir: ./operator-templates
  max_retries_per_provider: 2
  rotation: round_robin
  planner_context_token_budget: 9000
  max_tool_rounds: 30
  context_limit_tokens: 128k
  fallback:
    - provider: groq
      model: llama-3.3-70b-versatile
      api_key: groq-key
chains:
  eip155:1:
    rpc_url: https://rpc.evm.example
"#,
    );
    let config = load_runner_config(path.as_path()).expect("config must load");
    let llm = config.llm.as_ref().expect("llm config");
    assert_eq!(llm.provider, "openrouter");
    assert_eq!(llm.model, "openai/gpt-4.1-mini");
    assert_eq!(llm.api_key, "test-key");
    assert_eq!(llm.api_base, None);
    assert_eq!(llm.controller_prompts_dir.as_deref(), Some("./prompts"));
    assert_eq!(
        llm.operator_templates_dir.as_deref(),
        Some("./operator-templates")
    );
    assert_eq!(llm.max_retries_per_provider, Some(2));
    assert_eq!(llm.rotation, super::RunnerLlmRotationMode::RoundRobin);
    assert_eq!(llm.planner_context_token_budget, Some(9000));
    assert_eq!(llm.max_tool_rounds, Some(30));
    assert_eq!(llm.context_limit_tokens, Some(128000));
    assert_eq!(llm.fallback.len(), 1);
    assert_eq!(llm.fallback[0].provider, "groq");
}

#[test]
fn load_runner_config_rejects_invalid_llm_config() {
    let path = write_temp_file(
        "runner-config-llm-invalid",
        r#"
schema: ais-runner/0.0.1
llm:
  provider: ""
  model: ""
  api_key: ""
  fallback:
    - provider: ""
      model: ""
      api_key: ""
chains:
  eip155:1:
    rpc_url: https://rpc.evm.example
"#,
    );
    let error = load_runner_config(path.as_path()).expect_err("must reject");
    match error {
        RunnerConfigError::Validation(issues) => {
            let refs = issues
                .iter()
                .filter_map(|issue| issue.reference.as_deref())
                .collect::<Vec<_>>();
            assert!(refs.contains(&"runner.config.llm.provider"));
            assert!(refs.contains(&"runner.config.llm.model"));
            assert!(refs.contains(&"runner.config.llm.api_key"));
            assert_eq!(
                refs.iter()
                    .filter(|reference| **reference == "runner.config.llm.provider")
                    .count(),
                2
            );
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn load_runner_config_rejects_empty_controller_prompts_dir() {
    let path = write_temp_file(
        "runner-config-llm-empty-controller-prompts-dir",
        r#"
schema: ais-runner/0.0.1
llm:
  provider: openrouter
  model: openai/gpt-4.1-mini
  api_key: test-key
  controller_prompts_dir: "   "
chains:
  eip155:1:
    rpc_url: https://rpc.evm.example
"#,
    );
    let error = load_runner_config(path.as_path()).expect_err("must reject");
    match error {
        RunnerConfigError::Validation(issues) => {
            let refs = issues
                .iter()
                .filter_map(|issue| issue.reference.as_deref())
                .collect::<Vec<_>>();
            assert!(refs.contains(&"runner.config.llm.controller_prompts_dir"));
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn load_runner_config_rejects_empty_operator_templates_dir() {
    let path = write_temp_file(
        "runner-config-llm-empty-operator-templates-dir",
        r#"
schema: ais-runner/0.0.1
llm:
  provider: openrouter
  model: openai/gpt-4.1-mini
  api_key: test-key
  operator_templates_dir: "   "
chains:
  eip155:1:
    rpc_url: https://rpc.evm.example
"#,
    );
    let error = load_runner_config(path.as_path()).expect_err("must reject");
    match error {
        RunnerConfigError::Validation(issues) => {
            let refs = issues
                .iter()
                .filter_map(|issue| issue.reference.as_deref())
                .collect::<Vec<_>>();
            assert!(refs.contains(&"runner.config.llm.operator_templates_dir"));
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn load_runner_config_rejects_zero_planner_context_token_budget() {
    let path = write_temp_file(
        "runner-config-llm-zero-context-budget",
        r#"
schema: ais-runner/0.0.1
llm:
  provider: openrouter
  model: openai/gpt-4.1-mini
  api_key: test-key
  planner_context_token_budget: 0
chains:
  eip155:1:
    rpc_url: https://rpc.evm.example
"#,
    );
    let error = load_runner_config(path.as_path()).expect_err("must reject");
    match error {
        RunnerConfigError::Validation(issues) => {
            let refs = issues
                .iter()
                .filter_map(|issue| issue.reference.as_deref())
                .collect::<Vec<_>>();
            assert!(refs.contains(&"runner.config.llm.planner_context_token_budget"));
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn load_runner_config_rejects_zero_context_limit_tokens() {
    let path = write_temp_file(
        "runner-config-llm-zero-context-limit",
        r#"
schema: ais-runner/0.0.1
llm:
  provider: openrouter
  model: openai/gpt-4.1-mini
  api_key: test-key
  context_limit_tokens: 0
chains:
  eip155:1:
    rpc_url: https://rpc.evm.example
"#,
    );
    let error = load_runner_config(path.as_path()).expect_err("must reject");
    match error {
        RunnerConfigError::Validation(issues) => {
            let refs = issues
                .iter()
                .filter_map(|issue| issue.reference.as_deref())
                .collect::<Vec<_>>();
            assert!(refs.contains(&"runner.config.llm.context_limit_tokens"));
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn load_runner_config_rejects_zero_max_tool_rounds() {
    let path = write_temp_file(
        "runner-config-llm-zero-max-tool-rounds",
        r#"
schema: ais-runner/0.0.1
llm:
  provider: openrouter
  model: openai/gpt-4.1-mini
  api_key: test-key
  max_tool_rounds: 0
chains:
  eip155:1:
    rpc_url: https://rpc.evm.example
"#,
    );
    let error = load_runner_config(path.as_path()).expect_err("must reject");
    match error {
        RunnerConfigError::Validation(issues) => {
            let refs = issues
                .iter()
                .filter_map(|issue| issue.reference.as_deref())
                .collect::<Vec<_>>();
            assert!(refs.contains(&"runner.config.llm.max_tool_rounds"));
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn parse_token_count_supports_suffix_and_separators() {
    assert_eq!(super::parse_token_count("262k"), Some(262000));
    assert_eq!(super::parse_token_count("1M"), Some(1000000));
    assert_eq!(super::parse_token_count("262,144"), Some(262144));
    assert_eq!(super::parse_token_count(" 131_072 "), Some(131072));
    assert_eq!(super::parse_token_count(""), None);
    assert_eq!(super::parse_token_count("abc"), None);
}

fn write_temp_file(prefix: &str, content: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time must be monotonic")
        .as_nanos();
    path.push(format!(
        "ais-runner-{prefix}-{}-{nanos}.tmp",
        std::process::id()
    ));
    fs::write(&path, content).expect("must write temp file");
    path
}

#[allow(dead_code)]
fn read(path: impl AsRef<Path>) -> String {
    fs::read_to_string(path).expect("must read fixture")
}
