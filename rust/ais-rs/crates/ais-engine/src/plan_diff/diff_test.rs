use super::{diff_plans_json, diff_plans_text, PlanChange};
use ais_sdk::PlanDocument;
use serde_json::{json, Map};
use std::fs;
use std::path::PathBuf;

fn make_plan(nodes: Vec<serde_json::Value>) -> PlanDocument {
    PlanDocument {
        schema: "ais-plan/0.0.3".to_string(),
        meta: None,
        nodes,
        extensions: Map::new(),
    }
}

#[test]
fn diff_json_reports_added_removed_changed() {
    let before = make_plan(vec![
        json!({
            "id": "a",
            "chain": "eip155:1",
            "execution": {"type": "evm_read"},
            "writes": [{"path":"nodes.a.outputs"}]
        }),
        json!({
            "id": "b",
            "chain": "eip155:1",
            "deps": ["a"],
            "execution": {"type": "evm_call"},
            "writes": [{"path":"nodes.b.outputs"}]
        }),
    ]);
    let after = make_plan(vec![
        json!({
            "id": "b",
            "chain": "eip155:137",
            "deps": [],
            "execution": {"type": "evm_multicall"},
            "writes": [{"path":"nodes.b.result"}]
        }),
        json!({
            "id": "c",
            "chain": "solana:mainnet-beta",
            "execution": {"type": "solana_instruction"}
        }),
    ]);

    let diff = diff_plans_json(&before, &after);
    assert_eq!(diff.summary.added, 1);
    assert_eq!(diff.summary.removed, 1);
    assert_eq!(diff.summary.changed, 1);
    assert_eq!(diff.added[0].id, "c");
    assert_eq!(diff.removed[0].id, "a");
    assert_eq!(diff.changed[0].id, "b");
    assert!(diff.changed[0].changes.contains(&PlanChange::Deps));
    assert!(diff.changed[0].changes.contains(&PlanChange::Chain));
    assert!(diff.changed[0].changes.contains(&PlanChange::ExecutionType));
    assert!(diff.changed[0].changes.contains(&PlanChange::Writes));
}

#[test]
fn diff_text_is_stable() {
    let before = make_plan(vec![json!({
        "id": "a",
        "chain": "eip155:1",
        "execution": {"type": "evm_read"}
    })]);
    let after = make_plan(vec![json!({
        "id": "a",
        "chain": "eip155:1",
        "execution": {"type": "evm_call"}
    })]);

    let first = diff_plans_text(&before, &after);
    let second = diff_plans_text(&before, &after);
    assert_eq!(first, second);
    assert!(first.contains("changed=1"));
    assert!(first.contains("execution_type"));
}

#[test]
fn diff_works_with_fixture_files() {
    let before = load_plan_fixture("plan-diff/before.plan.json");
    let after = load_plan_fixture("plan-diff/after.plan.json");

    let diff = diff_plans_json(&before, &after);
    assert_eq!(diff.summary.added, 1);
    assert_eq!(diff.summary.removed, 1);
    assert_eq!(diff.summary.changed, 1);
}

fn load_plan_fixture(relative: &str) -> PlanDocument {
    let path = fixture_root().join(relative);
    let content = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read fixture {} failed: {error}", path.display()));
    serde_json::from_str::<PlanDocument>(content.as_str())
        .unwrap_or_else(|error| panic!("parse fixture {} failed: {error}", path.display()))
}

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/plan-events")
}
