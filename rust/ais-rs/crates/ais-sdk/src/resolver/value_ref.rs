use ais_cel::{evaluate_expression, CelContext, CelValue};
use num_bigint::BigInt;
use futures::future::LocalBoxFuture;
use futures::FutureExt;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

use super::{ResolverContext, ResolverError};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DetectSpec {
    pub kind: String,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub candidates: Vec<Value>,
    #[serde(default)]
    pub constraints: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ValueRef {
    Lit { lit: Value },
    Ref {
        #[serde(rename = "ref")]
        ref_path: String,
    },
    Cel { cel: String },
    Detect { detect: DetectSpec },
    Object { object: BTreeMap<String, ValueRef> },
    Array { array: Vec<ValueRef> },
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ValueRefEvalError {
    #[error("missing ref: {path}")]
    MissingRef {
        path: String,
        #[source]
        source: ResolverError,
    },
    #[error("CEL evaluation failed for `{expression}`: {reason}")]
    CelEvaluationFailed { expression: String, reason: String },
    #[error("detect requires async provider resolution: {kind}")]
    NeedDetect { kind: String },
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ValueRefEvalOptions {
    #[serde(default)]
    pub root_overrides: BTreeMap<String, Value>,
}

pub trait DetectResolver {
    fn resolve<'a>(
        &'a self,
        detect: &'a DetectSpec,
        context: &'a ResolverContext,
        options: &'a ValueRefEvalOptions,
    ) -> LocalBoxFuture<'a, Result<Value, ValueRefEvalError>>;
}

pub fn evaluate_value_ref(value_ref: &ValueRef, context: &ResolverContext) -> Result<Value, ValueRefEvalError> {
    evaluate_value_ref_with_options(value_ref, context, &ValueRefEvalOptions::default())
}

pub fn evaluate_value_ref_with_options(
    value_ref: &ValueRef,
    context: &ResolverContext,
    options: &ValueRefEvalOptions,
) -> Result<Value, ValueRefEvalError> {
    match value_ref {
        ValueRef::Lit { lit } => Ok(lit.clone()),
        ValueRef::Ref { ref_path } => resolve_ref_with_overrides(ref_path, context, options),
        ValueRef::Cel { cel } => evaluate_cel(cel, context, options),
        ValueRef::Detect { detect } => Err(ValueRefEvalError::NeedDetect {
            kind: detect.kind.clone(),
        }),
        ValueRef::Object { object } => {
            let mut result = Map::new();
            for (key, child) in object {
                result.insert(key.clone(), evaluate_value_ref_with_options(child, context, options)?);
            }
            Ok(Value::Object(result))
        }
        ValueRef::Array { array } => {
            let mut out = Vec::with_capacity(array.len());
            for item in array {
                out.push(evaluate_value_ref_with_options(item, context, options)?);
            }
            Ok(Value::Array(out))
        }
    }
}

pub async fn evaluate_value_ref_async(
    value_ref: &ValueRef,
    context: &ResolverContext,
    options: &ValueRefEvalOptions,
    detect_resolver: Option<&dyn DetectResolver>,
) -> Result<Value, ValueRefEvalError> {
    evaluate_value_ref_async_boxed(value_ref, context, options, detect_resolver).await
}

fn evaluate_value_ref_async_boxed<'a>(
    value_ref: &'a ValueRef,
    context: &'a ResolverContext,
    options: &'a ValueRefEvalOptions,
    detect_resolver: Option<&'a dyn DetectResolver>,
) -> LocalBoxFuture<'a, Result<Value, ValueRefEvalError>> {
    async move {
    match value_ref {
        ValueRef::Lit { lit } => Ok(lit.clone()),
        ValueRef::Ref { ref_path } => resolve_ref_with_overrides(ref_path, context, options),
        ValueRef::Cel { cel } => evaluate_cel(cel, context, options),
        ValueRef::Detect { detect } => match detect_resolver {
            Some(resolver) => resolver.resolve(detect, context, options).await,
            None => Err(ValueRefEvalError::NeedDetect {
                kind: detect.kind.clone(),
            }),
        },
        ValueRef::Object { object } => {
            let mut result = Map::new();
            for (key, child) in object {
                let value =
                    evaluate_value_ref_async_boxed(child, context, options, detect_resolver).await?;
                result.insert(key.clone(), value);
            }
            Ok(Value::Object(result))
        }
        ValueRef::Array { array } => {
            let mut out = Vec::with_capacity(array.len());
            for item in array {
                out.push(
                    evaluate_value_ref_async_boxed(item, context, options, detect_resolver).await?,
                );
            }
            Ok(Value::Array(out))
        }
    }
    }
    .boxed_local()
}

fn evaluate_cel(
    expression: &str,
    context: &ResolverContext,
    options: &ValueRefEvalOptions,
) -> Result<Value, ValueRefEvalError> {
    let cel_context = build_cel_context(context, options)?;
    let value = evaluate_expression(expression, &cel_context).map_err(|error| {
        ValueRefEvalError::CelEvaluationFailed {
            expression: expression.to_string(),
            reason: error.to_string(),
        }
    })?;
    cel_value_to_json_value(value)
}

fn build_cel_context(
    context: &ResolverContext,
    options: &ValueRefEvalOptions,
) -> Result<CelContext, ValueRefEvalError> {
    let mut out = CelContext::new();
    if let Some(runtime_object) = context.runtime.as_object() {
        for (key, value) in runtime_object {
            out.insert(key.clone(), json_value_to_cel_value(value)?);
        }
    }

    for (key, value) in &options.root_overrides {
        out.insert(key.clone(), json_value_to_cel_value(value)?);
    }

    Ok(out)
}

fn json_value_to_cel_value(value: &Value) -> Result<CelValue, ValueRefEvalError> {
    match value {
        Value::Null => Ok(CelValue::Null),
        Value::Bool(value) => Ok(CelValue::Bool(*value)),
        Value::Number(number) => {
            if let Some(value) = number.as_i64() {
                return Ok(CelValue::Integer(BigInt::from(value)));
            }
            if let Some(value) = number.as_u64() {
                return Ok(CelValue::Integer(BigInt::from(value)));
            }
            let raw = number.to_string();
            let decimal =
                ais_cel::Decimal::parse(raw.as_str()).map_err(|error| ValueRefEvalError::CelEvaluationFailed {
                    expression: "<json-context-conversion>".to_string(),
                    reason: format!("invalid decimal number `{raw}`: {error}"),
                })?;
            Ok(CelValue::Decimal(decimal))
        }
        Value::String(value) => Ok(CelValue::String(value.clone())),
        Value::Array(values) => {
            let mut out = Vec::with_capacity(values.len());
            for value in values {
                out.push(json_value_to_cel_value(value)?);
            }
            Ok(CelValue::List(out))
        }
        Value::Object(values) => {
            let mut out = BTreeMap::new();
            for (key, value) in values {
                out.insert(key.clone(), json_value_to_cel_value(value)?);
            }
            Ok(CelValue::Map(out))
        }
    }
}

fn cel_value_to_json_value(value: CelValue) -> Result<Value, ValueRefEvalError> {
    match value {
        CelValue::Null => Ok(Value::Null),
        CelValue::Bool(value) => Ok(Value::Bool(value)),
        CelValue::Integer(value) => Ok(integer_to_json_value(value)),
        CelValue::Decimal(value) => Ok(Value::String(value.to_string())),
        CelValue::String(value) => Ok(Value::String(value)),
        CelValue::List(values) => {
            let mut out = Vec::with_capacity(values.len());
            for value in values {
                out.push(cel_value_to_json_value(value)?);
            }
            Ok(Value::Array(out))
        }
        CelValue::Map(values) => {
            let mut out = Map::new();
            for (key, value) in values {
                out.insert(key, cel_value_to_json_value(value)?);
            }
            Ok(Value::Object(out))
        }
    }
}

fn integer_to_json_value(value: BigInt) -> Value {
    let raw = value.to_string();
    if let Ok(parsed) = raw.parse::<i64>() {
        return Value::Number(serde_json::Number::from(parsed));
    }
    if let Ok(parsed) = raw.parse::<u64>() {
        return Value::Number(serde_json::Number::from(parsed));
    }
    Value::String(raw)
}

fn resolve_ref_with_overrides(
    ref_path: &str,
    context: &ResolverContext,
    options: &ValueRefEvalOptions,
) -> Result<Value, ValueRefEvalError> {
    let Some((root_key, remainder)) = split_first_segment(ref_path) else {
        return Err(ValueRefEvalError::MissingRef {
            path: ref_path.to_string(),
            source: ResolverError::InvalidPath(ref_path.to_string()),
        });
    };

    if let Some(root_override) = options.root_overrides.get(root_key) {
        return walk_value_by_path(root_override.clone(), remainder, ref_path);
    }

    context
        .get_ref(ref_path)
        .map_err(|source| ValueRefEvalError::MissingRef {
            path: ref_path.to_string(),
            source,
        })
}

fn split_first_segment(path: &str) -> Option<(&str, &str)> {
    let normalized = path.trim().trim_start_matches('$').trim_start_matches('.');
    if normalized.is_empty() {
        return None;
    }
    if let Some(split_at) = normalized.find('.') {
        Some((&normalized[..split_at], &normalized[split_at + 1..]))
    } else {
        Some((normalized, ""))
    }
}

fn walk_value_by_path(root: Value, path: &str, full_path: &str) -> Result<Value, ValueRefEvalError> {
    if path.is_empty() {
        return Ok(root);
    }

    let mut current = root;
    for token in path.split('.').filter(|segment| !segment.is_empty()) {
        current = walk_token(current, token, full_path)?;
    }
    Ok(current)
}

fn walk_token(current: Value, token: &str, full_path: &str) -> Result<Value, ValueRefEvalError> {
    let bytes = token.as_bytes();
    let mut position = 0usize;
    let mut next = current;

    if bytes.first() != Some(&b'[') {
        let key_end = bytes.iter().position(|value| *value == b'[').unwrap_or(bytes.len());
        let key = &token[..key_end];
        next = next
            .as_object()
            .and_then(|object| object.get(key).cloned())
            .ok_or_else(|| ValueRefEvalError::MissingRef {
                path: full_path.to_string(),
                source: ResolverError::NotFound(full_path.to_string()),
            })?;
        position = key_end;
    }

    while position < bytes.len() {
        if bytes[position] != b'[' {
            return Err(ValueRefEvalError::MissingRef {
                path: full_path.to_string(),
                source: ResolverError::InvalidPath(full_path.to_string()),
            });
        }
        position += 1;
        let start = position;
        while position < bytes.len() && bytes[position].is_ascii_digit() {
            position += 1;
        }
        if start == position || position >= bytes.len() || bytes[position] != b']' {
            return Err(ValueRefEvalError::MissingRef {
                path: full_path.to_string(),
                source: ResolverError::InvalidPath(full_path.to_string()),
            });
        }
        let index = token[start..position]
            .parse::<usize>()
            .map_err(|_| ValueRefEvalError::MissingRef {
                path: full_path.to_string(),
                source: ResolverError::InvalidPath(full_path.to_string()),
            })?;
        next = next
            .as_array()
            .and_then(|items| items.get(index).cloned())
            .ok_or_else(|| ValueRefEvalError::MissingRef {
                path: full_path.to_string(),
                source: ResolverError::NotFound(full_path.to_string()),
            })?;
        position += 1;
    }

    Ok(next)
}

#[cfg(test)]
#[path = "value_ref_test.rs"]
mod tests;
