use crate::documents::{PlanDocument, PlanSkeletonDocument, WorkflowDocument};
use crate::resolver::ResolverContext;
use ais_core::{FieldPath, FieldPathSegment, IssueSeverity, StructuredIssue};
use ais_schema::versions::{SCHEMA_PLAN_0_0_3, SCHEMA_WORKFLOW_0_0_3};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Default)]
pub struct CompilePlanSkeletonOptions {
    pub default_chain: Option<String>,
}

#[derive(Debug, Clone)]
pub enum CompilePlanSkeletonResult {
    Ok {
        plan: PlanDocument,
        workflow: WorkflowDocument,
    },
    Err {
        issues: Vec<StructuredIssue>,
    },
}

pub fn compile_plan_skeleton(
    skeleton: &PlanSkeletonDocument,
    context: &ResolverContext,
    options: &CompilePlanSkeletonOptions,
) -> CompilePlanSkeletonResult {
    let mut issues = validate_skeleton_graph(&skeleton.nodes);

    let workflow = synthesize_workflow(skeleton, options);
    let default_chain = skeleton
        .default_chain
        .as_deref()
        .or(options.default_chain.as_deref());

    let mut plan_nodes = Vec::new();
    for (node_index, node) in skeleton.nodes.iter().enumerate() {
        match compile_node(node, node_index, default_chain, context) {
            Ok(plan_node) => plan_nodes.push(plan_node),
            Err(mut node_issues) => issues.append(&mut node_issues),
        }
    }

    if !issues.is_empty() {
        StructuredIssue::sort_stable(&mut issues);
        return CompilePlanSkeletonResult::Err { issues };
    }

    let mut extensions = Map::new();
    let mut skeleton_ext = Map::new();
    skeleton_ext.insert("schema".to_string(), Value::String(skeleton.schema.clone()));
    if let Some(policy_hints) = &skeleton.policy_hints {
        skeleton_ext.insert("policy_hints".to_string(), policy_hints.clone());
    }
    extensions.insert("plan_skeleton".to_string(), Value::Object(skeleton_ext));

    let plan = PlanDocument {
        schema: SCHEMA_PLAN_0_0_3.to_string(),
        meta: Some(json!({
            "name": "plan-skeleton",
            "description": "compiled from plan skeleton"
        })),
        nodes: plan_nodes,
        extensions,
    };

    CompilePlanSkeletonResult::Ok { plan, workflow }
}

fn synthesize_workflow(
    skeleton: &PlanSkeletonDocument,
    options: &CompilePlanSkeletonOptions,
) -> WorkflowDocument {
    let default_chain = skeleton
        .default_chain
        .clone()
        .or_else(|| options.default_chain.clone());

    let mut workflow_nodes = Vec::new();
    for node in &skeleton.nodes {
        if let Some(obj) = node.as_object() {
            let mut out = Map::new();
            copy_if_present(obj, &mut out, "id");
            copy_if_present(obj, &mut out, "type");
            copy_if_present(obj, &mut out, "protocol");
            copy_if_present(obj, &mut out, "action");
            copy_if_present(obj, &mut out, "query");
            copy_if_present(obj, &mut out, "deps");
            copy_if_present(obj, &mut out, "args");
            copy_if_present(obj, &mut out, "condition");
            copy_if_present(obj, &mut out, "assert");
            copy_if_present(obj, &mut out, "assert_message");
            copy_if_present(obj, &mut out, "until");
            copy_if_present(obj, &mut out, "retry");
            copy_if_present(obj, &mut out, "timeout_ms");
            if obj.get("chain").is_none() {
                if let Some(chain) = &default_chain {
                    out.insert("chain".to_string(), Value::String(chain.clone()));
                }
            } else {
                copy_if_present(obj, &mut out, "chain");
            }
            workflow_nodes.push(Value::Object(out));
        }
    }

    WorkflowDocument {
        schema: SCHEMA_WORKFLOW_0_0_3.to_string(),
        meta: json!({
            "name": "plan-skeleton",
            "version": "0.0.1"
        }),
        default_chain,
        imports: None,
        requires_pack: None,
        inputs: Map::new(),
        nodes: workflow_nodes,
        policy: None,
        preflight: None,
        outputs: Map::new(),
        extensions: Map::new(),
    }
}

fn compile_node(
    node: &Value,
    node_index: usize,
    default_chain: Option<&str>,
    context: &ResolverContext,
) -> Result<Value, Vec<StructuredIssue>> {
    let base_path = vec![
        FieldPathSegment::Key("nodes".to_string()),
        FieldPathSegment::Index(node_index),
    ];
    let Some(node_obj) = node.as_object() else {
        return Err(vec![issue(
            "plan_build_error",
            path_with_key(&base_path, "id"),
            "skeleton node must be an object",
            "skeleton.node.object",
        )]);
    };

    let Some(node_id) = node_obj.get("id").and_then(Value::as_str) else {
        return Err(vec![issue(
            "plan_build_error",
            path_with_key(&base_path, "id"),
            "skeleton node must include `id`",
            "skeleton.node.id_required",
        )]);
    };
    let Some(kind) = node_obj.get("type").and_then(Value::as_str) else {
        return Err(vec![issue(
            "plan_build_error",
            path_with_key(&base_path, "type"),
            "skeleton node must include `type`",
            "skeleton.node.type_required",
        )]);
    };
    let Some(protocol_ref) = node_obj.get("protocol").and_then(Value::as_str) else {
        return Err(vec![issue(
            "plan_build_error",
            path_with_key(&base_path, "protocol"),
            "skeleton node must include `protocol`",
            "skeleton.node.protocol_required",
        )]);
    };

    let Some((protocol_id, protocol_version)) = split_protocol_ref(protocol_ref) else {
        return Err(vec![issue(
            "reference_error",
            path_with_key(&base_path, "protocol"),
            "protocol must be in form protocol@version",
            "skeleton.node.protocol_format",
        )]);
    };
    let protocol_key = format!("{protocol_id}@{protocol_version}");
    let Some(protocol) = context.protocols.get(&protocol_key) else {
        return Err(vec![issue(
            "reference_error",
            path_with_key(&base_path, "protocol"),
            &format!("protocol not found: {protocol_key}"),
            "skeleton.node.protocol_missing",
        )]);
    };

    let selected_chain = node_obj
        .get("chain")
        .and_then(Value::as_str)
        .or(default_chain)
        .map(str::to_string);
    let Some(chain) = selected_chain else {
        return Err(vec![issue(
            "plan_build_error",
            path_with_key(&base_path, "chain"),
            "missing chain: set node.chain, skeleton.default_chain, or compile options default_chain",
            "skeleton.node.chain_required",
        )]);
    };

    let (leaf_key, operation_spec) = match kind {
        "action_ref" => {
            let Some(action) = node_obj.get("action").and_then(Value::as_str) else {
                return Err(vec![issue(
                    "reference_error",
                    path_with_key(&base_path, "action"),
                    "action_ref node must include `action`",
                    "skeleton.node.action_required",
                )]);
            };
            let Some(spec) = protocol.actions.get(action) else {
                return Err(vec![issue(
                    "reference_error",
                    path_with_key(&base_path, "action"),
                    &format!("action not found: {protocol_key}/{action}"),
                    "skeleton.node.action_missing",
                )]);
            };
            ("action", (action, spec))
        }
        "query_ref" => {
            let Some(query) = node_obj.get("query").and_then(Value::as_str) else {
                return Err(vec![issue(
                    "reference_error",
                    path_with_key(&base_path, "query"),
                    "query_ref node must include `query`",
                    "skeleton.node.query_required",
                )]);
            };
            let Some(spec) = protocol.queries.get(query) else {
                return Err(vec![issue(
                    "reference_error",
                    path_with_key(&base_path, "query"),
                    &format!("query not found: {protocol_key}/{query}"),
                    "skeleton.node.query_missing",
                )]);
            };
            ("query", (query, spec))
        }
        _ => {
            return Err(vec![issue(
                "plan_build_error",
                path_with_key(&base_path, "type"),
                "skeleton node type must be action_ref or query_ref",
                "skeleton.node.type_invalid",
            )])
        }
    };

    let execution = select_execution_for_chain(operation_spec.1, &chain).ok_or_else(|| {
        vec![issue(
            "reference_error",
            path_with_key(&base_path, "chain"),
            &format!(
                "no execution mapping for chain `{chain}` in {protocol_key}/{}",
                operation_spec.0
            ),
            "skeleton.node.execution_missing_for_chain",
        )]
    })?;

    let mut plan_node = Map::new();
    plan_node.insert("id".to_string(), Value::String(node_id.to_string()));
    plan_node.insert("kind".to_string(), Value::String(kind.to_string()));
    plan_node.insert("chain".to_string(), Value::String(chain));
    plan_node.insert("execution".to_string(), execution);

    copy_if_present(node_obj, &mut plan_node, "deps");
    copy_if_present(node_obj, &mut plan_node, "condition");
    copy_if_present(node_obj, &mut plan_node, "assert");
    copy_if_present(node_obj, &mut plan_node, "assert_message");
    copy_if_present(node_obj, &mut plan_node, "until");
    copy_if_present(node_obj, &mut plan_node, "retry");
    copy_if_present(node_obj, &mut plan_node, "timeout_ms");

    if let Some(args) = node_obj.get("args").and_then(Value::as_object) {
        plan_node.insert(
            "bindings".to_string(),
            json!({
                "params": args
            }),
        );
    }

    plan_node.insert(
        "writes".to_string(),
        Value::Array(vec![json!({
            "path": format!("nodes.{node_id}.outputs"),
            "mode": "set"
        })]),
    );

    let mut source = Map::new();
    source.insert(
        "workflow".to_string(),
        json!({
            "name": "plan-skeleton",
            "version": "0.0.1"
        }),
    );
    source.insert("node_id".to_string(), Value::String(node_id.to_string()));
    source.insert("protocol".to_string(), Value::String(protocol_key));
    source.insert(leaf_key.to_string(), Value::String(operation_spec.0.to_string()));
    plan_node.insert("source".to_string(), Value::Object(source));

    if let Some(description) = operation_spec
        .1
        .as_object()
        .and_then(|obj| obj.get("description"))
        .cloned()
    {
        plan_node.insert("description".to_string(), description);
    }

    Ok(Value::Object(plan_node))
}

fn select_execution_for_chain(operation_spec: &Value, chain: &str) -> Option<Value> {
    let execution_map = operation_spec
        .as_object()
        .and_then(|obj| obj.get("execution"))
        .and_then(Value::as_object)?;
    if let Some(execution) = execution_map.get(chain) {
        return Some(execution.clone());
    }
    if let Some((namespace, _)) = chain.split_once(':') {
        let wildcard = format!("{namespace}:*");
        if let Some(execution) = execution_map.get(&wildcard) {
            return Some(execution.clone());
        }
    }
    execution_map.get("*").cloned()
}

fn validate_skeleton_graph(nodes: &[Value]) -> Vec<StructuredIssue> {
    let mut issues = Vec::new();
    let mut ids = HashSet::<String>::new();
    let mut by_id = HashMap::<String, Vec<String>>::new();

    for (index, node) in nodes.iter().enumerate() {
        let base_path = vec![
            FieldPathSegment::Key("nodes".to_string()),
            FieldPathSegment::Index(index),
        ];
        let Some(node_obj) = node.as_object() else {
            continue;
        };
        let Some(node_id) = node_obj.get("id").and_then(Value::as_str) else {
            continue;
        };

        if !ids.insert(node_id.to_string()) {
            issues.push(issue(
                "dag_error",
                path_with_key(&base_path, "id"),
                &format!("duplicate node id: {node_id}"),
                "skeleton.graph.duplicate_id",
            ));
        }

        let deps = node_obj
            .get("deps")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        by_id.insert(node_id.to_string(), deps);
    }

    for (index, node) in nodes.iter().enumerate() {
        let base_path = vec![
            FieldPathSegment::Key("nodes".to_string()),
            FieldPathSegment::Index(index),
        ];
        let Some(node_obj) = node.as_object() else {
            continue;
        };
        let Some(node_id) = node_obj.get("id").and_then(Value::as_str) else {
            continue;
        };
        for (dep_index, dep) in by_id
            .get(node_id)
            .map(|deps| deps.iter().enumerate().collect::<Vec<_>>())
            .unwrap_or_default()
        {
            if !ids.contains(dep.as_str()) {
                issues.push(issue(
                    "dag_error",
                    path_with_key_index(&base_path, "deps", dep_index),
                    &format!("unknown dependency: {dep}"),
                    "skeleton.graph.unknown_dep",
                ));
            }
        }
    }

    let mut state = HashMap::<String, u8>::new();
    let mut stack = Vec::<String>::new();
    for node_id in ids {
        if state.get(&node_id).copied().unwrap_or(0) == 0 {
            if let Some(cycle) = dfs_cycle(node_id.as_str(), &by_id, &mut state, &mut stack) {
                let cycle_text = cycle.join(" -> ");
                for cycle_node in cycle {
                    issues.push(issue(
                        "dag_error",
                        vec![
                            FieldPathSegment::Key("nodes".to_string()),
                            FieldPathSegment::Key(cycle_node),
                            FieldPathSegment::Key("deps".to_string()),
                        ],
                        &format!("dependency cycle detected: {cycle_text}"),
                        "skeleton.graph.cycle",
                    ));
                }
                break;
            }
        }
    }

    issues
}

fn dfs_cycle(
    node_id: &str,
    edges: &HashMap<String, Vec<String>>,
    state: &mut HashMap<String, u8>,
    stack: &mut Vec<String>,
) -> Option<Vec<String>> {
    state.insert(node_id.to_string(), 1);
    stack.push(node_id.to_string());

    for dep in edges.get(node_id).cloned().unwrap_or_default() {
        match state.get(&dep).copied().unwrap_or(0) {
            0 => {
                if let Some(cycle) = dfs_cycle(dep.as_str(), edges, state, stack) {
                    return Some(cycle);
                }
            }
            1 => {
                if let Some(start) = stack.iter().position(|item| item == dep.as_str()) {
                    let mut cycle = stack[start..].to_vec();
                    cycle.push(dep);
                    return Some(cycle);
                }
            }
            _ => {}
        }
    }

    state.insert(node_id.to_string(), 2);
    stack.pop();
    None
}

fn split_protocol_ref(input: &str) -> Option<(&str, &str)> {
    let (protocol, version) = input.split_once('@')?;
    if protocol.is_empty() || version.is_empty() {
        return None;
    }
    Some((protocol, version))
}

fn copy_if_present(from: &Map<String, Value>, to: &mut Map<String, Value>, key: &str) {
    if let Some(value) = from.get(key) {
        to.insert(key.to_string(), value.clone());
    }
}

fn issue(kind: &str, path: Vec<FieldPathSegment>, message: &str, reference: &str) -> StructuredIssue {
    StructuredIssue {
        kind: kind.to_string(),
        severity: IssueSeverity::Error,
        node_id: None,
        field_path: FieldPath::from_segments(path),
        message: message.to_string(),
        reference: Some(reference.to_string()),
        related: None,
    }
}

fn path_with_key(path: &[FieldPathSegment], key: &str) -> Vec<FieldPathSegment> {
    let mut out = path.to_vec();
    out.push(FieldPathSegment::Key(key.to_string()));
    out
}

fn path_with_key_index(path: &[FieldPathSegment], key: &str, index: usize) -> Vec<FieldPathSegment> {
    let mut out = path.to_vec();
    out.push(FieldPathSegment::Key(key.to_string()));
    out.push(FieldPathSegment::Index(index));
    out
}

#[cfg(test)]
#[path = "compile_plan_skeleton_test.rs"]
mod tests;
