use regex::Regex;
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};

pub fn lower_composite_node(node: &Value) -> Result<Option<Vec<Value>>, String> {
    let Some(node_obj) = node.as_object() else {
        return Ok(None);
    };
    let Some(execution) = node_obj.get("execution").and_then(Value::as_object) else {
        return Ok(None);
    };
    if execution.get("type").and_then(Value::as_str) != Some("composite") {
        return Ok(None);
    }

    let base_id = node_obj
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| "composite node missing id".to_string())?;
    let steps = execution
        .get("steps")
        .and_then(Value::as_array)
        .ok_or_else(|| "composite execution must define steps[]".to_string())?;
    if steps.is_empty() {
        return Err("composite execution must define at least one step".to_string());
    }

    let step_node_ids = build_step_node_ids(base_id, steps)?;

    let base_deps = node_obj
        .get("deps")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let base_condition = node_obj.get("condition");

    let mut lowered = Vec::<Value>::with_capacity(steps.len());
    let mut previous_node_id = None::<String>;
    for (index, step) in steps.iter().enumerate() {
        let Some(step_obj) = step.as_object() else {
            return Err(format!("composite step[{index}] must be an object"));
        };
        let step_id = step_obj
            .get("id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("composite step[{index}] missing id"))?;
        let step_execution = step_obj
            .get("execution")
            .cloned()
            .ok_or_else(|| format!("composite step `{step_id}` missing execution"))?;
        let lowered_id = step_node_ids
            .get(step_id)
            .cloned()
            .ok_or_else(|| format!("composite step `{step_id}` missing lowered node id"))?;
        let effective_chain = step_obj
            .get("chain")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .or_else(|| node_obj.get("chain").and_then(Value::as_str))
            .ok_or_else(|| format!("composite step `{step_id}` missing effective chain"))?;

        let mut lowered_node = node_obj.clone();
        lowered_node.insert("id".to_string(), Value::String(lowered_id.clone()));
        lowered_node.insert(
            "chain".to_string(),
            Value::String(effective_chain.to_string()),
        );
        lowered_node.insert(
            "execution".to_string(),
            rewrite_local_node_refs(step_execution, &step_node_ids),
        );
        apply_step_protocol_extension(&mut lowered_node, step_obj);
        apply_step_operation_chain(&mut lowered_node, effective_chain);

        if let Some(description) = step_obj.get("description").cloned() {
            lowered_node.insert("description".to_string(), description);
        }

        let step_condition = step_obj
            .get("condition")
            .map(|value| rewrite_local_node_refs(value.clone(), &step_node_ids));
        match merged_condition(base_condition, step_condition.as_ref())? {
            Some(condition) => {
                lowered_node.insert("condition".to_string(), condition);
            }
            None => {
                lowered_node.remove("condition");
            }
        }

        let deps = if let Some(previous) = &previous_node_id {
            vec![Value::String(previous.clone())]
        } else {
            base_deps.clone()
        };
        if deps.is_empty() {
            lowered_node.remove("deps");
        } else {
            lowered_node.insert("deps".to_string(), Value::Array(deps));
        }

        if index + 1 != steps.len() {
            lowered_node.remove("assert");
            lowered_node.remove("assert_message");
            lowered_node.remove("until");
            lowered_node.remove("retry");
            lowered_node.remove("timeout_ms");
            lowered_node.insert(
                "writes".to_string(),
                Value::Array(vec![json!({
                    "path": format!("nodes.{lowered_id}.outputs"),
                    "mode": "set"
                })]),
            );
        }

        annotate_source_metadata(&mut lowered_node, step_id);
        annotate_plan_sketch_metadata(&mut lowered_node, step_id, index + 1 == steps.len());
        annotate_composite_metadata(
            &mut lowered_node,
            base_id,
            step_id,
            index,
            steps.len(),
            lowered_id.as_str(),
            &step_node_ids,
            effective_chain,
        );
        apply_step_policy_overlay(&mut lowered_node, step_id);
        lowered.push(Value::Object(lowered_node));
        previous_node_id = Some(lowered_id);
    }

    Ok(Some(lowered))
}

fn build_step_node_ids(base_id: &str, steps: &[Value]) -> Result<HashMap<String, String>, String> {
    let mut seen = HashSet::<String>::new();
    let mut out = HashMap::<String, String>::new();
    for (index, step) in steps.iter().enumerate() {
        let Some(step_obj) = step.as_object() else {
            return Err(format!("composite step[{index}] must be an object"));
        };
        let step_id = step_obj
            .get("id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("composite step[{index}] missing id"))?;
        if !seen.insert(step_id.to_string()) {
            return Err(format!("duplicate composite step id `{step_id}`"));
        }
        let lowered_id = if index + 1 == steps.len() {
            base_id.to_string()
        } else {
            format!("{base_id}__{step_id}")
        };
        out.insert(step_id.to_string(), lowered_id);
    }
    Ok(out)
}

fn annotate_composite_metadata(
    node: &mut Map<String, Value>,
    base_id: &str,
    step_id: &str,
    step_index: usize,
    step_count: usize,
    output_node_id: &str,
    step_node_ids: &HashMap<String, String>,
    step_chain: &str,
) {
    let extensions = node
        .entry("extensions".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    let Some(extensions_obj) = extensions.as_object_mut() else {
        return;
    };
    extensions_obj.insert(
        "composite".to_string(),
        json!({
            "parent_node_id": base_id,
            "step_id": step_id,
            "step_index": step_index,
            "step_count": step_count,
            "output_node_id": output_node_id,
            "output_ref": format!("nodes.{output_node_id}.outputs"),
            "local_step_node_ids": step_node_ids,
            "step_chain": step_chain
        }),
    );
}

fn apply_step_protocol_extension(node: &mut Map<String, Value>, step_obj: &Map<String, Value>) {
    let Some(step_protocol) = step_obj
        .get("extensions")
        .and_then(Value::as_object)
        .and_then(|extensions| extensions.get("protocol"))
        .cloned()
    else {
        return;
    };
    let extensions = node
        .entry("extensions".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    let Some(extensions_obj) = extensions.as_object_mut() else {
        return;
    };
    extensions_obj.insert("protocol".to_string(), step_protocol);
}

fn apply_step_operation_chain(node: &mut Map<String, Value>, step_chain: &str) {
    let Some(operation) = node
        .get_mut("extensions")
        .and_then(Value::as_object_mut)
        .and_then(|extensions| extensions.get_mut("operation"))
        .and_then(Value::as_object_mut)
    else {
        return;
    };
    operation.insert(
        "target_chain".to_string(),
        Value::String(step_chain.to_string()),
    );
}

fn annotate_source_metadata(node: &mut Map<String, Value>, step_id: &str) {
    let Some(source) = node.get_mut("source").and_then(Value::as_object_mut) else {
        return;
    };
    source.insert(
        "composite_step_id".to_string(),
        Value::String(step_id.to_string()),
    );
}

fn annotate_plan_sketch_metadata(node: &mut Map<String, Value>, step_id: &str, is_final: bool) {
    let Some(extensions) = node.get_mut("extensions").and_then(Value::as_object_mut) else {
        return;
    };
    let Some(plan_sketch) = extensions
        .get_mut("plan_sketch")
        .and_then(Value::as_object_mut)
    else {
        return;
    };
    plan_sketch.insert(
        "composite_step_id".to_string(),
        Value::String(step_id.to_string()),
    );
    if !is_final {
        plan_sketch.remove("stores");
    }
}

fn apply_step_policy_overlay(node: &mut Map<String, Value>, step_id: &str) {
    let semantic_kind = composite_step_semantic_kind(node.get("execution"));
    let Some(kind) = semantic_kind.as_deref() else {
        return;
    };

    if let Some(extensions) = node.get_mut("extensions").and_then(Value::as_object_mut) {
        append_risk_tag(extensions, kind);
        if kind == "approval" {
            overlay_approval_policy(extensions);
        }
    }

    if let Some(extensions) = node.get_mut("extensions").and_then(Value::as_object_mut) {
        if let Some(composite) = extensions
            .get_mut("composite")
            .and_then(Value::as_object_mut)
        {
            composite.insert("semantic_kind".to_string(), Value::String(kind.to_string()));
        }
    }

    if let Some(source) = node.get_mut("source").and_then(Value::as_object_mut) {
        source.insert(
            "composite_step_kind".to_string(),
            Value::String(kind.to_string()),
        );
        source.insert(
            "composite_step_id".to_string(),
            Value::String(step_id.to_string()),
        );
    }
}

fn composite_step_semantic_kind(execution: Option<&Value>) -> Option<String> {
    let execution = execution?.as_object()?;
    let execution_type = execution.get("type").and_then(Value::as_str)?;
    if execution_type != "evm_call" {
        return None;
    }
    let method = execution
        .get("method")
        .and_then(Value::as_str)
        .or_else(|| {
            execution
                .get("abi")
                .and_then(Value::as_object)
                .and_then(|abi| abi.get("name"))
                .and_then(Value::as_str)
        })?
        .trim()
        .to_ascii_lowercase();
    match method.as_str() {
        "approve" => Some("approval".to_string()),
        _ => None,
    }
}

fn append_risk_tag(extensions: &mut Map<String, Value>, tag: &str) {
    let risk_tags = extensions
        .entry("risk_tags".to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    if !risk_tags.is_array() {
        *risk_tags = Value::Array(Vec::new());
    }
    let Some(tags) = risk_tags.as_array_mut() else {
        return;
    };
    if tags
        .iter()
        .filter_map(Value::as_str)
        .any(|existing| existing == tag)
    {
        return;
    }
    tags.push(Value::String(tag.to_string()));
}

fn overlay_approval_policy(extensions: &mut Map<String, Value>) {
    let policy = extensions
        .entry("policy".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if !policy.is_object() {
        *policy = Value::Object(Map::new());
    }
    let Some(policy_obj) = policy.as_object_mut() else {
        return;
    };

    let param_roles = policy_obj
        .entry("param_roles".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if !param_roles.is_object() {
        *param_roles = Value::Object(Map::new());
    }
    if let Some(param_roles_obj) = param_roles.as_object_mut() {
        param_roles_obj.insert(
            "spender_address".to_string(),
            Value::String("spender".to_string()),
        );
        param_roles_obj.insert(
            "approval_amount".to_string(),
            Value::String("amount".to_string()),
        );
    }

    let required_fields = policy_obj
        .entry("required_fields".to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    if !required_fields.is_array() {
        *required_fields = Value::Array(Vec::new());
    }
    let Some(required_fields_array) = required_fields.as_array_mut() else {
        return;
    };
    for field in ["spender_address", "approval_amount"] {
        if required_fields_array
            .iter()
            .filter_map(Value::as_str)
            .any(|existing| existing == field)
        {
            continue;
        }
        required_fields_array.push(Value::String(field.to_string()));
    }
}

fn merged_condition(parent: Option<&Value>, step: Option<&Value>) -> Result<Option<Value>, String> {
    match (parent, step) {
        (None, None) => Ok(None),
        (Some(value), None) | (None, Some(value)) => Ok(Some(value.clone())),
        (Some(parent), Some(step)) => {
            if let Some(parent_literal) = bool_literal(parent) {
                return Ok(if parent_literal {
                    Some(step.clone())
                } else {
                    Some(json!({ "lit": false }))
                });
            }
            if let Some(step_literal) = bool_literal(step) {
                return Ok(if step_literal {
                    Some(parent.clone())
                } else {
                    Some(json!({ "lit": false }))
                });
            }
            let parent_cel = parent
                .as_object()
                .and_then(|obj| obj.get("cel"))
                .and_then(Value::as_str);
            let step_cel = step
                .as_object()
                .and_then(|obj| obj.get("cel"))
                .and_then(Value::as_str);
            match (parent_cel, step_cel) {
                (Some(parent_cel), Some(step_cel)) => Ok(Some(
                    json!({ "cel": format!("({parent_cel}) && ({step_cel})") }),
                )),
                _ => Err("composite lowering cannot merge non-CEL conditions".to_string()),
            }
        }
    }
}

fn bool_literal(value: &Value) -> Option<bool> {
    value
        .as_object()
        .and_then(|obj| obj.get("lit"))
        .and_then(Value::as_bool)
}

fn rewrite_local_node_refs(value: Value, step_node_ids: &HashMap<String, String>) -> Value {
    let mut rewritten = value;
    rewrite_local_node_refs_in_value(&mut rewritten, step_node_ids);
    rewritten
}

fn rewrite_local_node_refs_in_value(value: &mut Value, step_node_ids: &HashMap<String, String>) {
    match value {
        Value::Object(object) => {
            if let Some(reference) = object.get("ref").and_then(Value::as_str) {
                let rewritten = rewrite_local_node_ref_path(reference, step_node_ids);
                object.insert("ref".to_string(), Value::String(rewritten));
            }
            if let Some(cel) = object.get("cel").and_then(Value::as_str) {
                let rewritten = rewrite_local_node_refs_in_cel(cel, step_node_ids);
                object.insert("cel".to_string(), Value::String(rewritten));
            }
            let mut keys = object.keys().cloned().collect::<Vec<_>>();
            keys.sort();
            for key in keys {
                let Some(child) = object.get_mut(key.as_str()) else {
                    continue;
                };
                rewrite_local_node_refs_in_value(child, step_node_ids);
            }
        }
        Value::Array(items) => {
            for item in items {
                rewrite_local_node_refs_in_value(item, step_node_ids);
            }
        }
        _ => {}
    }
}

fn rewrite_local_node_ref_path(path: &str, step_node_ids: &HashMap<String, String>) -> String {
    if let Some(after_nodes) = path.strip_prefix("nodes.") {
        if let Some((raw_id, tail)) = after_nodes.split_once('.') {
            if let Some(mapped) = step_node_ids.get(raw_id) {
                return format!("nodes.{mapped}.{tail}");
            }
        } else if let Some(mapped) = step_node_ids.get(after_nodes) {
            return format!("nodes.{mapped}");
        }
    }
    rewrite_local_node_refs_in_cel(path, step_node_ids)
}

fn rewrite_local_node_refs_in_cel(cel: &str, step_node_ids: &HashMap<String, String>) -> String {
    let bracket_ref_re = Regex::new(r#"nodes\[\s*"([^"]+)"\s*\]|nodes\[\s*'([^']+)'\s*\]"#)
        .expect("valid composite bracket node ref regex");
    let dot_ref_re = Regex::new(r#"\bnodes\.([A-Za-z_][A-Za-z0-9_]*)"#)
        .expect("valid composite dot node ref regex");

    let bracket_rewritten = bracket_ref_re.replace_all(cel, |caps: &regex::Captures<'_>| {
        let raw_id = caps.get(1).or_else(|| caps.get(2)).map(|m| m.as_str());
        let Some(raw_id) = raw_id else {
            return caps
                .get(0)
                .map(|m| m.as_str().to_string())
                .unwrap_or_default();
        };
        if let Some(mapped) = step_node_ids.get(raw_id) {
            format!("nodes[\"{mapped}\"]")
        } else {
            caps.get(0)
                .map(|m| m.as_str().to_string())
                .unwrap_or_default()
        }
    });

    dot_ref_re
        .replace_all(bracket_rewritten.as_ref(), |caps: &regex::Captures<'_>| {
            let Some(raw_id) = caps.get(1).map(|m| m.as_str()) else {
                return caps
                    .get(0)
                    .map(|m| m.as_str().to_string())
                    .unwrap_or_default();
            };
            if let Some(mapped) = step_node_ids.get(raw_id) {
                format!("nodes.{mapped}")
            } else {
                caps.get(0)
                    .map(|m| m.as_str().to_string())
                    .unwrap_or_default()
            }
        })
        .into_owned()
}
