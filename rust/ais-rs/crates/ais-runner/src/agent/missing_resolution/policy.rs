use super::super::input_normalize;
use super::super::ref_catalog::RefCatalog;
use super::super::ref_model::RefPath;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub(crate) enum MissingResolutionDecision {
    BindFromRef { target: RefPath, source: RefPath },
    RunProducer { target: RefPath, query_ref: String },
    AskUser { target: RefPath, question: String },
    Abort { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct MissingResolutionPolicyIssue {
    pub code: String,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct MissingResolutionPolicyRejectedDecision {
    pub index: usize,
    pub decision: MissingResolutionDecision,
    pub issues: Vec<MissingResolutionPolicyIssue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub(crate) struct MissingResolutionPolicyValidation {
    pub accepted: bool,
    pub accepted_decisions: Vec<MissingResolutionDecision>,
    pub rejected_decisions: Vec<MissingResolutionPolicyRejectedDecision>,
    pub issues: Vec<MissingResolutionPolicyIssue>,
}

pub(crate) fn build_missing_resolution_decisions(
    resolution_payload: &Value,
) -> Vec<MissingResolutionDecision> {
    let explicit = resolution_payload
        .get("decisions")
        .and_then(Value::as_array)
        .into_iter()
        .flat_map(|items| items.iter())
        .filter_map(parse_decision_item)
        .collect::<Vec<_>>();
    if !explicit.is_empty() {
        return explicit;
    }
    build_missing_resolution_run_producer_decisions(resolution_payload)
}

pub(crate) fn build_missing_resolution_run_producer_decisions(
    resolution_payload: &Value,
) -> Vec<MissingResolutionDecision> {
    resolution_payload
        .get("resolved")
        .and_then(Value::as_array)
        .into_iter()
        .flat_map(|items| items.iter())
        .filter_map(|item| {
            let target = item.get("missing_ref").and_then(Value::as_str)?;
            let target = input_normalize::canonical_missing_ref_path(target)?;
            let query_ref = item
                .get("query_candidates")
                .and_then(Value::as_array)
                .and_then(|items| items.first())
                .and_then(|candidate| candidate.get("query_ref"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_string();
            Some(MissingResolutionDecision::RunProducer { target, query_ref })
        })
        .collect::<Vec<_>>()
}

pub(crate) fn selected_query_refs_from_missing_resolution_decisions(
    decisions: &[MissingResolutionDecision],
) -> Vec<String> {
    decisions
        .iter()
        .filter_map(|decision| match decision {
            MissingResolutionDecision::RunProducer { query_ref, .. }
                if !query_ref.trim().is_empty() =>
            {
                Some(query_ref.trim().to_string())
            }
            _ => None,
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
}

pub(crate) fn validate_missing_resolution_decisions(
    decisions: &[MissingResolutionDecision],
    missing_refs: &[String],
    catalog: &RefCatalog,
) -> MissingResolutionPolicyValidation {
    let mut global_issues = Vec::<MissingResolutionPolicyIssue>::new();
    if decisions.is_empty() {
        global_issues.push(MissingResolutionPolicyIssue {
            code: "empty_decision_set".to_string(),
            detail: "recovery decision set cannot be empty".to_string(),
        });
    }

    let missing_set = missing_refs
        .iter()
        .filter_map(|reference| input_normalize::canonical_missing_ref_path(reference))
        .map(|reference| reference.as_canonical_str())
        .collect::<BTreeSet<_>>();
    let catalog_map = catalog
        .entries
        .iter()
        .map(|entry| (entry.canonical_ref.clone(), entry))
        .collect::<BTreeMap<_, _>>();
    let mut target_decision_indexes = BTreeMap::<String, Vec<usize>>::new();
    let mut bind_edges = Vec::<(usize, String, String)>::new();
    let mut decision_issues = vec![Vec::<MissingResolutionPolicyIssue>::new(); decisions.len()];
    let mut edge_index = BTreeMap::<(String, String), Vec<usize>>::new();
    let mut bind_graph = BTreeMap::<String, BTreeSet<String>>::new();

    for (index, decision) in decisions.iter().enumerate() {
        match decision {
            MissingResolutionDecision::BindFromRef { target, source } => {
                validate_target_missing(target, &missing_set, &mut decision_issues[index]);
                let target_ref = target.as_canonical_str();
                target_decision_indexes
                    .entry(target_ref.clone())
                    .or_default()
                    .push(index);
                let source_ref = source.as_canonical_str();
                match catalog_map.get(source_ref.as_str()) {
                    Some(entry) if entry.value_available => {
                        if !binding_types_compatible(
                            target,
                            entry.value_type.as_str(),
                            &catalog_map,
                        ) {
                            decision_issues[index].push(MissingResolutionPolicyIssue {
                                code: "bind_type_incompatible".to_string(),
                                detail: format!(
                                    "target `{}` cannot bind from source `{}` with value_type `{}`",
                                    target_ref, source_ref, entry.value_type
                                ),
                            });
                        }
                    }
                    Some(_) => decision_issues[index].push(MissingResolutionPolicyIssue {
                        code: "bind_source_unavailable".to_string(),
                        detail: format!("source `{source_ref}` exists but has no value"),
                    }),
                    None => decision_issues[index].push(MissingResolutionPolicyIssue {
                        code: "bind_source_not_in_catalog".to_string(),
                        detail: format!("source `{source_ref}` does not exist in ref catalog"),
                    }),
                }
                if source_ref == target_ref {
                    decision_issues[index].push(MissingResolutionPolicyIssue {
                        code: "bind_source_equals_target".to_string(),
                        detail: format!("source and target are the same ref `{target_ref}`"),
                    });
                }
                bind_edges.push((index, source_ref.clone(), target_ref.clone()));
                edge_index
                    .entry((source_ref.clone(), target_ref.clone()))
                    .or_default()
                    .push(index);
                bind_graph.entry(source_ref).or_default().insert(target_ref);
            }
            MissingResolutionDecision::RunProducer { target, query_ref } => {
                validate_target_missing(target, &missing_set, &mut decision_issues[index]);
                let target_ref = target.as_canonical_str();
                target_decision_indexes
                    .entry(target_ref)
                    .or_default()
                    .push(index);
                if query_ref.trim().is_empty() {
                    decision_issues[index].push(MissingResolutionPolicyIssue {
                        code: "run_producer_query_ref_empty".to_string(),
                        detail: format!(
                            "target `{}` requires non-empty query_ref",
                            target.as_canonical_str()
                        ),
                    });
                }
            }
            MissingResolutionDecision::AskUser { target, question } => {
                validate_target_missing(target, &missing_set, &mut decision_issues[index]);
                let target_ref = target.as_canonical_str();
                target_decision_indexes
                    .entry(target_ref)
                    .or_default()
                    .push(index);
                if question.trim().is_empty() {
                    decision_issues[index].push(MissingResolutionPolicyIssue {
                        code: "ask_user_question_empty".to_string(),
                        detail: format!(
                            "target `{}` requires non-empty question text",
                            target.as_canonical_str()
                        ),
                    });
                }
            }
            MissingResolutionDecision::Abort { reason } => {
                if reason.trim().is_empty() {
                    decision_issues[index].push(MissingResolutionPolicyIssue {
                        code: "abort_reason_empty".to_string(),
                        detail: "abort decision requires non-empty reason".to_string(),
                    });
                }
            }
        }
    }

    for (target, indexes) in target_decision_indexes {
        if indexes.len() > 1 {
            let issue = MissingResolutionPolicyIssue {
                code: "duplicate_target_decision".to_string(),
                detail: format!("target `{target}` has {} decisions", indexes.len()),
            };
            for index in indexes {
                decision_issues[index].push(issue.clone());
            }
        }
    }

    for (index, source, target) in &bind_edges {
        if let Some(reverse_indexes) = edge_index.get(&(target.clone(), source.clone())) {
            let issue = MissingResolutionPolicyIssue {
                code: "bind_reverse_dependency".to_string(),
                detail: format!("reverse dependency detected: `{source}` <-> `{target}`"),
            };
            decision_issues[*index].push(issue.clone());
            for reverse_index in reverse_indexes {
                decision_issues[*reverse_index].push(issue.clone());
            }
        }
    }

    for (index, source, target) in &bind_edges {
        if graph_has_path(&bind_graph, target.as_str(), source.as_str()) {
            decision_issues[*index].push(MissingResolutionPolicyIssue {
                code: "bind_cycle_detected".to_string(),
                detail: format!("bind cycle detected through edge `{source}` -> `{target}`"),
            });
        }
    }

    let mut accepted_decisions = Vec::<MissingResolutionDecision>::new();
    let mut rejected_decisions = Vec::<MissingResolutionPolicyRejectedDecision>::new();
    for (index, decision) in decisions.iter().enumerate() {
        let issues = dedup_policy_issues(std::mem::take(&mut decision_issues[index]));
        if issues.is_empty() {
            accepted_decisions.push(decision.clone());
            continue;
        }
        global_issues.extend(issues.iter().cloned());
        rejected_decisions.push(MissingResolutionPolicyRejectedDecision {
            index,
            decision: decision.clone(),
            issues,
        });
    }

    let issues = dedup_policy_issues(global_issues);
    let accepted = !accepted_decisions.is_empty() && issues.is_empty();
    MissingResolutionPolicyValidation {
        accepted,
        accepted_decisions,
        rejected_decisions,
        issues,
    }
}

fn validate_target_missing(
    target: &RefPath,
    missing_set: &BTreeSet<String>,
    issues: &mut Vec<MissingResolutionPolicyIssue>,
) {
    let target_ref = target.as_canonical_str();
    if !missing_set.contains(target_ref.as_str()) {
        issues.push(MissingResolutionPolicyIssue {
            code: "target_not_missing".to_string(),
            detail: format!("target `{target_ref}` is not in current missing refs"),
        });
    }
}

fn dedup_policy_issues(
    issues: Vec<MissingResolutionPolicyIssue>,
) -> Vec<MissingResolutionPolicyIssue> {
    let mut seen = BTreeSet::<(String, String)>::new();
    let mut out = Vec::<MissingResolutionPolicyIssue>::new();
    for issue in issues {
        let key = (issue.code.clone(), issue.detail.clone());
        if seen.insert(key) {
            out.push(issue);
        }
    }
    out
}

fn parse_decision_item(item: &Value) -> Option<MissingResolutionDecision> {
    let kind = item
        .get("kind")
        .and_then(Value::as_str)?
        .trim()
        .to_ascii_lowercase();
    match kind.as_str() {
        "bind_from_ref" | "bind" => {
            let target = parse_ref_value(
                item.get("target")
                    .or_else(|| item.get("missing_ref"))
                    .or_else(|| item.get("ref"))?,
            )?;
            let source = parse_ref_value(item.get("source").or_else(|| item.get("source_ref"))?)?;
            Some(MissingResolutionDecision::BindFromRef { target, source })
        }
        "run_producer" | "run_query" | "query" => {
            let target = parse_ref_value(
                item.get("target")
                    .or_else(|| item.get("missing_ref"))
                    .or_else(|| item.get("ref"))?,
            )?;
            let query_ref = item
                .get("query_ref")
                .or_else(|| item.get("step_or_query_ref"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_string();
            Some(MissingResolutionDecision::RunProducer { target, query_ref })
        }
        "ask_user" | "ask" => {
            let target = parse_ref_value(
                item.get("target")
                    .or_else(|| item.get("missing_ref"))
                    .or_else(|| item.get("ref"))?,
            )?;
            let question = item
                .get("question")
                .or_else(|| item.get("message"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_string();
            Some(MissingResolutionDecision::AskUser { target, question })
        }
        "abort" => {
            let reason = item
                .get("reason")
                .or_else(|| item.get("message"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_string();
            Some(MissingResolutionDecision::Abort { reason })
        }
        _ => None,
    }
}

fn parse_ref_value(raw: &Value) -> Option<RefPath> {
    if let Some(reference) = raw.as_str() {
        return input_normalize::canonical_missing_ref_path(reference);
    }
    let parsed = serde_json::from_value::<RefPath>(raw.clone()).ok()?;
    match parsed {
        RefPath::Input { slot } => input_normalize::normalize_input_slot_key(slot.as_str())
            .map(|slot| RefPath::Input { slot }),
        RefPath::Fact { key } => Some(RefPath::Fact { key }),
        RefPath::NodeOutput {
            step_id,
            field_path,
        } => Some(RefPath::NodeOutput {
            step_id,
            field_path,
        }),
    }
}

fn binding_types_compatible(
    target: &RefPath,
    source_type: &str,
    catalog: &BTreeMap<String, &super::super::ref_catalog::RefCatalogEntry>,
) -> bool {
    let expected = target_expected_type(target, catalog);
    let source = normalize_type_label(source_type);
    match expected {
        "unknown" | "text" => true,
        "address" => matches!(source, "address" | "text" | "unknown"),
        "numeric" => matches!(source, "numeric" | "text" | "unknown"),
        "chain" => matches!(source, "chain" | "text" | "unknown"),
        "boolean" => matches!(source, "boolean" | "text" | "unknown"),
        _ => true,
    }
}

fn target_expected_type(
    target: &RefPath,
    catalog: &BTreeMap<String, &super::super::ref_catalog::RefCatalogEntry>,
) -> &'static str {
    let target_ref = target.as_canonical_str();
    if let Some(entry) = catalog.get(target_ref.as_str()) {
        let typed = normalize_type_label(entry.value_type.as_str());
        if typed != "unknown" {
            return typed;
        }
    }
    let semantic_key = match target {
        RefPath::Input { slot } => slot.to_ascii_lowercase(),
        RefPath::Fact { key } => key.to_ascii_lowercase(),
        RefPath::NodeOutput { field_path, .. } => field_path.to_ascii_lowercase(),
    };
    if semantic_key.contains("address")
        || semantic_key.contains("owner")
        || semantic_key.contains("recipient")
        || semantic_key.contains("spender")
        || semantic_key.contains("wallet")
    {
        return "address";
    }
    if semantic_key.contains("decimals")
        || semantic_key.contains("amount")
        || semantic_key.contains("balance")
        || semantic_key.contains("threshold")
        || semantic_key.contains("limit")
        || semantic_key.contains("price")
        || semantic_key.contains("fee")
        || semantic_key.contains("ratio")
        || semantic_key.contains("count")
        || semantic_key.contains("nonce")
    {
        return "numeric";
    }
    if semantic_key.contains("chain") || semantic_key.contains("network") {
        return "chain";
    }
    if semantic_key.starts_with("is_")
        || semantic_key.contains("enabled")
        || semantic_key.contains("allow")
        || semantic_key.contains("blocked")
    {
        return "boolean";
    }
    "unknown"
}

fn normalize_type_label(raw: &str) -> &'static str {
    match raw.trim().to_ascii_lowercase().as_str() {
        "address" => "address",
        "numeric" | "number" | "integer" | "float" => "numeric",
        "chain" => "chain",
        "boolean" | "bool" => "boolean",
        "text" | "string" => "text",
        _ => "unknown",
    }
}

fn graph_has_path(graph: &BTreeMap<String, BTreeSet<String>>, from: &str, to: &str) -> bool {
    if from == to {
        return true;
    }
    let mut stack = vec![from.to_string()];
    let mut visited = BTreeSet::<String>::new();
    while let Some(node) = stack.pop() {
        if !visited.insert(node.clone()) {
            continue;
        }
        for next in graph
            .get(node.as_str())
            .into_iter()
            .flat_map(|items| items.iter())
        {
            if next == to {
                return true;
            }
            stack.push(next.clone());
        }
    }
    false
}

#[cfg(test)]
#[path = "../tests/missing_resolution_policy.rs"]
mod tests;
