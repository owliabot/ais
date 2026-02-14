use crate::resolver::{
    evaluate_value_ref_async, evaluate_value_ref_with_options, DetectResolver, ResolverContext,
    ValueRef, ValueRefEvalError, ValueRefEvalOptions,
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
    pub needs_detect: bool,
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
    if let Some(condition) = node.as_object().and_then(|object| object.get("condition")) {
        let condition_ref = match parse_value_ref(condition, "condition") {
            Ok(value_ref) => value_ref,
            Err(error) => {
                return NodeReadinessResult {
                    state: NodeRunState::Blocked,
                    missing_refs: Vec::new(),
                    needs_detect: false,
                    errors: vec![error],
                    resolved_params: None,
                };
            }
        };
        match safe_eval_sync(&condition_ref, context, options) {
            SafeEvalResult::Err {
                missing_refs,
                needs_detect,
                errors,
            } => {
                return NodeReadinessResult {
                    state: NodeRunState::Blocked,
                    missing_refs,
                    needs_detect,
                    errors,
                    resolved_params: None,
                };
            }
            SafeEvalResult::Ok { value } => match value {
                Value::Bool(false) => {
                    return NodeReadinessResult {
                        state: NodeRunState::Skipped,
                        missing_refs: Vec::new(),
                        needs_detect: false,
                        errors: Vec::new(),
                        resolved_params: None,
                    };
                }
                Value::Bool(true) => {}
                _ => {
                    return NodeReadinessResult {
                        state: NodeRunState::Blocked,
                        missing_refs: Vec::new(),
                        needs_detect: false,
                        errors: vec![format!(
                            "condition must evaluate to boolean, got: {}",
                            json_type_name(&value)
                        )],
                        resolved_params: None,
                    };
                }
            },
        }
    }

    let mut resolved_params = Map::new();
    let mut missing_refs = Vec::<String>::new();
    let mut needs_detect = false;
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
                        needs_detect: eval_needs_detect,
                        errors: eval_errors,
                    } => {
                        missing_refs.extend(eval_missing_refs);
                        needs_detect = needs_detect || eval_needs_detect;
                        errors.extend(eval_errors);
                    }
                },
                Err(error) => errors.push(error),
            }
        }
    }

    if let Some(execution) = node.as_object().and_then(|object| object.get("execution")) {
        let execution_options = options_with_resolved_params(options, &resolved_params);
        for value_ref in collect_value_refs_deep(execution) {
            match safe_eval_sync(&value_ref, context, &execution_options) {
                SafeEvalResult::Ok { .. } => {}
                SafeEvalResult::Err {
                    missing_refs: eval_missing_refs,
                    needs_detect: eval_needs_detect,
                    errors: eval_errors,
                } => {
                    missing_refs.extend(eval_missing_refs);
                    needs_detect = needs_detect || eval_needs_detect;
                    errors.extend(eval_errors);
                }
            }
        }
    }

    missing_refs = dedup_sort_strings(missing_refs);

    if !missing_refs.is_empty() || needs_detect || !errors.is_empty() {
        return NodeReadinessResult {
            state: NodeRunState::Blocked,
            missing_refs,
            needs_detect,
            errors,
            resolved_params: Some(resolved_params),
        };
    }

    NodeReadinessResult {
        state: NodeRunState::Ready,
        missing_refs: Vec::new(),
        needs_detect: false,
        errors: Vec::new(),
        resolved_params: Some(resolved_params),
    }
}

pub async fn get_node_readiness_async(
    node: &Value,
    context: &ResolverContext,
    options: &ValueRefEvalOptions,
    detect_resolver: Option<&dyn DetectResolver>,
) -> NodeReadinessResult {
    if let Some(condition) = node.as_object().and_then(|object| object.get("condition")) {
        let condition_ref = match parse_value_ref(condition, "condition") {
            Ok(value_ref) => value_ref,
            Err(error) => {
                return NodeReadinessResult {
                    state: NodeRunState::Blocked,
                    missing_refs: Vec::new(),
                    needs_detect: false,
                    errors: vec![error],
                    resolved_params: None,
                };
            }
        };
        match safe_eval_async(&condition_ref, context, options, detect_resolver).await {
            SafeEvalResult::Err {
                missing_refs,
                needs_detect,
                errors,
            } => {
                return NodeReadinessResult {
                    state: NodeRunState::Blocked,
                    missing_refs,
                    needs_detect,
                    errors,
                    resolved_params: None,
                };
            }
            SafeEvalResult::Ok { value } => match value {
                Value::Bool(false) => {
                    return NodeReadinessResult {
                        state: NodeRunState::Skipped,
                        missing_refs: Vec::new(),
                        needs_detect: false,
                        errors: Vec::new(),
                        resolved_params: None,
                    };
                }
                Value::Bool(true) => {}
                _ => {
                    return NodeReadinessResult {
                        state: NodeRunState::Blocked,
                        missing_refs: Vec::new(),
                        needs_detect: false,
                        errors: vec![format!(
                            "condition must evaluate to boolean, got: {}",
                            json_type_name(&value)
                        )],
                        resolved_params: None,
                    };
                }
            },
        }
    }

    let mut resolved_params = Map::new();
    let mut missing_refs = Vec::<String>::new();
    let mut needs_detect = false;
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
                Ok(value_ref) => {
                    match safe_eval_async(&value_ref, context, options, detect_resolver).await {
                        SafeEvalResult::Ok { value } => {
                            resolved_params.insert(key.clone(), value);
                        }
                        SafeEvalResult::Err {
                            missing_refs: eval_missing_refs,
                            needs_detect: eval_needs_detect,
                            errors: eval_errors,
                        } => {
                            missing_refs.extend(eval_missing_refs);
                            needs_detect = needs_detect || eval_needs_detect;
                            errors.extend(eval_errors);
                        }
                    }
                }
                Err(error) => errors.push(error),
            }
        }
    }

    if let Some(execution) = node.as_object().and_then(|object| object.get("execution")) {
        let execution_options = options_with_resolved_params(options, &resolved_params);
        for value_ref in collect_value_refs_deep(execution) {
            match safe_eval_async(&value_ref, context, &execution_options, detect_resolver).await {
                SafeEvalResult::Ok { .. } => {}
                SafeEvalResult::Err {
                    missing_refs: eval_missing_refs,
                    needs_detect: eval_needs_detect,
                    errors: eval_errors,
                } => {
                    missing_refs.extend(eval_missing_refs);
                    needs_detect = needs_detect || eval_needs_detect;
                    errors.extend(eval_errors);
                }
            }
        }
    }

    missing_refs = dedup_sort_strings(missing_refs);

    if !missing_refs.is_empty() || needs_detect || !errors.is_empty() {
        return NodeReadinessResult {
            state: NodeRunState::Blocked,
            missing_refs,
            needs_detect,
            errors,
            resolved_params: Some(resolved_params),
        };
    }

    NodeReadinessResult {
        state: NodeRunState::Ready,
        missing_refs: Vec::new(),
        needs_detect: false,
        errors: Vec::new(),
        resolved_params: Some(resolved_params),
    }
}

fn options_with_resolved_params(
    options: &ValueRefEvalOptions,
    resolved_params: &Map<String, Value>,
) -> ValueRefEvalOptions {
    let mut root_overrides = options.root_overrides.clone();
    root_overrides.insert(
        "params".to_string(),
        Value::Object(resolved_params.clone()),
    );
    ValueRefEvalOptions { root_overrides }
}

#[derive(Debug, Clone, PartialEq)]
enum SafeEvalResult {
    Ok {
        value: Value,
    },
    Err {
        missing_refs: Vec<String>,
        needs_detect: bool,
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
    detect_resolver: Option<&dyn DetectResolver>,
) -> SafeEvalResult {
    match evaluate_value_ref_async(value_ref, context, options, detect_resolver).await {
        Ok(value) => SafeEvalResult::Ok { value },
        Err(error) => map_eval_error(error),
    }
}

fn map_eval_error(error: ValueRefEvalError) -> SafeEvalResult {
    match error {
        ValueRefEvalError::MissingRef { path, .. } => SafeEvalResult::Err {
            missing_refs: vec![path],
            needs_detect: false,
            errors: Vec::new(),
        },
        ValueRefEvalError::NeedDetect { kind } => SafeEvalResult::Err {
            missing_refs: Vec::new(),
            needs_detect: true,
            errors: vec![format!("detect requires async provider resolution: {kind}")],
        },
        ValueRefEvalError::CelEvaluationFailed { expression, reason } => SafeEvalResult::Err {
            missing_refs: Vec::new(),
            needs_detect: false,
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
    if !matches!(key.as_str(), "lit" | "ref" | "cel" | "detect" | "object" | "array") {
        return None;
    }
    serde_json::from_value::<ValueRef>(value.clone()).ok()
}

fn dedup_sort_strings(values: Vec<String>) -> Vec<String> {
    values.into_iter().collect::<BTreeSet<_>>().into_iter().collect()
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
