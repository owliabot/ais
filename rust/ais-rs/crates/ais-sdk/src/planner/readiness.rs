use crate::resolver::{
    evaluate_value_ref_async, evaluate_value_ref_with_options, resolve_calculated_bindings,
    resolve_calculated_bindings_async, resolve_node_bindings, resolve_query_bindings,
    ResolverContext, ValueRef, ValueRefEvalError, ValueRefEvalOptions,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeRunState {
    Ready,
    Blocked,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeReadinessResult {
    pub state: NodeRunState,
    #[serde(default)]
    pub missing_refs: Vec<String>,
    #[serde(default)]
    pub errors: Vec<String>,
    #[serde(default)]
    pub resolved_params: Option<Map<String, Value>>,
}

pub fn get_node_readiness(
    node: &Value,
    context: &ResolverContext,
    options: &ValueRefEvalOptions,
) -> NodeReadinessResult {
    let prepared = prepare_node_eval_sync(node, context, options);
    let condition_options = prepared.eval_options.as_ref().unwrap_or(options);
    if let Some(condition) = node.as_object().and_then(|object| object.get("condition")) {
        let condition_ref = match parse_value_ref(condition, "condition") {
            Ok(value_ref) => value_ref,
            Err(error) => {
                return NodeReadinessResult {
                    state: NodeRunState::Blocked,
                    missing_refs: Vec::new(),
                    errors: vec![error],
                    resolved_params: None,
                };
            }
        };
        match safe_eval_sync(&condition_ref, context, condition_options) {
            SafeEvalResult::Err {
                missing_refs,
                errors,
            } => {
                return NodeReadinessResult {
                    state: NodeRunState::Blocked,
                    missing_refs,
                    errors,
                    resolved_params: prepared.resolved_params.clone(),
                };
            }
            SafeEvalResult::Ok { value } => match value {
                Value::Bool(false) => {
                    return NodeReadinessResult {
                        state: NodeRunState::Skipped,
                        missing_refs: Vec::new(),
                        errors: Vec::new(),
                        resolved_params: prepared.resolved_params.clone(),
                    };
                }
                Value::Bool(true) => {}
                _ => {
                    return NodeReadinessResult {
                        state: NodeRunState::Blocked,
                        missing_refs: Vec::new(),
                        errors: vec![format!(
                            "condition must evaluate to boolean, got: {}",
                            json_type_name(&value)
                        )],
                        resolved_params: prepared.resolved_params.clone(),
                    };
                }
            },
        }
    }

    let resolved_params = prepared.resolved_params.unwrap_or_default();
    let mut missing_refs = prepared.missing_refs;
    let mut errors = prepared.errors;

    if let Some(execution) = node.as_object().and_then(|object| object.get("execution")) {
        let execution_options = prepared.eval_options.as_ref().unwrap_or(options);
        for value_ref in collect_value_refs_deep(execution) {
            match safe_eval_sync(&value_ref, context, execution_options) {
                SafeEvalResult::Ok { .. } => {}
                SafeEvalResult::Err {
                    missing_refs: eval_missing_refs,
                    errors: eval_errors,
                } => {
                    missing_refs.extend(eval_missing_refs);
                    errors.extend(eval_errors);
                }
            }
        }
    }

    missing_refs = dedup_sort_strings(missing_refs);

    if !missing_refs.is_empty() || !errors.is_empty() {
        return NodeReadinessResult {
            state: NodeRunState::Blocked,
            missing_refs,
            errors,
            resolved_params: Some(resolved_params),
        };
    }

    NodeReadinessResult {
        state: NodeRunState::Ready,
        missing_refs: Vec::new(),
        errors: Vec::new(),
        resolved_params: Some(resolved_params),
    }
}

pub async fn get_node_readiness_async(
    node: &Value,
    context: &ResolverContext,
    options: &ValueRefEvalOptions,
) -> NodeReadinessResult {
    let prepared = prepare_node_eval_async(node, context, options).await;
    let condition_options = prepared.eval_options.as_ref().unwrap_or(options);
    if let Some(condition) = node.as_object().and_then(|object| object.get("condition")) {
        let condition_ref = match parse_value_ref(condition, "condition") {
            Ok(value_ref) => value_ref,
            Err(error) => {
                return NodeReadinessResult {
                    state: NodeRunState::Blocked,
                    missing_refs: Vec::new(),
                    errors: vec![error],
                    resolved_params: None,
                };
            }
        };
        match safe_eval_async(&condition_ref, context, condition_options).await {
            SafeEvalResult::Err {
                missing_refs,
                errors,
            } => {
                return NodeReadinessResult {
                    state: NodeRunState::Blocked,
                    missing_refs,
                    errors,
                    resolved_params: prepared.resolved_params.clone(),
                };
            }
            SafeEvalResult::Ok { value } => match value {
                Value::Bool(false) => {
                    return NodeReadinessResult {
                        state: NodeRunState::Skipped,
                        missing_refs: Vec::new(),
                        errors: Vec::new(),
                        resolved_params: prepared.resolved_params.clone(),
                    };
                }
                Value::Bool(true) => {}
                _ => {
                    return NodeReadinessResult {
                        state: NodeRunState::Blocked,
                        missing_refs: Vec::new(),
                        errors: vec![format!(
                            "condition must evaluate to boolean, got: {}",
                            json_type_name(&value)
                        )],
                        resolved_params: prepared.resolved_params.clone(),
                    };
                }
            },
        }
    }

    let resolved_params = prepared.resolved_params.unwrap_or_default();
    let mut missing_refs = prepared.missing_refs;
    let mut errors = prepared.errors;

    if let Some(execution) = node.as_object().and_then(|object| object.get("execution")) {
        let execution_options = prepared.eval_options.as_ref().unwrap_or(options);
        for value_ref in collect_value_refs_deep(execution) {
            match safe_eval_async(&value_ref, context, execution_options).await {
                SafeEvalResult::Ok { .. } => {}
                SafeEvalResult::Err {
                    missing_refs: eval_missing_refs,
                    errors: eval_errors,
                } => {
                    missing_refs.extend(eval_missing_refs);
                    errors.extend(eval_errors);
                }
            }
        }
    }

    missing_refs = dedup_sort_strings(missing_refs);

    if !missing_refs.is_empty() || !errors.is_empty() {
        return NodeReadinessResult {
            state: NodeRunState::Blocked,
            missing_refs,
            errors,
            resolved_params: Some(resolved_params),
        };
    }

    NodeReadinessResult {
        state: NodeRunState::Ready,
        missing_refs: Vec::new(),
        errors: Vec::new(),
        resolved_params: Some(resolved_params),
    }
}

pub fn value_ref_eval_options_for_node(
    node: &Value,
    options: &ValueRefEvalOptions,
    resolved_params: Option<&Map<String, Value>>,
) -> ValueRefEvalOptions {
    resolve_node_bindings(node, None, resolved_params, None).to_eval_options(options)
}

#[derive(Debug, Clone, Default)]
struct PreparedNodeEval {
    resolved_params: Option<Map<String, Value>>,
    missing_refs: Vec<String>,
    errors: Vec<String>,
    eval_options: Option<ValueRefEvalOptions>,
}

fn prepare_node_eval_sync(
    node: &Value,
    context: &ResolverContext,
    options: &ValueRefEvalOptions,
) -> PreparedNodeEval {
    let node_options =
        resolve_node_bindings(node, Some(&context.runtime), None, None).to_eval_options(options);
    let (resolved_params, mut missing_refs, mut errors) =
        resolve_params_sync(node, context, &node_options);
    let resolved_params = has_param_bindings(node).then_some(resolved_params);
    let calculated = resolve_calculated_bindings(node, context, options, resolved_params.as_ref());
    let query = resolve_query_bindings(node, Some(&context.runtime));
    missing_refs.extend(query.missing_refs.iter().cloned());
    missing_refs.extend(calculated.missing_refs.iter().cloned());
    errors.extend(calculated.errors.iter().cloned());
    let calculated_root = node
        .get("calculated_overrides")
        .and_then(Value::as_object)
        .map(|_| &calculated.calculated);
    let eval_options = resolve_node_bindings(
        node,
        Some(&context.runtime),
        resolved_params.as_ref(),
        calculated_root,
    )
    .to_eval_options(options);
    PreparedNodeEval {
        resolved_params,
        missing_refs,
        errors,
        eval_options: Some(eval_options),
    }
}

async fn prepare_node_eval_async(
    node: &Value,
    context: &ResolverContext,
    options: &ValueRefEvalOptions,
) -> PreparedNodeEval {
    let node_options =
        resolve_node_bindings(node, Some(&context.runtime), None, None).to_eval_options(options);
    let (resolved_params, mut missing_refs, mut errors) =
        resolve_params_async(node, context, &node_options).await;
    let resolved_params = has_param_bindings(node).then_some(resolved_params);
    let calculated =
        resolve_calculated_bindings_async(node, context, options, resolved_params.as_ref()).await;
    let query = resolve_query_bindings(node, Some(&context.runtime));
    missing_refs.extend(query.missing_refs.iter().cloned());
    missing_refs.extend(calculated.missing_refs.iter().cloned());
    errors.extend(calculated.errors.iter().cloned());
    let calculated_root = node
        .get("calculated_overrides")
        .and_then(Value::as_object)
        .map(|_| &calculated.calculated);
    let eval_options = resolve_node_bindings(
        node,
        Some(&context.runtime),
        resolved_params.as_ref(),
        calculated_root,
    )
    .to_eval_options(options);
    PreparedNodeEval {
        resolved_params,
        missing_refs,
        errors,
        eval_options: Some(eval_options),
    }
}

fn has_param_bindings(node: &Value) -> bool {
    node.pointer("/bindings/params")
        .and_then(Value::as_object)
        .is_some()
}

fn resolve_params_sync(
    node: &Value,
    context: &ResolverContext,
    options: &ValueRefEvalOptions,
) -> (Map<String, Value>, Vec<String>, Vec<String>) {
    let mut resolved_params = Map::new();
    let mut missing_refs = Vec::<String>::new();
    let mut errors = Vec::<String>::new();

    if let Some(params) = node
        .as_object()
        .and_then(|object| object.get("bindings"))
        .and_then(Value::as_object)
        .and_then(|bindings| bindings.get("params"))
        .and_then(Value::as_object)
    {
        for (key, value) in params {
            match parse_value_ref(value, &format!("bindings.params.{key}")) {
                Ok(value_ref) => match safe_eval_sync(&value_ref, context, options) {
                    SafeEvalResult::Ok { value } => {
                        resolved_params.insert(key.clone(), value);
                    }
                    SafeEvalResult::Err {
                        missing_refs: eval_missing_refs,
                        errors: eval_errors,
                    } => {
                        missing_refs.extend(eval_missing_refs);
                        errors.extend(eval_errors);
                    }
                },
                Err(error) => errors.push(error),
            }
        }
    }

    (resolved_params, missing_refs, errors)
}

async fn resolve_params_async(
    node: &Value,
    context: &ResolverContext,
    options: &ValueRefEvalOptions,
) -> (Map<String, Value>, Vec<String>, Vec<String>) {
    let mut resolved_params = Map::new();
    let mut missing_refs = Vec::<String>::new();
    let mut errors = Vec::<String>::new();

    if let Some(params) = node
        .as_object()
        .and_then(|object| object.get("bindings"))
        .and_then(Value::as_object)
        .and_then(|bindings| bindings.get("params"))
        .and_then(Value::as_object)
    {
        for (key, value) in params {
            match parse_value_ref(value, &format!("bindings.params.{key}")) {
                Ok(value_ref) => match safe_eval_async(&value_ref, context, options).await {
                    SafeEvalResult::Ok { value } => {
                        resolved_params.insert(key.clone(), value);
                    }
                    SafeEvalResult::Err {
                        missing_refs: eval_missing_refs,
                        errors: eval_errors,
                    } => {
                        missing_refs.extend(eval_missing_refs);
                        errors.extend(eval_errors);
                    }
                },
                Err(error) => errors.push(error),
            }
        }
    }

    (resolved_params, missing_refs, errors)
}

#[derive(Debug, Clone, PartialEq)]
enum SafeEvalResult {
    Ok {
        value: Value,
    },
    Err {
        missing_refs: Vec<String>,
        errors: Vec<String>,
    },
}

fn safe_eval_sync(
    value_ref: &ValueRef,
    context: &ResolverContext,
    options: &ValueRefEvalOptions,
) -> SafeEvalResult {
    match evaluate_value_ref_with_options(value_ref, context, options) {
        Ok(value) => SafeEvalResult::Ok { value },
        Err(error) => map_eval_error(error),
    }
}

async fn safe_eval_async(
    value_ref: &ValueRef,
    context: &ResolverContext,
    options: &ValueRefEvalOptions,
) -> SafeEvalResult {
    match evaluate_value_ref_async(value_ref, context, options).await {
        Ok(value) => SafeEvalResult::Ok { value },
        Err(error) => map_eval_error(error),
    }
}

fn map_eval_error(error: ValueRefEvalError) -> SafeEvalResult {
    match error {
        ValueRefEvalError::MissingRef { path, .. } => SafeEvalResult::Err {
            missing_refs: vec![path],
            errors: Vec::new(),
        },
        ValueRefEvalError::CelEvaluationFailed { expression, reason } => SafeEvalResult::Err {
            missing_refs: Vec::new(),
            errors: vec![format!(
                "CEL evaluation failed for `{expression}`: {reason}"
            )],
        },
    }
}

fn parse_value_ref(value: &Value, path: &str) -> Result<ValueRef, String> {
    serde_json::from_value::<ValueRef>(value.clone())
        .map_err(|error| format!("invalid ValueRef at `{path}`: {error}"))
}

fn collect_value_refs_deep(value: &Value) -> Vec<ValueRef> {
    let mut out = Vec::<ValueRef>::new();
    walk_collect_value_refs(value, &mut out);
    out
}

fn walk_collect_value_refs(value: &Value, out: &mut Vec<ValueRef>) {
    if let Some(value_ref) = parse_value_ref_like(value) {
        out.push(value_ref);
        return;
    }

    match value {
        Value::Array(items) => {
            for item in items {
                walk_collect_value_refs(item, out);
            }
        }
        Value::Object(object) => {
            for value in object.values() {
                walk_collect_value_refs(value, out);
            }
        }
        _ => {}
    }
}

fn parse_value_ref_like(value: &Value) -> Option<ValueRef> {
    let object = value.as_object()?;
    if object.len() != 1 {
        return None;
    }
    let key = object.keys().next()?;
    if !matches!(key.as_str(), "lit" | "ref" | "cel" | "object" | "array") {
        return None;
    }
    serde_json::from_value::<ValueRef>(value.clone()).ok()
}

fn dedup_sort_strings(values: Vec<String>) -> Vec<String> {
    values
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn json_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[cfg(test)]
#[path = "readiness_test.rs"]
mod tests;
