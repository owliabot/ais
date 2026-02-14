use crate::documents::WorkflowDocument;
use crate::ValueRef;
use ais_cel::parse_expression;
use ais_core::{FieldPath, FieldPathSegment, IssueSeverity, StructuredIssue};
use regex::Regex;
use serde_json::Value;
use std::collections::{BTreeSet, HashMap, HashSet};

pub fn validate_workflow_document(workflow: &WorkflowDocument) -> Vec<StructuredIssue> {
    let mut issues = Vec::new();
    let mut node_indices: HashMap<String, usize> = HashMap::new();
    let mut explicit_deps_by_index: Vec<Vec<String>> = vec![Vec::new(); workflow.nodes.len()];
    let mut implicit_deps_by_index: Vec<Vec<String>> = vec![Vec::new(); workflow.nodes.len()];
    let declared_inputs: HashSet<String> = workflow.inputs.keys().cloned().collect();
    let mut has_duplicate_node_id = false;

    // First pass: collect node ids.
    for (index, node) in workflow.nodes.iter().enumerate() {
        let base_path = vec![
            FieldPathSegment::Key("nodes".to_string()),
            FieldPathSegment::Index(index),
        ];
        let Some(node_obj) = node.as_object() else {
            issues.push(issue(
                "workflow_error",
                IssueSeverity::Error,
                base_path,
                "workflow node must be an object",
                "workflow.node.object",
            ));
            continue;
        };

        let Some(node_id) = node_obj.get("id").and_then(Value::as_str).map(str::to_string) else {
            issues.push(issue(
                "workflow_error",
                IssueSeverity::Error,
                path_with_key(&base_path, "id"),
                "workflow node must contain string field `id`",
                "workflow.node.id_required",
            ));
            continue;
        };

        if let Some(previous_index) = node_indices.insert(node_id.clone(), index) {
            has_duplicate_node_id = true;
            issues.push(issue(
                "workflow_error",
                IssueSeverity::Error,
                path_with_key(&base_path, "id"),
                &format!(
                    "duplicate node id `{node_id}` found (first at nodes[{previous_index}])"
                ),
                "workflow.node.duplicate_id",
            ));
        }

    }

    let node_id_set: HashSet<String> = node_indices.keys().cloned().collect();

    // Second pass: validate deps and refs.
    for (index, node) in workflow.nodes.iter().enumerate() {
        let Some(node_obj) = node.as_object() else {
            continue;
        };
        let Some(node_id) = node_obj.get("id").and_then(Value::as_str) else {
            continue;
        };

        let base_path = vec![
            FieldPathSegment::Key("nodes".to_string()),
            FieldPathSegment::Index(index),
        ];

        let deps = extract_explicit_deps(node_obj, &base_path, node_id, &mut issues);
        explicit_deps_by_index[index] = deps.clone();

        let mut local_implicit = Vec::new();
        collect_refs_from_node(
            node_obj,
            node_id,
            &declared_inputs,
            &node_id_set,
            &base_path,
            &mut local_implicit,
            &mut issues,
        );
        validate_assert_fields(node_obj, &base_path, &mut issues);
        implicit_deps_by_index[index] = local_implicit;

        for (dep_idx, dep) in deps.iter().enumerate() {
            if dep == node_id {
                issues.push(issue(
                    "workflow_error",
                    IssueSeverity::Error,
                    vec![
                        FieldPathSegment::Key("nodes".to_string()),
                        FieldPathSegment::Index(index),
                        FieldPathSegment::Key("deps".to_string()),
                        FieldPathSegment::Index(dep_idx),
                    ],
                    "node cannot depend on itself",
                    "workflow.deps.self",
                ));
                continue;
            }
            if !node_id_set.contains(dep) {
                issues.push(issue(
                    "workflow_error",
                    IssueSeverity::Error,
                    vec![
                        FieldPathSegment::Key("nodes".to_string()),
                        FieldPathSegment::Index(index),
                        FieldPathSegment::Key("deps".to_string()),
                        FieldPathSegment::Index(dep_idx),
                    ],
                    &format!("dependency `{dep}` does not exist"),
                    "workflow.deps.unknown",
                ));
            }
        }
    }

    // Validate workflow outputs value-refs.
    let mut output_implicit = Vec::new();
    for (key, value_ref) in &workflow.outputs {
        let path = vec![
            FieldPathSegment::Key("outputs".to_string()),
            FieldPathSegment::Key(key.clone()),
        ];
        validate_value_ref_like(
            value_ref,
            None,
            false,
            &declared_inputs,
            &node_id_set,
            &path,
            &mut output_implicit,
            &mut issues,
        );
    }

    if !has_duplicate_node_id {
        let explicit_deps = build_node_deps_map(&workflow.nodes, &explicit_deps_by_index);
        let implicit_deps = build_node_deps_map(&workflow.nodes, &implicit_deps_by_index);
        if let Some(cycle) = detect_cycle(&node_id_set, &explicit_deps, &implicit_deps) {
            let cycle_text = cycle.join(" -> ");
            for cycle_node in cycle {
                if let Some(index) = node_indices.get(&cycle_node) {
                    issues.push(issue(
                        "workflow_error",
                        IssueSeverity::Error,
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
        }
    }

    issues.extend(validate_workflow_imports(workflow, None));
    StructuredIssue::sort_stable(&mut issues);
    issues
}

pub(crate) fn validate_workflow_imports(
    workflow: &WorkflowDocument,
    known_protocols: Option<&HashSet<String>>,
) -> Vec<StructuredIssue> {
    let mut issues = Vec::new();
    let mut node_protocols = Vec::<(usize, String)>::new();

    for (index, node) in workflow.nodes.iter().enumerate() {
        let Some(node_obj) = node.as_object() else {
            continue;
        };
        let base_path = vec![
            FieldPathSegment::Key("nodes".to_string()),
            FieldPathSegment::Index(index),
        ];
        let Some(protocol_ref) = node_obj.get("protocol").and_then(Value::as_str) else {
            issues.push(issue(
                "workflow_error",
                IssueSeverity::Error,
                path_with_key(&base_path, "protocol"),
                "workflow node must include `protocol`",
                "workflow.node.protocol_required",
            ));
            continue;
        };
        let Some(protocol_key) = parse_protocol_key(protocol_ref) else {
            issues.push(issue(
                "workflow_error",
                IssueSeverity::Error,
                path_with_key(&base_path, "protocol"),
                "protocol must be in form protocol@version",
                "workflow.node.protocol_format",
            ));
            continue;
        };
        node_protocols.push((index, protocol_key));
    }

    let Some(imports) = workflow.imports.as_ref() else {
        return issues;
    };
    let Some(imports_obj) = imports.as_object() else {
        issues.push(issue(
            "workflow_error",
            IssueSeverity::Error,
            vec![FieldPathSegment::Key("imports".to_string())],
            "imports must be an object",
            "workflow.imports.object",
        ));
        return issues;
    };
    let Some(protocols_value) = imports_obj.get("protocols") else {
        return issues;
    };
    let Some(protocols) = protocols_value.as_array() else {
        issues.push(issue(
            "workflow_error",
            IssueSeverity::Error,
            path_with_key(&[FieldPathSegment::Key("imports".to_string())], "protocols"),
            "imports.protocols must be an array",
            "workflow.imports.protocols.array",
        ));
        return issues;
    };

    let mut imported_protocols = HashSet::<String>::new();
    for (index, entry) in protocols.iter().enumerate() {
        let base_path = vec![
            FieldPathSegment::Key("imports".to_string()),
            FieldPathSegment::Key("protocols".to_string()),
            FieldPathSegment::Index(index),
        ];

        let Some(entry_obj) = entry.as_object() else {
            issues.push(issue(
                "workflow_error",
                IssueSeverity::Error,
                base_path.clone(),
                "imports.protocols entry must be an object",
                "workflow.imports.protocols.object",
            ));
            continue;
        };

        let protocol_path = path_with_key(&base_path, "protocol");
        let Some(protocol_ref) = entry_obj.get("protocol").and_then(Value::as_str) else {
            issues.push(issue(
                "workflow_error",
                IssueSeverity::Error,
                protocol_path,
                "imports.protocols entry must include string `protocol`",
                "workflow.imports.protocol_required",
            ));
            continue;
        };
        let Some(protocol_key) = parse_protocol_key(protocol_ref) else {
            issues.push(issue(
                "workflow_error",
                IssueSeverity::Error,
                path_with_key(&base_path, "protocol"),
                "imports.protocols protocol must be in form protocol@version",
                "workflow.imports.protocol_format",
            ));
            continue;
        };

        if entry_obj
            .get("path")
            .and_then(Value::as_str)
            .map(str::trim)
            .is_none_or(str::is_empty)
        {
            issues.push(issue(
                "workflow_error",
                IssueSeverity::Error,
                path_with_key(&base_path, "path"),
                "imports.protocols entry must include non-empty string `path`",
                "workflow.imports.path_required",
            ));
        }

        if entry_obj
            .get("integrity")
            .is_some_and(|value| value.as_str().map(str::trim).is_none_or(str::is_empty))
        {
            issues.push(issue(
                "workflow_error",
                IssueSeverity::Error,
                path_with_key(&base_path, "integrity"),
                "imports.protocols integrity must be a non-empty string",
                "workflow.imports.integrity_string",
            ));
        }

        if !imported_protocols.insert(protocol_key.clone()) {
            issues.push(issue(
                "workflow_error",
                IssueSeverity::Error,
                path_with_key(&base_path, "protocol"),
                &format!("duplicate imported protocol `{protocol_key}`"),
                "workflow.imports.protocol_duplicate",
            ));
        }

        if known_protocols.is_some_and(|known| !known.contains(&protocol_key)) {
            issues.push(issue(
                "workflow_error",
                IssueSeverity::Error,
                path_with_key(&base_path, "protocol"),
                &format!("imported protocol `{protocol_key}` not found in workspace"),
                "workflow.imports.protocol_missing_in_workspace",
            ));
        }
    }

    for (node_index, node_protocol) in node_protocols {
        if !imported_protocols.contains(&node_protocol) {
            issues.push(issue(
                "workflow_error",
                IssueSeverity::Error,
                vec![
                    FieldPathSegment::Key("nodes".to_string()),
                    FieldPathSegment::Index(node_index),
                    FieldPathSegment::Key("protocol".to_string()),
                ],
                &format!("node protocol `{node_protocol}` is not declared in imports.protocols"),
                "workflow.imports.node_protocol_not_imported",
            ));
        }
    }

    issues
}

fn extract_explicit_deps(
    node_obj: &serde_json::Map<String, Value>,
    base_path: &[FieldPathSegment],
    _node_id: &str,
    issues: &mut Vec<StructuredIssue>,
) -> Vec<String> {
    let mut deps = Vec::new();
    if let Some(value) = node_obj.get("deps") {
        match value.as_array() {
            Some(dep_arr) => {
                for (dep_idx, dep_value) in dep_arr.iter().enumerate() {
                    match dep_value.as_str() {
                        Some(dep) if !dep.trim().is_empty() => deps.push(dep.to_string()),
                        _ => issues.push(issue(
                            "workflow_error",
                            IssueSeverity::Error,
                            path_with_key_index(base_path, "deps", dep_idx),
                            "dependency id must be a non-empty string",
                            "workflow.deps.invalid",
                        )),
                    }
                }
            }
            None => issues.push(issue(
                "workflow_error",
                IssueSeverity::Error,
                path_with_key(base_path, "deps"),
                "deps must be an array",
                "workflow.deps.array",
            )),
        }
    }
    deps
}

fn collect_refs_from_node(
    node_obj: &serde_json::Map<String, Value>,
    node_id: &str,
    declared_inputs: &HashSet<String>,
    node_id_set: &HashSet<String>,
    base_path: &[FieldPathSegment],
    implicit_deps: &mut Vec<String>,
    issues: &mut Vec<StructuredIssue>,
) {
    if let Some(args) = node_obj.get("args").and_then(Value::as_object) {
        for (key, value_ref) in args {
            validate_value_ref_like(
                value_ref,
                Some(node_id),
                false,
                declared_inputs,
                node_id_set,
                &path_with_key(&path_with_key(base_path, "args"), key),
                implicit_deps,
                issues,
            );
        }
    }

    for field in ["condition", "assert", "until"] {
        if let Some(value_ref) = node_obj.get(field) {
            let allow_self = matches!(field, "assert" | "until");
            validate_value_ref_like(
                value_ref,
                Some(node_id),
                allow_self,
                declared_inputs,
                node_id_set,
                &path_with_key(base_path, field),
                implicit_deps,
                issues,
            );
        }
    }

    if let Some(overrides) = node_obj.get("calculated_overrides").and_then(Value::as_object) {
        for (key, item) in overrides {
            if let Some(expr) = item.as_object().and_then(|obj| obj.get("expr")) {
                validate_value_ref_like(
                    expr,
                    Some(node_id),
                    false,
                    declared_inputs,
                    node_id_set,
                    &path_with_key(
                        &path_with_key(&path_with_key(base_path, "calculated_overrides"), key),
                        "expr",
                    ),
                    implicit_deps,
                    issues,
                );
            }
        }
    }
}

fn validate_assert_fields(
    node_obj: &serde_json::Map<String, Value>,
    base_path: &[FieldPathSegment],
    issues: &mut Vec<StructuredIssue>,
) {
    let assert_value = node_obj.get("assert");
    let assert_message_value = node_obj.get("assert_message");

    if let Some(message) = assert_message_value {
        match message.as_str() {
            Some(value) if !value.trim().is_empty() => {}
            _ => issues.push(issue(
                "workflow_error",
                IssueSeverity::Error,
                path_with_key(base_path, "assert_message"),
                "assert_message must be a non-empty string",
                "workflow.assert.message_invalid",
            )),
        }
        if assert_value.is_none() {
            issues.push(issue(
                "workflow_error",
                IssueSeverity::Error,
                path_with_key(base_path, "assert_message"),
                "assert_message requires assert",
                "workflow.assert.message_without_assert",
            ));
        }
    }

    let Some(assert_value) = assert_value else {
        return;
    };

    let value_ref = match serde_json::from_value::<ValueRef>(assert_value.clone()) {
        Ok(value_ref) => value_ref,
        Err(error) => {
            issues.push(issue(
                "workflow_error",
                IssueSeverity::Error,
                path_with_key(base_path, "assert"),
                &format!("assert must be a valid ValueRef: {error}"),
                "workflow.assert.invalid",
            ));
            return;
        }
    };

    match value_ref {
        ValueRef::Lit { lit } => {
            if !lit.is_boolean() {
                issues.push(issue(
                    "workflow_error",
                    IssueSeverity::Error,
                    path_with_key(base_path, "assert"),
                    "assert literal must be boolean",
                    "workflow.assert.not_boolean",
                ));
            }
        }
        ValueRef::Cel { cel } => {
            if let Err(error) = parse_expression(cel.as_str()) {
                issues.push(issue(
                    "workflow_error",
                    IssueSeverity::Error,
                    path_with_key(base_path, "assert"),
                    &format!("assert CEL syntax error: {error}"),
                    "workflow.assert.cel_invalid",
                ));
            }
        }
        _ => {}
    }
}

fn build_node_deps_map(nodes: &[Value], deps_by_index: &[Vec<String>]) -> HashMap<String, Vec<String>> {
    let mut out = HashMap::new();
    for (index, node) in nodes.iter().enumerate() {
        let Some(node_id) = node
            .as_object()
            .and_then(|obj| obj.get("id"))
            .and_then(Value::as_str)
        else {
            continue;
        };
        out.entry(node_id.to_string())
            .or_insert_with(Vec::new)
            .extend(deps_by_index.get(index).cloned().unwrap_or_default());
    }
    out
}

fn validate_value_ref_like(
    value: &Value,
    current_node: Option<&str>,
    allow_self_node_ref: bool,
    declared_inputs: &HashSet<String>,
    node_id_set: &HashSet<String>,
    field_path: &[FieldPathSegment],
    implicit_deps: &mut Vec<String>,
    issues: &mut Vec<StructuredIssue>,
) {
    let mut ref_paths = Vec::new();
    let mut cel_expressions = Vec::new();
    collect_ref_paths_and_cel(value, &mut ref_paths, &mut cel_expressions);

    for path in ref_paths {
        validate_ref_path(
            path.as_str(),
            current_node,
            allow_self_node_ref,
            declared_inputs,
            node_id_set,
            field_path,
            implicit_deps,
            issues,
        );
    }

    for cel in cel_expressions {
        for input_id in extract_ids_from_cel(cel.as_str(), Namespace::Inputs) {
            if !declared_inputs.contains(&input_id) {
                issues.push(issue(
                    "workflow_error",
                    IssueSeverity::Error,
                    field_path.to_vec(),
                    &format!("input `{input_id}` referenced but not declared"),
                    "workflow.ref.input_missing",
                ));
            }
        }
        for node_id in extract_ids_from_cel(cel.as_str(), Namespace::Nodes) {
            if !node_id_set.contains(&node_id) {
                issues.push(issue(
                    "workflow_error",
                    IssueSeverity::Error,
                    field_path.to_vec(),
                    &format!("node `{node_id}` referenced but does not exist"),
                    "workflow.ref.node_missing",
                ));
                continue;
            }

            if !allow_self_node_ref && current_node == Some(node_id.as_str()) {
                issues.push(issue(
                    "workflow_error",
                    IssueSeverity::Error,
                    field_path.to_vec(),
                    "node cannot reference its own outputs in this field",
                    "workflow.ref.self_node",
                ));
                continue;
            }

            if current_node != Some(node_id.as_str()) {
                implicit_deps.push(node_id);
            }
        }
    }
}

fn validate_ref_path(
    path: &str,
    current_node: Option<&str>,
    allow_self_node_ref: bool,
    declared_inputs: &HashSet<String>,
    node_id_set: &HashSet<String>,
    field_path: &[FieldPathSegment],
    implicit_deps: &mut Vec<String>,
    issues: &mut Vec<StructuredIssue>,
) {
    let normalized = path.trim().trim_start_matches('$').trim_start_matches('.');
    if normalized.is_empty() {
        issues.push(issue(
            "workflow_error",
            IssueSeverity::Error,
            field_path.to_vec(),
            "ref path must not be empty",
            "workflow.ref.empty",
        ));
        return;
    }
    let parts: Vec<&str> = normalized.split('.').filter(|part| !part.is_empty()).collect();
    if parts.is_empty() {
        return;
    }

    if !is_allowed_runtime_root(parts[0]) {
        issues.push(issue(
            "workflow_error",
            IssueSeverity::Error,
            field_path.to_vec(),
            &format!("ref root `{}` is not resolvable under runtime roots", parts[0]),
            "workflow.ref.invalid_root",
        ));
        return;
    }

    match parts[0] {
        "inputs" => {
            if let Some(input_id) = parts.get(1) {
                if !declared_inputs.contains(*input_id) {
                    issues.push(issue(
                        "workflow_error",
                        IssueSeverity::Error,
                        field_path.to_vec(),
                        &format!("input `{input_id}` referenced but not declared"),
                        "workflow.ref.input_missing",
                    ));
                }
            }
        }
        "nodes" => {
            if let Some(node_id) = parts.get(1) {
                if !node_id_set.contains(*node_id) {
                    issues.push(issue(
                        "workflow_error",
                        IssueSeverity::Error,
                        field_path.to_vec(),
                        &format!("node `{node_id}` referenced but does not exist"),
                        "workflow.ref.node_missing",
                    ));
                    return;
                }
                if !allow_self_node_ref && current_node == Some(*node_id) {
                    issues.push(issue(
                        "workflow_error",
                        IssueSeverity::Error,
                        field_path.to_vec(),
                        "node cannot reference its own outputs in this field",
                        "workflow.ref.self_node",
                    ));
                    return;
                }
                if current_node != Some(*node_id) {
                    implicit_deps.push((*node_id).to_string());
                }
            }
        }
        _ => {}
    }
}

fn collect_ref_paths_and_cel(value: &Value, ref_paths: &mut Vec<String>, cel_expressions: &mut Vec<String>) {
    let Some(obj) = value.as_object() else {
        return;
    };

    if let Some(ref_path) = obj.get("ref").and_then(Value::as_str) {
        ref_paths.push(ref_path.to_string());
    }
    if let Some(cel) = obj.get("cel").and_then(Value::as_str) {
        cel_expressions.push(cel.to_string());
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

#[derive(Clone, Copy)]
enum Namespace {
    Nodes,
    Inputs,
}

fn extract_ids_from_cel(cel: &str, namespace: Namespace) -> Vec<String> {
    let pattern = match namespace {
        Namespace::Nodes => r"\bnodes\.([A-Za-z_][A-Za-z0-9_-]*)\b",
        Namespace::Inputs => r"\binputs\.([A-Za-z_][A-Za-z0-9_-]*)\b",
    };
    let regex = Regex::new(pattern).expect("valid regex");
    regex
        .captures_iter(cel)
        .filter_map(|capture| capture.get(1).map(|m| m.as_str().to_string()))
        .collect()
}

fn parse_protocol_key(input: &str) -> Option<String> {
    let (protocol, version) = input.split_once('@')?;
    let protocol = protocol.trim();
    let version = version.trim();
    if protocol.is_empty() || version.is_empty() {
        return None;
    }
    Some(format!("{protocol}@{version}"))
}

fn is_allowed_runtime_root(root: &str) -> bool {
    matches!(
        root,
        "inputs" | "params" | "ctx" | "contracts" | "nodes" | "policy" | "query" | "calculated"
    )
}

fn detect_cycle(
    node_ids: &HashSet<String>,
    explicit_deps: &HashMap<String, Vec<String>>,
    implicit_deps: &HashMap<String, Vec<String>>,
) -> Option<Vec<String>> {
    let mut combined: HashMap<String, BTreeSet<String>> = HashMap::new();
    for node in node_ids {
        combined.entry(node.clone()).or_default();
    }

    for (node, deps) in explicit_deps {
        let entry = combined.entry(node.clone()).or_default();
        for dep in deps {
            if node_ids.contains(dep) {
                entry.insert(dep.clone());
            }
        }
    }
    for (node, deps) in implicit_deps {
        let entry = combined.entry(node.clone()).or_default();
        for dep in deps {
            if node_ids.contains(dep) {
                entry.insert(dep.clone());
            }
        }
    }

    let mut state: HashMap<String, u8> = HashMap::new();
    let mut stack = Vec::<String>::new();

    for node in node_ids {
        if *state.get(node).unwrap_or(&0) != 0 {
            continue;
        }
        if let Some(cycle) = dfs_cycle(node, &combined, &mut state, &mut stack) {
            return Some(cycle);
        }
    }
    None
}

fn dfs_cycle(
    node: &str,
    deps: &HashMap<String, BTreeSet<String>>,
    state: &mut HashMap<String, u8>,
    stack: &mut Vec<String>,
) -> Option<Vec<String>> {
    state.insert(node.to_string(), 1);
    stack.push(node.to_string());

    if let Some(children) = deps.get(node) {
        for dep in children {
            match state.get(dep).copied().unwrap_or(0) {
                0 => {
                    if let Some(cycle) = dfs_cycle(dep, deps, state, stack) {
                        return Some(cycle);
                    }
                }
                1 => {
                    if let Some(start) = stack.iter().position(|item| item == dep) {
                        let mut cycle = stack[start..].to_vec();
                        cycle.push(dep.clone());
                        return Some(cycle);
                    }
                }
                _ => {}
            }
        }
    }

    state.insert(node.to_string(), 2);
    stack.pop();
    None
}

fn issue(
    kind: &str,
    severity: IssueSeverity,
    path: Vec<FieldPathSegment>,
    message: &str,
    reference: &str,
) -> StructuredIssue {
    StructuredIssue {
        kind: kind.to_string(),
        severity,
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
#[path = "workflow_test.rs"]
mod tests;
