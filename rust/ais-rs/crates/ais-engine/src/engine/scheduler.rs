use ais_sdk::PlanDocument;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashSet};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchedulerOptions {
    pub global_max_parallel: usize,
    pub default_per_chain_parallel: usize,
    #[serde(default)]
    pub per_chain_parallel_limits: BTreeMap<String, usize>,
    pub writes_per_chain_serial: bool,
}

impl Default for SchedulerOptions {
    fn default() -> Self {
        Self {
            global_max_parallel: 4,
            default_per_chain_parallel: 4,
            per_chain_parallel_limits: BTreeMap::new(),
            writes_per_chain_serial: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduledNode {
    pub id: String,
    pub chain: String,
    pub is_write: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduleBatch {
    pub nodes: Vec<ScheduledNode>,
}

pub fn schedule_ready_nodes(
    plan: &PlanDocument,
    completed_node_ids: &BTreeSet<String>,
    options: &SchedulerOptions,
) -> Vec<ScheduleBatch> {
    let candidates = collect_ready_candidates(plan, completed_node_ids);
    build_batches(candidates, options)
}

fn collect_ready_candidates(
    plan: &PlanDocument,
    completed_node_ids: &BTreeSet<String>,
) -> Vec<ScheduledNode> {
    let completed = completed_node_ids
        .iter()
        .cloned()
        .collect::<HashSet<_>>();
    let mut out = Vec::<ScheduledNode>::new();

    for node in &plan.nodes {
        let Some(object) = node.as_object() else {
            continue;
        };
        let Some(id) = object.get("id").and_then(Value::as_str).map(str::to_string) else {
            continue;
        };
        if completed.contains(&id) {
            continue;
        }
        if !deps_satisfied(object.get("deps"), &completed) {
            continue;
        }
        let chain = object
            .get("chain")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        out.push(ScheduledNode {
            id,
            chain,
            is_write: is_write_node(object),
        });
    }

    out
}

fn deps_satisfied(raw_deps: Option<&Value>, completed: &HashSet<String>) -> bool {
    let Some(deps) = raw_deps.and_then(Value::as_array) else {
        return true;
    };
    deps.iter()
        .filter_map(Value::as_str)
        .all(|dep| completed.contains(dep))
}

fn is_write_node(node: &serde_json::Map<String, Value>) -> bool {
    let has_writes = node
        .get("writes")
        .and_then(Value::as_array)
        .is_some_and(|writes| !writes.is_empty());
    if has_writes {
        return true;
    }
    let execution_type = node
        .get("execution")
        .and_then(Value::as_object)
        .and_then(|execution| execution.get("type"))
        .and_then(Value::as_str)
        .unwrap_or("");
    matches!(
        execution_type,
        "evm_call" | "evm_multicall" | "solana_instruction" | "bitcoin_psbt"
    )
}

fn build_batches(candidates: Vec<ScheduledNode>, options: &SchedulerOptions) -> Vec<ScheduleBatch> {
    if candidates.is_empty() {
        return Vec::new();
    }

    let mut pending = candidates;
    let mut batches = Vec::<ScheduleBatch>::new();
    while !pending.is_empty() {
        let mut batch_nodes = Vec::<ScheduledNode>::new();
        let mut chain_counts = BTreeMap::<String, usize>::new();
        let mut chains_with_write = BTreeSet::<String>::new();
        let mut next_pending = Vec::<ScheduledNode>::new();

        for node in pending {
            if batch_nodes.len() >= options.global_max_parallel.max(1) {
                next_pending.push(node);
                continue;
            }
            let chain_limit = options
                .per_chain_parallel_limits
                .get(&node.chain)
                .copied()
                .unwrap_or(options.default_per_chain_parallel)
                .max(1);
            let current_chain_count = chain_counts.get(&node.chain).copied().unwrap_or(0);
            if current_chain_count >= chain_limit {
                next_pending.push(node);
                continue;
            }
            if options.writes_per_chain_serial
                && (chains_with_write.contains(&node.chain) || (node.is_write && current_chain_count > 0))
            {
                next_pending.push(node);
                continue;
            }

            if node.is_write {
                chains_with_write.insert(node.chain.clone());
            }
            chain_counts.insert(node.chain.clone(), current_chain_count + 1);
            batch_nodes.push(node);
        }

        if batch_nodes.is_empty() {
            batch_nodes.push(next_pending.remove(0));
        }

        batches.push(ScheduleBatch { nodes: batch_nodes });
        pending = next_pending;
    }
    batches
}

#[cfg(test)]
#[path = "scheduler_test.rs"]
mod tests;
