use crate::catalog::ExecutableCandidates;
use crate::documents::{PlanDocument, PlanSketchDocument};
use crate::planner::lower_composite::lower_composite_node;
use crate::protocol::{
    annotate_composite_step_protocol_bindings, build_operation_extension, build_pack_extension,
    build_policy_extension, build_protocol_extension, pack_document_hash, resolve_operation_spec,
    ResolvedOperationKind,
};
use crate::resolver::{
    calculated_override_order_from_map, resolve_action_ref, resolve_query_ref, ResolverContext,
};
use crate::ValueRef;
use ais_core::{FieldPath, FieldPathSegment, IssueSeverity, StructuredIssue};
use ais_schema::versions::SCHEMA_PLAN_0_0_3;
use regex::Regex;
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Default)]
pub struct CompilePlanSketchOptions {
    pub default_chain: Option<String>,
    pub known_input_refs: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum CompilePlanSketchResult {
    Ok { plan: PlanDocument },
    Err { issues: Vec<StructuredIssue> },
}

pub fn compile_plan_sketch(
    sketch: &PlanSketchDocument,
    context: &ResolverContext,
    candidates: Option<&ExecutableCandidates>,
    options: &CompilePlanSketchOptions,
) -> CompilePlanSketchResult {
    let mut issues = Vec::<StructuredIssue>::new();
    let known_input_refs = build_known_input_ref_set(options);

    let default_chain = options
        .default_chain
        .as_deref()
        .map(str::to_string)
        .or_else(|| (sketch.chain_scope.len() == 1).then(|| sketch.chain_scope[0].clone()));

    let candidate_index = build_candidate_index(candidates);
    let resolved_pack = match validate_and_resolve_sketch_pack(sketch, context) {
        Ok(pack) => pack,
        Err(mut pack_issues) => {
            StructuredIssue::sort_stable(&mut pack_issues);
            return CompilePlanSketchResult::Err {
                issues: pack_issues,
            };
        }
    };

    let mut seen_node_ids = HashSet::<String>::new();
    for segment in &sketch.segments {
        for step in &segment.steps {
            let node_id = canonical_node_id(segment.segment_id.as_str(), step.id.as_str());
            if !seen_node_ids.insert(node_id.clone()) {
                issues.push(issue(
                    "compile_error",
                    vec![
                        FieldPathSegment::Key("segments".to_string()),
                        FieldPathSegment::Key(segment.segment_id.clone()),
                        FieldPathSegment::Key("steps".to_string()),
                        FieldPathSegment::Key(step.id.clone()),
                    ],
                    &format!("duplicate compiled node id: {node_id}"),
                    "constraint_violation",
                ));
            }
        }
    }

    let mut nodes = Vec::<Value>::new();
    let mut emitted_node_ids = HashSet::<String>::new();
    for (segment_index, segment) in sketch.segments.iter().enumerate() {
        let mut segment_step_to_node_id = HashMap::<String, String>::new();
        let mut segment_steps_by_id = HashMap::<String, crate::documents::PlanSketchStep>::new();
        let mut control_steps = HashMap::<String, ControlStepMeta>::new();
        let mut segment_step_ids = HashSet::<String>::new();
        let mut segment_node_ids = HashSet::<String>::new();
        for step in &segment.steps {
            let node_id = canonical_node_id(segment.segment_id.as_str(), step.id.as_str());
            segment_step_ids.insert(step.id.clone());
            segment_node_ids.insert(node_id.clone());
            segment_step_to_node_id.insert(step.id.clone(), node_id.clone());
            segment_step_to_node_id.insert(node_id.clone(), node_id);
            segment_steps_by_id.insert(step.id.clone(), step.clone());
            if matches!(step.kind.as_str(), "assert" | "branch") {
                control_steps.insert(
                    step.id.clone(),
                    ControlStepMeta {
                        depends_on: step.depends_on.clone(),
                        condition_cel: control_or_step_condition_cel(step),
                    },
                );
            }
        }
        for (step_index, step) in segment.steps.iter().enumerate() {
            let step_path = vec![
                FieldPathSegment::Key("segments".to_string()),
                FieldPathSegment::Index(segment_index),
                FieldPathSegment::Key("steps".to_string()),
                FieldPathSegment::Index(step_index),
            ];
            if matches!(step.kind.as_str(), "assert" | "branch") {
                continue;
            }
            let mut effective_step = step.clone();
            let mut inherited_conditions = Vec::<String>::new();
            effective_step.depends_on = resolve_non_control_dependencies(
                step.depends_on.as_slice(),
                &control_steps,
                &mut inherited_conditions,
            );
            if let Some(merged_condition) = merge_condition_cels(
                effective_step.when.as_ref().map(|when| when.cel.as_str()),
                inherited_conditions.as_slice(),
            ) {
                effective_step.when = Some(crate::documents::PlanSketchWhen {
                    cel: merged_condition,
                });
            }
            match compile_step(
                sketch,
                segment.segment_id.as_str(),
                segment
                    .extensions
                    .get("todo_id")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty()),
                &effective_step,
                step_path.as_slice(),
                default_chain.as_deref(),
                resolved_pack,
                &known_input_refs,
                context,
                &candidate_index,
                &segment_step_to_node_id,
                &segment_steps_by_id,
                &segment_step_ids,
                &segment_node_ids,
            ) {
                Ok(mut lowered_nodes) => {
                    let duplicate_issues = validate_emitted_node_ids(
                        lowered_nodes.as_slice(),
                        &mut emitted_node_ids,
                        &[
                            FieldPathSegment::Key("segments".to_string()),
                            FieldPathSegment::Index(segment_index),
                            FieldPathSegment::Key("steps".to_string()),
                            FieldPathSegment::Index(step_index),
                            FieldPathSegment::Key("id".to_string()),
                        ],
                    );
                    if duplicate_issues.is_empty() {
                        nodes.append(&mut lowered_nodes);
                    } else {
                        issues.extend(duplicate_issues);
                    }
                }
                Err(mut step_issues) => issues.append(&mut step_issues),
            }
        }
    }

    if !issues.is_empty() {
        StructuredIssue::sort_stable(&mut issues);
        return CompilePlanSketchResult::Err { issues };
    }

    let plan = PlanDocument {
        schema: SCHEMA_PLAN_0_0_3.to_string(),
        meta: Some(json!({
            "name": "plan-sketch",
            "description": "compiled from plan sketch",
            "plan_sketch": {
                "schema": sketch.schema,
                "segments": sketch.segments.len()
            }
        })),
        nodes,
        extensions: Map::new(),
    };

    CompilePlanSketchResult::Ok { plan }
}

#[derive(Debug, Clone)]
struct CandidateMeta {
    kind: CandidateKind,
    execution_types: Vec<String>,
    execution_chains: Vec<String>,
    risk_level: Option<u64>,
    risk_tags: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CandidateKind {
    Action,
    Query,
}

fn build_candidate_index(
    candidates: Option<&ExecutableCandidates>,
) -> HashMap<String, CandidateMeta> {
    let mut out = HashMap::<String, CandidateMeta>::new();
    let Some(candidates) = candidates else {
        return out;
    };

    for card in &candidates.actions {
        if let Some(reference) = card.get("ref").and_then(Value::as_str) {
            out.insert(
                reference.to_string(),
                CandidateMeta {
                    kind: CandidateKind::Action,
                    execution_types: string_array_field(card, "execution_types"),
                    execution_chains: string_array_field(card, "execution_chains"),
                    risk_level: risk_level_field(card),
                    risk_tags: risk_tags_field(card),
                },
            );
        }
    }
    for card in &candidates.queries {
        if let Some(reference) = card.get("ref").and_then(Value::as_str) {
            out.insert(
                reference.to_string(),
                CandidateMeta {
                    kind: CandidateKind::Query,
                    execution_types: string_array_field(card, "execution_types"),
                    execution_chains: string_array_field(card, "execution_chains"),
                    risk_level: risk_level_field(card),
                    risk_tags: risk_tags_field(card),
                },
            );
        }
    }
    out
}

fn compile_step(
    sketch: &PlanSketchDocument,
    segment_id: &str,
    segment_todo_id: Option<&str>,
    step: &crate::documents::PlanSketchStep,
    step_path: &[FieldPathSegment],
    default_chain: Option<&str>,
    pack: Option<&crate::documents::PackDocument>,
    known_input_refs: &HashSet<String>,
    context: &ResolverContext,
    candidate_index: &HashMap<String, CandidateMeta>,
    segment_step_to_node_id: &HashMap<String, String>,
    segment_steps_by_id: &HashMap<String, crate::documents::PlanSketchStep>,
    segment_step_ids: &HashSet<String>,
    segment_node_ids: &HashSet<String>,
) -> Result<Vec<Value>, Vec<StructuredIssue>> {
    let mut issues = Vec::<StructuredIssue>::new();
    let node_id = segment_step_to_node_id
        .get(step.id.as_str())
        .cloned()
        .unwrap_or_else(|| canonical_node_id(segment_id, step.id.as_str()));
    let resolved_kind = match step.kind.as_str() {
        "query" => CandidateKind::Query,
        "action" => CandidateKind::Action,
        _ => {
            issues.push(issue(
                "compile_error",
                path_with_key(step_path, "kind"),
                &format!("unsupported executable step kind `{}`", step.kind),
                "input_type_mismatch",
            ));
            return Err(issues);
        }
    };

    let Some(candidate_ref) = normalized_candidate_ref(step.candidate_ref.as_deref()) else {
        issues.push(issue(
            "compile_error",
            path_with_key(step_path, "candidate_ref"),
            "candidate_ref is required for query/action steps",
            "missing_required_input",
        ));
        return Err(issues);
    };
    let Some(chain) = step
        .chain
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or(default_chain)
    else {
        issues.push(issue(
            "compile_error",
            path_with_key(step_path, "chain"),
            "missing deterministic chain: set step.chain, compile default_chain, or a single sketch.chain_scope item",
            "missing_required_input",
        ));
        return Err(issues);
    };
    if !sketch.chain_scope.is_empty() && !sketch.chain_scope.iter().any(|allowed| allowed == chain)
    {
        issues.push(issue(
            "compile_error",
            path_with_key(step_path, "chain"),
            &format!("step chain `{chain}` is outside sketch.chain_scope"),
            "candidate_chain_not_allowed",
        ));
    }
    let effective_inputs = step.inputs.clone();

    if let Some(meta) = candidate_index.get(candidate_ref.as_str()) {
        if meta.kind != resolved_kind {
            issues.push(issue(
                "compile_error",
                path_with_key(step_path, "candidate_ref"),
                &format!(
                    "candidate kind mismatch for `{}`: expected {}, got {}",
                    candidate_ref,
                    kind_label(resolved_kind),
                    kind_label(meta.kind)
                ),
                "candidate_not_found",
            ));
        }
        if !meta.execution_chains.is_empty()
            && !meta
                .execution_chains
                .iter()
                .any(|pattern| chain_matches(pattern.as_str(), chain))
        {
            issues.push(issue(
                "compile_error",
                path_with_key(step_path, "candidate_ref"),
                &format!(
                    "candidate `{}` is not allowlisted for chain `{chain}`",
                    candidate_ref
                ),
                "candidate_chain_not_allowed",
            ));
        }
    } else if !candidate_index.is_empty() {
        issues.push(issue(
            "compile_error",
            path_with_key(step_path, "candidate_ref"),
            &format!("candidate not found: {}", candidate_ref),
            "candidate_not_found",
        ));
    }

    let (protocol_ref, protocol, operation_key, _operation_spec, source_key, source_leaf) =
        match resolved_kind {
            CandidateKind::Action => match resolve_action_ref(context, candidate_ref.as_str()) {
                Ok(resolved) => (
                    format!(
                        "{}@{}",
                        resolved.reference.protocol, resolved.reference.version
                    ),
                    resolved.protocol,
                    resolved.reference.action.clone(),
                    resolved.action_spec,
                    "action",
                    resolved.reference.action,
                ),
                Err(error) => {
                    issues.push(issue(
                        "compile_error",
                        path_with_key(step_path, "candidate_ref"),
                        &format!("unable to resolve action candidate: {error}"),
                        "candidate_not_found",
                    ));
                    return Err(issues);
                }
            },
            CandidateKind::Query => match resolve_query_ref(context, candidate_ref.as_str()) {
                Ok(resolved) => (
                    format!(
                        "{}@{}",
                        resolved.reference.protocol, resolved.reference.version
                    ),
                    resolved.protocol,
                    resolved.reference.query.clone(),
                    resolved.query_spec,
                    "query",
                    resolved.reference.query,
                ),
                Err(error) => {
                    issues.push(issue(
                        "compile_error",
                        path_with_key(step_path, "candidate_ref"),
                        &format!("unable to resolve query candidate: {error}"),
                        "candidate_not_found",
                    ));
                    return Err(issues);
                }
            },
        };

    let operation_kind = match resolved_kind {
        CandidateKind::Action => ResolvedOperationKind::Action,
        CandidateKind::Query => ResolvedOperationKind::Query,
    };
    let Some(resolved_spec) = resolve_operation_spec(
        protocol_ref.as_str(),
        protocol,
        operation_key.as_str(),
        operation_kind,
        chain,
        pack,
    ) else {
        issues.push(issue(
            "compile_error",
            path_with_key(step_path, "candidate_ref"),
            &format!(
                "no deployment mapping for chain `{chain}` in `{}`",
                protocol_ref
            ),
            "candidate_deployment_missing_for_chain",
        ));
        return Err(issues);
    };

    validate_required_params(
        &resolved_spec.merged_spec,
        &effective_inputs,
        candidate_ref.as_str(),
        &mut issues,
    );
    let mut normalized_inputs = normalize_step_inputs(
        &resolved_spec.merged_spec,
        &effective_inputs,
        chain,
        candidate_ref.as_str(),
        step_path,
        &mut issues,
    );
    lint_unknown_node_refs(
        step,
        &normalized_inputs,
        step_path,
        segment_step_ids,
        segment_node_ids,
        &mut issues,
    );
    for value in normalized_inputs.values_mut() {
        rewrite_node_refs_in_value(value, segment_step_to_node_id);
    }
    lint_unknown_input_refs(&normalized_inputs, known_input_refs, step_path, &mut issues);
    validate_step_runtime_controls(step, step_path, &mut issues);

    let execution = match select_execution_for_chain(&resolved_spec.merged_spec, chain) {
        Some(execution) => execution,
        None => {
            issues.push(issue(
                "compile_error",
                path_with_key(step_path, "candidate_ref"),
                &format!(
                    "no execution mapping for chain `{chain}` in `{}`",
                    candidate_ref
                ),
                "candidate_chain_not_allowed",
            ));
            return Err(issues);
        }
    };

    let execution_type = execution
        .as_object()
        .and_then(|obj| obj.get("type"))
        .and_then(Value::as_str);
    if let (Some(meta), Some(execution_type)) =
        (candidate_index.get(candidate_ref.as_str()), execution_type)
    {
        if !meta.execution_types.is_empty()
            && !meta.execution_types.iter().any(|v| v == execution_type)
        {
            issues.push(issue(
                "compile_error",
                path_with_key(step_path, "candidate_ref"),
                &format!(
                    "execution type `{execution_type}` is not allowlisted for candidate `{}`",
                    candidate_ref
                ),
                "execution_type_not_allowed",
            ));
        }
    }

    if !issues.is_empty() {
        return Err(issues);
    }

    let execution = match annotate_composite_step_protocol_bindings(
        &execution,
        protocol_ref.as_str(),
        protocol,
        chain,
    ) {
        Ok(execution) => execution,
        Err(error) => {
            issues.push(issue(
                "compile_error",
                path_with_key(step_path, "candidate_ref"),
                &error,
                "candidate_invalid_composite_chain",
            ));
            return Err(issues);
        }
    };

    let mut node = Map::<String, Value>::new();
    node.insert("id".to_string(), Value::String(node_id.clone()));
    node.insert(
        "kind".to_string(),
        Value::String(match resolved_kind {
            CandidateKind::Action => "action_ref".to_string(),
            CandidateKind::Query => "query_ref".to_string(),
        }),
    );
    node.insert("chain".to_string(), Value::String(chain.to_string()));
    node.insert("execution".to_string(), execution);
    node.insert(
        "bindings".to_string(),
        json!({
            "params": to_bindings_params(&normalized_inputs)
        }),
    );
    if let Some(calculated_overrides) = merged_calculated_overrides(&resolved_spec.merged_spec) {
        let order = match calculated_override_order_from_map(&calculated_overrides) {
            Ok(order) => order,
            Err(_) => {
                issues.push(issue(
                    "compile_error",
                    path_with_key(step_path, "candidate_ref"),
                    &format!(
                        "candidate `{}` has invalid calculated_fields metadata",
                        candidate_ref
                    ),
                    "candidate_invalid_calculated_fields",
                ));
                return Err(issues);
            }
        };
        let mut ordered = Map::<String, Value>::new();
        for key in &order {
            if let Some(value) = calculated_overrides.get(key.as_str()) {
                ordered.insert(key.clone(), value.clone());
            }
        }
        node.insert("calculated_overrides".to_string(), Value::Object(ordered));
        node.insert(
            "calculated_override_order".to_string(),
            Value::Array(order.into_iter().map(Value::String).collect()),
        );
    }
    node.insert(
        "writes".to_string(),
        Value::Array(vec![json!({
            "path": format!("nodes.{node_id}.outputs"),
            "mode": "set"
        })]),
    );
    node.insert(
        "source".to_string(),
        json!({
            "protocol": protocol_ref.clone(),
            source_key: source_leaf
        }),
    );
    let mut plan_sketch_extension = json!({
        "schema": sketch.schema,
        "segment_id": segment_id,
        "step_id": step.id
    });
    plan_sketch_extension["candidate_ref"] = Value::String(candidate_ref.clone());
    if let Some(todo_id) = segment_todo_id {
        plan_sketch_extension["todo_id"] = Value::String(todo_id.to_string());
    }
    if !step.stores.is_empty() {
        plan_sketch_extension["stores"] = json!(step.stores);
    }
    if let Some(token_resolution) = step.extensions.get("token_resolution") {
        plan_sketch_extension["token_resolution"] = token_resolution.clone();
    }
    if matches!(step.kind.as_str(), "assert" | "branch") {
        plan_sketch_extension["step_kind"] = Value::String(step.kind.clone());
    }
    let mut extensions = json!({
        "plan_sketch": plan_sketch_extension,
        "policy": {
            "constraint_templates": step.constraint_templates
        },
    });
    if let Some(meta) = candidate_index.get(candidate_ref.as_str()) {
        copy_risk_metadata_from_candidate(meta, &mut extensions);
    }
    extensions["operation"] = build_operation_extension(&resolved_spec);
    insert_required_query_metadata(
        &mut extensions,
        required_query_names(&resolved_spec),
        step_query_bindings(
            step,
            segment_steps_by_id,
            segment_step_to_node_id,
            protocol_ref.as_str(),
        ),
    );
    merge_policy_extension(&mut extensions, build_policy_extension(&resolved_spec));
    extensions["protocol"] =
        build_protocol_extension(protocol_ref.as_str(), &resolved_spec.deployment);
    insert_sketch_pack_extension(&mut extensions, sketch, resolved_spec.pack.as_ref());
    node.insert("extensions".to_string(), extensions);
    if let Some(when) = &step.when {
        let cel = when.cel.as_str();
        let cel = rewrite_node_refs_in_cel(cel, segment_step_to_node_id);
        node.insert("condition".to_string(), json!({"cel": cel}));
    }
    if let Some(until) = &step.until {
        let mut rewritten_until = until.clone();
        rewrite_node_refs_in_value(&mut rewritten_until, segment_step_to_node_id);
        node.insert("until".to_string(), rewritten_until);
    }
    if let Some(retry) = &step.retry {
        node.insert("retry".to_string(), json!(retry));
    }
    if let Some(timeout_ms) = step.timeout_ms {
        node.insert("timeout_ms".to_string(), Value::Number(timeout_ms.into()));
    }

    if !step.depends_on.is_empty() {
        let mut deps = Vec::<String>::new();
        for dep in &step.depends_on {
            if let Some(mapped) = segment_step_to_node_id.get(dep) {
                deps.push(mapped.clone());
            } else {
                return Err(vec![issue(
                    "compile_error",
                    path_with_key(step_path, "depends_on"),
                    &format!("unknown dependency step `{dep}`"),
                    "missing_required_input",
                )]);
            }
        }
        node.insert(
            "deps".to_string(),
            Value::Array(deps.into_iter().map(Value::String).collect()),
        );
    }

    let node = Value::Object(node);
    match lower_composite_node(&node) {
        Ok(Some(lowered)) => Ok(lowered),
        Ok(None) => Ok(vec![node]),
        Err(error) => Err(vec![issue(
            "compile_error",
            path_with_key(step_path, "candidate_ref"),
            &error,
            "candidate_invalid_composite_execution",
        )]),
    }
}

fn validate_and_resolve_sketch_pack<'a>(
    sketch: &PlanSketchDocument,
    context: &'a ResolverContext,
) -> Result<Option<&'a crate::documents::PackDocument>, Vec<StructuredIssue>> {
    let name = sketch.pack_snapshot.name.as_deref();
    let version = sketch.pack_snapshot.version.as_deref();
    match (name, version) {
        (None, None) => Ok(None),
        (Some(_), None) | (None, Some(_)) => Err(vec![issue(
            "compile_error",
            vec![FieldPathSegment::Key("pack_snapshot".to_string())],
            "pack_snapshot must include both name and version when binding a pack",
            "pack_snapshot_incomplete",
        )]),
        (Some(name), Some(version)) => {
            let pack_key = format!("{name}@{version}");
            let Some(pack) = context.packs.get(pack_key.as_str()) else {
                return Err(vec![issue(
                    "compile_error",
                    vec![
                        FieldPathSegment::Key("pack_snapshot".to_string()),
                        FieldPathSegment::Key("name".to_string()),
                    ],
                    &format!("pack snapshot could not be resolved: {pack_key}"),
                    "pack_snapshot_missing",
                )]);
            };
            let actual_hash = pack_document_hash(pack).unwrap_or_default();
            if !sketch.pack_snapshot.hash.trim().is_empty()
                && sketch.pack_snapshot.hash != actual_hash
            {
                return Err(vec![issue(
                    "compile_error",
                    vec![
                        FieldPathSegment::Key("pack_snapshot".to_string()),
                        FieldPathSegment::Key("hash".to_string()),
                    ],
                    &format!(
                        "pack snapshot hash mismatch for `{pack_key}`: expected `{}`, got `{}`",
                        actual_hash, sketch.pack_snapshot.hash
                    ),
                    "pack_snapshot_hash_mismatch",
                )]);
            }
            Ok(Some(pack))
        }
    }
}

fn insert_sketch_pack_extension(
    extensions: &mut Value,
    sketch: &PlanSketchDocument,
    pack: Option<&crate::protocol::ResolvedPackOperation>,
) {
    let Some(pack) = pack else {
        return;
    };
    let Some(extensions_obj) = extensions.as_object_mut() else {
        return;
    };
    let mut pack_extension = build_pack_extension(pack);
    let Some(pack_extension_obj) = pack_extension.as_object_mut() else {
        return;
    };
    if let Some(name) = sketch.pack_snapshot.name.clone() {
        pack_extension_obj.insert("name".to_string(), Value::String(name));
    }
    if let Some(version) = sketch.pack_snapshot.version.clone() {
        pack_extension_obj.insert("version".to_string(), Value::String(version));
    }
    if !sketch.pack_snapshot.hash.trim().is_empty() {
        pack_extension_obj.insert(
            "hash".to_string(),
            Value::String(sketch.pack_snapshot.hash.clone()),
        );
    }
    extensions_obj.insert("pack".to_string(), pack_extension);
}

fn merged_calculated_overrides(operation_spec: &Value) -> Option<Map<String, Value>> {
    let overrides = operation_spec
        .as_object()
        .and_then(|spec| spec.get("calculated_fields"))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if overrides.is_empty() {
        None
    } else {
        Some(overrides)
    }
}

fn insert_required_query_metadata(
    extensions: &mut Value,
    required_queries: Vec<String>,
    query_bindings: Map<String, Value>,
) {
    if required_queries.is_empty() && query_bindings.is_empty() {
        return;
    }
    let Some(operation) = extensions
        .as_object_mut()
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

fn merge_policy_extension(extensions: &mut Value, policy_extension: Value) {
    let Some(policy_extension_obj) = policy_extension.as_object() else {
        return;
    };
    if policy_extension_obj.is_empty() {
        return;
    }
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

fn step_query_bindings(
    step: &crate::documents::PlanSketchStep,
    segment_steps_by_id: &HashMap<String, crate::documents::PlanSketchStep>,
    segment_step_to_node_id: &HashMap<String, String>,
    protocol_ref: &str,
) -> Map<String, Value> {
    let mut bindings = Map::<String, Value>::new();
    for dep in &step.depends_on {
        let Some(dep_step) = segment_steps_by_id.get(dep) else {
            continue;
        };
        if dep_step.kind != "query" {
            continue;
        }
        let Some(candidate_ref) = normalized_candidate_ref(dep_step.candidate_ref.as_deref())
        else {
            continue;
        };
        let Some((dep_protocol_ref, query_name)) = candidate_ref.split_once('/') else {
            continue;
        };
        if dep_protocol_ref != protocol_ref {
            continue;
        }
        let Some(node_id) = segment_step_to_node_id.get(dep) else {
            continue;
        };
        bindings.entry(query_name.to_string()).or_insert_with(|| {
            json!({
                "node_id": node_id,
                "query_ref": candidate_ref
            })
        });
    }
    bindings
}

fn normalized_candidate_ref(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

#[derive(Debug, Clone)]
struct ControlStepMeta {
    depends_on: Vec<String>,
    condition_cel: Option<String>,
}

fn resolve_non_control_dependencies(
    deps: &[String],
    control_steps: &HashMap<String, ControlStepMeta>,
    inherited_conditions: &mut Vec<String>,
) -> Vec<String> {
    let mut out = Vec::<String>::new();
    let mut seen = HashSet::<String>::new();
    let mut stack_guard = HashSet::<String>::new();
    for dep in deps {
        expand_dependency(
            dep,
            control_steps,
            inherited_conditions,
            &mut out,
            &mut seen,
            &mut stack_guard,
        );
    }
    out
}

fn expand_dependency(
    dep: &str,
    control_steps: &HashMap<String, ControlStepMeta>,
    inherited_conditions: &mut Vec<String>,
    out: &mut Vec<String>,
    seen: &mut HashSet<String>,
    stack_guard: &mut HashSet<String>,
) {
    if let Some(control) = control_steps.get(dep) {
        if !stack_guard.insert(dep.to_string()) {
            return;
        }
        if let Some(cel) = control.condition_cel.as_ref() {
            inherited_conditions.push(cel.clone());
        }
        for upstream in &control.depends_on {
            expand_dependency(
                upstream.as_str(),
                control_steps,
                inherited_conditions,
                out,
                seen,
                stack_guard,
            );
        }
        stack_guard.remove(dep);
        return;
    }
    if seen.insert(dep.to_string()) {
        out.push(dep.to_string());
    }
}

fn merge_condition_cels(base: Option<&str>, inherited_conditions: &[String]) -> Option<String> {
    let mut conditions = Vec::<String>::new();
    let mut seen = HashSet::<String>::new();
    if let Some(base_condition) = base
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
    {
        if seen.insert(base_condition.clone()) {
            conditions.push(base_condition);
        }
    }
    for condition in inherited_conditions {
        let trimmed = condition.trim();
        if !trimmed.is_empty() {
            let normalized = trimmed.to_string();
            if seen.insert(normalized.clone()) {
                conditions.push(normalized);
            }
        }
    }
    if conditions.is_empty() {
        return None;
    }
    if conditions.len() == 1 {
        return conditions.first().cloned();
    }
    Some(
        conditions
            .into_iter()
            .map(|condition| format!("({condition})"))
            .collect::<Vec<_>>()
            .join(" && "),
    )
}

fn control_or_step_condition_cel(step: &crate::documents::PlanSketchStep) -> Option<String> {
    if let Some(when) = step.when.as_ref() {
        let cel = when.cel.trim();
        if !cel.is_empty() {
            return Some(cel.to_string());
        }
    }
    step.inputs.get("condition").and_then(|raw| {
        raw.get("cel")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
}

fn validate_required_params(
    operation_spec: &Value,
    inputs: &Map<String, Value>,
    candidate_ref: &str,
    issues: &mut Vec<StructuredIssue>,
) {
    let Some(params) = operation_spec
        .as_object()
        .and_then(|obj| obj.get("params"))
        .and_then(Value::as_array)
    else {
        return;
    };

    for (param_index, param) in params.iter().enumerate() {
        let Some(param_obj) = param.as_object() else {
            continue;
        };
        let required = param_obj
            .get("required")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if !required {
            continue;
        }
        let Some(name) = param_obj.get("name").and_then(Value::as_str) else {
            continue;
        };
        if !inputs.contains_key(name) {
            issues.push(issue(
                "compile_error",
                vec![
                    FieldPathSegment::Key("params".to_string()),
                    FieldPathSegment::Index(param_index),
                    FieldPathSegment::Key("name".to_string()),
                ],
                &format!("missing required input `{name}` for `{}`", candidate_ref),
                "missing_required_input",
            ));
        }
    }
}

fn validate_step_runtime_controls(
    step: &crate::documents::PlanSketchStep,
    step_path: &[FieldPathSegment],
    issues: &mut Vec<StructuredIssue>,
) {
    if let Some(until) = &step.until {
        if let Err(error) = serde_json::from_value::<ValueRef>(until.clone()) {
            issues.push(issue(
                "compile_error",
                path_with_key(step_path, "until"),
                &format!("until must be a valid ValueRef: {error}"),
                "input_type_mismatch",
            ));
        }
    }

    if let Some(retry) = &step.retry {
        if retry.interval_ms == 0 {
            issues.push(issue(
                "compile_error",
                path_with_key(step_path, "retry"),
                "retry.interval_ms must be a positive integer",
                "input_type_mismatch",
            ));
        }
        if retry.max_attempts == Some(0) {
            issues.push(issue(
                "compile_error",
                path_with_key(step_path, "retry"),
                "retry.max_attempts must be a positive integer when provided",
                "input_type_mismatch",
            ));
        }
    }

    if step.timeout_ms == Some(0) {
        issues.push(issue(
            "compile_error",
            path_with_key(step_path, "timeout_ms"),
            "timeout_ms must be a positive integer when provided",
            "input_type_mismatch",
        ));
    }
}

fn to_bindings_params(inputs: &Map<String, Value>) -> Map<String, Value> {
    let mut params = Map::<String, Value>::new();
    let mut keys = inputs.keys().cloned().collect::<Vec<_>>();
    keys.sort();
    for key in keys {
        let Some(value) = inputs.get(key.as_str()) else {
            continue;
        };
        if looks_like_valueref(value) {
            params.insert(key, value.clone());
        } else {
            params.insert(key, json!({ "lit": value }));
        }
    }
    params
}

fn normalize_step_inputs(
    operation_spec: &Value,
    inputs: &Map<String, Value>,
    chain: &str,
    candidate_ref: &str,
    step_path: &[FieldPathSegment],
    issues: &mut Vec<StructuredIssue>,
) -> Map<String, Value> {
    let mut param_types = HashMap::<String, String>::new();
    if let Some(params) = operation_spec
        .as_object()
        .and_then(|obj| obj.get("params"))
        .and_then(Value::as_array)
    {
        for param in params {
            let Some(param_obj) = param.as_object() else {
                continue;
            };
            let Some(name) = param_obj.get("name").and_then(Value::as_str) else {
                continue;
            };
            let Some(param_type) = param_obj.get("type").and_then(Value::as_str) else {
                continue;
            };
            param_types.insert(name.to_string(), param_type.to_string());
        }
    }

    let mut out = Map::<String, Value>::new();
    let mut keys = inputs.keys().cloned().collect::<Vec<_>>();
    keys.sort();
    for key in keys {
        let Some(value) = inputs.get(key.as_str()) else {
            continue;
        };
        if param_types.get(key.as_str()).map(String::as_str) == Some("asset") {
            match normalize_asset_input(value, chain) {
                Ok(normalized) => {
                    out.insert(key, normalized);
                }
                Err(reason) => {
                    let mut path = step_path.to_vec();
                    path.push(FieldPathSegment::Key("inputs".to_string()));
                    path.push(FieldPathSegment::Key(key.clone()));
                    issues.push(issue(
                        "compile_error",
                        path,
                        &format!("{reason}; candidate={candidate_ref}"),
                        "input_type_mismatch",
                    ));
                    out.insert(key, value.clone());
                }
            }
            continue;
        }
        out.insert(key, value.clone());
    }
    out
}

fn normalize_asset_input(value: &Value, chain: &str) -> Result<Value, String> {
    match value {
        Value::String(address) => Ok(asset_object_value_ref(address.as_str(), chain)),
        Value::Object(object) => {
            if let Some(lit) = object.get("lit") {
                return match lit {
                    Value::String(address) => Ok(asset_object_value_ref(address.as_str(), chain)),
                    Value::Object(asset_obj) => {
                        if asset_obj.get("address").is_none() {
                            return Err(
                                "asset literal object must include `address` field".to_string()
                            );
                        }
                        Ok(json!({
                            "lit": canonicalize_asset_literal_object(asset_obj, chain)
                        }))
                    }
                    _ => {
                        Err("asset input using `lit` must be address string or object".to_string())
                    }
                };
            }
            if looks_like_valueref(value) {
                if let Some(asset_obj) = object.get("object").and_then(Value::as_object) {
                    if asset_obj.get("address").is_none() {
                        return Err("asset input using `object` ValueRef must include `address`"
                            .to_string());
                    }
                    return Ok(json!({
                        "object": canonicalize_asset_valueref_object(asset_obj, chain)
                    }));
                }
                return Ok(value.clone());
            }
            if object.get("address").is_none() {
                return Err("asset input object must include `address` field".to_string());
            }
            Ok(plain_object_to_valueref_object(value, chain))
        }
        _ => Err("asset input must be address string or object".to_string()),
    }
}

fn asset_object_value_ref(address: &str, chain: &str) -> Value {
    json!({
        "object": {
            "address": {"lit": address},
            "chain_id": {"lit": chain}
        }
    })
}

fn plain_object_to_valueref_object(value: &Value, chain: &str) -> Value {
    let mut out = Map::<String, Value>::new();
    let Some(object) = value.as_object() else {
        return value.clone();
    };
    let mut keys = object.keys().cloned().collect::<Vec<_>>();
    keys.sort();
    for key in keys {
        let Some(item) = object.get(key.as_str()) else {
            continue;
        };
        if key == "chain_ref" && !object.contains_key("chain_id") {
            if looks_like_valueref(item) {
                out.insert("chain_id".to_string(), item.clone());
            } else {
                out.insert("chain_id".to_string(), json!({ "lit": item }));
            }
            continue;
        }
        if looks_like_valueref(item) {
            out.insert(key, item.clone());
        } else {
            out.insert(key, json!({ "lit": item }));
        }
    }
    out.entry("chain_id".to_string())
        .or_insert_with(|| json!({ "lit": chain }));
    json!({ "object": out })
}

fn canonicalize_asset_literal_object(asset_obj: &Map<String, Value>, chain: &str) -> Value {
    let mut out = asset_obj.clone();
    if !out.contains_key("chain_id") {
        if let Some(chain_ref) = out.remove("chain_ref") {
            out.insert("chain_id".to_string(), chain_ref);
        } else {
            out.insert("chain_id".to_string(), Value::String(chain.to_string()));
        }
    } else {
        out.remove("chain_ref");
    }
    Value::Object(out)
}

fn canonicalize_asset_valueref_object(asset_obj: &Map<String, Value>, chain: &str) -> Value {
    let mut out = asset_obj.clone();
    if !out.contains_key("chain_id") {
        if let Some(chain_ref) = out.remove("chain_ref") {
            out.insert("chain_id".to_string(), chain_ref);
        } else {
            out.insert("chain_id".to_string(), json!({ "lit": chain }));
        }
    } else {
        out.remove("chain_ref");
    }
    Value::Object(out)
}

fn looks_like_valueref(value: &Value) -> bool {
    let Some(obj) = value.as_object() else {
        return false;
    };
    let keys = ["lit", "ref", "cel", "object", "array"];
    obj.len() == 1 && keys.iter().any(|key| obj.contains_key(*key))
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

fn chain_matches(pattern: &str, chain: &str) -> bool {
    if pattern == chain || pattern == "*" {
        return true;
    }
    if let Some((pattern_ns, pattern_ref)) = pattern.split_once(':') {
        if let Some((chain_ns, chain_ref)) = chain.split_once(':') {
            if pattern_ns == chain_ns && (pattern_ref == "*" || chain_ref == "*") {
                return true;
            }
        }
    }
    false
}

fn kind_label(kind: CandidateKind) -> &'static str {
    match kind {
        CandidateKind::Action => "action",
        CandidateKind::Query => "query",
    }
}

fn string_array_field(value: &Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn risk_level_field(value: &Value) -> Option<u64> {
    value
        .get("risk_level")
        .and_then(Value::as_u64)
        .filter(|risk_level| (1..=5).contains(risk_level))
}

fn risk_tags_field(value: &Value) -> Vec<String> {
    string_array_field(value, "risk_tags")
        .into_iter()
        .map(|tag| tag.trim().to_string())
        .filter(|tag| !tag.is_empty())
        .collect::<Vec<_>>()
}

fn copy_risk_metadata_from_candidate(meta: &CandidateMeta, extensions: &mut Value) {
    if let Some(risk_level) = meta.risk_level {
        extensions["risk_level"] = Value::Number(risk_level.into());
    }
    if !meta.risk_tags.is_empty() {
        extensions["risk_tags"] = Value::Array(
            meta.risk_tags
                .iter()
                .cloned()
                .map(Value::String)
                .collect::<Vec<_>>(),
        );
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
                "compile_error",
                path.to_vec(),
                "lowered node is missing `id`",
                "candidate_invalid_lowered_missing_id",
            ));
            continue;
        };
        if !seen_node_ids.insert(node_id.to_string()) {
            issues.push(issue(
                "compile_error",
                path.to_vec(),
                &format!("duplicate emitted node id after lowering: {node_id}"),
                "candidate_invalid_lowered_duplicate_id",
            ));
        }
    }
    issues
}

fn issue(
    kind: &str,
    path: Vec<FieldPathSegment>,
    message: &str,
    reason_code: &str,
) -> StructuredIssue {
    StructuredIssue {
        kind: kind.to_string(),
        severity: IssueSeverity::Error,
        node_id: None,
        field_path: FieldPath::from_segments(path),
        message: message.to_string(),
        reference: Some(reason_code.to_string()),
        related: None,
    }
}

fn build_known_input_ref_set(options: &CompilePlanSketchOptions) -> HashSet<String> {
    options
        .known_input_refs
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .filter(|value| value.starts_with("inputs."))
        .map(str::to_string)
        .collect::<HashSet<_>>()
}

fn lint_unknown_input_refs(
    inputs: &Map<String, Value>,
    known_input_refs: &HashSet<String>,
    step_path: &[FieldPathSegment],
    issues: &mut Vec<StructuredIssue>,
) {
    if known_input_refs.is_empty() {
        return;
    }
    let mut input_keys = inputs.keys().cloned().collect::<Vec<_>>();
    input_keys.sort();
    for input_key in input_keys {
        let Some(value) = inputs.get(input_key.as_str()) else {
            continue;
        };
        let mut path = step_path.to_vec();
        path.push(FieldPathSegment::Key("inputs".to_string()));
        path.push(FieldPathSegment::Key(input_key));
        lint_unknown_input_refs_in_value(value, known_input_refs, path.as_slice(), issues);
    }
}

fn lint_unknown_input_refs_in_value(
    value: &Value,
    known_input_refs: &HashSet<String>,
    path: &[FieldPathSegment],
    issues: &mut Vec<StructuredIssue>,
) {
    match value {
        Value::Object(object) => {
            if let Some(reference) = object.get("ref").and_then(Value::as_str) {
                if reference.starts_with("inputs.") && !known_input_refs.contains(reference) {
                    let suggestion = suggest_input_ref(reference, known_input_refs);
                    let hint = suggestion
                        .map(|value| format!("; suggested_ref={value}"))
                        .unwrap_or_default();
                    issues.push(issue(
                        "compile_error",
                        path_with_key(path, "ref"),
                        &format!("unknown input ref `{reference}`{hint}"),
                        "unknown_input_ref",
                    ));
                }
            }
            let mut keys = object.keys().cloned().collect::<Vec<_>>();
            keys.sort();
            for key in keys {
                let Some(child) = object.get(key.as_str()) else {
                    continue;
                };
                let mut child_path = path.to_vec();
                child_path.push(FieldPathSegment::Key(key));
                lint_unknown_input_refs_in_value(
                    child,
                    known_input_refs,
                    child_path.as_slice(),
                    issues,
                );
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                let mut child_path = path.to_vec();
                child_path.push(FieldPathSegment::Index(index));
                lint_unknown_input_refs_in_value(
                    child,
                    known_input_refs,
                    child_path.as_slice(),
                    issues,
                );
            }
        }
        _ => {}
    }
}

fn suggest_input_ref(reference: &str, known_input_refs: &HashSet<String>) -> Option<String> {
    let leaf = reference.rsplit('.').next().unwrap_or(reference);
    let mut known = known_input_refs.iter().cloned().collect::<Vec<_>>();
    known.sort();
    if let Some(exact_leaf) = known
        .iter()
        .find(|candidate| candidate.rsplit('.').next().unwrap_or("") == leaf)
    {
        return Some(exact_leaf.clone());
    }
    let mut best: Option<(usize, String)> = None;
    for candidate in known {
        let score = common_prefix_len(reference, candidate.as_str());
        if score == 0 {
            continue;
        }
        match &best {
            Some((best_score, _)) if *best_score >= score => {}
            _ => best = Some((score, candidate)),
        }
    }
    best.map(|(_, candidate)| candidate)
}

fn common_prefix_len(a: &str, b: &str) -> usize {
    a.chars().zip(b.chars()).take_while(|(x, y)| x == y).count()
}

fn canonical_node_id(segment_id: &str, step_id: &str) -> String {
    format!(
        "{}__{}",
        canonical_identifier_component(segment_id),
        canonical_identifier_component(step_id)
    )
}

fn canonical_identifier_component(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut last_was_underscore = false;
    for ch in value.chars() {
        let allowed = ch.is_ascii_alphanumeric() || ch == '_';
        if allowed {
            out.push(ch);
            last_was_underscore = false;
        } else if !last_was_underscore {
            out.push('_');
            last_was_underscore = true;
        }
    }
    let trimmed = out.trim_matches('_').to_string();
    let mut normalized = if trimmed.is_empty() {
        "node".to_string()
    } else {
        trimmed
    };
    if normalized
        .chars()
        .next()
        .is_some_and(|ch| ch.is_ascii_digit())
    {
        normalized.insert_str(0, "n_");
    }
    normalized
}

fn rewrite_node_refs_in_value(value: &mut Value, step_ref_to_node_id: &HashMap<String, String>) {
    match value {
        Value::Object(object) => {
            if let Some(reference) = object.get("ref").and_then(Value::as_str) {
                let rewritten = rewrite_node_refs_in_path(reference, step_ref_to_node_id);
                object.insert("ref".to_string(), Value::String(rewritten));
            }
            if let Some(cel) = object.get("cel").and_then(Value::as_str) {
                let rewritten = rewrite_node_refs_in_cel(cel, step_ref_to_node_id);
                object.insert("cel".to_string(), Value::String(rewritten));
            }
            let mut keys = object.keys().cloned().collect::<Vec<_>>();
            keys.sort();
            for key in keys {
                let Some(child) = object.get_mut(key.as_str()) else {
                    continue;
                };
                rewrite_node_refs_in_value(child, step_ref_to_node_id);
            }
        }
        Value::Array(items) => {
            for child in items {
                rewrite_node_refs_in_value(child, step_ref_to_node_id);
            }
        }
        _ => {}
    }
}

fn rewrite_node_refs_in_path(path: &str, step_ref_to_node_id: &HashMap<String, String>) -> String {
    if let Some(after_nodes) = path.strip_prefix("nodes.") {
        if let Some((raw_id, tail)) = after_nodes.split_once('.') {
            if let Some(mapped) = resolve_node_ref(raw_id, step_ref_to_node_id) {
                return format!("nodes.{mapped}.{tail}");
            }
        } else if let Some(mapped) = resolve_node_ref(after_nodes, step_ref_to_node_id) {
            return format!("nodes.{mapped}");
        }
    }
    rewrite_node_refs_in_cel(path, step_ref_to_node_id)
}

fn rewrite_node_refs_in_cel(cel: &str, step_ref_to_node_id: &HashMap<String, String>) -> String {
    let bracket_ref_re = Regex::new(r#"nodes\[\s*"([^"]+)"\s*\]|nodes\[\s*'([^']+)'\s*\]"#)
        .expect("valid bracket node ref regex");
    let dot_ref_re =
        Regex::new(r#"\bnodes\.([A-Za-z_][A-Za-z0-9_]*)"#).expect("valid dot node ref regex");

    let bracket_rewritten = bracket_ref_re.replace_all(cel, |caps: &regex::Captures<'_>| {
        let raw_id = caps.get(1).or_else(|| caps.get(2)).map(|m| m.as_str());
        let Some(raw_id) = raw_id else {
            return caps
                .get(0)
                .map(|m| m.as_str().to_string())
                .unwrap_or_default();
        };
        if let Some(mapped) = resolve_node_ref(raw_id, step_ref_to_node_id) {
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
            if let Some(mapped) = resolve_node_ref(raw_id, step_ref_to_node_id) {
                format!("nodes.{mapped}")
            } else {
                caps.get(0)
                    .map(|m| m.as_str().to_string())
                    .unwrap_or_default()
            }
        })
        .into_owned()
}

fn resolve_node_ref(raw_id: &str, step_ref_to_node_id: &HashMap<String, String>) -> Option<String> {
    step_ref_to_node_id.get(raw_id).cloned()
}

fn lint_unknown_node_refs(
    step: &crate::documents::PlanSketchStep,
    normalized_inputs: &Map<String, Value>,
    step_path: &[FieldPathSegment],
    segment_step_ids: &HashSet<String>,
    segment_node_ids: &HashSet<String>,
    issues: &mut Vec<StructuredIssue>,
) {
    if let Some(when) = &step.when {
        lint_unknown_node_refs_in_cel(
            when.cel.as_str(),
            &path_with_key(step_path, "when"),
            segment_step_ids,
            segment_node_ids,
            issues,
        );
    }
    if let Some(until) = &step.until {
        lint_unknown_node_refs_in_value(
            until,
            &path_with_key(step_path, "until"),
            segment_step_ids,
            segment_node_ids,
            issues,
        );
    }
    let mut keys = normalized_inputs.keys().cloned().collect::<Vec<_>>();
    keys.sort();
    for key in keys {
        let Some(value) = normalized_inputs.get(key.as_str()) else {
            continue;
        };
        let mut path = step_path.to_vec();
        path.push(FieldPathSegment::Key("inputs".to_string()));
        path.push(FieldPathSegment::Key(key));
        lint_unknown_node_refs_in_value(
            value,
            path.as_slice(),
            segment_step_ids,
            segment_node_ids,
            issues,
        );
    }
}

fn lint_unknown_node_refs_in_value(
    value: &Value,
    path: &[FieldPathSegment],
    segment_step_ids: &HashSet<String>,
    segment_node_ids: &HashSet<String>,
    issues: &mut Vec<StructuredIssue>,
) {
    match value {
        Value::Object(object) => {
            if let Some(reference) = object.get("ref").and_then(Value::as_str) {
                lint_unknown_node_refs_in_ref_path(
                    reference,
                    &path_with_key(path, "ref"),
                    segment_step_ids,
                    segment_node_ids,
                    issues,
                );
            }
            if let Some(cel) = object.get("cel").and_then(Value::as_str) {
                lint_unknown_node_refs_in_cel(
                    cel,
                    &path_with_key(path, "cel"),
                    segment_step_ids,
                    segment_node_ids,
                    issues,
                );
            }
            let mut keys = object.keys().cloned().collect::<Vec<_>>();
            keys.sort();
            for key in keys {
                let Some(child) = object.get(key.as_str()) else {
                    continue;
                };
                let mut child_path = path.to_vec();
                child_path.push(FieldPathSegment::Key(key));
                lint_unknown_node_refs_in_value(
                    child,
                    child_path.as_slice(),
                    segment_step_ids,
                    segment_node_ids,
                    issues,
                );
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                let mut child_path = path.to_vec();
                child_path.push(FieldPathSegment::Index(index));
                lint_unknown_node_refs_in_value(
                    child,
                    child_path.as_slice(),
                    segment_step_ids,
                    segment_node_ids,
                    issues,
                );
            }
        }
        _ => {}
    }
}

fn lint_unknown_node_refs_in_ref_path(
    reference: &str,
    path: &[FieldPathSegment],
    segment_step_ids: &HashSet<String>,
    segment_node_ids: &HashSet<String>,
    issues: &mut Vec<StructuredIssue>,
) {
    if let Some(after_nodes) = reference.strip_prefix("nodes.") {
        let raw_id = after_nodes.split('.').next().unwrap_or_default();
        lint_node_ref_id(raw_id, path, segment_step_ids, segment_node_ids, issues);
        return;
    }
    for raw_id in extract_node_refs(reference) {
        lint_node_ref_id(
            raw_id.as_str(),
            path,
            segment_step_ids,
            segment_node_ids,
            issues,
        );
    }
}

fn lint_unknown_node_refs_in_cel(
    cel: &str,
    path: &[FieldPathSegment],
    segment_step_ids: &HashSet<String>,
    segment_node_ids: &HashSet<String>,
    issues: &mut Vec<StructuredIssue>,
) {
    for raw_id in extract_node_refs(cel) {
        lint_node_ref_id(
            raw_id.as_str(),
            path,
            segment_step_ids,
            segment_node_ids,
            issues,
        );
    }
}

fn extract_node_refs(text: &str) -> Vec<String> {
    let bracket_ref_re = Regex::new(r#"nodes\[\s*"([^"]+)"\s*\]|nodes\[\s*'([^']+)'\s*\]"#)
        .expect("valid bracket node ref regex");
    let dot_ref_re = Regex::new(r#"\bnodes\.([A-Za-z0-9_/-]+)"#).expect("valid dot node ref regex");
    let mut out = Vec::<String>::new();
    for caps in bracket_ref_re.captures_iter(text) {
        if let Some(raw_id) = caps.get(1).or_else(|| caps.get(2)).map(|m| m.as_str()) {
            out.push(raw_id.to_string());
        }
    }
    for caps in dot_ref_re.captures_iter(text) {
        if let Some(raw_id) = caps.get(1).map(|m| m.as_str()) {
            out.push(raw_id.to_string());
        }
    }
    out
}

fn lint_node_ref_id(
    raw_id: &str,
    path: &[FieldPathSegment],
    segment_step_ids: &HashSet<String>,
    segment_node_ids: &HashSet<String>,
    issues: &mut Vec<StructuredIssue>,
) {
    if raw_id.trim().is_empty() {
        return;
    }
    if raw_id.contains('/') {
        issues.push(issue(
            "compile_error",
            path.to_vec(),
            &format!(
                "node ref `{raw_id}` is not allowed; use step ids in the same segment (no `segment/step` form)"
            ),
            "non_local_node_ref",
        ));
        return;
    }
    if segment_step_ids.contains(raw_id) || segment_node_ids.contains(raw_id) {
        return;
    }
    issues.push(issue(
        "compile_error",
        path.to_vec(),
        &format!("unknown node ref `{raw_id}` in segment"),
        "unknown_node_ref",
    ));
}

fn path_with_key(path: &[FieldPathSegment], key: &str) -> Vec<FieldPathSegment> {
    let mut out = path.to_vec();
    out.push(FieldPathSegment::Key(key.to_string()));
    out
}

#[cfg(test)]
#[path = "compile_plan_sketch_test.rs"]
mod tests;
