use super::{hash_plan, maybe_save_checkpoint, process_replace_plan_commands, read_command_jsonl};
use crate::checkpoint_ledger::RunnerCheckpointLedger;
use crate::cli::{OutputFormat, PlanCommand};
use crate::config::RunnerConfig;
use crate::error::RunnerError;
use crate::{
    execute_plan_diff, execute_replay, execute_run_plan, execute_run_workflow, PlanDiffCommand,
    ReplayCommand, WorkflowCommand,
};
use ais_engine::{
    create_checkpoint_document, encode_event_jsonl_line, load_checkpoint_from_path,
    save_checkpoint_to_path, CheckpointEngineState, CheckpointSideEffectRecord, EngineCommandType,
    EngineEvent, EngineEventRecord, EngineEventType, EngineRunnerOptions, EngineRunnerState,
};
use ais_sdk::{
    parse_document_with_options, validate_document_semantics, DocumentFormat, ParseDocumentOptions,
    PlanDocument,
};
use serde_json::{json, Value};
use std::fs;
use std::io::Cursor;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn run_plan_dry_run_json_includes_nodes_and_issues() {
    let plan_path = write_temp_file(
        "plan-json",
        r#"{
  "schema":"ais-plan/0.0.3",
  "meta": {},
  "nodes":[
    {
      "id":"node-1",
      "chain":"eip155:1",
      "kind":"execution",
      "execution":{"type":"custom"}
    }
  ]
}"#,
    );

    let output = execute_run_plan(&PlanCommand {
        plan: plan_path.clone(),
        config: None,
        runtime: None,
        dry_run: true,
        events_jsonl: None,
        trace: None,
        checkpoint: None,
        commands_stdin_jsonl: false,
        verbose: false,
        format: OutputFormat::Json,
    })
    .expect("dry run json must succeed");

    let parsed: Value = serde_json::from_str(output.as_str()).expect("must be valid json");
    assert!(parsed.get("nodes").and_then(Value::as_array).is_some());
    assert!(parsed.get("issues").and_then(Value::as_array).is_some());
}

#[test]
fn write_event_sinks_without_persisted_engine_sink_does_not_advance_watermark() {
    let command = PlanCommand {
        plan: PathBuf::from("ignored.plan.json"),
        config: None,
        runtime: None,
        dry_run: false,
        events_jsonl: None,
        trace: None,
        checkpoint: None,
        commands_stdin_jsonl: false,
        verbose: false,
        format: OutputFormat::Text,
    };
    let events = vec![EngineEventRecord::new(
        "run-test",
        3,
        "2026-03-07T00:00:00Z",
        EngineEvent::new(EngineEventType::EnginePaused),
    )];
    let mut audit_attempt = crate::audit_contract::AuditStreamAttempt::fresh();

    super::write_event_sinks(&command, &events, &mut audit_attempt).expect("write events");

    assert_eq!(audit_attempt.last_event_seq, None);
    assert_eq!(audit_attempt.last_event_ts, None);
}

#[test]
fn run_plan_dry_run_text_is_stable_and_readable() {
    let plan_path = write_temp_file(
        "plan-text",
        r#"{
  "schema":"ais-plan/0.0.3",
  "meta": {},
  "nodes":[
    {
      "id":"node-1",
      "chain":"eip155:1",
      "kind":"execution",
      "execution":{"type":"custom"}
    }
  ]
}"#,
    );

    let output = execute_run_plan(&PlanCommand {
        plan: plan_path,
        config: None,
        runtime: None,
        dry_run: true,
        events_jsonl: None,
        trace: None,
        checkpoint: None,
        commands_stdin_jsonl: false,
        verbose: false,
        format: OutputFormat::Text,
    })
    .expect("dry run text must succeed");

    assert!(output.contains("AIS dry-run"));
    assert!(output.contains("summary: total=1"));
    assert!(output.contains("nodes:"));
    assert!(output.contains("id=node-1"));
}

#[test]
fn run_plan_runtime_yaml_dispatches_and_unblocks_refs() {
    let plan_path = write_temp_file(
        "plan-runtime",
        r#"{
  "schema":"ais-plan/0.0.3",
  "meta": {},
  "nodes":[
    {
      "id":"node-1",
      "chain":"eip155:1",
      "kind":"execution",
      "execution":{
        "type":"custom",
        "amount":{"ref":"inputs.amount"}
      }
    }
  ]
}"#,
    );
    let runtime_path = write_temp_file(
        "runtime-yaml",
        r#"
inputs:
  amount: "100"
"#,
    );

    let output = execute_run_plan(&PlanCommand {
        plan: plan_path,
        config: None,
        runtime: Some(runtime_path),
        dry_run: true,
        events_jsonl: None,
        trace: None,
        checkpoint: None,
        commands_stdin_jsonl: false,
        verbose: false,
        format: OutputFormat::Json,
    })
    .expect("dry run with runtime must succeed");

    let parsed: Value = serde_json::from_str(output.as_str()).expect("must be valid json");
    let state = parsed
        .get("nodes")
        .and_then(Value::as_array)
        .and_then(|nodes| nodes.first())
        .and_then(|node| node.get("readiness"))
        .and_then(|readiness| readiness.get("state"))
        .and_then(Value::as_str)
        .expect("state must exist");
    assert_eq!(state, "ready");
}

#[test]
fn run_workflow_loads_workspace_documents() {
    let workspace_dir = temp_dir("workspace-ok");
    let workflow_path = write_temp_file_in(
        workspace_dir.as_path(),
        "workflow.yaml",
        r#"
schema: ais-flow/0.0.3
meta:
  name: wf
  version: 0.0.1
requires_pack:
  name: safe-defi
  version: 0.0.2
nodes:
  - id: swap
    type: action_ref
    protocol: uniswap-v3@0.0.2
    action: swap_exact_in
    chain: eip155:1
"#,
    );
    write_temp_file_in(
        workspace_dir.as_path(),
        "protocol.json",
        r#"{
  "schema":"ais/0.0.2",
  "meta":{"protocol":"uniswap-v3","version":"0.0.2"},
  "deployments":[{"chain":"eip155:1","contracts":{}}],
  "actions":{
    "swap_exact_in":{
      "description":"swap exact in",
      "risk_level":3,
      "params":[],
      "execution":{
        "eip155:*":{
          "type":"evm_call",
          "to":{"lit":"0x0000000000000000000000000000000000000001"},
          "abi":{"type":"function","name":"swapExactTokensForTokens","inputs":[],"outputs":[]},
          "args":{}
        }
      }
    }
  },
  "queries":{
    "quote":{
      "description":"quote",
      "params":[],
      "returns":[],
      "execution":{
        "eip155:*":{
          "type":"evm_read",
          "to":{"lit":"0x0000000000000000000000000000000000000001"},
          "abi":{"type":"function","name":"quote","inputs":[],"outputs":[]},
          "args":{}
        }
      }
    }
  }
}"#,
    );
    write_temp_file_in(
        workspace_dir.as_path(),
        "pack.json",
        r#"{
  "schema":"ais-pack/0.0.2",
  "name":"safe-defi",
  "version":"0.0.2",
  "includes":[{"protocol":"uniswap-v3","version":"0.0.2","chain_scope":["eip155:1"]}]
}"#,
    );

    let output = execute_run_workflow(&WorkflowCommand {
        workflow: workflow_path,
        workspace: Some(workspace_dir),
        config: None,
        runtime: None,
        dry_run: true,
        events_jsonl: None,
        trace: None,
        checkpoint: None,
        outputs: None,
        commands_stdin_jsonl: false,
        verbose: false,
        format: OutputFormat::Json,
    })
    .expect("run workflow must succeed");

    let parsed: Value = serde_json::from_str(output.as_str()).expect("must be valid json");
    assert_eq!(
        parsed.get("schema").and_then(Value::as_str),
        Some("ais-runner-run-workflow/0.0.1")
    );
    assert_eq!(
        parsed
            .get("plan")
            .and_then(|plan| plan.get("schema"))
            .and_then(Value::as_str),
        Some("ais-plan/0.0.3")
    );
    assert_eq!(
        parsed
            .get("documents")
            .and_then(|documents| documents.get("protocols"))
            .and_then(Value::as_u64),
        Some(1)
    );
}

#[test]
fn run_workflow_merges_input_defaults_into_runtime_for_dry_run() {
    let workspace_dir = temp_dir("workspace-input-defaults");
    let workflow_path = write_temp_file_in(
        workspace_dir.as_path(),
        "workflow.yaml",
        r#"
schema: ais-flow/0.0.3
meta:
  name: wf-default-inputs
  version: 0.0.1
requires_pack:
  name: safe-defi
  version: 0.0.2
inputs:
  amount:
    type: string
    required: false
    default: "100"
nodes:
  - id: quote
    type: query_ref
    protocol: uniswap-v3@0.0.2
    query: quote
    chain: eip155:1
    args:
      amount_in:
        ref: inputs.amount
"#,
    );
    write_temp_file_in(
        workspace_dir.as_path(),
        "protocol.json",
        r#"{
  "schema":"ais/0.0.2",
  "meta":{"protocol":"uniswap-v3","version":"0.0.2"},
  "deployments":[{"chain":"eip155:1","contracts":{}}],
  "actions":{},
  "queries":{
    "quote":{
      "description":"quote",
      "params":[],
      "returns":[],
      "execution":{
        "eip155:*":{
          "type":"evm_read",
          "to":{"lit":"0x0000000000000000000000000000000000000001"},
          "abi":{"type":"function","name":"quote","inputs":[],"outputs":[]},
          "args":{}
        }
      }
    }
  }
}"#,
    );
    write_temp_file_in(
        workspace_dir.as_path(),
        "pack.json",
        r#"{
  "schema":"ais-pack/0.0.2",
  "name":"safe-defi",
  "version":"0.0.2",
  "includes":[{"protocol":"uniswap-v3","version":"0.0.2","chain_scope":["eip155:1"]}]
}"#,
    );

    let output = execute_run_workflow(&WorkflowCommand {
        workflow: workflow_path,
        workspace: Some(workspace_dir),
        config: None,
        runtime: None,
        dry_run: true,
        events_jsonl: None,
        trace: None,
        checkpoint: None,
        outputs: None,
        commands_stdin_jsonl: false,
        verbose: false,
        format: OutputFormat::Json,
    })
    .expect("run workflow dry-run must succeed");

    let parsed: Value = serde_json::from_str(output.as_str()).expect("must be valid json");
    let readiness = parsed
        .get("dry_run")
        .and_then(|dry_run| dry_run.get("nodes"))
        .and_then(Value::as_array)
        .and_then(|nodes| nodes.first())
        .and_then(|node| node.get("readiness"))
        .and_then(|readiness| readiness.get("state"))
        .and_then(Value::as_str)
        .expect("readiness state must exist");
    assert_eq!(readiness, "ready");
}

#[test]
fn run_workflow_workspace_validation_issues_return_error() {
    let workspace_dir = temp_dir("workspace-issue");
    let workflow_path = write_temp_file_in(
        workspace_dir.as_path(),
        "workflow.yaml",
        r#"
schema: ais-flow/0.0.3
meta:
  name: wf
  version: 0.0.1
requires_pack:
  name: missing-pack
  version: 0.0.1
nodes: []
"#,
    );

    let error = execute_run_workflow(&WorkflowCommand {
        workflow: workflow_path,
        workspace: Some(workspace_dir),
        config: None,
        runtime: None,
        dry_run: true,
        events_jsonl: None,
        trace: None,
        checkpoint: None,
        outputs: None,
        commands_stdin_jsonl: false,
        verbose: false,
        format: OutputFormat::Text,
    })
    .expect_err("run workflow must fail on workspace validation");
    assert!(error.to_string().contains("workspace validation failed"));
}

#[test]
fn run_workflow_execute_requires_config_and_runs_engine_path() {
    let workspace_dir = temp_dir("workflow-exec");
    let workflow_path = write_temp_file_in(
        workspace_dir.as_path(),
        "workflow.yaml",
        r#"
schema: ais-flow/0.0.3
meta:
  name: wf-exec
  version: 0.0.1
requires_pack:
  name: safe-defi
  version: 0.0.2
nodes:
  - id: swap
    type: action_ref
    protocol: uniswap-v3@0.0.2
    action: swap_exact_in
    chain: eip155:1
"#,
    );
    write_temp_file_in(
        workspace_dir.as_path(),
        "protocol.json",
        r#"{
  "schema":"ais/0.0.2",
  "meta":{"protocol":"uniswap-v3","version":"0.0.2"},
  "deployments":[{"chain":"eip155:1","contracts":{}}],
  "actions":{
    "swap_exact_in":{
      "description":"swap exact in",
      "risk_level":3,
      "params":[],
      "execution":{
        "eip155:*":{
          "type":"evm_call",
          "to":{"lit":"0x0000000000000000000000000000000000000001"},
          "abi":{"type":"function","name":"swapExactTokensForTokens","inputs":[],"outputs":[]},
          "args":{}
        }
      }
    }
  },
  "queries":{}
}"#,
    );
    write_temp_file_in(
        workspace_dir.as_path(),
        "pack.json",
        r#"{
  "schema":"ais-pack/0.0.2",
  "name":"safe-defi",
  "version":"0.0.2",
  "includes":[{"protocol":"uniswap-v3","version":"0.0.2","chain_scope":["eip155:1"]}]
}"#,
    );

    let missing_config_error = execute_run_workflow(&WorkflowCommand {
        workflow: workflow_path.clone(),
        workspace: Some(workspace_dir.clone()),
        config: None,
        runtime: None,
        dry_run: false,
        events_jsonl: None,
        trace: None,
        checkpoint: None,
        outputs: None,
        commands_stdin_jsonl: false,
        verbose: false,
        format: OutputFormat::Json,
    })
    .expect_err("run workflow execute must require config");
    assert!(missing_config_error.to_string().contains("--config"));

    let config_path = write_temp_file(
        "runner-config-workflow-exec",
        r#"
schema: ais-runner/0.0.1
chains:
  eip155:1:
    rpc_url: https://rpc.evm.example
"#,
    );
    let output = execute_run_workflow(&WorkflowCommand {
        workflow: workflow_path,
        workspace: Some(workspace_dir),
        config: Some(config_path),
        runtime: None,
        dry_run: false,
        events_jsonl: None,
        trace: None,
        checkpoint: None,
        outputs: None,
        commands_stdin_jsonl: false,
        verbose: false,
        format: OutputFormat::Json,
    })
    .expect("run workflow execute path must succeed");
    let parsed: Value = serde_json::from_str(output.as_str()).expect("must be valid json");
    assert!(matches!(
        parsed.get("status").and_then(Value::as_str),
        Some("paused") | Some("completed") | Some("stopped")
    ));
}

#[test]
fn run_workflow_execute_can_write_outputs_file() {
    let workspace_dir = temp_dir("workflow-outputs");
    let workflow_path = write_temp_file_in(
        workspace_dir.as_path(),
        "workflow.yaml",
        r#"
schema: ais-flow/0.0.3
meta:
  name: wf-outputs
  version: 0.0.1
inputs:
  amount:
    type: token_amount
    required: false
    default: "1.5"
nodes: []
outputs:
  atomic:
    cel: "to_atomic(inputs.amount, 6)"
  human:
    cel: "to_human(to_atomic(inputs.amount, 6), 6)"
"#,
    );
    let config_path = write_temp_file(
        "runner-config-workflow-outputs",
        r#"
schema: ais-runner/0.0.1
chains:
  eip155:1:
    rpc_url: https://rpc.evm.example
"#,
    );
    let outputs_path = workspace_dir.join("outputs.json");

    let output = execute_run_workflow(&WorkflowCommand {
        workflow: workflow_path,
        workspace: Some(workspace_dir.clone()),
        config: Some(config_path),
        runtime: None,
        dry_run: false,
        events_jsonl: None,
        trace: None,
        checkpoint: None,
        outputs: Some(outputs_path.clone()),
        commands_stdin_jsonl: false,
        verbose: false,
        format: OutputFormat::Json,
    })
    .expect("run workflow execute with outputs must succeed");
    let run_summary: Value = serde_json::from_str(output.as_str()).expect("must be valid json");
    assert_eq!(
        run_summary.get("status").and_then(Value::as_str),
        Some("completed")
    );

    let outputs_json = fs::read_to_string(outputs_path).expect("must write outputs file");
    let parsed: Value = serde_json::from_str(outputs_json.as_str()).expect("must be valid json");
    assert_eq!(
        parsed.get("schema").and_then(Value::as_str),
        Some("ais-runner-workflow-outputs/0.0.1")
    );
    assert_eq!(
        parsed.pointer("/outputs/atomic").and_then(Value::as_u64),
        Some(1_500_000)
    );
    assert_eq!(
        parsed.pointer("/outputs/human").and_then(Value::as_str),
        Some("1.5")
    );
}

#[test]
fn repo_examples_now_parse_and_validate_for_raw_ingestion() {
    let aave = read_example_fixture("aave-v3.ais.yaml");
    let aave_document = parse_document_with_options(
        aave.as_str(),
        ParseDocumentOptions {
            format: DocumentFormat::Auto,
            validate_schema: true,
        },
    )
    .expect("raw aave example must now parse");
    assert!(validate_document_semantics(&aave_document).is_empty());

    let uniswap = read_example_fixture("uniswap-v3.ais.yaml");
    let uniswap_document = parse_document_with_options(
        uniswap.as_str(),
        ParseDocumentOptions {
            format: DocumentFormat::Auto,
            validate_schema: true,
        },
    )
    .expect("raw uniswap example must now parse");
    assert!(validate_document_semantics(&uniswap_document).is_empty());

    let safe_pack = read_example_fixture("safe-defi-pack.ais-pack.yaml");
    let safe_pack_document = parse_document_with_options(
        safe_pack.as_str(),
        ParseDocumentOptions {
            format: DocumentFormat::Auto,
            validate_schema: true,
        },
    )
    .expect("raw safe defi pack must now parse");
    assert!(validate_document_semantics(&safe_pack_document).is_empty());

    let erc20 = read_example_fixture("erc20.ais.yaml");
    let erc20_document = parse_document_with_options(
        erc20.as_str(),
        ParseDocumentOptions {
            format: DocumentFormat::Auto,
            validate_schema: true,
        },
    )
    .expect("raw erc20 example must now parse");
    assert!(validate_document_semantics(&erc20_document).is_empty());

    let spl_token = read_example_fixture("spl-token.ais.yaml");
    let spl_token_document = parse_document_with_options(
        spl_token.as_str(),
        ParseDocumentOptions {
            format: DocumentFormat::Auto,
            validate_schema: true,
        },
    )
    .expect("raw spl-token example must now parse");
    assert!(validate_document_semantics(&spl_token_document).is_empty());
}

#[test]
fn example_aave_supply_raw_dry_run_is_ready_with_deployment_contracts() {
    let workspace_dir = temp_dir("example-aave-supply-raw");
    write_temp_file_in(
        workspace_dir.as_path(),
        "aave-v3.ais.yaml",
        read_example_fixture("aave-v3.ais.yaml").as_str(),
    );
    let workflow_path = write_temp_file_in(
        workspace_dir.as_path(),
        "workflow.yaml",
        r#"
schema: ais-flow/0.0.3
meta:
  name: aave-supply-raw
  version: 0.0.1
nodes:
  - id: supply_raw
    type: action_ref
    protocol: aave-v3@0.0.2
    action: supply-raw
    chain: eip155:1
    args:
      token:
        object:
          address: { lit: "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48" }
          decimals: { lit: 6 }
          chain_id: { lit: "eip155:1" }
      amount_atomic: { lit: "1000000" }
      on_behalf_of: { lit: "0x1111111111111111111111111111111111111111" }
"#,
    );

    let output = execute_run_workflow(&WorkflowCommand {
        workflow: workflow_path,
        workspace: Some(workspace_dir),
        config: None,
        runtime: None,
        dry_run: true,
        events_jsonl: None,
        trace: None,
        checkpoint: None,
        outputs: None,
        commands_stdin_jsonl: false,
        verbose: false,
        format: OutputFormat::Json,
    })
    .expect("aave raw workflow dry-run must succeed");

    let parsed: Value = serde_json::from_str(output.as_str()).expect("json");
    assert_eq!(
        parsed.pointer("/plan/nodes/0/extensions/protocol/contracts/pool"),
        Some(&json!("0x87870Bca3F3fD6335C3F4ce8392D69350B4fA4E2"))
    );
    assert_eq!(
        parsed.pointer("/dry_run/nodes/0/readiness/state"),
        Some(&json!("ready"))
    );
}

#[test]
fn example_aave_withdraw_dry_run_resolves_calculated_fields() {
    let workspace_dir = temp_dir("example-aave-withdraw");
    write_temp_file_in(
        workspace_dir.as_path(),
        "aave-v3.ais.yaml",
        read_example_fixture("aave-v3.ais.yaml").as_str(),
    );
    let workflow_path = write_temp_file_in(
        workspace_dir.as_path(),
        "workflow.yaml",
        r#"
schema: ais-flow/0.0.3
meta:
  name: aave-withdraw
  version: 0.0.1
nodes:
  - id: withdraw
    type: action_ref
    protocol: aave-v3@0.0.2
    action: withdraw
    chain: eip155:1
    args:
      token:
        object:
          address: { lit: "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48" }
          decimals: { lit: 6 }
          chain_id: { lit: "eip155:1" }
      amount: { lit: "1.5" }
      recipient: { lit: "0x1111111111111111111111111111111111111111" }
"#,
    );

    let output = execute_run_workflow(&WorkflowCommand {
        workflow: workflow_path,
        workspace: Some(workspace_dir),
        config: None,
        runtime: None,
        dry_run: true,
        events_jsonl: None,
        trace: None,
        checkpoint: None,
        outputs: None,
        commands_stdin_jsonl: false,
        verbose: false,
        format: OutputFormat::Json,
    })
    .expect("aave withdraw workflow dry-run must succeed");

    let parsed: Value = serde_json::from_str(output.as_str()).expect("json");
    assert_eq!(
        parsed.pointer("/dry_run/nodes/0/readiness/state"),
        Some(&json!("ready"))
    );
    assert_eq!(
        parsed.pointer("/dry_run/nodes/0/readiness/missing_refs"),
        Some(&json!([]))
    );
}

#[test]
fn example_aave_supply_dry_run_lowers_composite_execution() {
    let workspace_dir = temp_dir("example-aave-supply");
    write_temp_file_in(
        workspace_dir.as_path(),
        "aave-v3.ais.yaml",
        read_example_fixture("aave-v3.ais.yaml").as_str(),
    );
    let workflow_path = write_temp_file_in(
        workspace_dir.as_path(),
        "workflow.yaml",
        r#"
schema: ais-flow/0.0.3
meta:
  name: aave-supply
  version: 0.0.1
nodes:
  - id: supply
    type: action_ref
    protocol: aave-v3@0.0.2
    action: supply
    chain: eip155:1
    args:
      token:
        object:
          address: { lit: "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48" }
          decimals: { lit: 6 }
          chain_id: { lit: "eip155:1" }
      amount: { lit: "1.5" }
      on_behalf_of: { lit: "0x1111111111111111111111111111111111111111" }
"#,
    );

    let output = execute_run_workflow(&WorkflowCommand {
        workflow: workflow_path,
        workspace: Some(workspace_dir),
        config: None,
        runtime: None,
        dry_run: true,
        events_jsonl: None,
        trace: None,
        checkpoint: None,
        outputs: None,
        commands_stdin_jsonl: false,
        verbose: false,
        format: OutputFormat::Json,
    })
    .expect("aave supply workflow dry-run must succeed");

    let parsed: Value = serde_json::from_str(output.as_str()).expect("json");
    assert_eq!(
        parsed.pointer("/plan/nodes/0/id"),
        Some(&json!("supply__approve_if_needed"))
    );
    assert_eq!(parsed.pointer("/plan/nodes/1/id"), Some(&json!("supply")));
    assert_eq!(
        parsed.pointer("/plan/nodes/0/execution/type"),
        Some(&json!("evm_call"))
    );
    assert_eq!(
        parsed.pointer("/plan/nodes/1/execution/type"),
        Some(&json!("evm_call"))
    );
    assert_eq!(
        parsed.pointer("/plan/nodes/0/extensions/composite/step_id"),
        Some(&json!("approve_if_needed"))
    );
    assert_eq!(
        parsed.pointer("/plan/nodes/1/extensions/composite/step_id"),
        Some(&json!("supply"))
    );
    assert_eq!(
        parsed.pointer("/plan/nodes/1/deps/0"),
        Some(&json!("supply__approve_if_needed"))
    );
    assert_eq!(
        parsed.pointer("/plan/nodes/0/calculated_overrides/amount_atomic/expr/cel"),
        Some(&json!("to_atomic(params.amount, params.token)"))
    );
    assert_eq!(
        parsed.pointer("/plan/nodes/0/calculated_overrides/recipient/expr/cel"),
        Some(&json!(
            "params.on_behalf_of != null ? params.on_behalf_of : ctx.wallet_address"
        ))
    );
    assert_eq!(
        parsed.pointer("/dry_run/nodes/0/readiness/state"),
        Some(&json!("blocked"))
    );
    let missing_refs = parsed
        .pointer("/dry_run/nodes/0/readiness/missing_refs")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert!(
        !missing_refs
            .iter()
            .filter_map(Value::as_str)
            .any(|path| path == "calculated.amount_atomic" || path == "calculated.recipient"),
        "calculated bindings should resolve before prerequisite query blocks execution: {missing_refs:?}"
    );
}

#[test]
fn example_safe_defi_pack_registry_source_loads_snapshot_protocol_copy() {
    let workspace_dir = temp_dir("example-safe-pack-registry");
    write_temp_file_in(
        workspace_dir.as_path(),
        "safe-defi-pack.ais-pack.yaml",
        safe_defi_pack_registry_only_fixture().as_str(),
    );
    let workflow_path = write_temp_file_in(
        workspace_dir.as_path(),
        "workflow.yaml",
        r#"
schema: ais-flow/0.0.3
meta:
  name: swap
  version: 0.0.1
requires_pack:
  name: safe-defi-pack
  version: 0.0.2
nodes:
  - id: swap
    type: action_ref
    protocol: uniswap-v3@0.0.2
    action: swap-exact-in
    chain: eip155:8453
"#,
    );
    write_temp_file_in(
        workspace_dir.as_path(),
        ".ais/registry/protocols/uniswap-v3/0.0.2.yaml",
        r#"
schema: ais/0.0.2
meta:
  protocol: uniswap-v3
  version: 0.0.2
deployments:
  - chain: eip155:8453
    contracts: {}
actions:
  swap-exact-in:
    description: swap exact in
    risk_level: 3
    params: []
    execution:
      "eip155:*":
        type: evm_call
        to: { lit: "0x0000000000000000000000000000000000000001" }
        abi: { type: function, name: swap, inputs: [], outputs: [] }
        args: {}
queries: {}
"#,
    );

    let output = execute_run_workflow(&WorkflowCommand {
        workflow: workflow_path,
        workspace: Some(workspace_dir),
        config: None,
        runtime: None,
        dry_run: true,
        events_jsonl: None,
        trace: None,
        checkpoint: None,
        outputs: None,
        commands_stdin_jsonl: false,
        verbose: false,
        format: OutputFormat::Json,
    })
    .expect("registry-backed include must load snapshot protocol");

    let parsed: Value = serde_json::from_str(output.as_str()).expect("json");
    assert_eq!(parsed.pointer("/documents/protocols"), Some(&json!(1)));
    assert_eq!(
        parsed.pointer("/plan/nodes/0/execution/type"),
        Some(&json!("evm_call"))
    );
}

#[test]
fn example_safe_defi_raw_example_dry_run_lowers_pack_constrained_executor_nodes() {
    let workspace_dir = temp_dir("example-safe-defi-swap");
    write_temp_file_in(
        workspace_dir.as_path(),
        "safe-defi-pack.ais-pack.yaml",
        read_example_fixture("safe-defi-pack.ais-pack.yaml").as_str(),
    );
    write_temp_file_in(
        workspace_dir.as_path(),
        ".ais/registry/protocols/uniswap-v3/0.0.2.yaml",
        read_example_fixture("uniswap-v3.ais.yaml").as_str(),
    );
    let workflow_path = write_temp_file_in(
        workspace_dir.as_path(),
        "workflow.yaml",
        r#"
schema: ais-flow/0.0.3
meta:
  name: safe-defi-uniswap
  version: 0.0.1
requires_pack:
  name: safe-defi-pack
  version: 0.0.2
nodes:
  - id: quote
    type: query_ref
    protocol: uniswap-v3@0.0.2
    query: quote-exact-in-single
    chain: eip155:8453
    args:
      token_in:
        object:
          address: { lit: "0x4200000000000000000000000000000000000006" }
          decimals: { lit: 18 }
          chain_id: { lit: "eip155:8453" }
      token_out:
        object:
          address: { lit: "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913" }
          decimals: { lit: 6 }
          chain_id: { lit: "eip155:8453" }
      amount_in: { lit: "1.0" }
      fee: { lit: 500 }
  - id: allowance
    type: query_ref
    protocol: uniswap-v3@0.0.2
    query: allowance-token-in
    chain: eip155:8453
    args:
      token_in:
        object:
          address: { lit: "0x4200000000000000000000000000000000000006" }
          decimals: { lit: 18 }
          chain_id: { lit: "eip155:8453" }
  - id: swap
    type: action_ref
    protocol: uniswap-v3@0.0.2
    action: swap-exact-in
    chain: eip155:8453
    deps: ["quote", "allowance"]
    args:
      token_in:
        object:
          address: { lit: "0x4200000000000000000000000000000000000006" }
          decimals: { lit: 18 }
          chain_id: { lit: "eip155:8453" }
      token_out:
        object:
          address: { lit: "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913" }
          decimals: { lit: 6 }
          chain_id: { lit: "eip155:8453" }
      amount_in: { lit: "1.0" }
      slippage_bps: { lit: 20 }
      fee: { lit: 500 }
      recipient: { lit: "0x1111111111111111111111111111111111111111" }
"#,
    );
    let runtime_path = write_temp_file_in(
        workspace_dir.as_path(),
        "runtime.local.yaml",
        r#"
ctx:
  wallet_address: "0x1111111111111111111111111111111111111111"
  now: 1700000000
query:
  quote-exact-in-single:
    amount_out_atomic: "3500000000"
  allowance-token-in:
    allowance_atomic: "2000000000000000000"
nodes:
  quote:
    outputs:
      amount_out_atomic: "3500000000"
  allowance:
    outputs:
      allowance_atomic: "2000000000000000000"
"#,
    );

    let output = execute_run_workflow(&WorkflowCommand {
        workflow: workflow_path,
        workspace: Some(workspace_dir),
        config: None,
        runtime: Some(runtime_path),
        dry_run: true,
        events_jsonl: None,
        trace: None,
        checkpoint: None,
        outputs: None,
        commands_stdin_jsonl: false,
        verbose: false,
        format: OutputFormat::Json,
    })
    .expect("safe defi raw example workflow dry-run must succeed");

    let parsed: Value = serde_json::from_str(output.as_str()).expect("json");
    assert_eq!(parsed.pointer("/documents/protocols"), Some(&json!(1)));
    assert_eq!(parsed.pointer("/documents/packs"), Some(&json!(1)));

    let plan_nodes = parsed
        .pointer("/plan/nodes")
        .and_then(Value::as_array)
        .expect("plan nodes");
    let swap_index = plan_nodes
        .iter()
        .position(|node| node.get("id").and_then(Value::as_str) == Some("swap"))
        .expect("swap node index");
    let swap_node = plan_nodes
        .iter()
        .find(|node| node.get("id").and_then(Value::as_str) == Some("swap"))
        .cloned()
        .expect("lowered swap node");
    assert_eq!(
        swap_node.pointer("/execution/type"),
        Some(&json!("evm_call"))
    );
    assert_eq!(
        swap_node.pointer("/extensions/pack/ref"),
        Some(&json!("safe-defi-pack@0.0.2"))
    );
    assert_eq!(
        swap_node.pointer("/extensions/pack/matched_action_rule_ids/0"),
        Some(&json!("action_rule_0"))
    );
    assert_eq!(
        swap_node.pointer("/extensions/protocol/contracts/router"),
        Some(&json!("0x2626664c2603336E57B271c5C0b26F421741e481"))
    );
    assert_eq!(
        swap_node
            .pointer("/extensions/policy/effective_constraints")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(3)
    );
    assert_eq!(
        swap_node.pointer("/extensions/operation/query_bindings/quote-exact-in-single/node_id"),
        Some(&json!("quote"))
    );
    assert_eq!(
        swap_node.pointer("/extensions/operation/query_bindings/allowance-token-in/node_id"),
        Some(&json!("allowance"))
    );
    assert_eq!(
        swap_node.pointer("/calculated_overrides/fee_tier/expr/cel"),
        Some(&json!("params.fee != null ? params.fee : 500"))
    );
    assert_eq!(
        swap_node.pointer("/calculated_overrides/min_out_atomic/expr/cel"),
        Some(&json!(
            "mul_div(query[\"quote-exact-in-single\"].amount_out_atomic, (10000 - params.slippage_bps), 10000)"
        ))
    );

    let dry_run_nodes = parsed
        .pointer("/dry_run/nodes")
        .and_then(Value::as_array)
        .expect("dry run nodes");
    let approval_readiness = dry_run_nodes
        .get(swap_index.saturating_sub(1))
        .cloned()
        .expect("approval dry-run node");
    let swap_readiness = dry_run_nodes
        .get(swap_index)
        .cloned()
        .expect("swap dry-run node");
    assert_eq!(
        approval_readiness.pointer("/readiness/state"),
        Some(&json!("skipped"))
    );
    assert_eq!(
        approval_readiness.pointer("/readiness/missing_refs"),
        Some(&json!([]))
    );
    assert_eq!(
        swap_readiness.pointer("/readiness/state"),
        Some(&json!("ready"))
    );
    assert_eq!(
        swap_readiness.pointer("/readiness/missing_refs"),
        Some(&json!([]))
    );
}

#[test]
fn plan_diff_text_outputs_summary() {
    let before = write_temp_file(
        "plan-diff-before",
        r#"{
  "schema":"ais-plan/0.0.3",
  "meta": {},
  "nodes":[
    {"id":"a","kind":"execution","chain":"eip155:1","execution":{"type":"evm_read"}}
  ]
}"#,
    );
    let after = write_temp_file(
        "plan-diff-after",
        r#"{
  "schema":"ais-plan/0.0.3",
  "meta": {},
  "nodes":[
    {"id":"a","kind":"execution","chain":"eip155:1","execution":{"type":"evm_call"}},
    {"id":"b","kind":"execution","chain":"solana:mainnet","execution":{"type":"solana_read"}}
  ]
}"#,
    );
    let output = execute_plan_diff(&PlanDiffCommand {
        before,
        after,
        format: OutputFormat::Text,
    })
    .expect("plan diff text must succeed");
    assert!(output.contains("plan diff: added=1 removed=0 changed=1"));
    assert!(output.contains("added:"));
    assert!(output.contains("changed:"));
}

#[test]
fn plan_diff_json_outputs_structured_summary() {
    let before = write_temp_file(
        "plan-diff-json-before",
        r#"{
  "schema":"ais-plan/0.0.3",
  "meta": {},
  "nodes":[
    {"id":"a","kind":"execution","chain":"eip155:1","execution":{"type":"evm_read"}}
  ]
}"#,
    );
    let after = write_temp_file(
        "plan-diff-json-after",
        r#"{
  "schema":"ais-plan/0.0.3",
  "meta": {},
  "nodes":[
    {"id":"a","kind":"execution","chain":"eip155:1","execution":{"type":"evm_read"}}
  ]
}"#,
    );
    let output = execute_plan_diff(&PlanDiffCommand {
        before,
        after,
        format: OutputFormat::Json,
    })
    .expect("plan diff json must succeed");
    let parsed: Value = serde_json::from_str(output.as_str()).expect("must be valid json");
    assert_eq!(
        parsed
            .get("summary")
            .and_then(|summary| summary.get("added"))
            .and_then(Value::as_u64),
        Some(0)
    );
    assert_eq!(
        parsed
            .get("summary")
            .and_then(|summary| summary.get("changed"))
            .and_then(Value::as_u64),
        Some(0)
    );
}

#[test]
fn replay_trace_jsonl_until_node_json_output() {
    let event1 = EngineEventRecord::new(
        "run-replay",
        0,
        "2026-02-14T00:00:00Z",
        EngineEvent::new(EngineEventType::NodeReady),
    );
    let mut event2 = EngineEvent::new(EngineEventType::TxConfirmed);
    event2.node_id = Some("node-2".to_string());
    let event2 = EngineEventRecord::new("run-replay", 1, "2026-02-14T00:00:01Z", event2);
    let mut event3 = EngineEvent::new(EngineEventType::TxConfirmed);
    event3.node_id = Some("node-3".to_string());
    let event3 = EngineEventRecord::new("run-replay", 2, "2026-02-14T00:00:02Z", event3);
    let trace = format!(
        "{}{}{}",
        encode_event_jsonl_line(&event1).expect("encode"),
        encode_event_jsonl_line(&event2).expect("encode"),
        encode_event_jsonl_line(&event3).expect("encode"),
    );
    let trace_path = write_temp_file("replay-trace", trace.as_str());

    let output = execute_replay(&ReplayCommand {
        trace_jsonl: Some(trace_path),
        checkpoint: None,
        plan: None,
        config: None,
        until_node: Some("node-2".to_string()),
        format: OutputFormat::Json,
    })
    .expect("replay trace must succeed");

    let parsed: Value = serde_json::from_str(output.as_str()).expect("must be valid json");
    assert_eq!(
        parsed.get("status").and_then(Value::as_str),
        Some("reached_until_node")
    );
    assert_eq!(
        parsed.get("events_emitted").and_then(Value::as_u64),
        Some(2)
    );
}

#[test]
fn replay_checkpoint_requires_plan_and_config() {
    let checkpoint_path = write_temp_file("replay-checkpoint-only", "{}");
    let error = execute_replay(&ReplayCommand {
        trace_jsonl: None,
        checkpoint: Some(checkpoint_path),
        plan: None,
        config: None,
        until_node: None,
        format: OutputFormat::Text,
    })
    .expect_err("must require plan and config");
    assert!(error.to_string().contains("--plan"));
}

#[test]
fn run_plan_execute_writes_events_trace_and_checkpoint() {
    let plan_path = write_temp_file(
        "plan-exec",
        r#"{
  "schema":"ais-plan/0.0.3",
  "meta": {},
  "nodes":[
    {
      "id":"node-1",
      "chain":"eip155:1",
      "kind":"execution",
      "execution":{
        "type":"evm_call",
        "to":{"lit":"0x0000000000000000000000000000000000000001"},
        "abi":{"name":"ping","inputs":[],"outputs":[]},
        "args":{}
      }
    }
  ]
}"#,
    );
    let config_path = write_temp_file(
        "runner-config-exec",
        r#"
schema: ais-runner/0.0.1
chains:
  eip155:1:
    rpc_url: https://rpc.evm.example
"#,
    );
    let events_path = write_temp_file("events", "");
    let trace_path = write_temp_file("trace", "");
    let checkpoint_path = write_temp_file("checkpoint", "");

    let output = execute_run_plan(&PlanCommand {
        plan: plan_path,
        config: Some(config_path),
        runtime: None,
        dry_run: false,
        events_jsonl: Some(events_path.display().to_string()),
        trace: Some(trace_path.clone()),
        checkpoint: Some(checkpoint_path.clone()),
        commands_stdin_jsonl: false,
        verbose: false,
        format: OutputFormat::Json,
    })
    .expect("run execute must succeed");

    let parsed: Value = serde_json::from_str(output.as_str()).expect("must be valid json");
    assert_eq!(parsed.get("status").and_then(Value::as_str), Some("paused"));
    assert!(parsed
        .get("paused_reason")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .starts_with("executor_error:"));
    assert_eq!(
        parsed.get("command_accepted").and_then(Value::as_u64),
        Some(0)
    );
    assert_eq!(
        parsed.get("command_rejected").and_then(Value::as_u64),
        Some(0)
    );

    let events_content = fs::read_to_string(events_path).expect("events must exist");
    assert!(events_content.contains("\"type\":\"engine_paused\""));
    for line in events_content.lines() {
        let record: Value = serde_json::from_str(line).expect("events jsonl line");
        let ts = record
            .get("ts")
            .and_then(Value::as_str)
            .expect("event timestamp");
        assert_timestamp_is_wall_clock_rfc3339(ts);
    }
    let trace_content = fs::read_to_string(trace_path).expect("trace must exist");
    assert!(trace_content.contains("\"schema\":\"ais-engine-event/0.0.3\""));
    for line in trace_content.lines() {
        let record: Value = serde_json::from_str(line).expect("trace jsonl line");
        let ts = record
            .get("ts")
            .and_then(Value::as_str)
            .expect("trace timestamp");
        assert_timestamp_is_wall_clock_rfc3339(ts);
    }
    let checkpoint_content = fs::read_to_string(checkpoint_path).expect("checkpoint must exist");
    assert!(checkpoint_content.contains("\"schema\": \"ais-checkpoint/0.0.1\""));
}

#[test]
fn run_plan_execute_can_resume_from_checkpoint() {
    let plan_path = write_temp_file(
        "plan-resume",
        r#"{
  "schema":"ais-plan/0.0.3",
  "meta": {},
  "nodes":[
    {
      "id":"node-1",
      "chain":"eip155:1",
      "kind":"execution",
      "execution":{
        "type":"evm_call",
        "to":{"lit":"0x0000000000000000000000000000000000000001"},
        "abi":{"name":"ping","inputs":[],"outputs":[]},
        "args":{}
      }
    }
  ]
}"#,
    );
    let config_path = write_temp_file(
        "runner-config-resume",
        r#"
schema: ais-runner/0.0.1
chains:
  eip155:1:
    rpc_url: https://rpc.evm.example
"#,
    );
    let checkpoint_path = write_temp_file("checkpoint-resume", "");

    let _ = execute_run_plan(&PlanCommand {
        plan: plan_path.clone(),
        config: Some(config_path.clone()),
        runtime: None,
        dry_run: false,
        events_jsonl: None,
        trace: None,
        checkpoint: Some(checkpoint_path.clone()),
        commands_stdin_jsonl: false,
        verbose: false,
        format: OutputFormat::Json,
    })
    .expect("first run must succeed");

    let second = execute_run_plan(&PlanCommand {
        plan: plan_path,
        config: Some(config_path),
        runtime: None,
        dry_run: false,
        events_jsonl: None,
        trace: None,
        checkpoint: Some(checkpoint_path),
        commands_stdin_jsonl: false,
        verbose: false,
        format: OutputFormat::Json,
    })
    .expect("second run must succeed");

    let parsed: Value = serde_json::from_str(second.as_str()).expect("must be valid json");
    assert_eq!(
        parsed
            .get("resumed_from_checkpoint")
            .and_then(Value::as_bool),
        Some(true)
    );
}

#[test]
fn run_plan_rejects_unregistered_execution_type_in_execute_path() {
    let plan_path = write_temp_file(
        "plan-unregistered-exec-type",
        r#"{
  "schema":"ais-plan/0.0.3",
  "meta": {},
  "nodes":[
    {
      "id":"node-1",
      "chain":"eip155:1",
      "kind":"execution",
      "execution":{
        "type":"offchain_apy_query",
        "args":{}
      }
    }
  ]
}"#,
    );
    let config_path = write_temp_file(
        "runner-config-unregistered-exec-type",
        r#"
schema: ais-runner/0.0.1
chains:
  eip155:1:
    rpc_url: https://rpc.evm.example
"#,
    );

    let error = execute_run_plan(&PlanCommand {
        plan: plan_path,
        config: Some(config_path),
        runtime: None,
        dry_run: false,
        events_jsonl: None,
        trace: None,
        checkpoint: None,
        commands_stdin_jsonl: false,
        verbose: false,
        format: OutputFormat::Json,
    })
    .expect_err("must reject unregistered execution type");
    match error {
        RunnerError::ConfigInvalidForPlan(issues) => {
            assert!(!issues.is_empty());
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn checkpoint_resume_keeps_need_user_confirm_decision_stable() {
    let plan_path = write_temp_file(
        "plan-need-user-confirm-stable",
        r#"{
  "schema":"ais-plan/0.0.3",
  "meta": {},
  "nodes":[
    {
      "id":"swap-1",
      "chain":"eip155:1",
      "kind":"execution",
      "execution":{
        "type":"evm_call",
        "to":{"lit":"0x0000000000000000000000000000000000000001"},
        "abi":{"name":"swapExactTokensForTokens","inputs":[],"outputs":[]},
        "args":{}
      },
      "bindings":{
        "params":{
          "spend_amount":{"lit":"10"}
        }
      },
      "extensions":{
        "policy":{
          "constraint_templates":[
            {"name":"max_spend","params":{"amount_atomic":"1"}}
          ]
        }
      }
    }
  ]
}"#,
    );
    let config_path = write_temp_file(
        "runner-config-need-user-confirm-stable",
        r#"
schema: ais-runner/0.0.1
chains:
  eip155:1:
    rpc_url: https://rpc.evm.example
"#,
    );
    let checkpoint_path = write_temp_file("checkpoint-need-user-confirm-stable", "");
    let events_first = write_temp_file("events-first-need-user-confirm-stable", "");
    let events_second = write_temp_file("events-second-need-user-confirm-stable", "");

    let first = execute_run_plan(&PlanCommand {
        plan: plan_path.clone(),
        config: Some(config_path.clone()),
        runtime: None,
        dry_run: false,
        events_jsonl: Some(events_first.display().to_string()),
        trace: None,
        checkpoint: Some(checkpoint_path.clone()),
        commands_stdin_jsonl: false,
        verbose: false,
        format: OutputFormat::Json,
    })
    .expect("first run must pause for confirm");
    let second = execute_run_plan(&PlanCommand {
        plan: plan_path,
        config: Some(config_path),
        runtime: None,
        dry_run: false,
        events_jsonl: Some(events_second.display().to_string()),
        trace: None,
        checkpoint: Some(checkpoint_path),
        commands_stdin_jsonl: false,
        verbose: false,
        format: OutputFormat::Json,
    })
    .expect("second run must preserve decision");

    let first_value: Value = serde_json::from_str(first.as_str()).expect("json");
    let second_value: Value = serde_json::from_str(second.as_str()).expect("json");
    assert_eq!(
        first_value.get("paused_reason").and_then(Value::as_str),
        Some("need_user_confirm:swap-1")
    );
    assert_eq!(
        second_value.get("paused_reason").and_then(Value::as_str),
        Some("need_user_confirm:swap-1")
    );
    assert_eq!(
        second_value
            .get("resumed_from_checkpoint")
            .and_then(Value::as_bool),
        Some(true)
    );

    let first_events = fs::read_to_string(events_first).expect("events file");
    let second_events = fs::read_to_string(events_second).expect("events file");
    let first_confirm = first_events
        .lines()
        .find(|line| line.contains("\"type\":\"need_user_confirm\""))
        .expect("first need_user_confirm event");
    let second_confirm = second_events
        .lines()
        .find(|line| line.contains("\"type\":\"need_user_confirm\""))
        .expect("second need_user_confirm event");
    let first_record: Value = serde_json::from_str(first_confirm).expect("jsonl event");
    let second_record: Value = serde_json::from_str(second_confirm).expect("jsonl event");
    assert_eq!(
        first_record
            .pointer("/event/data/checks/gate/result")
            .and_then(Value::as_bool),
        Some(false)
    );
    assert_eq!(
        second_record
            .pointer("/event/data/checks/gate/result")
            .and_then(Value::as_bool),
        Some(false)
    );
    let first_hash = first_record
        .pointer("/event/data/details/confirmation_hash")
        .and_then(Value::as_str)
        .expect("first confirmation hash");
    let second_hash = second_record
        .pointer("/event/data/details/confirmation_hash")
        .and_then(Value::as_str)
        .expect("second confirmation hash");
    assert_eq!(first_hash, second_hash);
}

#[test]
fn checkpoint_resume_appends_attempt_scoped_event_identity_to_same_events_file() {
    let plan_path = write_temp_file(
        "plan-attempt-seq-stable",
        r#"{
  "schema":"ais-plan/0.0.3",
  "meta": {},
  "nodes":[
    {
      "id":"swap-1",
      "chain":"eip155:1",
      "kind":"execution",
      "execution":{
        "type":"evm_call",
        "to":{"lit":"0x0000000000000000000000000000000000000001"},
        "abi":{"name":"swapExactTokensForTokens","inputs":[],"outputs":[]},
        "args":{}
      },
      "bindings":{
        "params":{
          "spend_amount":{"lit":"10"}
        }
      },
      "extensions":{
        "policy":{
          "constraint_templates":[
            {"name":"max_spend","params":{"amount_atomic":"1"}}
          ]
        }
      }
    }
  ]
}"#,
    );
    let config_path = write_temp_file(
        "runner-config-attempt-seq-stable",
        r#"
schema: ais-runner/0.0.1
chains:
  eip155:1:
    rpc_url: https://rpc.evm.example
"#,
    );
    let checkpoint_path = write_temp_file("checkpoint-attempt-seq-stable", "");
    let events_path = write_temp_file("events-attempt-seq-stable", "");

    let _first = execute_run_plan(&PlanCommand {
        plan: plan_path.clone(),
        config: Some(config_path.clone()),
        runtime: None,
        dry_run: false,
        events_jsonl: Some(events_path.display().to_string()),
        trace: None,
        checkpoint: Some(checkpoint_path.clone()),
        commands_stdin_jsonl: false,
        verbose: false,
        format: OutputFormat::Json,
    })
    .expect("first run must pause for confirm");
    let _second = execute_run_plan(&PlanCommand {
        plan: plan_path,
        config: Some(config_path),
        runtime: None,
        dry_run: false,
        events_jsonl: Some(events_path.display().to_string()),
        trace: None,
        checkpoint: Some(checkpoint_path.clone()),
        commands_stdin_jsonl: false,
        verbose: false,
        format: OutputFormat::Json,
    })
    .expect("second run must resume and append");

    let records = fs::read_to_string(events_path)
        .expect("events")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("jsonl"))
        .collect::<Vec<_>>();
    let confirm_records = records
        .iter()
        .filter(|record| {
            record.get("type").and_then(Value::as_str) == Some("need_user_confirm")
                || record.pointer("/event/type").and_then(Value::as_str)
                    == Some("need_user_confirm")
        })
        .collect::<Vec<_>>();
    assert!(confirm_records.len() >= 2);
    assert_eq!(
        confirm_records[0].get("attempt_id"),
        Some(&json!("attempt-1"))
    );
    assert_eq!(
        confirm_records[1].get("attempt_id"),
        Some(&json!("attempt-2"))
    );
    assert_eq!(
        confirm_records[0].get("seq_scope"),
        Some(&json!("attempt_local"))
    );
    assert_eq!(
        confirm_records[1].get("seq_scope"),
        Some(&json!("attempt_local"))
    );
    assert_eq!(confirm_records[0].get("seq"), confirm_records[1].get("seq"));

    let last_record = records.last().expect("last event");
    let checkpoint = load_checkpoint_from_path(&checkpoint_path).expect("checkpoint");
    assert_eq!(
        checkpoint
            .extensions
            .get("resume_core")
            .and_then(|value| value.get("audit_stream"))
            .and_then(|value| value.get("last_event_attempt_id"))
            .and_then(Value::as_str),
        last_record.get("attempt_id").and_then(Value::as_str)
    );
    assert_eq!(
        checkpoint
            .extensions
            .get("resume_core")
            .and_then(|value| value.get("audit_stream"))
            .and_then(|value| value.get("last_event_seq"))
            .and_then(Value::as_u64),
        last_record.get("seq").and_then(Value::as_u64)
    );
    assert_eq!(
        checkpoint
            .extensions
            .get("resume_core")
            .and_then(|value| value.get("audit_stream"))
            .and_then(|value| value.get("last_event_ts"))
            .and_then(Value::as_str),
        last_record.get("ts").and_then(Value::as_str)
    );
    assert_eq!(
        checkpoint
            .extensions
            .get("resume_core")
            .and_then(|value| value.get("audit_stream"))
            .and_then(|value| value.get("last_event_run_id"))
            .and_then(Value::as_str),
        last_record.get("run_id").and_then(Value::as_str)
    );
}

#[test]
fn checkpoint_save_persists_approval_and_side_effect_ledgers() {
    let checkpoint_path = write_temp_file("checkpoint-ledger", "");
    let command = PlanCommand {
        plan: PathBuf::from("unused.plan.json"),
        config: None,
        runtime: None,
        dry_run: false,
        events_jsonl: None,
        trace: None,
        checkpoint: Some(checkpoint_path.clone()),
        commands_stdin_jsonl: false,
        verbose: false,
        format: OutputFormat::Text,
    };
    let plan = PlanDocument {
        schema: "ais-plan/0.0.3".to_string(),
        meta: None,
        nodes: vec![json!({"id":"swap-1","chain":"eip155:1","execution":{"type":"evm_call"}})],
        extensions: serde_json::Map::new(),
    };
    let state = EngineRunnerState {
        runtime: json!({
            "nodes":{
                "swap-1":{
                    "outputs":{
                        "tx_hash":"0xtx1",
                        "tx":{"nonce":7}
                    }
                }
            }
        }),
        approved_node_ids: vec!["swap-1".to_string()],
        ..EngineRunnerState::default()
    };
    let mut ledger = RunnerCheckpointLedger::default();
    let mut event = EngineEvent::new(EngineEventType::NeedUserConfirm);
    event.node_id = Some("swap-1".to_string());
    event.data.insert(
        "details".to_string(),
        json!({"confirmation_hash":"0xconfirm1"}),
    );
    let records = vec![
        EngineEventRecord {
            schema: "ais-engine-event/0.0.3".to_string(),
            run_id: "run-ledger".to_string(),
            seq: 1,
            ts: "2026-02-23T00:00:00Z".to_string(),
            event,
        },
        {
            let mut side_effect_event = EngineEvent::new(EngineEventType::SideEffectObserved);
            side_effect_event.data.insert(
                "record".to_string(),
                json!({
                    "schema":"ais-side-effect-record/0.1.0",
                    "effect_type":"tx",
                    "idempotency_key":"tx:swap-1:0xtx1",
                    "node_id":"swap-1",
                    "chain":"eip155:1",
                    "execution_type":"evm_call",
                    "status":"sent",
                    "observed_at":"2026-02-23T00:00:02Z",
                    "tx_hash":"0xtx1",
                    "nonce":7
                }),
            );
            EngineEventRecord {
                schema: "ais-engine-event/0.0.3".to_string(),
                run_id: "run-ledger".to_string(),
                seq: 2,
                ts: "2026-02-23T00:00:02Z".to_string(),
                event: side_effect_event,
            }
        },
    ];
    ledger.absorb_events(&records);
    ledger.mark_approved_nodes(&state.approved_node_ids, "2026-02-23T00:00:01Z");

    maybe_save_checkpoint(
        &command,
        "run-ledger",
        "plan-hash-ledger",
        &plan,
        &state,
        &ledger,
        &crate::audit_contract::AuditStreamAttempt::fresh(),
    )
    .expect("checkpoint save must succeed");

    let checkpoint = load_checkpoint_from_path(&checkpoint_path).expect("checkpoint load");
    assert_eq!(
        checkpoint
            .extensions
            .get("resume_core")
            .and_then(|value| value.get("audit_stream"))
            .and_then(|value| value.get("attempt_id"))
            .and_then(Value::as_str),
        Some("attempt-1")
    );
    assert_eq!(
        checkpoint
            .extensions
            .get("resume_core")
            .and_then(|value| value.get("audit_stream"))
            .and_then(|value| value.get("seq_scope"))
            .and_then(Value::as_str),
        Some("attempt_local")
    );
    assert!(!checkpoint.approvals_ledger.is_empty());
    assert!(checkpoint
        .approvals_ledger
        .iter()
        .any(|entry| entry.decision == "approve" && entry.node_id == "swap-1"));
    assert!(checkpoint
        .side_effects
        .iter()
        .any(|entry| entry.tx_hash.as_deref() == Some("0xtx1") && entry.node_id == "swap-1"));
}

#[test]
fn runtime_side_effect_without_checkpoint_does_not_guard_replay() {
    let plan_path = write_temp_file(
        "plan-runtime-side-effect-replay-guard",
        r#"{
  "schema":"ais-plan/0.0.3",
  "meta": {},
  "nodes":[
    {
      "id":"swap-1",
      "chain":"eip155:1",
      "kind":"execution",
      "execution":{
        "type":"evm_call",
        "to":{"lit":"0x0000000000000000000000000000000000000001"},
        "abi":{"name":"swapExactTokensForTokens","inputs":[],"outputs":[]},
        "args":{}
      }
    }
  ]
}"#,
    );
    let runtime_path = write_temp_file(
        "runtime-side-effect-replay-guard",
        r#"{
  "nodes":{
    "swap-1":{
      "outputs":{
        "tx_hash":"0xsent_without_checkpoint",
        "tx":{"nonce":11}
      }
    }
  }
}"#,
    );
    let config_path = write_temp_file(
        "runner-config-runtime-side-effect-replay-guard",
        r#"
schema: ais-runner/0.0.1
chains:
  eip155:1:
    rpc_url: https://rpc.evm.example
"#,
    );

    let output = execute_run_plan(&PlanCommand {
        plan: plan_path,
        config: Some(config_path),
        runtime: Some(runtime_path),
        dry_run: false,
        events_jsonl: None,
        trace: None,
        checkpoint: None,
        commands_stdin_jsonl: false,
        verbose: false,
        format: OutputFormat::Json,
    })
    .expect("run should complete with executor failure pause");
    let parsed: Value = serde_json::from_str(output.as_str()).expect("must be valid json");
    assert_eq!(parsed.get("status").and_then(Value::as_str), Some("paused"));
    assert_eq!(
        parsed.get("paused_reason").and_then(Value::as_str),
        Some("executor_error:swap-1")
    );
    let completed = parsed
        .get("completed_node_ids")
        .and_then(Value::as_array)
        .expect("completed_node_ids array");
    assert!(completed.is_empty());
}

#[test]
fn checkpoint_side_effect_sent_prevents_tx_replay_after_restart() {
    let plan: PlanDocument = serde_json::from_value(json!({
      "schema":"ais-plan/0.0.3",
      "meta": {},
      "nodes":[
        {
          "id":"swap-1",
          "chain":"eip155:1",
          "kind":"execution",
          "execution":{
            "type":"evm_call",
            "to":{"lit":"0x0000000000000000000000000000000000000001"},
            "abi":{"name":"swapExactTokensForTokens","inputs":[],"outputs":[]},
            "args":{}
          }
        }
      ]
    }))
    .expect("plan");
    let plan_path = write_temp_file(
        "plan-checkpoint-side-effect-replay-guard",
        serde_json::to_string(&plan).expect("serialize").as_str(),
    );
    let config_path = write_temp_file(
        "runner-config-checkpoint-side-effect-replay-guard",
        r#"
schema: ais-runner/0.0.1
chains:
  eip155:1:
    rpc_url: https://rpc.evm.example
"#,
    );
    let checkpoint_path = write_temp_file("checkpoint-side-effect-replay-guard", "");
    let plan_hash = hash_plan(&plan).expect("hash");
    let mut checkpoint = create_checkpoint_document(
        "run-side-effect-replay-guard",
        plan_hash,
        CheckpointEngineState::default(),
        None,
        None,
        None,
    );
    checkpoint.side_effects.push(CheckpointSideEffectRecord {
        schema: Some("ais-side-effect-record/0.1.0".to_string()),
        idempotency_key: "tx:swap-1:0xsent_checkpoint".to_string(),
        node_id: "swap-1".to_string(),
        effect_type: "tx".to_string(),
        chain: Some("eip155:1".to_string()),
        execution_type: Some("evm_call".to_string()),
        tx_hash: Some("0xsent_checkpoint".to_string()),
        nonce: Some(12),
        provider_ref: None,
        reason_code: None,
        details: None,
        status: "sent".to_string(),
        observed_at: "2026-02-24T00:00:00Z".to_string(),
    });
    save_checkpoint_to_path(&checkpoint_path, &checkpoint).expect("save checkpoint");

    let output = execute_run_plan(&PlanCommand {
        plan: plan_path,
        config: Some(config_path),
        runtime: None,
        dry_run: false,
        events_jsonl: None,
        trace: None,
        checkpoint: Some(checkpoint_path),
        commands_stdin_jsonl: false,
        verbose: false,
        format: OutputFormat::Json,
    })
    .expect("checkpoint side-effect should prevent replay");
    let parsed: Value = serde_json::from_str(output.as_str()).expect("must be valid json");
    assert_eq!(parsed.get("status").and_then(Value::as_str), Some("paused"));
    assert!(parsed
        .get("paused_reason")
        .and_then(Value::as_str)
        .is_some_and(|reason| reason.starts_with("side_effect_reconcile_pending:")));
    assert_eq!(
        parsed
            .get("resumed_from_checkpoint")
            .and_then(Value::as_bool),
        Some(true)
    );
    let completed = parsed
        .get("completed_node_ids")
        .and_then(Value::as_array)
        .expect("completed_node_ids array");
    assert!(completed.is_empty());
}

#[test]
fn checkpoint_side_effect_reverted_does_not_mark_node_completed() {
    let plan_path = write_temp_file(
        "plan-checkpoint-reverted-side-effect",
        r#"{
  "schema":"ais-plan/0.0.3",
  "meta": {},
  "nodes":[
    {
      "id":"swap-1",
      "chain":"eip155:1",
      "kind":"execution",
      "execution":{
        "type":"evm_call",
        "to":{"lit":"0x0000000000000000000000000000000000000001"},
        "abi":{"name":"swapExactTokensForTokens","inputs":[],"outputs":[]},
        "args":{}
      }
    }
  ]
}"#,
    );
    let config_path = write_temp_file(
        "runner-config-checkpoint-reverted-side-effect",
        r#"
schema: ais-runner/0.0.1
chains:
  eip155:1:
    rpc_url: https://rpc.evm.example
"#,
    );
    let checkpoint_path = write_temp_file("checkpoint-reverted-side-effect", "");
    let plan: PlanDocument =
        serde_json::from_str(fs::read_to_string(&plan_path).expect("read plan").as_str())
            .expect("plan");
    let plan_hash = hash_plan(&plan).expect("hash");
    let mut checkpoint = create_checkpoint_document(
        "run-side-effect-reverted",
        plan_hash,
        CheckpointEngineState::default(),
        None,
        None,
        None,
    );
    checkpoint.side_effects.push(CheckpointSideEffectRecord {
        schema: Some("ais-side-effect-record/0.1.0".to_string()),
        idempotency_key: "tx:swap-1:0xfailed".to_string(),
        node_id: "swap-1".to_string(),
        effect_type: "tx".to_string(),
        chain: Some("eip155:1".to_string()),
        execution_type: Some("evm_call".to_string()),
        tx_hash: Some("0xfailed".to_string()),
        nonce: Some(7),
        provider_ref: None,
        reason_code: None,
        details: None,
        status: "reverted".to_string(),
        observed_at: "2026-02-24T00:00:00Z".to_string(),
    });
    save_checkpoint_to_path(&checkpoint_path, &checkpoint).expect("save checkpoint");

    let output = execute_run_plan(&PlanCommand {
        plan: plan_path,
        config: Some(config_path),
        runtime: None,
        dry_run: false,
        events_jsonl: None,
        trace: None,
        checkpoint: Some(checkpoint_path),
        commands_stdin_jsonl: false,
        verbose: false,
        format: OutputFormat::Json,
    })
    .expect("run must finish");
    let parsed: Value = serde_json::from_str(output.as_str()).expect("must be valid json");
    assert_eq!(parsed.get("status").and_then(Value::as_str), Some("paused"));
    assert_eq!(
        parsed.get("paused_reason").and_then(Value::as_str),
        Some("executor_error:swap-1")
    );
    assert!(parsed
        .get("completed_node_ids")
        .and_then(Value::as_array)
        .is_some_and(|items| items.is_empty()));
}

#[test]
fn checkpoint_side_effect_cannot_bypass_unregistered_execution_type_guard() {
    let plan_path = write_temp_file(
        "plan-unregistered-with-checkpoint-side-effect",
        r#"{
  "schema":"ais-plan/0.0.3",
  "meta": {},
  "nodes":[
    {
      "id":"node-1",
      "chain":"eip155:1",
      "kind":"execution",
      "execution":{"type":"sui_tx","args":{}}
    }
  ]
}"#,
    );
    let config_path = write_temp_file(
        "runner-config-unregistered-with-checkpoint-side-effect",
        r#"
schema: ais-runner/0.0.1
chains:
  eip155:1:
    rpc_url: https://rpc.evm.example
"#,
    );
    let checkpoint_path = write_temp_file("checkpoint-unregistered-with-side-effect", "");
    let plan: PlanDocument =
        serde_json::from_str(fs::read_to_string(&plan_path).expect("read plan").as_str())
            .expect("plan");
    let plan_hash = hash_plan(&plan).expect("hash");
    let mut checkpoint = create_checkpoint_document(
        "run-unregistered-with-side-effect",
        plan_hash,
        CheckpointEngineState::default(),
        None,
        None,
        None,
    );
    checkpoint.side_effects.push(CheckpointSideEffectRecord {
        schema: Some("ais-side-effect-record/0.1.0".to_string()),
        idempotency_key: "tx:node-1:0xsent".to_string(),
        node_id: "node-1".to_string(),
        effect_type: "tx".to_string(),
        chain: Some("eip155:1".to_string()),
        execution_type: Some("sui_tx".to_string()),
        tx_hash: Some("0xsent".to_string()),
        nonce: None,
        provider_ref: None,
        reason_code: None,
        details: None,
        status: "sent".to_string(),
        observed_at: "2026-02-24T00:00:00Z".to_string(),
    });
    save_checkpoint_to_path(&checkpoint_path, &checkpoint).expect("save checkpoint");

    let error = execute_run_plan(&PlanCommand {
        plan: plan_path,
        config: Some(config_path),
        runtime: None,
        dry_run: false,
        events_jsonl: None,
        trace: None,
        checkpoint: Some(checkpoint_path),
        commands_stdin_jsonl: false,
        verbose: false,
        format: OutputFormat::Json,
    })
    .expect_err("must still reject unregistered execution type");
    match error {
        RunnerError::ConfigInvalidForPlan(issues) => {
            assert_eq!(issues.len(), 1);
            assert_eq!(
                issues[0].reference.as_deref(),
                Some("runner.config.execution_type_unregistered")
            );
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn read_command_jsonl_parses_supported_command_types() {
    let input = r#"{"schema":"ais-engine-command/0.0.1","command":{"id":"cmd-patch","type":"apply_patches","data":{"patches":[]}}}
{"schema":"ais-engine-command/0.0.1","command":{"id":"cmd-confirm","type":"user_confirm","data":{"node_id":"n1","decision":"approve"}}}
{"schema":"ais-engine-command/0.0.1","command":{"id":"cmd-input","type":"user_input","data":{"input_id":"owner","value":"0xabc"}}}
{"schema":"ais-engine-command/0.0.1","command":{"id":"cmd-select","type":"user_select","data":{"input_id":"token","selected_index":1,"options":[{"label":"USDC","value":"0x1"}]}}}
{"schema":"ais-engine-command/0.0.1","command":{"id":"cmd-cancel","type":"cancel","data":{}}}
{"schema":"ais-engine-command/0.0.1","command":{"id":"cmd-replace","type":"replace_plan","data":{"plan":{"schema":"ais-plan/0.0.3","nodes":[]}}}}
"#;
    let commands = read_command_jsonl(Cursor::new(input)).expect("must parse");
    assert_eq!(commands.len(), 6);
    assert_eq!(commands[0].command.id, "cmd-patch");
    assert_eq!(commands[1].command.id, "cmd-confirm");
    assert_eq!(
        commands[2].command.command_type,
        EngineCommandType::UserInput
    );
    assert_eq!(
        commands[3].command.command_type,
        EngineCommandType::UserSelect
    );
    assert_eq!(commands[4].command.id, "cmd-cancel");
    assert_eq!(
        commands[5].command.command_type,
        EngineCommandType::ReplacePlan
    );
}

#[test]
fn process_replace_plan_command_updates_active_plan_and_epoch() {
    let config: RunnerConfig = serde_yaml::from_str(
        r#"
schema: ais-runner/0.0.1
chains:
  eip155:1:
    rpc_url: https://rpc.evm.example
"#,
    )
    .expect("config must parse");
    let mut active_plan = sample_plan("balanceOf", false);
    let before_hash = hash_plan(&active_plan).expect("hash before");
    let command = format!(
        r#"{{"schema":"ais-engine-command/0.0.1","command":{{"id":"cmd-replace-ok","type":"replace_plan","data":{{"plan":{}}}}}}}
"#,
        sample_plan_json("balanceOf", true)
    );
    let commands = read_command_jsonl(Cursor::new(command)).expect("commands parse");
    let mut state = EngineRunnerState {
        runtime: json!({}),
        plan_hash_history: vec![before_hash.clone()],
        ..EngineRunnerState::default()
    };
    let mut active_hash = before_hash.clone();
    let processed = process_replace_plan_commands(
        "run-test",
        &config,
        commands.as_slice(),
        &EngineRunnerOptions::default(),
        &mut state,
        &mut active_plan,
        &mut active_hash,
    )
    .expect("replace plan succeeds");

    assert!(processed.plan_replaced);
    assert!(!processed.pause_after_processing);
    assert_eq!(processed.forward_commands.len(), 0);
    assert_eq!(active_plan.nodes.len(), 2);
    assert_eq!(state.plan_epoch, 1);
    assert_eq!(state.plan_hash_history.len(), 2);
    assert_ne!(active_hash, before_hash);
    assert!(processed
        .events
        .iter()
        .any(|record| record.event.event_type == EngineEventType::PlanReplaced));
    let command_accepted = processed
        .events
        .iter()
        .find(|record| record.event.event_type == EngineEventType::CommandAccepted)
        .expect("command_accepted event");
    assert_timestamp_is_wall_clock_rfc3339(command_accepted.ts.as_str());
    let plan_replaced = processed
        .events
        .iter()
        .find(|record| record.event.event_type == EngineEventType::PlanReplaced)
        .expect("plan_replaced event");
    assert_timestamp_is_wall_clock_rfc3339(plan_replaced.ts.as_str());
}

#[test]
fn process_replace_plan_rejects_mutating_completed_node() {
    let config: RunnerConfig = serde_yaml::from_str(
        r#"
schema: ais-runner/0.0.1
chains:
  eip155:1:
    rpc_url: https://rpc.evm.example
"#,
    )
    .expect("config must parse");
    let mut active_plan = sample_plan("balanceOf", false);
    let original_plan = active_plan.clone();
    let before_hash = hash_plan(&active_plan).expect("hash before");
    let command = format!(
        r#"{{"schema":"ais-engine-command/0.0.1","command":{{"id":"cmd-replace-bad","type":"replace_plan","data":{{"plan":{}}}}}}}
"#,
        sample_plan_json("allowance", false)
    );
    let commands = read_command_jsonl(Cursor::new(command)).expect("commands parse");
    let mut state = EngineRunnerState {
        runtime: json!({}),
        completed_node_ids: vec!["node-1".to_string()],
        plan_hash_history: vec![before_hash.clone()],
        ..EngineRunnerState::default()
    };
    let mut active_hash = before_hash.clone();
    let processed = process_replace_plan_commands(
        "run-test",
        &config,
        commands.as_slice(),
        &EngineRunnerOptions::default(),
        &mut state,
        &mut active_plan,
        &mut active_hash,
    )
    .expect("processing must finish");

    assert!(!processed.plan_replaced);
    assert!(processed.pause_after_processing);
    assert_eq!(
        state.paused_reason.as_deref(),
        Some("replace_plan_rejected:replace_plan_completed_node_mutated")
    );
    assert_eq!(active_plan, original_plan);
    assert_eq!(active_hash, before_hash);
    assert_eq!(state.plan_epoch, 0);
    assert!(processed
        .events
        .iter()
        .any(|record| record.event.event_type == EngineEventType::Error));
    assert!(processed
        .events
        .iter()
        .any(|record| record.event.event_type == EngineEventType::EnginePaused));
    let command_accepted = processed
        .events
        .iter()
        .find(|record| record.event.event_type == EngineEventType::CommandAccepted)
        .expect("command_accepted event");
    assert_timestamp_is_wall_clock_rfc3339(command_accepted.ts.as_str());
    let replace_error = processed
        .events
        .iter()
        .find(|record| record.event.event_type == EngineEventType::Error)
        .expect("replace_plan error event");
    assert_timestamp_is_wall_clock_rfc3339(replace_error.ts.as_str());
}

fn sample_plan(method: &str, include_second_node: bool) -> PlanDocument {
    let mut nodes = vec![json!({
      "id":"node-1",
      "chain":"eip155:1",
      "kind":"execution",
      "execution":{
        "type":"evm_read",
        "to":{"lit":"0x0000000000000000000000000000000000000001"},
        "abi":{"name":method,"inputs":[],"outputs":[]},
        "args":{}
      }
    })];
    if include_second_node {
        nodes.push(json!({
          "id":"node-2",
          "chain":"eip155:1",
          "kind":"execution",
          "deps":["node-1"],
          "execution":{
            "type":"evm_read",
            "to":{"lit":"0x0000000000000000000000000000000000000001"},
            "abi":{"name":"totalSupply","inputs":[],"outputs":[]},
            "args":{}
          }
        }));
    }
    PlanDocument {
        schema: "ais-plan/0.0.3".to_string(),
        meta: Some(json!({})),
        nodes,
        extensions: serde_json::Map::new(),
    }
}

fn sample_plan_json(method: &str, include_second_node: bool) -> String {
    serde_json::to_string(&sample_plan(method, include_second_node)).expect("plan json")
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

fn temp_dir(prefix: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time must be monotonic")
        .as_nanos();
    path.push(format!(
        "ais-runner-{prefix}-{}-{nanos}",
        std::process::id()
    ));
    fs::create_dir_all(&path).expect("must create temp dir");
    path
}

fn write_temp_file_in(dir: &std::path::Path, name: &str, content: &str) -> PathBuf {
    let path = dir.join(name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("must create temp parent");
    }
    fs::write(&path, content).expect("must write file");
    path
}

fn example_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../../examples")
}

fn read_example_fixture(name: &str) -> String {
    fs::read_to_string(example_root().join(name)).expect("must read example fixture")
}

fn safe_defi_pack_registry_only_fixture() -> String {
    r#"
schema: ais-pack/0.0.2
name: safe-defi-pack
version: 0.0.2
meta:
  name: safe-defi-pack
  version: 0.0.2
includes:
  - protocol: uniswap-v3
    version: 0.0.2
    source: registry
    chain_scope: ["eip155:8453"]
"#
    .to_string()
}

fn assert_timestamp_is_wall_clock_rfc3339(ts: &str) {
    assert_ne!(ts, "1970-01-01T00:00:00Z");
    assert_eq!(ts.len(), 20);
    assert_eq!(&ts[4..5], "-");
    assert_eq!(&ts[7..8], "-");
    assert_eq!(&ts[10..11], "T");
    assert_eq!(&ts[13..14], ":");
    assert_eq!(&ts[16..17], ":");
    assert_eq!(&ts[19..20], "Z");
    assert!(ts[..4].chars().all(|ch| ch.is_ascii_digit()));
    assert!(ts[5..7].chars().all(|ch| ch.is_ascii_digit()));
    assert!(ts[8..10].chars().all(|ch| ch.is_ascii_digit()));
    assert!(ts[11..13].chars().all(|ch| ch.is_ascii_digit()));
    assert!(ts[14..16].chars().all(|ch| ch.is_ascii_digit()));
    assert!(ts[17..19].chars().all(|ch| ch.is_ascii_digit()));
}
