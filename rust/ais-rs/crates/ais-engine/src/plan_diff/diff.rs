use ais_sdk::PlanDocument;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanDiffSummary {
    pub added: usize,
    pub removed: usize,
    pub changed: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanDiffNodeIdentity {
    pub id: String,
    pub chain: Option<String>,
    pub execution_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanChange {
    Deps,
    Chain,
    ExecutionType,
    Writes,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanDiffNodeChanged {
    pub id: String,
    pub changes: Vec<PlanChange>,
    pub before: PlanDiffNodeIdentity,
    pub after: PlanDiffNodeIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanDiffJson {
    pub summary: PlanDiffSummary,
    pub added: Vec<PlanDiffNodeIdentity>,
    pub removed: Vec<PlanDiffNodeIdentity>,
    pub changed: Vec<PlanDiffNodeChanged>,
}

pub fn diff_plans_json(before: &PlanDocument, after: &PlanDocument) -> PlanDiffJson {
    let before_nodes = index_nodes(before);
    let after_nodes = index_nodes(after);

    let before_ids = before_nodes.keys().cloned().collect::<BTreeSet<_>>();
    let after_ids = after_nodes.keys().cloned().collect::<BTreeSet<_>>();

    let mut added = Vec::<PlanDiffNodeIdentity>::new();
    let mut removed = Vec::<PlanDiffNodeIdentity>::new();
    let mut changed = Vec::<PlanDiffNodeChanged>::new();

    for id in after_ids.difference(&before_ids) {
        if let Some(node) = after_nodes.get(id) {
            added.push(node_identity(node));
        }
    }
    for id in before_ids.difference(&after_ids) {
        if let Some(node) = before_nodes.get(id) {
            removed.push(node_identity(node));
        }
    }
    for id in before_ids.intersection(&after_ids) {
        let Some(left) = before_nodes.get(id) else { continue };
        let Some(right) = after_nodes.get(id) else { continue };
        let changes = detect_changes(left, right);
        if !changes.is_empty() {
            changed.push(PlanDiffNodeChanged {
                id: id.clone(),
                changes,
                before: node_identity(left),
                after: node_identity(right),
            });
        }
    }

    PlanDiffJson {
        summary: PlanDiffSummary {
            added: added.len(),
            removed: removed.len(),
            changed: changed.len(),
        },
        added,
        removed,
        changed,
    }
}

pub fn diff_plans_text(before: &PlanDocument, after: &PlanDocument) -> String {
    let diff = diff_plans_json(before, after);
    let mut lines = Vec::<String>::new();
    lines.push(format!(
        "plan diff: added={} removed={} changed={}",
        diff.summary.added, diff.summary.removed, diff.summary.changed
    ));
    if !diff.added.is_empty() {
        lines.push("added:".to_string());
        for node in &diff.added {
            lines.push(format!(
                "- id={} chain={} exec={}",
                node.id,
                node.chain.as_deref().unwrap_or("-"),
                node.execution_type.as_deref().unwrap_or("-")
            ));
        }
    }
    if !diff.removed.is_empty() {
        lines.push("removed:".to_string());
        for node in &diff.removed {
            lines.push(format!(
                "- id={} chain={} exec={}",
                node.id,
                node.chain.as_deref().unwrap_or("-"),
                node.execution_type.as_deref().unwrap_or("-")
            ));
        }
    }
    if !diff.changed.is_empty() {
        lines.push("changed:".to_string());
        for node in &diff.changed {
            lines.push(format!(
                "- id={} changes={}",
                node.id,
                node.changes
                    .iter()
                    .map(change_label)
                    .collect::<Vec<_>>()
                    .join(",")
            ));
        }
    }
    lines.join("\n")
}

fn change_label(change: &PlanChange) -> &'static str {
    match change {
        PlanChange::Deps => "deps",
        PlanChange::Chain => "chain",
        PlanChange::ExecutionType => "execution_type",
        PlanChange::Writes => "writes",
    }
}

fn index_nodes<'a>(plan: &'a PlanDocument) -> BTreeMap<String, &'a Value> {
    let mut out = BTreeMap::<String, &'a Value>::new();
    for node in &plan.nodes {
        let Some(id) = node
            .as_object()
            .and_then(|obj| obj.get("id"))
            .and_then(Value::as_str)
            .map(str::to_string)
        else {
            continue;
        };
        out.insert(id, node);
    }
    out
}

fn node_identity(node: &Value) -> PlanDiffNodeIdentity {
    PlanDiffNodeIdentity {
        id: node
            .as_object()
            .and_then(|obj| obj.get("id"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        chain: node
            .as_object()
            .and_then(|obj| obj.get("chain"))
            .and_then(Value::as_str)
            .map(str::to_string),
        execution_type: node
            .as_object()
            .and_then(|obj| obj.get("execution"))
            .and_then(Value::as_object)
            .and_then(|execution| execution.get("type"))
            .and_then(Value::as_str)
            .map(str::to_string),
    }
}

fn detect_changes(left: &Value, right: &Value) -> Vec<PlanChange> {
    let mut changes = Vec::<PlanChange>::new();

    if extract_deps(left) != extract_deps(right) {
        changes.push(PlanChange::Deps);
    }
    if extract_chain(left) != extract_chain(right) {
        changes.push(PlanChange::Chain);
    }
    if extract_execution_type(left) != extract_execution_type(right) {
        changes.push(PlanChange::ExecutionType);
    }
    if extract_writes(left) != extract_writes(right) {
        changes.push(PlanChange::Writes);
    }

    changes
}

fn extract_deps(node: &Value) -> Vec<String> {
    let mut deps = node
        .as_object()
        .and_then(|obj| obj.get("deps"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    deps.sort();
    deps
}

fn extract_chain(node: &Value) -> Option<String> {
    node.as_object()
        .and_then(|obj| obj.get("chain"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn extract_execution_type(node: &Value) -> Option<String> {
    node.as_object()
        .and_then(|obj| obj.get("execution"))
        .and_then(Value::as_object)
        .and_then(|execution| execution.get("type"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn extract_writes(node: &Value) -> Vec<String> {
    let mut writes = node
        .as_object()
        .and_then(|obj| obj.get("writes"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_object)
                .filter_map(|item| item.get("path"))
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    writes.sort();
    writes
}

#[cfg(test)]
#[path = "diff_test.rs"]
mod tests;
