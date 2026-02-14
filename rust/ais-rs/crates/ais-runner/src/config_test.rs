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
    assert_eq!(router.registrations().len(), 2);
    assert!(router.registrations().iter().any(|reg| reg.chain == "eip155:1"));
    assert!(router.registrations().iter().any(|reg| reg.chain == "solana:mainnet"));
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
    assert_eq!(issues[0].reference.as_deref(), Some("runner.config.chain_missing"));
    assert_eq!(issues[0].field_path.to_string(), "$.nodes[1].chain");
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
fn load_runner_config_expands_env_placeholders() {
    let env_key = format!(
        "AIS_RUNNER_TEST_RPC_{}",
        std::process::id()
    );
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
