use super::{compile_plan_skeleton, CompilePlanSkeletonOptions, CompilePlanSkeletonResult};
use crate::documents::{PlanSkeletonDocument, ProtocolDocument};
use crate::resolver::ResolverContext;
use serde_json::{json, Map, Value};

#[test]
fn compiles_minimal_skeleton_into_plan() {
    let mut context = ResolverContext::new();
    context.register_protocol(demo_protocol());

    let skeleton = PlanSkeletonDocument {
        schema: "ais-plan-skeleton/0.0.1".to_string(),
        default_chain: Some("eip155:1".to_string()),
        nodes: vec![
            json!({
                "id":"quote",
                "type":"query_ref",
                "protocol":"demo@0.0.2",
                "query":"quote",
                "args":{"amount_in":{"lit":"7"}}
            }),
            json!({
                "id":"swap",
                "type":"action_ref",
                "protocol":"demo@0.0.2",
                "action":"swap",
                "deps":["quote"],
                "args":{"amount_in":{"lit":"7"},"min_out":{"ref":"nodes.quote.outputs.amount_out"}}
            }),
        ],
        policy_hints: Some(json!({"risk_preference":"low"})),
        extensions: Map::new(),
    };

    let result = compile_plan_skeleton(&skeleton, &context, &CompilePlanSkeletonOptions::default());
    match result {
        CompilePlanSkeletonResult::Ok { plan, workflow } => {
            assert_eq!(plan.schema, "ais-plan/0.0.3");
            assert_eq!(plan.nodes.len(), 2);
            assert_eq!(
                workflow.default_chain.as_deref(),
                Some("eip155:1")
            );
            assert_eq!(
                plan.extensions
                    .get("plan_skeleton")
                    .and_then(Value::as_object)
                    .and_then(|ps| ps.get("policy_hints"))
                    .and_then(Value::as_object)
                    .and_then(|h| h.get("risk_preference"))
                    .and_then(Value::as_str),
                Some("low")
            );
        }
        CompilePlanSkeletonResult::Err { issues } => {
            panic!("compile must succeed, issues: {issues:?}");
        }
    }
}

#[test]
fn returns_issues_on_unknown_dependency() {
    let mut context = ResolverContext::new();
    context.register_protocol(demo_protocol());

    let skeleton = PlanSkeletonDocument {
        schema: "ais-plan-skeleton/0.0.1".to_string(),
        default_chain: Some("eip155:1".to_string()),
        nodes: vec![json!({
            "id":"a",
            "type":"action_ref",
            "protocol":"demo@0.0.2",
            "action":"swap",
            "deps":["missing"]
        })],
        policy_hints: None,
        extensions: Map::new(),
    };

    let result = compile_plan_skeleton(&skeleton, &context, &CompilePlanSkeletonOptions::default());
    match result {
        CompilePlanSkeletonResult::Ok { .. } => panic!("must fail"),
        CompilePlanSkeletonResult::Err { issues } => {
            assert!(issues
                .iter()
                .any(|issue| issue.reference.as_deref() == Some("skeleton.graph.unknown_dep")));
        }
    }
}

#[test]
fn returns_issues_on_missing_action_reference() {
    let mut context = ResolverContext::new();
    context.register_protocol(demo_protocol());

    let skeleton = PlanSkeletonDocument {
        schema: "ais-plan-skeleton/0.0.1".to_string(),
        default_chain: Some("eip155:1".to_string()),
        nodes: vec![json!({
            "id":"a",
            "type":"action_ref",
            "protocol":"demo@0.0.2",
            "action":"nope"
        })],
        policy_hints: None,
        extensions: Map::new(),
    };

    let result = compile_plan_skeleton(&skeleton, &context, &CompilePlanSkeletonOptions::default());
    match result {
        CompilePlanSkeletonResult::Ok { .. } => panic!("must fail"),
        CompilePlanSkeletonResult::Err { issues } => {
            assert!(issues
                .iter()
                .any(|issue| issue.reference.as_deref() == Some("skeleton.node.action_missing")));
        }
    }
}

fn demo_protocol() -> ProtocolDocument {
    let mut actions = Map::new();
    actions.insert(
        "swap".to_string(),
        json!({
            "description":"swap",
            "execution": {
                "eip155:*": {
                    "type":"evm_call",
                    "to":{"ref":"contracts.router"},
                    "abi":{"type":"function","name":"swap","inputs":[],"outputs":[]},
                    "args":{"amount_in":{"ref":"params.amount_in"},"min_out":{"ref":"params.min_out"}}
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
                    "args":{"amount_in":{"ref":"params.amount_in"}}
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
