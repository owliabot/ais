use crate::cli::{OutputFormat, PlanCommand};
use crate::{
    execute_plan_diff, execute_replay, execute_run_plan, execute_run_workflow, PlanDiffCommand,
    ReplayCommand, WorkflowCommand,
};
use ais_engine::{encode_event_jsonl_line, EngineEvent, EngineEventRecord, EngineEventType};
use super::read_command_jsonl;
use serde_json::Value;
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
    assert_eq!(parsed.get("events_emitted").and_then(Value::as_u64), Some(2));
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
    assert_eq!(parsed.get("command_accepted").and_then(Value::as_u64), Some(0));
    assert_eq!(parsed.get("command_rejected").and_then(Value::as_u64), Some(0));

    let events_content = fs::read_to_string(events_path).expect("events must exist");
    assert!(events_content.contains("\"type\":\"engine_paused\""));
    let trace_content = fs::read_to_string(trace_path).expect("trace must exist");
    assert!(trace_content.contains("\"schema\":\"ais-engine-event/0.0.3\""));
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
fn read_command_jsonl_parses_supported_command_types() {
    let input = r#"{"schema":"ais-engine-command/0.0.1","command":{"id":"cmd-patch","type":"apply_patches","data":{"patches":[]}}}
{"schema":"ais-engine-command/0.0.1","command":{"id":"cmd-confirm","type":"user_confirm","data":{"node_id":"n1","decision":"approve"}}}
{"schema":"ais-engine-command/0.0.1","command":{"id":"cmd-cancel","type":"cancel","data":{}}}
"#;
    let commands = read_command_jsonl(Cursor::new(input)).expect("must parse");
    assert_eq!(commands.len(), 3);
    assert_eq!(commands[0].command.id, "cmd-patch");
    assert_eq!(commands[1].command.id, "cmd-confirm");
    assert_eq!(commands[2].command.id, "cmd-cancel");
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
    fs::write(&path, content).expect("must write file");
    path
}
