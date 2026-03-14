use ais_agent_control::execution_artifact::ValueRef;
use ais_agent_expr::cel::{CelEvaluator, CelScope};
use serde_json::Value;

use crate::{runtime::ActiveRun, runtime_expr_scope::artifact_refs_value};

pub(crate) fn resolve_value_ref(
    runtime: &ActiveRun,
    value_ref: &ValueRef,
) -> Result<Value, String> {
    match value_ref {
        ValueRef::Literal { value } => Ok(value.clone()),
        ValueRef::Ref { reference } => resolve_reference(runtime, reference),
        ValueRef::Cel { expression } => evaluate_cel_value(runtime, expression),
    }
}

pub(crate) fn resolve_reference(runtime: &ActiveRun, reference: &str) -> Result<Value, String> {
    let refs = artifact_refs_value(runtime)?;
    if reference == "refs" {
        return Ok(refs);
    }
    let Some(path) = reference.strip_prefix("refs.") else {
        return Err(format!(
            "execution_artifact reference `{reference}` must start with `refs.`"
        ));
    };

    refs.pointer(&json_pointer(path))
        .cloned()
        .ok_or_else(|| format!("execution_artifact reference `{reference}` is not available"))
}

pub(crate) fn cel_scope_with_refs(runtime: &ActiveRun) -> Result<CelScope, String> {
    let mut scope = CelScope::new();
    scope.insert_json("refs", artifact_refs_value(runtime)?);
    Ok(scope)
}

pub(crate) fn evaluate_cel_value(runtime: &ActiveRun, expression: &str) -> Result<Value, String> {
    let mut evaluator = CelEvaluator::new();
    let value = evaluator
        .evaluate_value(expression, &cel_scope_with_refs(runtime)?)
        .map_err(|error| format!("execution_artifact CEL `{expression}` failed: {error}"))?;
    cel_value_to_json(value)
}

fn cel_value_to_json(value: ais_agent_expr::cel::CelValue) -> Result<Value, String> {
    match value {
        ais_agent_expr::cel::CelValue::Null => Ok(Value::Null),
        ais_agent_expr::cel::CelValue::Bool(value) => Ok(Value::Bool(value)),
        ais_agent_expr::cel::CelValue::Integer(value) => Ok(Value::String(value.to_string())),
        ais_agent_expr::cel::CelValue::Decimal(value) => Ok(Value::String(value.to_string())),
        ais_agent_expr::cel::CelValue::String(value) => Ok(Value::String(value)),
        ais_agent_expr::cel::CelValue::List(values) => values
            .into_iter()
            .map(cel_value_to_json)
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        ais_agent_expr::cel::CelValue::Map(values) => values
            .into_iter()
            .map(|(key, value)| cel_value_to_json(value).map(|value| (key, value)))
            .collect::<Result<serde_json::Map<_, _>, _>>()
            .map(Value::Object),
    }
}

fn json_pointer(path: &str) -> String {
    let mut pointer = String::new();
    for segment in path.split('.') {
        if segment.is_empty() {
            continue;
        }
        pointer.push('/');
        pointer.push_str(&segment.replace('~', "~0").replace('/', "~1"));
    }
    pointer
}
