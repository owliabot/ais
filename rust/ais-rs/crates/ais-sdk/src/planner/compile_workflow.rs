use crate::documents::{PlanDocument, ProtocolDocument, WorkflowDocument};
use crate::planner::lower_composite::lower_composite_node;
use crate::protocol::{
    annotate_composite_step_protocol_bindings, build_operation_extension, build_pack_extension,
    build_policy_extension, build_protocol_extension, pack_document_hash, resolve_operation_spec,
    ResolvedOperationKind,
};
use crate::resolver::{
    calculated_override_order_from_map, CalculatedOverrideError, ResolverContext,
};
use crate::ValueRef;
use ais_cel::parse_expression;
use ais_core::{FieldPath, FieldPathSegment, IssueSeverity, StructuredIssue};
use ais_schema::versions::SCHEMA_PLAN_0_0_3;
use regex::Regex;
use serde_json::{json, Map, Value};
use std::collections::{BTreeSet, HashMap, HashSet};

#[derive(Debug, Clone)]
pub struct CompileWorkflowOptions {
    pub default_chain: Option<String>,
    pub include_implicit_deps: bool,
}

impl Default for CompileWorkflowOptions {
    fn default() -> Self {
        Self {
            default_chain: None,
            include_implicit_deps: true,
        }
    }
}

#[derive(Debug, Clone)]
pub enum CompileWorkflowResult {
    Ok { plan: PlanDocument },
    Err { issues: Vec<StructuredIssue> },
}

pub fn compile_workflow(
    workflow: &WorkflowDocument,
    context: &ResolverContext,
    options: &CompileWorkflowOptions,
) -> CompileWorkflowResult {
    let mut issues = Vec::new();

    let mut nodes = Vec::<NodeCompileInput>::new();
    let mut node_ids = HashSet::<String>::new();
    let mut node_index_by_id = HashMap::<String, usize>::new();

    for (index, node) in workflow.nodes.iter().enumerate() {
        let base_path = vec![
            FieldPathSegment::Key("nodes".to_string()),
            FieldPathSegment::Index(index),
        ];
        let Some(node_obj) = node.as_object() else {
            issues.push(issue(
                "plan_build_error",
                path_with_key(&base_path, "id"),
                "workflow node must be an object",
                "workflow.node.object",
            ));
            continue;
        };
        let Some(node_id) = node_obj
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_string)
        else {
            issues.push(issue(
                "plan_build_error",
                path_with_key(&base_path, "id"),
                "workflow node must include `id`",
                "workflow.node.id_required",
            ));
            continue;
        };
        if !node_ids.insert(node_id.clone()) {
            issues.push(issue(
                "dag_error",
                path_with_key(&base_path, "id"),
                &format!("duplicate node id: {node_id}"),
                "workflow.node.duplicate_id",
            ));
            continue;
        }
        node_index_by_id.insert(node_id.clone(), index);

        let explicit_deps = extract_explicit_deps(node_obj, &base_path, &mut issues);
        let implicit_deps = if options.include_implicit_deps {
            collect_implicit_deps(node_obj)
        } else {
            Vec::new()
        };
        nodes.push(NodeCompileInput {
            index,
            id: node_id,
            node_obj: node_obj.clone(),
            explicit_deps,
            implicit_deps,
        });
    }

    let all_node_ids = nodes
        .iter()
        .map(|node| node.id.clone())
        .collect::<HashSet<_>>();
    for node in &nodes {
        for (dep_index, dep) in node.explicit_deps.iter().enumerate() {
            if dep == &node.id {
                issues.push(issue(
                    "dag_error",
                    vec![
                        FieldPathSegment::Key("nodes".to_string()),
                        FieldPathSegment::Index(node.index),
                        FieldPathSegment::Key("deps".to_string()),
                        FieldPathSegment::Index(dep_index),
                    ],
                    "node cannot depend on itself",
                    "workflow.deps.self",
                ));
                continue;
            }
            if !all_node_ids.contains(dep) {
                issues.push(issue(
                    "dag_error",
                    vec![
                        FieldPathSegment::Key("nodes".to_string()),
                        FieldPathSegment::Index(node.index),
                        FieldPathSegment::Key("deps".to_string()),
                        FieldPathSegment::Index(dep_index),
                    ],
                    &format!("unknown dependency: {dep}"),
                    "workflow.deps.unknown",
                ));
            }
        }
        for dep in &node.implicit_deps {
            if dep == &node.id {
                continue;
            }
            if !all_node_ids.contains(dep) {
                issues.push(issue(
                    "dag_error",
                    vec![
                        FieldPathSegment::Key("nodes".to_string()),
                        FieldPathSegment::Index(node.index),
                    ],
                    &format!("unknown implicit dependency: {dep}"),
                    "workflow.deps.implicit_unknown",
                ));
            }
        }
    }

    if !issues.is_empty() {
        StructuredIssue::sort_stable(&mut issues);
        return CompileWorkflowResult::Err { issues };
    }

    let sorted_ids = match stable_topological_order(&nodes, &node_index_by_id) {
        Ok(order) => order,
        Err(cycle) => {
            let mut cycle_issues = Vec::new();
            let cycle_text = cycle.join(" -> ");
            for node_id in cycle {
                if let Some(index) = node_index_by_id.get(&node_id) {
                    cycle_issues.push(issue(
                        "dag_error",
                        vec![
                            FieldPathSegment::Key("nodes".to_string()),
                            FieldPathSegment::Index(*index),
                            FieldPathSegment::Key("deps".to_string()),
                        ],
                        &format!("dependency cycle detected: {cycle_text}"),
                        "workflow.deps.cycle",
                    ));
                }
            }
            StructuredIssue::sort_stable(&mut cycle_issues);
            return CompileWorkflowResult::Err {
                issues: cycle_issues,
            };
        }
    };

    let mut node_by_id = HashMap::<String, NodeCompileInput>::new();
    for node in nodes {
        node_by_id.insert(node.id.clone(), node);
    }

    let workflow_name = workflow
        .meta
        .as_object()
        .and_then(|meta| meta.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("workflow");
    let workflow_version = workflow
        .meta
        .as_object()
        .and_then(|meta| meta.get("version"))
        .and_then(Value::as_str)
        .unwrap_or("0.0.0");

    let workflow_default_chain = workflow
        .default_chain
        .as_deref()
        .or(options.default_chain.as_deref());

    let workflow_pack_resolution = workflow_pack(workflow, context);
    if let Err(issue) = workflow_pack_resolution {
        return CompileWorkflowResult::Err {
            issues: vec![issue],
        };
    }
    let workflow_pack = workflow_pack_resolution.ok().flatten();
    let workflow_pack_hash = workflow_pack.and_then(pack_document_hash);

    let mut plan_nodes = Vec::new();
    let mut emitted_node_ids = HashSet::<String>::new();
    let mut compile_issues = Vec::new();
    for node_id in sorted_ids {
        let Some(node) = node_by_id.get(&node_id) else {
            continue;
        };
        match compile_node(
            node,
            &node_by_id,
            workflow_default_chain,
            workflow_name,
            workflow_version,
            workflow_pack,
            workflow_pack_hash.as_deref(),
            context,
            options.include_implicit_deps,
        ) {
            Ok(mut lowered_nodes) => {
                let duplicate_issues = validate_emitted_node_ids(
                    lowered_nodes.as_slice(),
                    &mut emitted_node_ids,
                    &[
                        FieldPathSegment::Key("nodes".to_string()),
                        FieldPathSegment::Index(node.index),
                        FieldPathSegment::Key("id".to_string()),
                    ],
                );
                if duplicate_issues.is_empty() {
                    plan_nodes.append(&mut lowered_nodes);
                } else {
                    compile_issues.extend(duplicate_issues);
                }
            }
            Err(mut node_issues) => compile_issues.append(&mut node_issues),
        }
    }

    if !compile_issues.is_empty() {
        StructuredIssue::sort_stable(&mut compile_issues);
        return CompileWorkflowResult::Err {
            issues: compile_issues,
        };
    }

    let mut plan_meta = json!({
        "name": workflow_name,
        "description": "compiled from workflow"
    });
    if let Some(preflight) = workflow.preflight.clone() {
        if let Some(meta_obj) = plan_meta.as_object_mut() {
            meta_obj.insert("preflight".to_string(), preflight);
        }
    }

    let plan = PlanDocument {
        schema: SCHEMA_PLAN_0_0_3.to_string(),
        meta: Some(plan_meta),
        nodes: plan_nodes,
        extensions: Map::new(),
    };

    CompileWorkflowResult::Ok { plan }
}

#[derive(Debug, Clone)]
struct NodeCompileInput {
    index: usize,
    id: String,
    node_obj: Map<String, Value>,
    explicit_deps: Vec<String>,
    implicit_deps: Vec<String>,
}

#[derive(Debug)]
struct ResolvedWorkflowOperation {
    executable_kind: &'static str,
    source_leaf_key: &'static str,
    source_leaf_value: String,
    control_step_kind: Option<String>,
}

fn compile_node(
    node: &NodeCompileInput,
    nodes_by_id: &HashMap<String, NodeCompileInput>,
    default_chain: Option<&str>,
    workflow_name: &str,
    workflow_version: &str,
    pack: Option<&crate::documents::PackDocument>,
    pack_hash: Option<&str>,
    context: &ResolverContext,
    include_implicit_deps: bool,
) -> Result<Vec<Value>, Vec<StructuredIssue>> {
    let base_path = vec![
        FieldPathSegment::Key("nodes".to_string()),
        FieldPathSegment::Index(node.index),
    ];

    let Some(kind) = node.node_obj.get("type").and_then(Value::as_str) else {
        return Err(vec![issue(
            "plan_build_error",
            path_with_key(&base_path, "type"),
            "workflow node must include `type`",
            "workflow.node.type_required",
        )]);
    };
    let Some(protocol_ref) = node.node_obj.get("protocol").and_then(Value::as_str) else {
        return Err(vec![issue(
            "plan_build_error",
            path_with_key(&base_path, "protocol"),
            "workflow node must include `protocol`",
            "workflow.node.protocol_required",
        )]);
    };

    let Some((protocol_id, protocol_version)) = split_protocol_ref(protocol_ref) else {
        return Err(vec![issue(
            "reference_error",
            path_with_key(&base_path, "protocol"),
            "protocol must be in form protocol@version",
            "workflow.node.protocol_format",
        )]);
    };
    let protocol_key = format!("{protocol_id}@{protocol_version}");
    let Some(protocol) = context.protocols.get(&protocol_key) else {
        return Err(vec![issue(
            "reference_error",
            path_with_key(&base_path, "protocol"),
            &format!("protocol not found: {protocol_key}"),
            "workflow.node.protocol_missing",
        )]);
    };

    let chain = node
        .node_obj
        .get("chain")
        .and_then(Value::as_str)
        .or(default_chain)
        .map(str::to_string);
    let Some(chain) = chain else {
        return Err(vec![issue(
            "plan_build_error",
            path_with_key(&base_path, "chain"),
            "missing chain: set nodes[].chain, workflow.default_chain, or compile options default_chain",
            "workflow.node.chain_required",
        )]);
    };

    let resolved = resolve_workflow_operation(
        &node.node_obj,
        &base_path,
        protocol,
        protocol_key.as_str(),
        kind,
    )?;

    let operation_kind = match resolved.executable_kind {
        "action_ref" => ResolvedOperationKind::Action,
        "query_ref" => ResolvedOperationKind::Query,
        _ => {
            return Err(vec![issue(
                "plan_build_error",
                path_with_key(&base_path, "type"),
                "workflow node resolved to unsupported executable kind",
                "workflow.node.type_invalid",
            )]);
        }
    };
    let Some(resolved_spec) = resolve_operation_spec(
        protocol_key.as_str(),
        protocol,
        resolved.source_leaf_value.as_str(),
        operation_kind,
        &chain,
        pack,
    ) else {
        return Err(vec![issue(
            "reference_error",
            path_with_key(&base_path, "chain"),
            &format!("no deployment mapping for chain `{chain}` in {protocol_key}"),
            "workflow.node.deployment_missing_for_chain",
        )]);
    };

    let Some(execution) = select_execution_for_chain(&resolved_spec.merged_spec, &chain) else {
        return Err(vec![issue(
            "reference_error",
            path_with_key(&base_path, "chain"),
            &format!(
                "no execution mapping for chain `{chain}` in {protocol_key}/{}",
                resolved.source_leaf_value
            ),
            "workflow.node.execution_missing_for_chain",
        )]);
    };

    let assert_issues = validate_assert_semantics(&node.node_obj, &base_path);
    if !assert_issues.is_empty() {
        return Err(assert_issues);
    }

    let execution = match annotate_composite_step_protocol_bindings(
        &execution,
        protocol_key.as_str(),
        protocol,
        chain.as_str(),
    ) {
        Ok(execution) => execution,
        Err(error) => {
            return Err(vec![issue(
                "plan_build_error",
                path_with_key(&base_path, "execution"),
                &error,
                "workflow.node.composite_chain_invalid",
            )]);
        }
    };

    let mut plan_node = Map::new();
    plan_node.insert("id".to_string(), Value::String(node.id.clone()));
    plan_node.insert(
        "kind".to_string(),
        Value::String(resolved.executable_kind.to_string()),
    );
    plan_node.insert("chain".to_string(), Value::String(chain));
    plan_node.insert("execution".to_string(), execution);
    copy_if_present(&node.node_obj, &mut plan_node, "condition");
    copy_if_present(&node.node_obj, &mut plan_node, "assert");
    copy_if_present(&node.node_obj, &mut plan_node, "assert_message");
    copy_if_present(&node.node_obj, &mut plan_node, "until");
    copy_if_present(&node.node_obj, &mut plan_node, "retry");
    copy_if_present(&node.node_obj, &mut plan_node, "timeout_ms");

    let deps = if include_implicit_deps {
        merge_deps(&node.explicit_deps, &node.implicit_deps, &node.id)
    } else {
        node.explicit_deps.clone()
    };
    if !deps.is_empty() {
        plan_node.insert(
            "deps".to_string(),
            Value::Array(deps.into_iter().map(Value::String).collect()),
        );
    }

    if let Some(args) = node.node_obj.get("args").and_then(Value::as_object) {
        plan_node.insert("bindings".to_string(), json!({ "params": args }));
    }
    if let Some(overrides) = merged_calculated_overrides(
        &resolved_spec.merged_spec,
        node.node_obj
            .get("calculated_overrides")
            .and_then(Value::as_object),
    ) {
        let mut override_issues = Vec::<StructuredIssue>::new();
        let order = match calculated_override_order_from_map(&overrides) {
            Ok(order) => order,
            Err(errors) => {
                override_issues.extend(map_calculated_override_errors(errors, &base_path));
                Vec::new()
            }
        };
        if !override_issues.is_empty() {
            return Err(override_issues);
        }
        let mut ordered = Map::<String, Value>::new();
        for key in &order {
            if let Some(value) = overrides.get(key.as_str()) {
                ordered.insert(key.clone(), value.clone());
            }
        }
        plan_node.insert("calculated_overrides".to_string(), Value::Object(ordered));
        plan_node.insert(
            "calculated_override_order".to_string(),
            Value::Array(order.into_iter().map(Value::String).collect()),
        );
    }

    plan_node.insert(
        "writes".to_string(),
        Value::Array(vec![json!({
            "path": format!("nodes.{}.outputs", node.id),
            "mode": "set"
        })]),
    );

    let mut source = Map::new();
    source.insert(
        "workflow".to_string(),
        json!({
            "name": workflow_name,
            "version": workflow_version
        }),
    );
    source.insert("node_id".to_string(), Value::String(node.id.clone()));
    source.insert("protocol".to_string(), Value::String(protocol_key.clone()));
    source.insert(
        resolved.source_leaf_key.to_string(),
        Value::String(resolved.source_leaf_value.clone()),
    );
    plan_node.insert("source".to_string(), Value::Object(source));
    insert_protocol_extension(
        &mut plan_node,
        protocol_key.as_str(),
        &resolved_spec.deployment,
    );
    insert_operation_extension(&mut plan_node, &resolved_spec);
    insert_required_query_extensions(
        &mut plan_node,
        required_query_names(&resolved_spec),
        workflow_query_bindings(node, nodes_by_id, protocol_ref, include_implicit_deps),
    );
    insert_pack_extension(&mut plan_node, resolved_spec.pack.as_ref(), pack, pack_hash);
    if let Some(control_step_kind) = resolved.control_step_kind.as_deref() {
        insert_control_step_kind_extension(&mut plan_node, control_step_kind);
    }

    if let Some(description) = resolved_spec
        .merged_spec
        .as_object()
        .and_then(|obj| obj.get("description"))
        .cloned()
    {
        plan_node.insert("description".to_string(), description);
    }

    copy_risk_metadata_from_operation_spec(&resolved_spec.merged_spec, &mut plan_node);
    merge_policy_extension(&mut plan_node, build_policy_extension(&resolved_spec));

    let plan_node = Value::Object(plan_node);
    match lower_composite_node(&plan_node) {
        Ok(Some(lowered)) => Ok(lowered),
        Ok(None) => Ok(vec![plan_node]),
        Err(error) => Err(vec![issue(
            "plan_build_error",
            path_with_key(&base_path, "execution"),
            &error,
            "workflow.node.composite_invalid",
        )]),
    }
}

fn merged_calculated_overrides(
    operation_spec: &Value,
    local_overrides: Option<&Map<String, Value>>,
) -> Option<Map<String, Value>> {
    let mut merged = operation_spec
        .as_object()
        .and_then(|spec| spec.get("calculated_fields"))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if let Some(local_overrides) = local_overrides {
        for (key, value) in local_overrides {
            merged.insert(key.clone(), value.clone());
        }
    }
    if merged.is_empty() {
        None
    } else {
        Some(merged)
    }
}

fn validate_emitted_node_ids(
    emitted_nodes: &[Value],
    seen_node_ids: &mut HashSet<String>,
    path: &[FieldPathSegment],
) -> Vec<StructuredIssue> {
    let mut issues = Vec::<StructuredIssue>::new();
    for node in emitted_nodes {
        let Some(node_id) = node
            .as_object()
            .and_then(|object| object.get("id"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            issues.push(issue(
                "plan_build_error",
                path.to_vec(),
                "lowered node is missing `id`",
                "workflow.node.lowered_missing_id",
            ));
            continue;
        };
        if !seen_node_ids.insert(node_id.to_string()) {
            issues.push(issue(
                "dag_error",
                path.to_vec(),
                &format!("duplicate emitted node id after lowering: {node_id}"),
                "workflow.node.lowered_duplicate_id",
            ));
        }
    }
    issues
}

fn insert_protocol_extension(
    plan_node: &mut Map<String, Value>,
    protocol_ref: &str,
    deployment: &crate::protocol::ResolvedDeployment,
) {
    let extensions = plan_node
        .entry("extensions".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    let Some(extensions_obj) = extensions.as_object_mut() else {
        return;
    };
    extensions_obj.insert(
        "protocol".to_string(),
        build_protocol_extension(protocol_ref, deployment),
    );
}

fn insert_pack_extension(
    plan_node: &mut Map<String, Value>,
    pack: Option<&crate::protocol::ResolvedPackOperation>,
    pack_document: Option<&crate::documents::PackDocument>,
    pack_hash: Option<&str>,
) {
    let Some(pack) = pack else {
        return;
    };
    let extensions = plan_node
        .entry("extensions".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    let Some(extensions_obj) = extensions.as_object_mut() else {
        return;
    };
    let mut pack_extension = build_pack_extension(pack);
    let Some(pack_extension_obj) = pack_extension.as_object_mut() else {
        return;
    };
    if let Some(name) = pack_name(pack_document) {
        pack_extension_obj.insert("name".to_string(), Value::String(name));
    }
    if let Some(version) = pack_version(pack_document) {
        pack_extension_obj.insert("version".to_string(), Value::String(version));
    }
    if let Some(hash) = pack_hash {
        pack_extension_obj.insert("hash".to_string(), Value::String(hash.to_string()));
    }
    extensions_obj.insert("pack".to_string(), pack_extension);
}

fn insert_operation_extension(
    plan_node: &mut Map<String, Value>,
    resolved_spec: &crate::protocol::ResolvedOperationSpec,
) {
    let extensions = plan_node
        .entry("extensions".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    let Some(extensions_obj) = extensions.as_object_mut() else {
        return;
    };
    extensions_obj.insert(
        "operation".to_string(),
        build_operation_extension(resolved_spec),
    );
}

fn insert_required_query_extensions(
    plan_node: &mut Map<String, Value>,
    required_queries: Vec<String>,
    query_bindings: Map<String, Value>,
) {
    if required_queries.is_empty() && query_bindings.is_empty() {
        return;
    }
    let Some(operation) = plan_node
        .get_mut("extensions")
        .and_then(Value::as_object_mut)
        .and_then(|extensions| extensions.get_mut("operation"))
        .and_then(Value::as_object_mut)
    else {
        return;
    };
    if !required_queries.is_empty() {
        operation.insert(
            "requires_queries".to_string(),
            Value::Array(required_queries.into_iter().map(Value::String).collect()),
        );
    }
    if !query_bindings.is_empty() {
        operation.insert("query_bindings".to_string(), Value::Object(query_bindings));
    }
}

fn merge_policy_extension(plan_node: &mut Map<String, Value>, policy_extension: Value) {
    let Some(policy_extension_obj) = policy_extension.as_object() else {
        return;
    };
    if policy_extension_obj.is_empty() {
        return;
    }
    let extensions = plan_node
        .entry("extensions".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    let Some(extensions_obj) = extensions.as_object_mut() else {
        return;
    };
    let policy = extensions_obj
        .entry("policy".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    let Some(policy_obj) = policy.as_object_mut() else {
        return;
    };
    for (key, value) in policy_extension_obj {
        policy_obj.insert(key.clone(), value.clone());
    }
}

fn required_query_names(resolved_spec: &crate::protocol::ResolvedOperationSpec) -> Vec<String> {
    resolved_spec
        .merged_spec
        .as_object()
        .and_then(|spec| spec.get("requires_queries"))
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

fn workflow_query_bindings(
    node: &NodeCompileInput,
    nodes_by_id: &HashMap<String, NodeCompileInput>,
    protocol_ref: &str,
    include_implicit_deps: bool,
) -> Map<String, Value> {
    let mut bindings = Map::<String, Value>::new();
    let dep_ids = node_dependency_ids(node, include_implicit_deps);
    for dep_id in dep_ids {
        let Some(dep_node) = nodes_by_id.get(dep_id.as_str()) else {
            continue;
        };
        let Some(query_name) = dep_node
            .node_obj
            .get("query")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty())
        else {
            continue;
        };
        if dep_node.node_obj.get("type").and_then(Value::as_str) != Some("query_ref") {
            continue;
        }
        if dep_node.node_obj.get("protocol").and_then(Value::as_str) != Some(protocol_ref) {
            continue;
        }
        bindings.entry(query_name.to_string()).or_insert_with(|| {
            json!({
                "node_id": dep_node.id,
                "query_ref": format!("{protocol_ref}/{query_name}")
            })
        });
    }
    bindings
}

fn node_dependency_ids(node: &NodeCompileInput, include_implicit_deps: bool) -> Vec<String> {
    let mut deps = node.explicit_deps.clone();
    if include_implicit_deps {
        for dep in &node.implicit_deps {
            if !deps.contains(dep) {
                deps.push(dep.clone());
            }
        }
    }
    deps
}

fn workflow_pack<'a>(
    workflow: &WorkflowDocument,
    context: &'a ResolverContext,
) -> Result<Option<&'a crate::documents::PackDocument>, StructuredIssue> {
    let Some(requires_pack) = workflow.requires_pack.as_ref().and_then(Value::as_object) else {
        return Ok(None);
    };
    let Some(name) = requires_pack.get("name").and_then(Value::as_str) else {
        return Ok(None);
    };
    let Some(version) = requires_pack.get("version").and_then(Value::as_str) else {
        return Ok(None);
    };
    let pack_key = format!("{name}@{version}");
    context
        .packs
        .get(pack_key.as_str())
        .map(Some)
        .ok_or_else(|| {
            issue(
                "reference_error",
                vec![
                    FieldPathSegment::Key("requires_pack".to_string()),
                    FieldPathSegment::Key("name".to_string()),
                ],
                &format!("required pack not found: {pack_key}"),
                "workflow.requires_pack.missing",
            )
        })
}

fn pack_name(pack: Option<&crate::documents::PackDocument>) -> Option<String> {
    pack.and_then(|pack| {
        pack.name.clone().or_else(|| {
            pack.meta
                .as_ref()
                .and_then(Value::as_object)
                .and_then(|meta| meta.get("name"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
    })
}

fn pack_version(pack: Option<&crate::documents::PackDocument>) -> Option<String> {
    pack.and_then(|pack| {
        pack.version.clone().or_else(|| {
            pack.meta
                .as_ref()
                .and_then(Value::as_object)
                .and_then(|meta| meta.get("version"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
    })
}

fn resolve_workflow_operation(
    node_obj: &Map<String, Value>,
    base_path: &[FieldPathSegment],
    protocol: &ProtocolDocument,
    protocol_key: &str,
    kind: &str,
) -> Result<ResolvedWorkflowOperation, Vec<StructuredIssue>> {
    let action = node_obj.get("action").and_then(Value::as_str);
    let query = node_obj.get("query").and_then(Value::as_str);

    let invalid_type = || {
        vec![issue(
            "plan_build_error",
            path_with_key(base_path, "type"),
            "workflow node type must be action_ref/query_ref/assert/branch",
            "workflow.node.type_invalid",
        )]
    };

    match kind {
        "action_ref" => {
            let Some(action) = action else {
                return Err(vec![issue(
                    "reference_error",
                    path_with_key(base_path, "action"),
                    "action_ref node must include `action`",
                    "workflow.node.action_required",
                )]);
            };
            let Some(_spec) = protocol.actions.get(action) else {
                return Err(vec![issue(
                    "reference_error",
                    path_with_key(base_path, "action"),
                    &format!("action not found: {protocol_key}/{action}"),
                    "workflow.node.action_missing",
                )]);
            };
            Ok(ResolvedWorkflowOperation {
                executable_kind: "action_ref",
                source_leaf_key: "action",
                source_leaf_value: action.to_string(),
                control_step_kind: None,
            })
        }
        "query_ref" => {
            let Some(query) = query else {
                return Err(vec![issue(
                    "reference_error",
                    path_with_key(base_path, "query"),
                    "query_ref node must include `query`",
                    "workflow.node.query_required",
                )]);
            };
            let Some(_spec) = protocol.queries.get(query) else {
                return Err(vec![issue(
                    "reference_error",
                    path_with_key(base_path, "query"),
                    &format!("query not found: {protocol_key}/{query}"),
                    "workflow.node.query_missing",
                )]);
            };
            Ok(ResolvedWorkflowOperation {
                executable_kind: "query_ref",
                source_leaf_key: "query",
                source_leaf_value: query.to_string(),
                control_step_kind: None,
            })
        }
        "assert" | "branch" => match (action, query) {
            (Some(action), None) => {
                let Some(_spec) = protocol.actions.get(action) else {
                    return Err(vec![issue(
                        "reference_error",
                        path_with_key(base_path, "action"),
                        &format!("action not found: {protocol_key}/{action}"),
                        "workflow.node.action_missing",
                    )]);
                };
                Ok(ResolvedWorkflowOperation {
                    executable_kind: "action_ref",
                    source_leaf_key: "action",
                    source_leaf_value: action.to_string(),
                    control_step_kind: Some(kind.to_string()),
                })
            }
            (None, Some(query)) => {
                let Some(_spec) = protocol.queries.get(query) else {
                    return Err(vec![issue(
                        "reference_error",
                        path_with_key(base_path, "query"),
                        &format!("query not found: {protocol_key}/{query}"),
                        "workflow.node.query_missing",
                    )]);
                };
                Ok(ResolvedWorkflowOperation {
                    executable_kind: "query_ref",
                    source_leaf_key: "query",
                    source_leaf_value: query.to_string(),
                    control_step_kind: Some(kind.to_string()),
                })
            }
            (Some(_), Some(_)) => Err(vec![issue(
                "plan_build_error",
                path_with_key(base_path, "type"),
                "control node type assert/branch must set exactly one of `action` or `query`",
                "workflow.node.control_target_ambiguous",
            )]),
            (None, None) => Err(vec![issue(
                "plan_build_error",
                path_with_key(base_path, "type"),
                "control node type assert/branch requires `action` or `query`",
                "workflow.node.control_target_required",
            )]),
        },
        _ => Err(invalid_type()),
    }
}

fn insert_control_step_kind_extension(plan_node: &mut Map<String, Value>, step_kind: &str) {
    let extensions = plan_node
        .entry("extensions".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    let Some(extensions_obj) = extensions.as_object_mut() else {
        return;
    };
    let control = extensions_obj
        .entry("control".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    let Some(control_obj) = control.as_object_mut() else {
        return;
    };
    control_obj.insert(
        "step_kind".to_string(),
        Value::String(step_kind.to_string()),
    );
}

fn copy_risk_metadata_from_operation_spec(
    operation_spec: &Value,
    plan_node: &mut Map<String, Value>,
) {
    let Some(obj) = operation_spec.as_object() else {
        return;
    };

    let risk_level = obj.get("risk_level").and_then(Value::as_u64);
    let risk_tags = obj.get("risk_tags").and_then(Value::as_array);
    if risk_level.is_none() && risk_tags.is_none() {
        return;
    }

    let extensions = plan_node
        .entry("extensions".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    let Some(extensions_obj) = extensions.as_object_mut() else {
        return;
    };

    if let Some(risk_level) = risk_level {
        if (1..=5).contains(&risk_level) {
            extensions_obj.insert("risk_level".to_string(), Value::Number(risk_level.into()));
        }
    }

    if let Some(risk_tags) = risk_tags {
        let tags = risk_tags
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .filter(|tag| !tag.trim().is_empty())
            .collect::<Vec<_>>();
        if !tags.is_empty() {
            extensions_obj.insert(
                "risk_tags".to_string(),
                Value::Array(tags.into_iter().map(Value::String).collect()),
            );
        }
    }
}

fn validate_assert_semantics(
    node_obj: &Map<String, Value>,
    base_path: &[FieldPathSegment],
) -> Vec<StructuredIssue> {
    let mut issues = Vec::new();

    let assert_value = node_obj.get("assert");
    let assert_message_value = node_obj.get("assert_message");

    if let Some(message) = assert_message_value {
        match message.as_str() {
            Some(value) if !value.trim().is_empty() => {}
            _ => issues.push(issue(
                "plan_build_error",
                path_with_key(base_path, "assert_message"),
                "assert_message must be a non-empty string",
                "workflow.node.assert_message_invalid",
            )),
        }
        if assert_value.is_none() {
            issues.push(issue(
                "plan_build_error",
                path_with_key(base_path, "assert_message"),
                "assert_message requires assert",
                "workflow.node.assert_message_without_assert",
            ));
        }
    }

    let Some(assert_value) = assert_value else {
        return issues;
    };

    let value_ref = match serde_json::from_value::<ValueRef>(assert_value.clone()) {
        Ok(value_ref) => value_ref,
        Err(error) => {
            issues.push(issue(
                "plan_build_error",
                path_with_key(base_path, "assert"),
                &format!("assert must be a valid ValueRef: {error}"),
                "workflow.node.assert_invalid",
            ));
            return issues;
        }
    };

    match value_ref {
        ValueRef::Lit { lit } => {
            if !lit.is_boolean() {
                issues.push(issue(
                    "plan_build_error",
                    path_with_key(base_path, "assert"),
                    "assert literal must be boolean",
                    "workflow.node.assert_not_boolean",
                ));
            }
        }
        ValueRef::Cel { cel } => {
            if let Err(error) = parse_expression(cel.as_str()) {
                issues.push(issue(
                    "plan_build_error",
                    path_with_key(base_path, "assert"),
                    &format!("assert CEL syntax error: {error}"),
                    "workflow.node.assert_cel_invalid",
                ));
            }
        }
        _ => {}
    }

    issues
}

fn map_calculated_override_errors(
    errors: Vec<CalculatedOverrideError>,
    base_path: &[FieldPathSegment],
) -> Vec<StructuredIssue> {
    errors
        .into_iter()
        .map(|error| match error {
            CalculatedOverrideError::OverridesMustBeObject => issue(
                "plan_build_error",
                path_with_key(base_path, "calculated_overrides"),
                "calculated_overrides must be an object",
                "workflow.node.calculated_overrides.object",
            ),
            CalculatedOverrideError::EntryMustBeObject { key } => issue(
                "plan_build_error",
                path_with_key(
                    &path_with_key(base_path, "calculated_overrides"),
                    key.as_str(),
                ),
                "calculated override entry must be an object",
                "workflow.node.calculated_overrides.entry_object",
            ),
            CalculatedOverrideError::ExprMissing { key } => issue(
                "plan_build_error",
                path_with_key(
                    &path_with_key(
                        &path_with_key(base_path, "calculated_overrides"),
                        key.as_str(),
                    ),
                    "expr",
                ),
                "calculated override must contain expr",
                "workflow.node.calculated_overrides.expr_required",
            ),
            CalculatedOverrideError::ExprInvalid { key, reason } => issue(
                "plan_build_error",
                path_with_key(
                    &path_with_key(
                        &path_with_key(base_path, "calculated_overrides"),
                        key.as_str(),
                    ),
                    "expr",
                ),
                &format!("calculated override expr must be a valid ValueRef: {reason}"),
                "workflow.node.calculated_overrides.expr_invalid",
            ),
            CalculatedOverrideError::MissingDependency { key, dependency } => issue(
                "plan_build_error",
                path_with_key(
                    &path_with_key(
                        &path_with_key(base_path, "calculated_overrides"),
                        key.as_str(),
                    ),
                    "expr",
                ),
                &format!("calculated override depends on missing `calculated.{dependency}`"),
                "workflow.node.calculated_overrides.missing_dependency",
            ),
            CalculatedOverrideError::DependencyCycle { cycle } => issue(
                "plan_build_error",
                path_with_key(base_path, "calculated_overrides"),
                &format!(
                    "calculated_overrides dependency cycle detected: {}",
                    cycle.join(" -> ")
                ),
                "workflow.node.calculated_overrides.cycle",
            ),
        })
        .collect()
}

fn merge_deps(explicit: &[String], implicit: &[String], node_id: &str) -> Vec<String> {
    let mut seen = HashSet::<String>::new();
    let mut out = Vec::<String>::new();

    for dep in explicit {
        if dep != node_id && seen.insert(dep.clone()) {
            out.push(dep.clone());
        }
    }
    let mut implicit_sorted = implicit.to_vec();
    implicit_sorted.sort();
    for dep in implicit_sorted {
        if dep != node_id && seen.insert(dep.clone()) {
            out.push(dep);
        }
    }
    out
}

fn stable_topological_order(
    nodes: &[NodeCompileInput],
    node_index_by_id: &HashMap<String, usize>,
) -> Result<Vec<String>, Vec<String>> {
    let mut deps_map = HashMap::<String, BTreeSet<String>>::new();
    let mut reverse = HashMap::<String, Vec<String>>::new();
    let mut indegree = HashMap::<String, usize>::new();

    for node in nodes {
        let deps = node
            .explicit_deps
            .iter()
            .chain(node.implicit_deps.iter())
            .filter(|dep| dep.as_str() != node.id)
            .cloned()
            .collect::<BTreeSet<_>>();
        indegree.insert(node.id.clone(), deps.len());
        deps_map.insert(node.id.clone(), deps.clone());
        for dep in deps {
            reverse.entry(dep).or_default().push(node.id.clone());
        }
    }

    let mut ready = nodes
        .iter()
        .filter(|node| indegree.get(&node.id).copied().unwrap_or(0) == 0)
        .map(|node| node.id.clone())
        .collect::<Vec<_>>();
    ready.sort_by_key(|id| node_index_by_id.get(id).copied().unwrap_or(usize::MAX));

    let mut out = Vec::<String>::new();
    while let Some(node_id) = ready.first().cloned() {
        ready.remove(0);
        out.push(node_id.clone());

        for child in reverse.get(&node_id).cloned().unwrap_or_default() {
            let entry = indegree.entry(child.clone()).or_insert(0);
            if *entry > 0 {
                *entry -= 1;
                if *entry == 0 {
                    ready.push(child);
                }
            }
        }
        ready.sort_by_key(|id| node_index_by_id.get(id).copied().unwrap_or(usize::MAX));
    }

    if out.len() == nodes.len() {
        return Ok(out);
    }

    let unresolved = nodes
        .iter()
        .map(|node| node.id.clone())
        .filter(|id| !out.contains(id))
        .collect::<Vec<_>>();
    Err(unresolved)
}

fn extract_explicit_deps(
    node_obj: &Map<String, Value>,
    base_path: &[FieldPathSegment],
    issues: &mut Vec<StructuredIssue>,
) -> Vec<String> {
    let mut deps = Vec::new();
    if let Some(raw_deps) = node_obj.get("deps") {
        match raw_deps.as_array() {
            Some(items) => {
                for (dep_index, dep) in items.iter().enumerate() {
                    match dep.as_str() {
                        Some(dep) if !dep.trim().is_empty() => deps.push(dep.to_string()),
                        _ => issues.push(issue(
                            "dag_error",
                            path_with_key_index(base_path, "deps", dep_index),
                            "dependency id must be a non-empty string",
                            "workflow.deps.invalid",
                        )),
                    }
                }
            }
            None => issues.push(issue(
                "dag_error",
                path_with_key(base_path, "deps"),
                "deps must be an array",
                "workflow.deps.array",
            )),
        }
    }
    deps
}

fn collect_implicit_deps(node_obj: &Map<String, Value>) -> Vec<String> {
    let mut ref_paths = Vec::<String>::new();
    let mut cel_expressions = Vec::<String>::new();

    if let Some(args) = node_obj.get("args").and_then(Value::as_object) {
        for value in args.values() {
            collect_ref_paths_and_cel(value, &mut ref_paths, &mut cel_expressions);
        }
    }
    for key in ["condition", "assert", "until"] {
        if let Some(value) = node_obj.get(key) {
            collect_ref_paths_and_cel(value, &mut ref_paths, &mut cel_expressions);
        }
    }
    if let Some(overrides) = node_obj
        .get("calculated_overrides")
        .and_then(Value::as_object)
    {
        for override_value in overrides.values() {
            if let Some(expr) = override_value.as_object().and_then(|obj| obj.get("expr")) {
                collect_ref_paths_and_cel(expr, &mut ref_paths, &mut cel_expressions);
            }
        }
    }

    let mut out = BTreeSet::<String>::new();
    for path in ref_paths {
        if let Some(node_id) = extract_node_id_from_ref(path.as_str()) {
            out.insert(node_id.to_string());
        }
    }
    for expression in cel_expressions {
        for node_id in extract_node_ids_from_cel(expression.as_str()) {
            out.insert(node_id);
        }
    }
    out.into_iter().collect()
}

fn collect_ref_paths_and_cel(
    value: &Value,
    ref_paths: &mut Vec<String>,
    cel_expressions: &mut Vec<String>,
) {
    let Some(obj) = value.as_object() else {
        return;
    };
    if let Some(path) = obj.get("ref").and_then(Value::as_str) {
        ref_paths.push(path.to_string());
    }
    if let Some(expr) = obj.get("cel").and_then(Value::as_str) {
        cel_expressions.push(expr.to_string());
    }
    if let Some(children) = obj.get("object").and_then(Value::as_object) {
        for child in children.values() {
            collect_ref_paths_and_cel(child, ref_paths, cel_expressions);
        }
    }
    if let Some(children) = obj.get("array").and_then(Value::as_array) {
        for child in children {
            collect_ref_paths_and_cel(child, ref_paths, cel_expressions);
        }
    }
}

fn extract_node_id_from_ref(path: &str) -> Option<&str> {
    let normalized = path.trim().trim_start_matches('$').trim_start_matches('.');
    let mut it = normalized.split('.');
    match (it.next(), it.next()) {
        (Some("nodes"), Some(node_id)) if !node_id.is_empty() => Some(node_id),
        _ => None,
    }
}

fn extract_node_ids_from_cel(cel: &str) -> Vec<String> {
    let regex = Regex::new(r"\bnodes\.([A-Za-z_][A-Za-z0-9_-]*)\b").expect("valid regex");
    regex
        .captures_iter(cel)
        .filter_map(|capture| capture.get(1).map(|m| m.as_str().to_string()))
        .collect()
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

fn issue(
    kind: &str,
    path: Vec<FieldPathSegment>,
    message: &str,
    reference: &str,
) -> StructuredIssue {
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

fn path_with_key_index(
    path: &[FieldPathSegment],
    key: &str,
    index: usize,
) -> Vec<FieldPathSegment> {
    let mut out = path.to_vec();
    out.push(FieldPathSegment::Key(key.to_string()));
    out.push(FieldPathSegment::Index(index));
    out
}

#[cfg(test)]
#[path = "compile_workflow_test.rs"]
mod tests;
