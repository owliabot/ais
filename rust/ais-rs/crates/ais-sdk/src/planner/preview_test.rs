use super::{dry_run_json, dry_run_json_async, dry_run_text, render_dry_run_text};
use crate::planner::NodeRunState;
use crate::documents::PlanDocument;
use crate::resolver::{DetectResolver, DetectSpec, ResolverContext, ValueRefEvalError, ValueRefEvalOptions};
use futures::executor::block_on;
use futures::FutureExt;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

fn sample_plan() -> PlanDocument {
    PlanDocument {
        schema: "ais-plan/0.0.3".to_string(),
        meta: Some(json!({
            "name": "preview-test"
        })),
        nodes: vec![
            json!({
                "id": "node-ready",
                "kind": "execution",
                "chain": "eip155:1",
                "execution": {
                    "type": "evm_read",
                    "to": {"lit": "0x0000000000000000000000000000000000000001"},
                    "abi": {"type": "function", "name": "balanceOf", "inputs": [], "outputs": []},
                    "method": "balanceOf",
                    "args": {}
                },
                "writes": [{"path": "nodes.node-ready.outputs", "mode": "set"}]
            }),
            json!({
                "id": "node-blocked",
                "kind": "execution",
                "chain": "eip155:1",
                "execution": {
                    "type": "evm_call",
                    "to": {"ref": "contracts.router"},
                    "abi": {"type": "function", "name": "swap", "inputs": [], "outputs": []},
                    "method": "swap",
                    "args": {
                        "amount": {"ref": "inputs.amount"}
                    }
                }
            }),
        ],
        extensions: Map::new(),
    }
}

#[test]
fn dry_run_json_contains_per_node_report_and_issues() {
    let context = ResolverContext::with_runtime(json!({
        "inputs": {
            "amount": "100"
        }
    }));
    let report = dry_run_json(&sample_plan(), &context, &ValueRefEvalOptions::default());

    assert_eq!(report.schema, "ais-dry-run-report/0.0.1");
    assert_eq!(report.summary.total_nodes, 2);
    assert_eq!(report.summary.ready_nodes, 1);
    assert_eq!(report.summary.blocked_nodes, 1);
    assert_eq!(report.summary.skipped_nodes, 0);
    assert_eq!(report.summary.estimated_confirmation_points, 1);
    assert_eq!(report.nodes.len(), 2);
    assert_eq!(report.nodes[0].readiness.state, NodeRunState::Ready);
    assert_eq!(report.nodes[1].readiness.state, NodeRunState::Blocked);
    assert_eq!(report.nodes[1].readiness.missing_refs, vec!["contracts.router".to_string()]);
    assert!(!report.issues.is_empty());
    assert!(!report.plan_hash.is_empty());
    assert!(!report.report_hash.is_empty());
}

#[test]
fn dry_run_text_is_stable() {
    let context = ResolverContext::with_runtime(json!({
        "inputs": {
            "amount": "100"
        }
    }));

    let text_first = dry_run_text(&sample_plan(), &context, &ValueRefEvalOptions::default());
    let text_second = dry_run_text(&sample_plan(), &context, &ValueRefEvalOptions::default());
    assert_eq!(text_first, text_second);
    assert!(text_first.contains("AIS dry-run"));
    assert!(text_first.contains("id=node-ready"));
    assert!(text_first.contains("id=node-blocked"));
}

struct StaticDetectResolver;

impl DetectResolver for StaticDetectResolver {
    fn resolve<'a>(
        &'a self,
        detect: &'a DetectSpec,
        _context: &'a ResolverContext,
        _options: &'a ValueRefEvalOptions,
    ) -> futures::future::LocalBoxFuture<'a, Result<Value, ValueRefEvalError>> {
        async move {
            Ok(json!({
                "selected": detect.kind
            }))
        }
        .boxed_local()
    }
}

#[test]
fn dry_run_json_async_resolves_detect_and_becomes_ready() {
    let plan = PlanDocument {
        schema: "ais-plan/0.0.3".to_string(),
        meta: None,
        nodes: vec![json!({
            "id": "node-detect",
            "kind": "execution",
            "chain": "eip155:1",
            "execution": {
                "type": "evm_call",
                "to": {"lit": "0x0000000000000000000000000000000000000001"},
                "abi": {"type": "function", "name": "swap", "inputs": [], "outputs": []},
                "method": "swap",
                "args": {
                    "route": {"detect": {"kind": "choose_one"}}
                }
            }
        })],
        extensions: Map::new(),
    };
    let context = ResolverContext::new();
    let resolver = StaticDetectResolver;

    let report = block_on(dry_run_json_async(
        &plan,
        &context,
        &ValueRefEvalOptions {
            root_overrides: BTreeMap::new(),
        },
        Some(&resolver),
    ));

    assert_eq!(report.nodes.len(), 1);
    assert_eq!(report.nodes[0].readiness.state, NodeRunState::Ready);
    assert_eq!(render_dry_run_text(&report), render_dry_run_text(&report));
}
