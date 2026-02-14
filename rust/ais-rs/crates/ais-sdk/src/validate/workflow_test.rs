use super::{validate_workflow_document, validate_workflow_imports};
use crate::documents::WorkflowDocument;
use crate::parse::{parse_document_with_options, AisDocument, DocumentFormat, ParseDocumentOptions};
use serde_json::{json, Map};
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

#[test]
fn reports_duplicate_unknown_and_self_deps() {
    let workflow = workflow_doc(
        vec![
            json!({
                "id": "a",
                "type": "action_ref",
                "protocol": "p@0.0.1",
                "action": "x",
                "deps": ["a", "missing"]
            }),
            json!({
                "id": "a",
                "type": "query_ref",
                "protocol": "p@0.0.1",
                "query": "y"
            }),
        ],
        Map::new(),
    );

    let issues = validate_workflow_document(&workflow);
    assert!(has_issue(&issues, "workflow.node.duplicate_id", "$.nodes[1].id"));
    assert!(has_issue(&issues, "workflow.deps.self", "$.nodes[0].deps[0]"));
    assert!(has_issue(&issues, "workflow.deps.unknown", "$.nodes[0].deps[1]"));
}

#[test]
fn reports_dependency_cycle() {
    let workflow = workflow_doc(
        vec![
            json!({
                "id": "a",
                "type": "action_ref",
                "protocol": "p@0.0.1",
                "action": "x",
                "deps": ["b"]
            }),
            json!({
                "id": "b",
                "type": "query_ref",
                "protocol": "p@0.0.1",
                "query": "q",
                "deps": ["a"]
            }),
        ],
        Map::new(),
    );

    let issues = validate_workflow_document(&workflow);
    assert!(issues
        .iter()
        .any(|issue| issue.reference.as_deref() == Some("workflow.deps.cycle")));
}

#[test]
fn validates_value_refs_and_cel_bindings() {
    let mut inputs = Map::new();
    inputs.insert("amount".to_string(), json!({ "type": "string" }));

    let workflow = workflow_doc(
        vec![
            json!({
                "id": "step1",
                "type": "query_ref",
                "protocol": "p@0.0.1",
                "query": "q1",
                "args": {
                    "ok": { "ref": "inputs.amount" },
                    "bad_root": { "ref": "unknown_root.x" },
                    "bad_input": { "ref": "inputs.missing" },
                    "bad_node": { "ref": "nodes.not_exists.outputs.v" },
                    "self_ref": { "ref": "nodes.step1.outputs.v" }
                },
                "condition": { "cel": "inputs.missing2 != '' || nodes.not_exists.outputs.ok" },
                "until": { "ref": "nodes.step1.outputs.done" }
            }),
        ],
        inputs,
    );

    let issues = validate_workflow_document(&workflow);
    assert!(has_reference(&issues, "workflow.ref.invalid_root"));
    assert!(has_reference(&issues, "workflow.ref.input_missing"));
    assert!(has_reference(&issues, "workflow.ref.node_missing"));
    assert!(has_reference(&issues, "workflow.ref.self_node"));
    assert!(!has_issue(
        &issues,
        "workflow.ref.self_node",
        "$.nodes[0].until"
    ));
}

#[test]
fn outputs_refs_must_target_declared_inputs_or_nodes() {
    let mut outputs = Map::new();
    outputs.insert("result".to_string(), json!({ "ref": "nodes.step2.outputs.out" }));
    outputs.insert("in2".to_string(), json!({ "ref": "inputs.unknown" }));
    outputs.insert(
        "cel".to_string(),
        json!({ "cel": "nodes.step2.outputs.ok && inputs.unknown2 != ''" }),
    );

    let workflow = WorkflowDocument {
        schema: "ais-flow/0.0.3".to_string(),
        meta: json!({ "name": "wf", "version": "0.0.1" }),
        default_chain: None,
        imports: None,
        requires_pack: None,
        inputs: Map::new(),
        nodes: vec![json!({
            "id": "step1",
            "type": "query_ref",
            "protocol": "p@0.0.1",
            "query": "q1"
        })],
        policy: None,
        preflight: None,
        outputs,
        extensions: Map::new(),
    };

    let issues = validate_workflow_document(&workflow);
    assert!(has_reference(&issues, "workflow.ref.node_missing"));
    assert!(has_reference(&issues, "workflow.ref.input_missing"));
}

#[test]
fn imports_protocols_fixture_cases() {
    let valid = load_workflow_fixture("imports/valid.json", true);
    let valid_issues = validate_workflow_document(&valid);
    assert!(!has_reference(
        &valid_issues,
        "workflow.imports.node_protocol_not_imported"
    ));

    let invalid_path = load_workflow_fixture("imports/invalid-missing-path.json", false);
    let invalid_path_issues = validate_workflow_document(&invalid_path);
    assert!(has_reference(
        &invalid_path_issues,
        "workflow.imports.path_required"
    ));

    let invalid_format = load_workflow_fixture("imports/invalid-bad-protocol-format.json", false);
    let invalid_format_issues = validate_workflow_document(&invalid_format);
    assert!(has_reference(
        &invalid_format_issues,
        "workflow.imports.protocol_format"
    ));

    let node_not_imported = load_workflow_fixture("imports/invalid-node-not-imported.json", true);
    let node_not_imported_issues = validate_workflow_document(&node_not_imported);
    assert!(has_reference(
        &node_not_imported_issues,
        "workflow.imports.node_protocol_not_imported"
    ));
}

#[test]
fn imports_can_validate_against_workspace_protocols() {
    let workflow = load_workflow_fixture("imports/valid.json", true);
    let known_protocols = HashSet::from([String::from("erc20@0.0.2")]);
    let issues = validate_workflow_imports(&workflow, Some(&known_protocols));
    assert!(has_reference(
        &issues,
        "workflow.imports.protocol_missing_in_workspace"
    ));
}

#[test]
fn assert_semantics_are_validated() {
    let workflow = workflow_doc(
        vec![json!({
            "id":"s1",
            "type":"query_ref",
            "protocol":"p@0.0.1",
            "query":"q",
            "assert":{"cel":"size(nodes.s1.outputs"},
            "assert_message":""
        })],
        Map::new(),
    );
    let issues = validate_workflow_document(&workflow);
    assert!(has_reference(&issues, "workflow.assert.cel_invalid"));
    assert!(has_reference(&issues, "workflow.assert.message_invalid"));
}

#[test]
fn assert_message_without_assert_is_invalid() {
    let workflow = workflow_doc(
        vec![json!({
            "id":"s1",
            "type":"query_ref",
            "protocol":"p@0.0.1",
            "query":"q",
            "assert_message":"must have assert"
        })],
        Map::new(),
    );
    let issues = validate_workflow_document(&workflow);
    assert!(has_reference(
        &issues,
        "workflow.assert.message_without_assert"
    ));
}

fn workflow_doc(nodes: Vec<serde_json::Value>, inputs: Map<String, serde_json::Value>) -> WorkflowDocument {
    WorkflowDocument {
        schema: "ais-flow/0.0.3".to_string(),
        meta: json!({ "name": "wf", "version": "0.0.1" }),
        default_chain: None,
        imports: None,
        requires_pack: None,
        inputs,
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

fn has_reference(issues: &[ais_core::StructuredIssue], reference: &str) -> bool {
    issues
        .iter()
        .any(|issue| issue.reference.as_deref() == Some(reference))
}

fn load_workflow_fixture(relative: &str, validate_schema: bool) -> WorkflowDocument {
    let path = fixture_root().join(relative);
    let content = fs::read_to_string(&path).expect("must read fixture");
    let parsed = parse_document_with_options(
        content.as_str(),
        ParseDocumentOptions {
            format: DocumentFormat::Auto,
            validate_schema,
        },
    )
    .unwrap_or_else(|issues| panic!("fixture parse failed for {}: {issues:?}", path.display()));
    let AisDocument::Workflow(workflow) = parsed else {
        panic!("fixture {} must be workflow", path.display());
    };
    workflow
}

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/workflow-0.0.3")
}
