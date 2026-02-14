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
    write(root.join("broken.yaml"), "schema: ais-flow/0.0.3\nmeta: [\n");

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

fn write(path: impl AsRef<Path>, content: &str) {
    fs::write(path, content).expect("must write fixture");
}
