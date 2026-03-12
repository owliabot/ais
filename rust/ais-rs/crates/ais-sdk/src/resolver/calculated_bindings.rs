use super::{
    evaluate_value_ref_async, evaluate_value_ref_with_options, resolve_node_bindings,
    ResolverContext, ValueRef, ValueRefEvalError, ValueRefEvalOptions,
};
use serde_json::{Map, Value};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct CalculatedBindingsResult {
    pub calculated: Map<String, Value>,
    pub missing_refs: Vec<String>,
    pub errors: Vec<String>,
}

pub fn resolve_calculated_bindings(
    node: &Value,
    context: &ResolverContext,
    options: &ValueRefEvalOptions,
    resolved_params: Option<&Map<String, Value>>,
) -> CalculatedBindingsResult {
    let Some(overrides) = calculated_entries(node) else {
        return CalculatedBindingsResult::default();
    };
    let order = calculated_order(node, overrides);
    let mut result = CalculatedBindingsResult::default();

    for key in order {
        let Some(entry) = overrides.get(key.as_str()).and_then(Value::as_object) else {
            continue;
        };
        let Some(expr) = entry.get("expr") else {
            result
                .errors
                .push(format!("calculated `{key}` is missing expr"));
            continue;
        };
        let value_ref = match serde_json::from_value::<ValueRef>(expr.clone()) {
            Ok(value_ref) => value_ref,
            Err(error) => {
                result
                    .errors
                    .push(format!("calculated `{key}` has invalid expr: {error}"));
                continue;
            }
        };
        let eval_options = resolve_node_bindings(
            node,
            Some(&context.runtime),
            resolved_params,
            Some(&result.calculated),
        )
        .to_eval_options(options);
        match evaluate_value_ref_with_options(&value_ref, context, &eval_options) {
            Ok(value) => {
                result.calculated.insert(key, value);
            }
            Err(error) => extend_result_with_eval_error(&mut result, error),
        }
    }

    result.missing_refs.sort();
    result.missing_refs.dedup();
    result
}

pub async fn resolve_calculated_bindings_async(
    node: &Value,
    context: &ResolverContext,
    options: &ValueRefEvalOptions,
    resolved_params: Option<&Map<String, Value>>,
) -> CalculatedBindingsResult {
    let Some(overrides) = calculated_entries(node) else {
        return CalculatedBindingsResult::default();
    };
    let order = calculated_order(node, overrides);
    let mut result = CalculatedBindingsResult::default();

    for key in order {
        let Some(entry) = overrides.get(key.as_str()).and_then(Value::as_object) else {
            continue;
        };
        let Some(expr) = entry.get("expr") else {
            result
                .errors
                .push(format!("calculated `{key}` is missing expr"));
            continue;
        };
        let value_ref = match serde_json::from_value::<ValueRef>(expr.clone()) {
            Ok(value_ref) => value_ref,
            Err(error) => {
                result
                    .errors
                    .push(format!("calculated `{key}` has invalid expr: {error}"));
                continue;
            }
        };
        let eval_options = resolve_node_bindings(
            node,
            Some(&context.runtime),
            resolved_params,
            Some(&result.calculated),
        )
        .to_eval_options(options);
        match evaluate_value_ref_async(&value_ref, context, &eval_options).await {
            Ok(value) => {
                result.calculated.insert(key, value);
            }
            Err(error) => extend_result_with_eval_error(&mut result, error),
        }
    }

    result.missing_refs.sort();
    result.missing_refs.dedup();
    result
}

fn calculated_entries(node: &Value) -> Option<&Map<String, Value>> {
    node.get("calculated_overrides").and_then(Value::as_object)
}

fn calculated_order(node: &Value, overrides: &Map<String, Value>) -> Vec<String> {
    let mut order = node
        .get("calculated_override_order")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if order.is_empty() {
        order = overrides.keys().cloned().collect::<Vec<_>>();
        order.sort();
    }
    order
}

fn extend_result_with_eval_error(result: &mut CalculatedBindingsResult, error: ValueRefEvalError) {
    match error {
        ValueRefEvalError::MissingRef { path, .. } => result.missing_refs.push(path),
        ValueRefEvalError::CelEvaluationFailed { expression, reason } => result.errors.push(
            format!("CEL evaluation failed for `{expression}`: {reason}"),
        ),
    }
}
