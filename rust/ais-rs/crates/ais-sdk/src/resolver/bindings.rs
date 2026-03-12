use super::ValueRefEvalOptions;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ResolvedQueryBindings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<Value>,
    #[serde(default)]
    pub missing_refs: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ResolvedNodeBindings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Map<String, Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contracts: Option<Map<String, Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub calculated: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<Value>,
}

impl ResolvedNodeBindings {
    pub fn to_eval_options(&self, base_options: &ValueRefEvalOptions) -> ValueRefEvalOptions {
        let mut root_overrides = base_options.root_overrides.clone();
        if let Some(params) = &self.params {
            root_overrides.insert("params".to_string(), Value::Object(params.clone()));
        }
        if let Some(contracts) = &self.contracts {
            root_overrides.insert("contracts".to_string(), Value::Object(contracts.clone()));
        }
        if let Some(calculated) = &self.calculated {
            root_overrides.insert("calculated".to_string(), calculated.clone());
        }
        if let Some(query) = &self.query {
            root_overrides.insert("query".to_string(), query.clone());
        }
        if let Some(policy) = &self.policy {
            root_overrides.insert("policy".to_string(), policy.clone());
        }
        ValueRefEvalOptions { root_overrides }
    }
}

pub fn resolve_node_bindings(
    node: &Value,
    runtime: Option<&Value>,
    resolved_params: Option<&Map<String, Value>>,
    resolved_calculated: Option<&Map<String, Value>>,
) -> ResolvedNodeBindings {
    let query = resolve_query_bindings(node, runtime);
    ResolvedNodeBindings {
        params: resolved_params.cloned(),
        contracts: node
            .pointer("/extensions/protocol/contracts")
            .and_then(Value::as_object)
            .cloned(),
        calculated: resolved_calculated
            .map(|calculated| Value::Object(calculated.clone()))
            .or_else(|| {
                runtime
                    .and_then(Value::as_object)
                    .and_then(|object| object.get("calculated"))
                    .cloned()
            }),
        query: query.query,
        policy: node.pointer("/extensions/policy").cloned(),
    }
}

pub fn resolve_query_bindings(node: &Value, runtime: Option<&Value>) -> ResolvedQueryBindings {
    let mut query = runtime
        .and_then(Value::as_object)
        .and_then(|object| object.get("query"))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let mut missing_refs = Vec::<String>::new();
    let required_queries = required_query_names(node);

    if let Some(bindings) = node
        .pointer("/extensions/operation/query_bindings")
        .and_then(Value::as_object)
    {
        for (query_name, binding) in bindings {
            let Some(node_id) = binding.get("node_id").and_then(Value::as_str) else {
                continue;
            };
            let path = format!("/nodes/{node_id}/outputs");
            if let Some(outputs) = runtime.and_then(|value| value.pointer(path.as_str())) {
                query.insert(query_name.clone(), outputs.clone());
            }
        }
    }

    for query_name in required_queries {
        if !query.contains_key(query_name.as_str()) {
            missing_refs.push(format!("query.{query_name}"));
        }
    }
    missing_refs.sort();
    missing_refs.dedup();

    ResolvedQueryBindings {
        query: (!query.is_empty()).then_some(Value::Object(query)),
        missing_refs,
    }
}

fn required_query_names(node: &Value) -> Vec<String> {
    node.pointer("/extensions/operation/requires_queries")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

#[cfg(test)]
#[path = "bindings_test.rs"]
mod tests;
