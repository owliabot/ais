use super::{load_workspace_documents, load_workspace_documents_excluding};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn load_workspace_documents_classifies_protocol_pack_workflow_plan() {
    let root = temp_dir("workspace-classify");
    write(
        root.join("protocol.json"),
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
    write(
        root.join("pack.yaml"),
        r#"
schema: ais-pack/0.0.2
name: safe-defi
version: 0.0.2
includes:
  - protocol: uniswap-v3
    version: 0.0.2
"#,
    );
    write(
        root.join("workflow.yaml"),
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
    write(
        root.join("plan.json"),
        r#"{
  "schema":"ais-plan/0.0.3",
  "meta":{},
  "nodes":[
    {"id":"swap","chain":"eip155:1","kind":"execution","execution":{"type":"custom"}}
  ]
}"#,
    );

    let loaded = load_workspace_documents(root.as_path()).expect("must load");
    assert_eq!(loaded.protocols.len(), 1);
    assert_eq!(loaded.packs.len(), 1);
    assert_eq!(loaded.workflows.len(), 1);
    assert_eq!(loaded.plans.len(), 1);
}

#[test]
fn load_workspace_documents_reports_parse_issues_with_file_context() {
    let root = temp_dir("workspace-issues");
    write(
        root.join("broken.yaml"),
        "schema: ais-flow/0.0.3\nmeta: [\n",
    );

    let issues = load_workspace_documents(root.as_path()).expect_err("must fail");
    assert!(!issues.is_empty());
    assert!(issues.iter().all(|issue| issue.related.is_some()));
    assert!(issues.iter().any(|issue| {
        issue
            .related
            .as_ref()
            .and_then(|related| related.get("file"))
            .is_some()
    }));
}

#[test]
fn load_workspace_documents_excluding_skips_target_file() {
    let root = temp_dir("workspace-exclude");
    let workflow_path = root.join("workflow.yaml");
    write(
        workflow_path.clone(),
        r#"
schema: ais-flow/0.0.3
meta:
  name: wf
  version: 0.0.1
nodes: []
"#,
    );

    let loaded = load_workspace_documents_excluding(root.as_path(), &[workflow_path])
        .expect("must load while skipping excluded file");
    assert_eq!(loaded.workflows.len(), 0);
}

#[test]
fn load_workspace_documents_ignores_runtime_and_runner_config_sidecars() {
    let root = temp_dir("workspace-sidecars");
    write(
        root.join("protocol.yaml"),
        r#"
schema: ais/0.0.2
meta:
  protocol: demo
  version: 0.0.2
deployments:
  - chain: eip155:1
    contracts: {}
actions: {}
queries: {}
"#,
    );
    write(
        root.join("runtime.yaml"),
        r#"
ctx:
  wallet_address: "0x1111111111111111111111111111111111111111"
"#,
    );
    write(
        root.join("runner.local.yaml"),
        r#"
schema: ais-runner/0.0.1
chains:
  eip155:1:
    rpc_url: https://rpc.evm.example
"#,
    );

    let loaded = load_workspace_documents(root.as_path()).expect("must load");
    assert_eq!(loaded.protocols.len(), 1);
    assert!(loaded.packs.is_empty());
    assert!(loaded.workflows.is_empty());
    assert!(loaded.plans.is_empty());
}

#[test]
fn load_workspace_documents_resolves_registry_protocol_snapshots_for_registry_includes() {
    let root = temp_dir("workspace-registry-include");
    write(
        root.join("pack.yaml"),
        r#"
schema: ais-pack/0.0.2
name: safe-defi
version: 0.0.2
includes:
  - protocol: uniswap-v3
    version: 0.0.2
    source: registry
    chain_scope: ["eip155:8453"]
"#,
    );
    write(
        root.join(".ais/registry/protocols/uniswap-v3/0.0.2.yaml"),
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

    let loaded = load_workspace_documents(root.as_path()).expect("must load");
    assert_eq!(loaded.packs.len(), 1);
    assert_eq!(loaded.protocols.len(), 1);
    assert_eq!(
        loaded.protocols[0]
            .meta
            .get("protocol")
            .and_then(|value| value.as_str()),
        Some("uniswap-v3")
    );
}

#[test]
fn load_workspace_documents_reports_missing_registry_protocol_snapshot() {
    let root = temp_dir("workspace-registry-missing");
    write(
        root.join("pack.yaml"),
        r#"
schema: ais-pack/0.0.2
name: safe-defi
version: 0.0.2
includes:
  - protocol: uniswap-v3
    version: 0.0.2
    source: registry
"#,
    );

    let issues = load_workspace_documents(root.as_path()).expect_err("must fail");
    assert!(issues.iter().any(|issue| {
        issue.reference.as_deref() == Some("runner.workspace.registry_protocol_missing")
            && issue.message.contains("uniswap-v3@0.0.2")
    }));
}

#[test]
fn load_workspace_documents_reports_invalid_deployment_contracts_with_file_context() {
    let root = temp_dir("workspace-invalid-deployment-contracts");
    write(
        root.join("protocol.yaml"),
        r#"
schema: ais/0.0.2
meta:
  protocol: demo
  version: 0.0.2
deployments:
  - chain: eip155:1
    contracts: "0xrouter"
actions: {}
queries: {}
"#,
    );

    let issues = load_workspace_documents(root.as_path()).expect_err("must fail");
    assert!(issues.iter().any(|issue| {
        issue
            .related
            .as_ref()
            .and_then(|related| related.get("file"))
            .and_then(|value| value.as_str())
            .is_some_and(|file| file.ends_with("protocol.yaml"))
    }));
}

#[test]
fn load_workspace_documents_ingests_raw_example_protocols_and_pack() {
    let root = temp_dir("workspace-raw-examples");
    write(
        root.join("aave-v3.ais.yaml"),
        read_example_fixture("aave-v3.ais.yaml").as_str(),
    );
    write(
        root.join("safe-defi-pack.ais-pack.yaml"),
        read_example_fixture("safe-defi-pack.ais-pack.yaml").as_str(),
    );
    write(
        root.join(".ais/registry/protocols/uniswap-v3/0.0.2.yaml"),
        read_example_fixture("uniswap-v3.ais.yaml").as_str(),
    );

    let loaded = load_workspace_documents(root.as_path()).expect("must ingest raw examples");
    assert_eq!(loaded.packs.len(), 1);
    assert_eq!(loaded.protocols.len(), 2);
    assert!(loaded.protocols.iter().any(|protocol| {
        protocol
            .meta
            .get("protocol")
            .and_then(|value| value.as_str())
            == Some("aave-v3")
    }));
    assert!(loaded.protocols.iter().any(|protocol| {
        protocol
            .meta
            .get("protocol")
            .and_then(|value| value.as_str())
            == Some("uniswap-v3")
    }));
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

fn example_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../../examples")
}

fn read_example_fixture(name: &str) -> String {
    fs::read_to_string(example_root().join(name)).expect("must read example fixture")
}

fn write(path: impl AsRef<Path>, content: &str) {
    if let Some(parent) = path.as_ref().parent() {
        fs::create_dir_all(parent).expect("must create parent dirs");
    }
    fs::write(path, content).expect("must write fixture");
}
