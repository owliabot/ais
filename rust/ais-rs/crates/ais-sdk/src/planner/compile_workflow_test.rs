use super::{compile_workflow, CompileWorkflowOptions, CompileWorkflowResult};
use crate::documents::{ProtocolDocument, WorkflowDocument};
use crate::parse::{
    parse_document_with_options, AisDocument, DocumentFormat, ParseDocumentOptions,
};
use crate::resolver::ResolverContext;
use serde_json::{json, Map, Value};
use std::fs;
use std::path::PathBuf;

#[test]
fn compiles_workflow_with_stable_topological_order() {
    let mut context = ResolverContext::new();
    context.register_protocol(demo_protocol());

    let workflow = WorkflowDocument {
        schema: "ais-flow/0.0.3".to_string(),
        meta: json!({ "name":"wf", "version":"0.0.1" }),
        default_chain: Some("eip155:1".to_string()),
        imports: None,
        requires_pack: None,
        inputs: Map::new(),
        nodes: vec![
            json!({
                "id":"swap",
                "type":"action_ref",
                "protocol":"demo@0.0.2",
                "action":"swap",
                "deps":["quote"],
                "args":{"min_out":{"ref":"nodes.quote.outputs.amount_out"}}
            }),
            json!({
                "id":"quote",
                "type":"query_ref",
                "protocol":"demo@0.0.2",
                "query":"quote"
            }),
        ],
        policy: None,
        preflight: None,
        outputs: Map::new(),
        extensions: Map::new(),
    };

    let result = compile_workflow(&workflow, &context, &CompileWorkflowOptions::default());
    match result {
        CompileWorkflowResult::Ok { plan } => {
            let ids = plan
                .nodes
                .iter()
                .filter_map(Value::as_object)
                .filter_map(|node| node.get("id"))
                .filter_map(Value::as_str)
                .collect::<Vec<_>>();
            assert_eq!(ids, vec!["quote", "swap"]);
        }
        CompileWorkflowResult::Err { issues } => {
            panic!("compile should succeed, issues: {issues:?}");
        }
    }
}

#[test]
fn compiles_control_type_nodes_into_executable_kinds() {
    let mut context = ResolverContext::new();
    context.register_protocol(demo_protocol());

    let workflow = WorkflowDocument {
        schema: "ais-flow/0.0.3".to_string(),
        meta: json!({ "name":"wf", "version":"0.0.1" }),
        default_chain: Some("eip155:1".to_string()),
        imports: None,
        requires_pack: None,
        inputs: Map::new(),
        nodes: vec![
            json!({
                "id":"c_assert",
                "type":"assert",
                "protocol":"demo@0.0.2",
                "query":"quote"
            }),
            json!({
                "id":"c_branch",
                "type":"branch",
                "protocol":"demo@0.0.2",
                "action":"swap"
            }),
        ],
        policy: None,
        preflight: None,
        outputs: Map::new(),
        extensions: Map::new(),
    };

    let result = compile_workflow(&workflow, &context, &CompileWorkflowOptions::default());
    let plan = match result {
        CompileWorkflowResult::Ok { plan } => plan,
        CompileWorkflowResult::Err { issues } => panic!("must compile: {issues:?}"),
    };

    let assert_node = plan
        .nodes
        .iter()
        .filter_map(Value::as_object)
        .find(|node| node.get("id").and_then(Value::as_str) == Some("c_assert"))
        .expect("assert node");
    assert_eq!(
        assert_node.get("kind").and_then(Value::as_str),
        Some("query_ref")
    );
    assert_eq!(
        assert_node
            .get("extensions")
            .and_then(Value::as_object)
            .and_then(|ext| ext.get("control"))
            .and_then(Value::as_object)
            .and_then(|control| control.get("step_kind")),
        Some(&json!("assert"))
    );

    let branch_node = plan
        .nodes
        .iter()
        .filter_map(Value::as_object)
        .find(|node| node.get("id").and_then(Value::as_str) == Some("c_branch"))
        .expect("branch node");
    assert_eq!(
        branch_node.get("kind").and_then(Value::as_str),
        Some("action_ref")
    );
    assert_eq!(
        branch_node
            .get("extensions")
            .and_then(Value::as_object)
            .and_then(|ext| ext.get("control"))
            .and_then(Value::as_object)
            .and_then(|control| control.get("step_kind")),
        Some(&json!("branch"))
    );
}

#[test]
fn control_type_nodes_require_exactly_one_target() {
    let mut context = ResolverContext::new();
    context.register_protocol(demo_protocol());

    let workflow = WorkflowDocument {
        schema: "ais-flow/0.0.3".to_string(),
        meta: json!({ "name":"wf", "version":"0.0.1" }),
        default_chain: Some("eip155:1".to_string()),
        imports: None,
        requires_pack: None,
        inputs: Map::new(),
        nodes: vec![
            json!({
                "id":"c1",
                "type":"assert",
                "protocol":"demo@0.0.2"
            }),
            json!({
                "id":"c2",
                "type":"branch",
                "protocol":"demo@0.0.2",
                "action":"swap",
                "query":"quote"
            }),
        ],
        policy: None,
        preflight: None,
        outputs: Map::new(),
        extensions: Map::new(),
    };

    let result = compile_workflow(&workflow, &context, &CompileWorkflowOptions::default());
    match result {
        CompileWorkflowResult::Ok { .. } => panic!("must fail"),
        CompileWorkflowResult::Err { issues } => {
            assert!(issues.iter().any(|issue| {
                issue.reference.as_deref() == Some("workflow.node.control_target_required")
            }));
            assert!(issues.iter().any(|issue| {
                issue.reference.as_deref() == Some("workflow.node.control_target_ambiguous")
            }));
        }
    }
}

#[test]
fn compiles_action_nodes_with_risk_metadata_in_extensions() {
    let mut context = ResolverContext::new();
    context.register_protocol(demo_protocol());

    let workflow = WorkflowDocument {
        schema: "ais-flow/0.0.3".to_string(),
        meta: json!({ "name":"wf", "version":"0.0.1" }),
        default_chain: Some("eip155:1".to_string()),
        imports: None,
        requires_pack: None,
        inputs: Map::new(),
        nodes: vec![json!({
            "id":"swap",
            "type":"action_ref",
            "protocol":"demo@0.0.2",
            "action":"swap",
            "args":{
                "amount_in": {"lit":"1"},
                "slippage_bps": {"lit": 50}
            }
        })],
        policy: None,
        preflight: None,
        outputs: Map::new(),
        extensions: Map::new(),
    };

    let result = compile_workflow(&workflow, &context, &CompileWorkflowOptions::default());
    let plan = match result {
        CompileWorkflowResult::Ok { plan } => plan,
        CompileWorkflowResult::Err { issues } => panic!("must compile: {issues:?}"),
    };
    let swap = plan
        .nodes
        .iter()
        .filter_map(Value::as_object)
        .find(|node| node.get("id").and_then(Value::as_str) == Some("swap"))
        .expect("swap node exists");
    let extensions = swap
        .get("extensions")
        .and_then(Value::as_object)
        .expect("extensions must exist");
    assert_eq!(extensions.get("risk_level"), Some(&json!(3)));
    assert_eq!(extensions.get("risk_tags"), Some(&json!(["slippage"])));
    let policy = extensions
        .get("policy")
        .and_then(Value::as_object)
        .expect("extensions.policy must exist");
    assert_eq!(
        policy
            .get("param_roles")
            .and_then(Value::as_object)
            .and_then(|map| map.get("spend_amount"))
            .and_then(Value::as_str),
        None
    );
    assert_eq!(
        policy
            .get("param_roles")
            .and_then(Value::as_object)
            .and_then(|map| map.get("slippage_bps"))
            .and_then(Value::as_str),
        Some("slippage_bps")
    );
    assert!(policy
        .get("required_fields")
        .and_then(Value::as_array)
        .is_some());
}

#[test]
fn implicit_deps_can_be_disabled() {
    let mut context = ResolverContext::new();
    context.register_protocol(demo_protocol());

    let workflow = WorkflowDocument {
        schema: "ais-flow/0.0.3".to_string(),
        meta: json!({ "name":"wf", "version":"0.0.1" }),
        default_chain: Some("eip155:1".to_string()),
        imports: None,
        requires_pack: None,
        inputs: Map::new(),
        nodes: vec![
            json!({
                "id":"a",
                "type":"query_ref",
                "protocol":"demo@0.0.2",
                "query":"quote"
            }),
            json!({
                "id":"b",
                "type":"action_ref",
                "protocol":"demo@0.0.2",
                "action":"swap",
                "args":{"min_out":{"ref":"nodes.a.outputs.amount_out"}}
            }),
        ],
        policy: None,
        preflight: None,
        outputs: Map::new(),
        extensions: Map::new(),
    };

    let result = compile_workflow(
        &workflow,
        &context,
        &CompileWorkflowOptions {
            default_chain: None,
            include_implicit_deps: false,
        },
    );
    let plan = match result {
        CompileWorkflowResult::Ok { plan } => plan,
        CompileWorkflowResult::Err { issues } => panic!("must compile: {issues:?}"),
    };
    let b_node = plan
        .nodes
        .iter()
        .filter_map(Value::as_object)
        .find(|node| node.get("id").and_then(Value::as_str) == Some("b"))
        .expect("b node exists");
    assert!(b_node.get("deps").is_none());
}

#[test]
fn missing_reference_returns_structured_issue() {
    let mut context = ResolverContext::new();
    context.register_protocol(demo_protocol());

    let workflow = WorkflowDocument {
        schema: "ais-flow/0.0.3".to_string(),
        meta: json!({ "name":"wf", "version":"0.0.1" }),
        default_chain: Some("eip155:1".to_string()),
        imports: None,
        requires_pack: None,
        inputs: Map::new(),
        nodes: vec![json!({
            "id":"a",
            "type":"action_ref",
            "protocol":"demo@0.0.2",
            "action":"missing"
        })],
        policy: None,
        preflight: None,
        outputs: Map::new(),
        extensions: Map::new(),
    };

    let result = compile_workflow(&workflow, &context, &CompileWorkflowOptions::default());
    match result {
        CompileWorkflowResult::Ok { .. } => panic!("must fail"),
        CompileWorkflowResult::Err { issues } => {
            assert!(issues
                .iter()
                .any(|issue| issue.reference.as_deref() == Some("workflow.node.action_missing")));
        }
    }
}

#[test]
fn plan_nodes_have_default_writes() {
    let mut context = ResolverContext::new();
    context.register_protocol(demo_protocol());

    let workflow = WorkflowDocument {
        schema: "ais-flow/0.0.3".to_string(),
        meta: json!({ "name":"wf", "version":"0.0.1" }),
        default_chain: Some("eip155:1".to_string()),
        imports: None,
        requires_pack: None,
        inputs: Map::new(),
        nodes: vec![json!({
            "id":"q",
            "type":"query_ref",
            "protocol":"demo@0.0.2",
            "query":"quote"
        })],
        policy: None,
        preflight: None,
        outputs: Map::new(),
        extensions: Map::new(),
    };

    let result = compile_workflow(&workflow, &context, &CompileWorkflowOptions::default());
    let plan = match result {
        CompileWorkflowResult::Ok { plan } => plan,
        CompileWorkflowResult::Err { issues } => panic!("must compile: {issues:?}"),
    };
    let q_node = plan
        .nodes
        .first()
        .and_then(Value::as_object)
        .expect("q node");
    assert_eq!(
        q_node
            .get("writes")
            .and_then(Value::as_array)
            .and_then(|writes| writes.first())
            .and_then(Value::as_object)
            .and_then(|write| write.get("path"))
            .and_then(Value::as_str),
        Some("nodes.q.outputs")
    );
}

#[test]
fn workflow_preflight_is_copied_into_plan_meta() {
    let mut context = ResolverContext::new();
    context.register_protocol(demo_protocol());

    let workflow = WorkflowDocument {
        schema: "ais-flow/0.0.3".to_string(),
        meta: json!({ "name":"wf", "version":"0.0.1" }),
        default_chain: Some("eip155:1".to_string()),
        imports: None,
        requires_pack: None,
        inputs: Map::new(),
        nodes: vec![json!({
            "id":"q",
            "type":"query_ref",
            "protocol":"demo@0.0.2",
            "query":"quote"
        })],
        policy: None,
        preflight: Some(json!({
            "simulate": {
                "q": true
            }
        })),
        outputs: Map::new(),
        extensions: Map::new(),
    };

    let result = compile_workflow(&workflow, &context, &CompileWorkflowOptions::default());
    let plan = match result {
        CompileWorkflowResult::Ok { plan } => plan,
        CompileWorkflowResult::Err { issues } => panic!("must compile: {issues:?}"),
    };
    assert_eq!(
        plan.meta
            .as_ref()
            .and_then(Value::as_object)
            .and_then(|meta| meta.get("preflight")),
        Some(&json!({
            "simulate": {
                "q": true
            }
        }))
    );
}

#[test]
fn compiles_assert_and_assert_message_from_fixture() {
    let mut context = ResolverContext::new();
    context.register_protocol(demo_protocol());
    let workflow = load_workflow_fixture("assert/success.json");

    let result = compile_workflow(&workflow, &context, &CompileWorkflowOptions::default());
    let plan = match result {
        CompileWorkflowResult::Ok { plan } => plan,
        CompileWorkflowResult::Err { issues } => panic!("must compile: {issues:?}"),
    };
    let node = plan
        .nodes
        .first()
        .and_then(Value::as_object)
        .expect("compiled node");
    assert!(node.get("assert").is_some());
    assert_eq!(
        node.get("assert_message").and_then(Value::as_str),
        Some("quote output must be present")
    );
}

#[test]
fn invalid_assert_cel_is_reported_in_compile() {
    let mut context = ResolverContext::new();
    context.register_protocol(demo_protocol());
    let workflow = load_workflow_fixture("assert/fail-invalid-cel.json");

    let result = compile_workflow(&workflow, &context, &CompileWorkflowOptions::default());
    match result {
        CompileWorkflowResult::Ok { .. } => panic!("must fail"),
        CompileWorkflowResult::Err { issues } => {
            assert!(issues.iter().any(
                |issue| issue.reference.as_deref() == Some("workflow.node.assert_cel_invalid")
            ));
        }
    }
}

#[test]
fn non_boolean_assert_literal_is_reported_in_compile() {
    let mut context = ResolverContext::new();
    context.register_protocol(demo_protocol());
    let workflow = load_workflow_fixture("assert/type-error.json");

    let result = compile_workflow(&workflow, &context, &CompileWorkflowOptions::default());
    match result {
        CompileWorkflowResult::Ok { .. } => panic!("must fail"),
        CompileWorkflowResult::Err { issues } => {
            assert!(issues.iter().any(
                |issue| issue.reference.as_deref() == Some("workflow.node.assert_not_boolean")
            ));
        }
    }
}

#[test]
fn calculated_overrides_chained_order_is_stable() {
    let mut context = ResolverContext::new();
    context.register_protocol(demo_protocol());
    let workflow = load_workflow_fixture("calculated_overrides/chain.json");

    let result = compile_workflow(&workflow, &context, &CompileWorkflowOptions::default());
    let plan = match result {
        CompileWorkflowResult::Ok { plan } => plan,
        CompileWorkflowResult::Err { issues } => panic!("must compile: {issues:?}"),
    };
    let node = plan.nodes.first().and_then(Value::as_object).expect("node");
    let ordered_keys = node
        .get("calculated_override_order")
        .and_then(Value::as_array)
        .expect("calculated override order")
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect::<Vec<_>>();
    assert_eq!(
        ordered_keys,
        vec![
            "slippage_bps".to_string(),
            "amount_out_limit".to_string(),
            "final_min_out".to_string()
        ]
    );
    assert!(node
        .get("calculated_overrides")
        .and_then(Value::as_object)
        .is_some());
}

#[test]
fn calculated_overrides_missing_dependency_is_reported() {
    let mut context = ResolverContext::new();
    context.register_protocol(demo_protocol());
    let workflow = load_workflow_fixture("calculated_overrides/missing-ref.json");

    let result = compile_workflow(&workflow, &context, &CompileWorkflowOptions::default());
    match result {
        CompileWorkflowResult::Ok { .. } => panic!("must fail"),
        CompileWorkflowResult::Err { issues } => {
            assert!(issues.iter().any(|issue| {
                issue.reference.as_deref()
                    == Some("workflow.node.calculated_overrides.missing_dependency")
            }));
        }
    }
}

#[test]
fn calculated_overrides_cycle_is_reported() {
    let mut context = ResolverContext::new();
    context.register_protocol(demo_protocol());
    let workflow = load_workflow_fixture("calculated_overrides/cycle.json");

    let result = compile_workflow(&workflow, &context, &CompileWorkflowOptions::default());
    match result {
        CompileWorkflowResult::Ok { .. } => panic!("must fail"),
        CompileWorkflowResult::Err { issues } => {
            assert!(issues.iter().any(|issue| issue.reference.as_deref()
                == Some("workflow.node.calculated_overrides.cycle")));
        }
    }
}

fn demo_protocol() -> ProtocolDocument {
    let mut actions = Map::new();
    actions.insert(
        "swap".to_string(),
        json!({
            "description":"swap",
            "risk_level": 3,
            "risk_tags": ["slippage"],
            "params": [
                {"name":"amount_in","type":"token_amount","description":"amount in"},
                {"name":"slippage_bps","type":"uint32","description":"slippage bps"}
            ],
            "execution": {
                "eip155:*": {
                    "type":"evm_call",
                    "to":{"ref":"contracts.router"},
                    "abi":{"type":"function","name":"swap","inputs":[],"outputs":[]},
                    "args":{"min_out":{"ref":"params.min_out"}}
                }
            }
        }),
    );
    let mut queries = Map::new();
    queries.insert(
        "quote".to_string(),
        json!({
            "description":"quote",
            "execution": {
                "eip155:*": {
                    "type":"evm_read",
                    "to":{"ref":"contracts.router"},
                    "abi":{"type":"function","name":"quote","inputs":[],"outputs":[]},
                    "args":{}
                }
            }
        }),
    );

    ProtocolDocument {
        schema: "ais/0.0.2".to_string(),
        meta: json!({
            "protocol":"demo",
            "version":"0.0.2"
        }),
        deployments: vec![json!({
            "chain":"eip155:1",
            "contracts":{"router":"0x1111111111111111111111111111111111111111"}
        })],
        actions,
        queries,
        risks: Vec::new(),
        supported_assets: Vec::new(),
        capabilities_required: Vec::new(),
        tests: Vec::new(),
        extensions: Map::new(),
    }
}

fn load_workflow_fixture(relative: &str) -> WorkflowDocument {
    let path = fixture_root().join(relative);
    let content = fs::read_to_string(&path).expect("must read fixture");
    let parsed = parse_document_with_options(
        content.as_str(),
        ParseDocumentOptions {
            format: DocumentFormat::Auto,
            validate_schema: false,
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
