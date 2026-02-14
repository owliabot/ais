use super::validate_document_semantics;
use crate::documents::{PackDocument, PlanDocument, ProtocolDocument, WorkflowDocument};
use crate::parse::AisDocument;
use serde_json::{json, Map};

#[test]
fn protocol_semantic_checks_report_paths() {
    let mut actions = Map::new();
    actions.insert(
        "swap".to_string(),
        json!({
            "capabilities_required": [""],
            "execution": {
                "eip155:1": {
                    "type": "BAD-TYPE"
                }
            }
        }),
    );

    let document = AisDocument::Protocol(ProtocolDocument {
        schema: "ais/0.0.1".to_string(),
        meta: json!(null),
        deployments: Vec::new(),
        actions,
        queries: Map::new(),
        risks: Vec::new(),
        supported_assets: Vec::new(),
        capabilities_required: vec![" ".to_string()],
        tests: Vec::new(),
        extensions: Map::new(),
    });

    let issues = validate_document_semantics(&document);
    assert!(has_issue(&issues, "document.schema_mismatch", "$.schema"));
    assert!(has_issue(&issues, "protocol.meta.object", "$.meta"));
    assert!(has_issue(&issues, "protocol.deployments.non_empty", "$.deployments"));
    assert!(has_issue(
        &issues,
        "capabilities.non_empty",
        "$.capabilities_required[0]"
    ));
    assert!(has_issue(
        &issues,
        "protocol.capabilities.non_empty",
        "$.actions.swap.capabilities_required[0]"
    ));
    assert!(has_issue(
        &issues,
        "protocol.execution.type_format",
        "$.actions.swap.execution.eip155:1.type"
    ));
}

#[test]
fn workflow_semantic_checks_refs_and_deps() {
    let document = AisDocument::Workflow(WorkflowDocument {
        schema: "ais-flow/0.0.3".to_string(),
        meta: json!({}),
        default_chain: None,
        imports: None,
        requires_pack: None,
        inputs: Map::new(),
        nodes: vec![json!({
            "id": "n1",
            "action_ref": "bad-ref",
            "deps": ["ok", ""]
        })],
        policy: None,
        preflight: None,
        outputs: Map::new(),
        extensions: Map::new(),
    });

    let issues = validate_document_semantics(&document);
    assert!(has_issue(
        &issues,
        "workflow.nodes.action_ref",
        "$.nodes[0].action_ref"
    ));
    assert!(has_issue(&issues, "workflow.nodes.deps", "$.nodes[0].deps[1]"));
}

#[test]
fn pack_semantic_checks_include_format() {
    let document = AisDocument::Pack(PackDocument {
        schema: "ais-pack/0.0.2".to_string(),
        name: None,
        version: None,
        description: None,
        meta: None,
        includes: vec![json!({
            "protocol": "Bad Protocol",
            "version": "1.0"
        })],
        policy: None,
        token_policy: None,
        providers: None,
        plugins: None,
        overrides: None,
        extensions: Map::new(),
    });

    let issues = validate_document_semantics(&document);
    assert!(has_issue(
        &issues,
        "pack.includes.protocol",
        "$.includes[0].protocol"
    ));
    assert!(has_issue(
        &issues,
        "pack.includes.version",
        "$.includes[0].version"
    ));
}

#[test]
fn plan_semantic_checks_non_empty_nodes() {
    let document = AisDocument::Plan(PlanDocument {
        schema: "ais-plan/0.0.3".to_string(),
        meta: None,
        nodes: Vec::new(),
        extensions: Map::new(),
    });

    let issues = validate_document_semantics(&document);
    assert!(has_issue(&issues, "plan.nodes.non_empty", "$.nodes"));
}

fn has_issue(issues: &[ais_core::StructuredIssue], reference: &str, field_path: &str) -> bool {
    issues.iter().any(|issue| {
        issue.reference.as_deref() == Some(reference) && issue.field_path.to_string() == field_path
    })
}
