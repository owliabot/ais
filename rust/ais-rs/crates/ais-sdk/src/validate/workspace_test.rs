use super::{validate_workspace_references, WorkspaceDocuments};
use crate::documents::{PackDocument, ProtocolDocument, WorkflowDocument};
use crate::parse::{parse_document_with_options, AisDocument, DocumentFormat, ParseDocumentOptions};
use serde_json::{json, Map};
use std::fs;
use std::path::PathBuf;

#[test]
fn workflow_requires_missing_pack_is_reported() {
    let workflow = workflow_doc(
        json!({
            "name": "wf",
            "version": "0.0.1"
        }),
        Some(json!({
            "name": "safe-defi",
            "version": "0.0.2"
        })),
        vec![],
        None,
    );

    let issues = validate_workspace_references(WorkspaceDocuments {
        protocols: &[],
        packs: &[],
        workflows: &[workflow],
    });

    assert!(has_issue(
        &issues,
        "workspace.workflow.requires_pack_missing",
        "$.requires_pack"
    ));
}

#[test]
fn pack_include_missing_protocol_is_reported() {
    let pack = pack_doc(
        Some("safe-defi"),
        Some("0.0.2"),
        vec![json!({
            "protocol": "uniswap-v3",
            "version": "0.0.2"
        })],
    );

    let issues = validate_workspace_references(WorkspaceDocuments {
        protocols: &[],
        packs: &[pack],
        workflows: &[],
    });

    assert!(has_issue(
        &issues,
        "workspace.pack.include_missing_protocol",
        "$.includes[0]"
    ));
}

#[test]
fn requires_pack_enforces_includes_and_chain_scope() {
    let protocol = protocol_doc("uniswap-v3", "0.0.2", &["swap_exact_in"], &["quote"]);

    let pack = pack_doc(
        Some("safe-defi"),
        Some("0.0.2"),
        vec![json!({
            "protocol": "uniswap-v3",
            "version": "0.0.2",
            "chain_scope": ["eip155:1"]
        })],
    );

    let workflow = workflow_doc(
        json!({ "name": "wf", "version": "0.0.1" }),
        Some(json!({ "name": "safe-defi", "version": "0.0.2" })),
        vec![json!({
            "id": "swap",
            "type": "action_ref",
            "protocol": "uniswap-v3@0.0.2",
            "action": "swap_exact_in",
            "chain": "eip155:137"
        })],
        None,
    );

    let issues = validate_workspace_references(WorkspaceDocuments {
        protocols: &[protocol],
        packs: &[pack],
        workflows: &[workflow],
    });

    assert!(has_issue(
        &issues,
        "workspace.workflow.chain_scope_violation",
        "$.nodes[0].chain"
    ));
}

#[test]
fn action_and_query_must_exist_in_protocol() {
    let protocol = protocol_doc("aave-v3", "0.0.2", &["supply"], &["position"]);
    let workflow = workflow_doc(
        json!({ "name": "wf", "version": "0.0.1" }),
        None,
        vec![
            json!({
                "id": "a1",
                "type": "action_ref",
                "protocol": "aave-v3@0.0.2",
                "action": "borrow"
            }),
            json!({
                "id": "q1",
                "type": "query_ref",
                "protocol": "aave-v3@0.0.2",
                "query": "health_factor"
            }),
        ],
        None,
    );

    let issues = validate_workspace_references(WorkspaceDocuments {
        protocols: &[protocol],
        packs: &[],
        workflows: &[workflow],
    });

    assert!(has_issue(
        &issues,
        "workspace.workflow.action_missing",
        "$.nodes[0].action"
    ));
    assert!(has_issue(
        &issues,
        "workspace.workflow.query_missing",
        "$.nodes[1].query"
    ));
}

#[test]
fn valid_workspace_has_no_issues() {
    let protocol = protocol_doc("jupiter", "0.0.2", &["swap"], &["quote"]);
    let pack = pack_doc(
        Some("safe-sol"),
        Some("0.0.1"),
        vec![json!({
            "protocol": "jupiter",
            "version": "0.0.2",
            "chain_scope": ["solana:mainnet"]
        })],
    );
    let workflow = workflow_doc(
        json!({ "name": "wf", "version": "0.0.1" }),
        Some(json!({ "name": "safe-sol", "version": "0.0.1" })),
        vec![json!({
            "id": "q",
            "type": "query_ref",
            "protocol": "jupiter@0.0.2",
            "query": "quote",
            "chain": "solana:mainnet"
        })],
        Some("solana:mainnet"),
    );

    let issues = validate_workspace_references(WorkspaceDocuments {
        protocols: &[protocol],
        packs: &[pack],
        workflows: &[workflow],
    });

    assert!(issues.is_empty());
}

#[test]
fn fixture_workspace_valid_evm_policy_has_no_issues() {
    let workspace = load_fixture_workspace("valid-evm-policy");
    let issues = validate_workspace_references(WorkspaceDocuments {
        protocols: &workspace.protocols,
        packs: &workspace.packs,
        workflows: &workspace.workflows,
    });
    assert!(issues.is_empty(), "unexpected issues: {issues:?}");
}

#[test]
fn fixture_workspace_invalid_chain_scope_reports_issue() {
    let workspace = load_fixture_workspace("invalid-chain-scope");
    let issues = validate_workspace_references(WorkspaceDocuments {
        protocols: &workspace.protocols,
        packs: &workspace.packs,
        workflows: &workspace.workflows,
    });
    assert!(has_issue(
        &issues,
        "workspace.workflow.chain_scope_violation",
        "$.nodes[0].chain"
    ));
}

fn protocol_doc(
    protocol: &str,
    version: &str,
    actions: &[&str],
    queries: &[&str],
) -> ProtocolDocument {
    let mut action_map = Map::new();
    for action in actions {
        action_map.insert((*action).to_string(), json!({}));
    }
    let mut query_map = Map::new();
    for query in queries {
        query_map.insert((*query).to_string(), json!({}));
    }

    ProtocolDocument {
        schema: "ais/0.0.2".to_string(),
        meta: json!({
            "protocol": protocol,
            "version": version
        }),
        deployments: vec![json!({
            "chain": "eip155:1",
            "contracts": {}
        })],
        actions: action_map,
        queries: query_map,
        risks: Vec::new(),
        supported_assets: Vec::new(),
        capabilities_required: Vec::new(),
        tests: Vec::new(),
        extensions: Map::new(),
    }
}

fn pack_doc(name: Option<&str>, version: Option<&str>, includes: Vec<serde_json::Value>) -> PackDocument {
    PackDocument {
        schema: "ais-pack/0.0.2".to_string(),
        name: name.map(str::to_string),
        version: version.map(str::to_string),
        description: None,
        meta: None,
        includes,
        policy: None,
        token_policy: None,
        providers: None,
        plugins: None,
        overrides: None,
        extensions: Map::new(),
    }
}

fn workflow_doc(
    meta: serde_json::Value,
    requires_pack: Option<serde_json::Value>,
    nodes: Vec<serde_json::Value>,
    default_chain: Option<&str>,
) -> WorkflowDocument {
    WorkflowDocument {
        schema: "ais-flow/0.0.3".to_string(),
        meta,
        default_chain: default_chain.map(str::to_string),
        imports: None,
        requires_pack,
        inputs: Map::new(),
        nodes,
        policy: None,
        preflight: None,
        outputs: Map::new(),
        extensions: Map::new(),
    }
}

fn has_issue(issues: &[ais_core::StructuredIssue], reference: &str, field_path: &str) -> bool {
    issues.iter().any(|issue| {
        issue.reference.as_deref() == Some(reference) && issue.field_path.to_string() == field_path
    })
}

struct ParsedWorkspace {
    protocols: Vec<ProtocolDocument>,
    packs: Vec<PackDocument>,
    workflows: Vec<WorkflowDocument>,
}

fn load_fixture_workspace(case: &str) -> ParsedWorkspace {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/workspace-minimal")
        .join(case);
    let mut protocols = Vec::<ProtocolDocument>::new();
    let mut packs = Vec::<PackDocument>::new();
    let mut workflows = Vec::<WorkflowDocument>::new();

    for entry in fs::read_dir(&root).expect("must read fixture directory") {
        let path = entry.expect("entry").path();
        let Some(ext) = path.extension().and_then(|ext| ext.to_str()) else {
            continue;
        };
        if !matches!(ext, "json" | "yaml" | "yml") {
            continue;
        }
        let content = fs::read_to_string(&path).expect("must read fixture file");
        let parsed = parse_document_with_options(
            content.as_str(),
            ParseDocumentOptions {
                format: DocumentFormat::Auto,
                validate_schema: true,
            },
        )
        .unwrap_or_else(|issues| panic!("fixture parse failed for {}: {issues:?}", path.display()));

        match parsed {
            AisDocument::Protocol(doc) => protocols.push(doc),
            AisDocument::Pack(doc) => packs.push(doc),
            AisDocument::Workflow(doc) => workflows.push(doc),
            _ => {}
        }
    }

    ParsedWorkspace {
        protocols,
        packs,
        workflows,
    }
}
