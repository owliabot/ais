use super::super::input_normalize;
use super::super::ref_model::RefPath;
use super::policy::MissingResolutionDecision;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct MissingResolutionBindAction {
    pub target: RefPath,
    pub source: RefPath,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct MissingResolutionAskUserAction {
    pub target: RefPath,
    pub question: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct MissingResolutionRunProducerAction {
    pub target: RefPath,
    pub query_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub(crate) struct MissingResolutionExecutionPlan {
    pub bindings: Vec<MissingResolutionBindAction>,
    pub run_producers: Vec<MissingResolutionRunProducerAction>,
    pub query_refs: Vec<String>,
    pub ask_user: Vec<MissingResolutionAskUserAction>,
    pub abort_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub(crate) struct MissingResolutionBindingExecution {
    pub resolved_targets: Vec<String>,
    pub unresolved_targets: Vec<String>,
    pub issues: Vec<String>,
}

pub(crate) fn build_missing_resolution_execution_plan(
    decisions: &[MissingResolutionDecision],
) -> MissingResolutionExecutionPlan {
    let mut plan = MissingResolutionExecutionPlan::default();
    let mut query_refs = BTreeSet::<String>::new();
    for decision in decisions {
        match decision {
            MissingResolutionDecision::BindFromRef { target, source } => {
                plan.bindings.push(MissingResolutionBindAction {
                    target: target.clone(),
                    source: source.clone(),
                });
            }
            MissingResolutionDecision::RunProducer { target, query_ref } => {
                let query_ref = query_ref.trim();
                if !query_ref.is_empty() {
                    query_refs.insert(query_ref.to_string());
                    plan.run_producers.push(MissingResolutionRunProducerAction {
                        target: target.clone(),
                        query_ref: query_ref.to_string(),
                    });
                }
            }
            MissingResolutionDecision::AskUser { target, question } => {
                plan.ask_user.push(MissingResolutionAskUserAction {
                    target: target.clone(),
                    question: question.clone(),
                });
            }
            MissingResolutionDecision::Abort { reason } => {
                let reason = reason.trim();
                if !reason.is_empty() && plan.abort_reason.is_none() {
                    plan.abort_reason = Some(reason.to_string());
                }
            }
        }
    }
    plan.query_refs = query_refs.into_iter().collect::<Vec<_>>();
    plan
}

pub(crate) fn apply_missing_resolution_bindings(
    runtime: &mut Value,
    input_store: &mut super::super::InputStore,
    state_summary: Option<&Value>,
    bindings: &[MissingResolutionBindAction],
    provenance_scope: &str,
) -> MissingResolutionBindingExecution {
    let mut resolved_targets = BTreeSet::<String>::new();
    let mut unresolved_targets = BTreeSet::<String>::new();
    let mut issues = Vec::<String>::new();
    for binding in bindings {
        let source_ref = binding.source.as_canonical_str();
        let target_ref = binding.target.as_canonical_str();
        let Some(value) = read_ref_value(state_summary, &binding.source) else {
            unresolved_targets.insert(target_ref.clone());
            issues.push(format!(
                "bind_source_value_missing:source={source_ref}:target={target_ref}"
            ));
            continue;
        };
        if apply_target_value(
            runtime,
            input_store,
            &binding.target,
            value,
            provenance_scope,
            source_ref.as_str(),
        ) {
            resolved_targets.insert(target_ref);
        } else {
            unresolved_targets.insert(target_ref.clone());
            issues.push(format!(
                "bind_target_not_writable:source={source_ref}:target={target_ref}"
            ));
        }
    }

    MissingResolutionBindingExecution {
        resolved_targets: resolved_targets.into_iter().collect::<Vec<_>>(),
        unresolved_targets: unresolved_targets.into_iter().collect::<Vec<_>>(),
        issues,
    }
}

pub(crate) fn set_runtime_intent_fact(runtime: &mut Value, key: &str, value: Value) {
    if !runtime.is_object() {
        *runtime = Value::Object(Map::new());
    }
    let Some(root) = runtime.as_object_mut() else {
        return;
    };
    let agent = root
        .entry("agent".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if !agent.is_object() {
        *agent = Value::Object(Map::new());
    }
    let Some(agent_obj) = agent.as_object_mut() else {
        return;
    };
    let intent_grounding = agent_obj
        .entry("intent_grounding".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if !intent_grounding.is_object() {
        *intent_grounding = Value::Object(Map::new());
    }
    let Some(grounding_obj) = intent_grounding.as_object_mut() else {
        return;
    };
    let intent_facts = grounding_obj
        .entry("intent_facts".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if !intent_facts.is_object() {
        *intent_facts = Value::Object(Map::new());
    }
    let Some(facts_obj) = intent_facts.as_object_mut() else {
        return;
    };
    let segments = key
        .split('.')
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    if segments.is_empty() {
        return;
    }
    set_nested_object_value(facts_obj, segments.as_slice(), value);
}

fn read_ref_value(state_summary: Option<&Value>, reference: &RefPath) -> Option<Value> {
    let summary = state_summary?;
    match reference {
        RefPath::Input { slot } => summary
            .pointer("/input_store/facts")
            .and_then(|facts| value_at_dotted_path(facts, slot.as_str()))
            .cloned()
            .map(unwrap_input_value),
        RefPath::Fact { key } => {
            let full_ref = format!("facts.{key}");
            summary
                .pointer("/runtime_facts/facts")
                .and_then(|facts| {
                    facts
                        .as_object()
                        .and_then(|object| object.get(full_ref.as_str()))
                        .or_else(|| value_at_dotted_path(facts, full_ref.as_str()))
                })
                .cloned()
        }
        RefPath::NodeOutput {
            step_id,
            field_path,
        } => {
            let escaped = step_id.replace('~', "~0").replace('/', "~1");
            summary
                .pointer(format!("/nodes/{escaped}/outputs").as_str())
                .and_then(|outputs| value_at_dotted_path(outputs, field_path.as_str()))
                .cloned()
        }
    }
}

fn apply_target_value(
    runtime: &mut Value,
    input_store: &mut super::super::InputStore,
    target: &RefPath,
    value: Value,
    provenance_scope: &str,
    source_ref: &str,
) -> bool {
    let target_ref = target.as_canonical_str();
    match target {
        RefPath::Input { slot } => {
            input_normalize::set_runtime_input_value(runtime, slot.as_str(), value.clone());
            let _ = super::super::upsert_store_value_with_source(
                input_store,
                slot.as_str(),
                value,
                super::super::input_store::InputValueLayer::Derived,
                "host.missing_resolution_executor",
                90,
                format!("recovery.bind.{provenance_scope}.{source_ref}->{target_ref}"),
            );
            true
        }
        RefPath::Fact { key } => {
            let _ = input_store;
            set_runtime_intent_fact(runtime, key.as_str(), value);
            true
        }
        RefPath::NodeOutput { .. } => false,
    }
}

fn value_at_dotted_path<'a>(value: &'a Value, dotted: &str) -> Option<&'a Value> {
    if dotted.is_empty() {
        return None;
    }
    let mut current = value;
    for segment in dotted.split('.') {
        if segment.is_empty() {
            continue;
        }
        current = current.get(segment)?;
    }
    Some(current)
}

fn unwrap_input_value(value: Value) -> Value {
    value
        .as_object()
        .and_then(|object| object.get("value"))
        .cloned()
        .unwrap_or(value)
}

fn set_nested_object_value(root: &mut Map<String, Value>, path: &[&str], value: Value) {
    if path.is_empty() {
        return;
    }
    if path.len() == 1 {
        root.insert(path[0].to_string(), value);
        return;
    }
    let child = root
        .entry(path[0].to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if !child.is_object() {
        *child = Value::Object(Map::new());
    }
    if let Some(child_obj) = child.as_object_mut() {
        set_nested_object_value(child_obj, &path[1..], value);
    }
}

#[cfg(test)]
#[path = "../tests/missing_resolution_executor.rs"]
mod tests;
