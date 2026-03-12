use super::{compile_workflow, CompileWorkflowOptions, CompileWorkflowResult};
use crate::documents::{PackDocument, ProtocolDocument, WorkflowDocument};
use crate::parse::{
    parse_document_with_options, AisDocument, DocumentFormat, ParseDocumentOptions,
};
use crate::resolver::ResolverContext;
use ais_core::{stable_hash_hex, StableJsonOptions};
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
fn compiles_protocol_deployment_contracts_into_extensions() {
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
            "id":"quote",
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

    let node = plan.nodes.first().expect("node");
    assert_eq!(
        node.pointer("/extensions/protocol/ref"),
        Some(&json!("demo@0.0.2"))
    );
    assert_eq!(
        node.pointer("/extensions/protocol/deployment_chain"),
        Some(&json!("eip155:1"))
    );
    assert_eq!(
        node.pointer("/extensions/protocol/contracts/router"),
        Some(&json!("0x1111111111111111111111111111111111111111"))
    );
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
    let node = plan.nodes.first().expect("node");
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

#[test]
fn compile_workflow_applies_pack_action_override_and_pack_extensions() {
    let mut context = ResolverContext::new();
    context.register_protocol(demo_protocol());
    context.register_pack(demo_pack());

    let workflow = WorkflowDocument {
        schema: "ais-flow/0.0.3".to_string(),
        meta: json!({ "name":"wf", "version":"0.0.1" }),
        default_chain: Some("eip155:1".to_string()),
        imports: None,
        requires_pack: Some(json!({"name":"safe-defi","version":"0.0.2"})),
        inputs: Map::new(),
        nodes: vec![json!({
            "id":"swap",
            "type":"action_ref",
            "protocol":"demo@0.0.2",
            "action":"swap"
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
    assert_eq!(
        plan.nodes[0].get("description"),
        Some(&json!("pack merged swap"))
    );
    assert_eq!(
        plan.nodes[0]
            .pointer("/extensions/pack/ref")
            .and_then(Value::as_str),
        Some("safe-defi@0.0.2")
    );
    assert_eq!(
        plan.nodes[0]
            .pointer("/extensions/pack/matched_action_rule_ids/0")
            .and_then(Value::as_str),
        Some("swap-rule")
    );
    assert_eq!(
        plan.nodes[0]
            .pointer("/extensions/pack/hash")
            .and_then(Value::as_str),
        Some(pack_hash(demo_pack()).as_str())
    );
    assert_eq!(
        plan.nodes[0]
            .pointer("/extensions/operation/selector")
            .and_then(Value::as_str),
        Some("demo.swap")
    );
    assert_eq!(
        plan.nodes[0]
            .pointer("/extensions/operation/kind")
            .and_then(Value::as_str),
        Some("action")
    );
    assert_eq!(
        plan.nodes[0]
            .pointer("/extensions/policy/effective_constraints")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(3)
    );
}

#[test]
fn compile_workflow_lowers_requires_queries_into_operation_query_bindings() {
    let mut context = ResolverContext::new();
    context.register_protocol(demo_protocol());
    context.register_pack(demo_pack());

    let workflow = WorkflowDocument {
        schema: "ais-flow/0.0.3".to_string(),
        meta: json!({ "name":"wf", "version":"0.0.1" }),
        default_chain: Some("eip155:1".to_string()),
        imports: None,
        requires_pack: Some(json!({"name":"safe-defi","version":"0.0.2"})),
        inputs: Map::new(),
        nodes: vec![
            json!({
                "id":"quote",
                "type":"query_ref",
                "protocol":"demo@0.0.2",
                "query":"quote"
            }),
            json!({
                "id":"swap",
                "type":"action_ref",
                "protocol":"demo@0.0.2",
                "action":"swap",
                "deps":["quote"]
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

    let swap_node = plan.nodes.get(1).expect("swap node");
    assert_eq!(
        swap_node.pointer("/extensions/operation/requires_queries"),
        Some(&json!(["quote", "allowance"]))
    );
    assert_eq!(
        swap_node.pointer("/extensions/operation/query_bindings/quote/node_id"),
        Some(&json!("quote"))
    );
    assert_eq!(
        swap_node.pointer("/extensions/operation/query_bindings/quote/query_ref"),
        Some(&json!("demo@0.0.2/quote"))
    );
    assert!(swap_node
        .pointer("/extensions/operation/query_bindings/allowance")
        .is_none());
}

#[test]
fn compile_workflow_requires_pack_to_be_present_in_context() {
    let mut context = ResolverContext::new();
    context.register_protocol(demo_protocol());

    let workflow = WorkflowDocument {
        schema: "ais-flow/0.0.3".to_string(),
        meta: json!({ "name":"wf", "version":"0.0.1" }),
        default_chain: Some("eip155:1".to_string()),
        imports: None,
        requires_pack: Some(json!({"name":"safe-defi","version":"0.0.2"})),
        inputs: Map::new(),
        nodes: vec![json!({
            "id":"swap",
            "type":"action_ref",
            "protocol":"demo@0.0.2",
            "action":"swap"
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
                .any(|issue| issue.reference.as_deref() == Some("workflow.requires_pack.missing")));
        }
    }
}

#[test]
fn compile_workflow_lowers_protocol_calculated_fields_into_node_bindings() {
    let mut context = ResolverContext::new();
    context.register_protocol(calculated_protocol());

    let workflow = WorkflowDocument {
        schema: "ais-flow/0.0.3".to_string(),
        meta: json!({ "name":"wf", "version":"0.0.1" }),
        default_chain: Some("eip155:1".to_string()),
        imports: None,
        requires_pack: None,
        inputs: Map::new(),
        nodes: vec![json!({
            "id":"supply",
            "type":"action_ref",
            "protocol":"calc-demo@0.0.2",
            "action":"supply"
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
    let node = plan.nodes.first().expect("node");
    assert_eq!(
        node.pointer("/calculated_override_order/0")
            .and_then(Value::as_str),
        Some("amount_atomic")
    );
    assert_eq!(
        node.pointer("/calculated_override_order/1")
            .and_then(Value::as_str),
        Some("recipient")
    );
    assert_eq!(
        node.pointer("/calculated_overrides/amount_atomic/expr/cel")
            .and_then(Value::as_str),
        Some("to_atomic(params.amount, params.token)")
    );
    assert_eq!(
        node.pointer("/calculated_overrides/recipient/expr/cel")
            .and_then(Value::as_str),
        Some("params.recipient != null ? params.recipient : ctx.wallet_address")
    );
}

#[test]
fn compile_workflow_lowers_composite_execution_into_base_nodes() {
    let mut context = ResolverContext::new();
    context.register_protocol(composite_protocol());

    let workflow = WorkflowDocument {
        schema: "ais-flow/0.0.3".to_string(),
        meta: json!({ "name":"wf", "version":"0.0.1" }),
        default_chain: Some("eip155:1".to_string()),
        imports: None,
        requires_pack: None,
        inputs: Map::new(),
        nodes: vec![json!({
            "id":"supply",
            "type":"action_ref",
            "protocol":"composite-demo@0.0.2",
            "action":"supply"
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
    assert_eq!(plan.nodes.len(), 2);
    assert_eq!(
        plan.nodes[0].pointer("/id"),
        Some(&json!("supply__approve"))
    );
    assert_eq!(plan.nodes[1].pointer("/id"), Some(&json!("supply")));
    assert_eq!(
        plan.nodes[0].pointer("/execution/type"),
        Some(&json!("evm_call"))
    );
    assert_eq!(plan.nodes[0].pointer("/chain"), Some(&json!("eip155:8453")));
    assert_eq!(plan.nodes[1].pointer("/chain"), Some(&json!("eip155:1")));
    assert_eq!(
        plan.nodes[1].pointer("/execution/type"),
        Some(&json!("evm_call"))
    );
    assert_eq!(
        plan.nodes[0].pointer("/extensions/composite/step_id"),
        Some(&json!("approve"))
    );
    assert_eq!(
        plan.nodes[1].pointer("/extensions/composite/step_id"),
        Some(&json!("supply"))
    );
    assert_eq!(
        plan.nodes[1].pointer("/deps/0"),
        Some(&json!("supply__approve"))
    );
    assert_eq!(
        plan.nodes[0].pointer("/source/composite_step_id"),
        Some(&json!("approve"))
    );
    assert_eq!(
        plan.nodes[1].pointer("/source/composite_step_id"),
        Some(&json!("supply"))
    );
    assert_eq!(
        plan.nodes[1].pointer("/condition/cel"),
        Some(&json!("nodes.supply__approve.outputs.tx_hash != null"))
    );
    assert_eq!(
        plan.nodes[1].pointer("/execution/args/approval_tx/ref"),
        Some(&json!("nodes.supply__approve.outputs.tx_hash"))
    );
    assert_eq!(
        plan.nodes[0].pointer("/extensions/composite/output_ref"),
        Some(&json!("nodes.supply__approve.outputs"))
    );
    assert_eq!(
        plan.nodes[0].pointer("/extensions/composite/step_chain"),
        Some(&json!("eip155:8453"))
    );
    assert_eq!(
        plan.nodes[1].pointer("/extensions/composite/local_step_node_ids/approve"),
        Some(&json!("supply__approve"))
    );
    assert_eq!(
        plan.nodes[0].pointer("/extensions/protocol/contracts/router"),
        Some(&json!("0x2222222222222222222222222222222222222222"))
    );
    assert_eq!(
        plan.nodes[1].pointer("/extensions/protocol/contracts/router"),
        Some(&json!("0x1111111111111111111111111111111111111111"))
    );
    assert_eq!(
        plan.nodes[0].pointer("/extensions/operation/target_chain"),
        Some(&json!("eip155:8453"))
    );
    assert_eq!(
        plan.nodes[1].pointer("/extensions/operation/target_chain"),
        Some(&json!("eip155:1"))
    );
    assert_eq!(
        plan.nodes[0].pointer("/extensions/composite/semantic_kind"),
        Some(&json!("approval"))
    );
    assert_eq!(
        plan.nodes[0].pointer("/source/composite_step_kind"),
        Some(&json!("approval"))
    );
    assert_eq!(
        plan.nodes[0].pointer("/extensions/risk_level"),
        Some(&json!(3))
    );
    assert_eq!(
        plan.nodes[1].pointer("/extensions/risk_level"),
        Some(&json!(3))
    );
    assert_eq!(
        plan.nodes[0].pointer("/extensions/risk_tags"),
        Some(&json!(["lend", "approval"]))
    );
    assert_eq!(
        plan.nodes[1].pointer("/extensions/risk_tags"),
        Some(&json!(["lend"]))
    );
    assert_eq!(
        plan.nodes[0].pointer("/extensions/policy/param_roles/spender_address"),
        Some(&json!("spender"))
    );
    assert_eq!(
        plan.nodes[0].pointer("/extensions/policy/param_roles/approval_amount"),
        Some(&json!("amount"))
    );
    assert_eq!(
        plan.nodes[1].pointer("/extensions/policy/param_roles/spend_amount"),
        Some(&json!("amount_atomic"))
    );
    assert_eq!(
        plan.nodes[0].pointer("/extensions/policy/required_fields"),
        Some(&json!([
            "spend_amount",
            "spender_address",
            "approval_amount"
        ]))
    );
    assert_eq!(
        plan.nodes[1].pointer("/extensions/policy/required_fields"),
        Some(&json!(["spend_amount"]))
    );
}

#[test]
fn compile_workflow_rejects_duplicate_emitted_node_ids_after_composite_lowering() {
    let mut context = ResolverContext::new();
    context.register_protocol(composite_protocol());

    let workflow = WorkflowDocument {
        schema: "ais-flow/0.0.3".to_string(),
        meta: json!({ "name":"wf", "version":"0.0.1" }),
        default_chain: Some("eip155:1".to_string()),
        imports: None,
        requires_pack: None,
        inputs: Map::new(),
        nodes: vec![
            json!({
                "id":"supply",
                "type":"action_ref",
                "protocol":"composite-demo@0.0.2",
                "action":"supply"
            }),
            json!({
                "id":"supply__approve",
                "type":"action_ref",
                "protocol":"composite-demo@0.0.2",
                "action":"approve"
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
                issue.reference.as_deref() == Some("workflow.node.lowered_duplicate_id")
                    && issue.message.contains("supply__approve")
            }));
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
        supported_assets: Vec::new(),
        extensions: Map::new(),
    }
}

fn calculated_protocol() -> ProtocolDocument {
    let mut actions = Map::new();
    actions.insert(
        "supply".to_string(),
        json!({
            "description":"supply",
            "risk_level": 3,
            "risk_tags": ["lend"],
            "params": [
                {"name":"token","type":"asset","description":"token"},
                {"name":"amount","type":"token_amount","description":"amount"},
                {"name":"recipient","type":"address","description":"recipient","required":false}
            ],
            "calculated_fields": {
                "amount_atomic": {
                    "expr": {"cel":"to_atomic(params.amount, params.token)"},
                    "inputs":["params.amount", "params.token"]
                },
                "recipient": {
                    "expr": {"cel":"params.recipient != null ? params.recipient : ctx.wallet_address"},
                    "inputs":["params.recipient", "ctx.wallet_address"]
                }
            },
            "execution": {
                "eip155:*": {
                    "type":"evm_call",
                    "to":{"ref":"contracts.router"},
                    "abi":{"type":"function","name":"supply","inputs":[],"outputs":[]},
                    "args":{"amount":{"ref":"calculated.amount_atomic"}}
                }
            }
        }),
    );

    ProtocolDocument {
        schema: "ais/0.0.2".to_string(),
        meta: json!({
            "protocol":"calc-demo",
            "version":"0.0.2"
        }),
        deployments: vec![json!({
            "chain":"eip155:1",
            "contracts":{"router":"0x1111111111111111111111111111111111111111"}
        })],
        actions,
        queries: Map::new(),
        supported_assets: Vec::new(),
        extensions: Map::new(),
    }
}

fn composite_protocol() -> ProtocolDocument {
    let mut actions = Map::new();
    actions.insert(
        "approve".to_string(),
        json!({
            "description":"approve",
            "risk_level": 3,
            "risk_tags": ["approval"],
            "params": [
                {"name":"amount_atomic","type":"uint256","description":"amount"}
            ],
            "execution": {
                "eip155:*": {
                    "type":"evm_call",
                    "to":{"ref":"contracts.router"},
                    "abi":{"type":"function","name":"approve","inputs":[],"outputs":[{"name":"tx_hash","type":"bytes32"}]},
                    "args":{"amount":{"ref":"params.amount_atomic"}}
                }
            }
        }),
    );
    actions.insert(
        "supply".to_string(),
        json!({
            "description":"supply",
            "risk_level": 3,
            "risk_tags": ["lend"],
            "params": [
                {"name":"token","type":"asset","description":"token"},
                {"name":"amount_atomic","type":"uint256","description":"amount"}
            ],
            "execution": {
                "eip155:*": {
                    "type":"composite",
                "steps":[
                        {
                            "id":"approve",
                            "chain":"eip155:8453",
                            "execution":{
                                "type":"evm_call",
                                "to":{"ref":"contracts.router"},
                                "abi":{"type":"function","name":"approve","inputs":[],"outputs":[{"name":"tx_hash","type":"bytes32"}]},
                                "args":{"amount":{"ref":"params.amount_atomic"}}
                            }
                        },
                        {
                            "id":"supply",
                            "condition":{"cel":"nodes.approve.outputs.tx_hash != null"},
                            "execution":{
                                "type":"evm_call",
                                "to":{"ref":"contracts.router"},
                                "abi":{"type":"function","name":"supply","inputs":[],"outputs":[]},
                                "args":{
                                    "amount":{"ref":"params.amount_atomic"},
                                    "approval_tx":{"ref":"nodes.approve.outputs.tx_hash"}
                                }
                            }
                        }
                    ]
                }
            }
        }),
    );

    ProtocolDocument {
        schema: "ais/0.0.2".to_string(),
        meta: json!({
            "protocol":"composite-demo",
            "version":"0.0.2"
        }),
        deployments: vec![
            json!({
                "chain":"eip155:1",
                "contracts":{"router":"0x1111111111111111111111111111111111111111"}
            }),
            json!({
                "chain":"eip155:8453",
                "contracts":{"router":"0x2222222222222222222222222222222222222222"}
            }),
        ],
        actions,
        queries: Map::new(),
        supported_assets: Vec::new(),
        extensions: Map::new(),
    }
}

fn demo_pack() -> PackDocument {
    serde_json::from_value(json!({
      "schema":"ais-pack/0.0.2",
      "name":"safe-defi",
      "version":"0.0.2",
      "includes":[{"protocol":"demo","version":"0.0.2","source":"registry"}],
      "policy":{
        "constraints":[{"id":"global","effect":"hard_block","assert":"inputs.slippage_bps <= 50"}]
      },
      "overrides":{
        "action_rules":[
          {
            "id":"swap-rule",
            "actions":["demo.swap"],
            "constraints":[{"id":"rule","effect":"hard_block","assert":"params.slippage_bps <= 30"}]
          }
        ],
        "actions":{
          "demo.swap":{
            "description":"pack merged swap",
            "requires_queries":["quote","allowance"],
            "constraints":[{"id":"action","effect":"hard_block","assert":"params.slippage_bps <= 20"}]
          }
        }
      }
    }))
    .expect("pack")
}

fn pack_hash(pack: PackDocument) -> String {
    stable_hash_hex(
        &serde_json::to_value(pack).expect("pack json"),
        &StableJsonOptions::default(),
    )
    .expect("pack hash")
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
