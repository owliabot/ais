use super::{schedule_ready_nodes, SchedulerOptions};
use ais_sdk::PlanDocument;
use serde_json::{json, Map};
use std::collections::{BTreeMap, BTreeSet};

fn plan_with_nodes(nodes: Vec<serde_json::Value>) -> PlanDocument {
    PlanDocument {
        schema: "ais-plan/0.0.3".to_string(),
        meta: None,
        nodes,
        extensions: Map::new(),
    }
}

#[test]
fn scheduler_reads_parallel_under_global_limit() {
    let plan = plan_with_nodes(vec![
        json!({"id":"r1","chain":"eip155:1","execution":{"type":"evm_read"},"writes":[]}),
        json!({"id":"r2","chain":"eip155:1","execution":{"type":"evm_read"},"writes":[]}),
        json!({"id":"r3","chain":"solana:mainnet-beta","execution":{"type":"solana_read"},"writes":[]}),
    ]);
    let options = SchedulerOptions {
        global_max_parallel: 2,
        ..SchedulerOptions::default()
    };
    let batches = schedule_ready_nodes(&plan, &BTreeSet::new(), &options);
    assert_eq!(batches.len(), 2);
    assert_eq!(
        batches[0].nodes.iter().map(|node| node.id.as_str()).collect::<Vec<_>>(),
        vec!["r1", "r2"]
    );
}

#[test]
fn scheduler_writes_per_chain_serial_by_default() {
    let plan = plan_with_nodes(vec![
        json!({"id":"w1","chain":"eip155:1","execution":{"type":"evm_call"}}),
        json!({"id":"w2","chain":"eip155:1","execution":{"type":"evm_call"}}),
        json!({"id":"w3","chain":"solana:mainnet-beta","execution":{"type":"solana_instruction"}}),
    ]);
    let batches = schedule_ready_nodes(&plan, &BTreeSet::new(), &SchedulerOptions::default());
    assert_eq!(batches.len(), 2);
    assert_eq!(
        batches[0].nodes.iter().map(|node| node.id.as_str()).collect::<Vec<_>>(),
        vec!["w1", "w3"]
    );
    assert_eq!(
        batches[1].nodes.iter().map(|node| node.id.as_str()).collect::<Vec<_>>(),
        vec!["w2"]
    );
}

#[test]
fn scheduler_per_chain_limit_is_configurable() {
    let plan = plan_with_nodes(vec![
        json!({"id":"a1","chain":"eip155:1","execution":{"type":"evm_read"},"writes":[]}),
        json!({"id":"a2","chain":"eip155:1","execution":{"type":"evm_read"},"writes":[]}),
        json!({"id":"a3","chain":"eip155:1","execution":{"type":"evm_read"},"writes":[]}),
    ]);
    let options = SchedulerOptions {
        global_max_parallel: 3,
        default_per_chain_parallel: 3,
        per_chain_parallel_limits: BTreeMap::from([("eip155:1".to_string(), 1)]),
        writes_per_chain_serial: true,
    };
    let batches = schedule_ready_nodes(&plan, &BTreeSet::new(), &options);
    assert_eq!(batches.len(), 3);
    assert_eq!(
        batches
            .iter()
            .map(|batch| batch.nodes[0].id.clone())
            .collect::<Vec<_>>(),
        vec!["a1".to_string(), "a2".to_string(), "a3".to_string()]
    );
}

#[test]
fn scheduler_skips_nodes_with_unmet_deps_and_completed_nodes() {
    let plan = plan_with_nodes(vec![
        json!({"id":"n1","chain":"eip155:1","execution":{"type":"evm_read"},"writes":[]}),
        json!({"id":"n2","chain":"eip155:1","execution":{"type":"evm_read"},"deps":["n1"],"writes":[]}),
        json!({"id":"n3","chain":"eip155:1","execution":{"type":"evm_read"},"deps":["n9"],"writes":[]}),
    ]);
    let completed = BTreeSet::from(["n1".to_string()]);
    let batches = schedule_ready_nodes(&plan, &completed, &SchedulerOptions::default());
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].nodes.len(), 1);
    assert_eq!(batches[0].nodes[0].id, "n2");
}
