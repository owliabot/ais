use crate::documents::{PackDocument, ProtocolDocument, WorkflowDocument};
use crate::validate::workflow::validate_workflow_imports;
use ais_core::{FieldPath, FieldPathSegment, IssueSeverity, StructuredIssue};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy)]
pub struct WorkspaceDocuments<'a> {
    pub protocols: &'a [ProtocolDocument],
    pub packs: &'a [PackDocument],
    pub workflows: &'a [WorkflowDocument],
}

pub fn validate_workspace_references(docs: WorkspaceDocuments<'_>) -> Vec<StructuredIssue> {
    let mut issues = Vec::new();

    let mut protocol_by_key: HashMap<String, &ProtocolDocument> = HashMap::new();
    let mut protocol_versions_by_id: HashMap<String, HashSet<String>> = HashMap::new();

    for protocol in docs.protocols {
        let Some((protocol_id, version)) = protocol_identity(protocol) else {
            issues.push(issue(
                "workspace_error",
                IssueSeverity::Error,
                vec![FieldPathSegment::Key("meta".to_string())],
                "protocol meta must contain protocol+version",
                "workspace.protocol.identity",
                None,
            ));
            continue;
        };

        let key = format!("{protocol_id}@{version}");
        if protocol_by_key.insert(key.clone(), protocol).is_some() {
            issues.push(issue(
                "workspace_error",
                IssueSeverity::Error,
                vec![FieldPathSegment::Key("meta".to_string())],
                &format!("duplicate protocol version in workspace: {key}"),
                "workspace.protocol.duplicate",
                Some(json!({ "protocol": key })),
            ));
        }

        protocol_versions_by_id
            .entry(protocol_id)
            .or_default()
            .insert(version);
    }

    for (protocol_id, versions) in protocol_versions_by_id {
        if versions.len() > 1 {
            issues.push(issue(
                "workspace_error",
                IssueSeverity::Error,
                vec![
                    FieldPathSegment::Key("meta".to_string()),
                    FieldPathSegment::Key("version".to_string()),
                ],
                &format!("multiple protocol versions found for `{protocol_id}`"),
                "workspace.protocol.multiple_versions",
                Some(json!({
                    "protocol": protocol_id,
                    "versions": versions.into_iter().collect::<Vec<_>>()
                })),
            ));
        }
    }

    let mut pack_by_key: HashMap<String, &PackDocument> = HashMap::new();
    let mut pack_versions_by_name: HashMap<String, HashSet<String>> = HashMap::new();

    for pack in docs.packs {
        let Some((pack_name, pack_version)) = pack_identity(pack) else {
            issues.push(issue(
                "workspace_error",
                IssueSeverity::Error,
                vec![FieldPathSegment::Key("name".to_string())],
                "pack must contain name+version (top-level or meta)",
                "workspace.pack.identity",
                None,
            ));
            continue;
        };

        let key = format!("{pack_name}@{pack_version}");
        if pack_by_key.insert(key.clone(), pack).is_some() {
            issues.push(issue(
                "workspace_error",
                IssueSeverity::Error,
                vec![FieldPathSegment::Key("meta".to_string())],
                &format!("duplicate pack version in workspace: {key}"),
                "workspace.pack.duplicate",
                Some(json!({ "pack": key })),
            ));
        }

        pack_versions_by_name
            .entry(pack_name)
            .or_default()
            .insert(pack_version);
    }

    for (pack_name, versions) in pack_versions_by_name {
        if versions.len() > 1 {
            issues.push(issue(
                "workspace_error",
                IssueSeverity::Info,
                vec![
                    FieldPathSegment::Key("meta".to_string()),
                    FieldPathSegment::Key("version".to_string()),
                ],
                &format!("multiple pack versions found for `{pack_name}`"),
                "workspace.pack.multiple_versions",
                Some(json!({
                    "pack": pack_name,
                    "versions": versions.into_iter().collect::<Vec<_>>()
                })),
            ));
        }
    }

    for pack in docs.packs {
        for (include_index, include) in pack.includes.iter().enumerate() {
            let Some((protocol_id, version)) = include_protocol_key(include) else {
                issues.push(issue(
                    "workspace_error",
                    IssueSeverity::Error,
                    vec![
                        FieldPathSegment::Key("includes".to_string()),
                        FieldPathSegment::Index(include_index),
                    ],
                    "pack include must contain protocol+version",
                    "workspace.pack.include_identity",
                    None,
                ));
                continue;
            };

            let key = format!("{protocol_id}@{version}");
            if !protocol_by_key.contains_key(&key) {
                issues.push(issue(
                    "workspace_error",
                    IssueSeverity::Error,
                    vec![
                        FieldPathSegment::Key("includes".to_string()),
                        FieldPathSegment::Index(include_index),
                    ],
                    &format!("pack include references missing protocol: {key}"),
                    "workspace.pack.include_missing_protocol",
                    Some(json!({ "protocol": key })),
                ));
            }

            if let Some(scope) = include_chain_scope(include) {
                let unique = scope.iter().collect::<HashSet<_>>();
                if unique.len() != scope.len() {
                    issues.push(issue(
                        "workspace_error",
                        IssueSeverity::Warning,
                        vec![
                            FieldPathSegment::Key("includes".to_string()),
                            FieldPathSegment::Index(include_index),
                            FieldPathSegment::Key("chain_scope".to_string()),
                        ],
                        &format!("pack include `{key}` has duplicate chain_scope entries"),
                        "workspace.pack.include_chain_scope_duplicates",
                        None,
                    ));
                }
            }
        }
    }

    let known_protocol_keys = protocol_by_key.keys().cloned().collect::<HashSet<_>>();

    for workflow in docs.workflows {
        issues.extend(validate_workflow_imports(
            workflow,
            Some(&known_protocol_keys),
        ));

        let required_pack = workflow_requires_pack(workflow);
        let selected_pack = required_pack
            .as_ref()
            .and_then(|pack_key| pack_by_key.get(pack_key).copied());

        if let Some(pack_key) = &required_pack {
            if selected_pack.is_none() {
                issues.push(issue(
                    "workspace_error",
                    IssueSeverity::Error,
                    vec![FieldPathSegment::Key("requires_pack".to_string())],
                    &format!("workflow requires missing pack: {pack_key}"),
                    "workspace.workflow.requires_pack_missing",
                    Some(json!({ "pack": pack_key })),
                ));
            }
        }

        let include_index = selected_pack.map(build_pack_include_index);
        let default_chain = workflow.default_chain.as_deref();

        for (node_index, node) in workflow.nodes.iter().enumerate() {
            let Some((protocol_key, node_type, leaf, chain)) =
                extract_node_reference(node, default_chain)
            else {
                issues.push(issue(
                    "workspace_error",
                    IssueSeverity::Error,
                    vec![
                        FieldPathSegment::Key("nodes".to_string()),
                        FieldPathSegment::Index(node_index),
                    ],
                    "workflow node must declare protocol reference",
                    "workspace.workflow.node_protocol_missing",
                    None,
                ));
                continue;
            };

            let protocol = match protocol_by_key.get(&protocol_key) {
                Some(protocol) => *protocol,
                None => {
                    issues.push(issue(
                        "workspace_error",
                        IssueSeverity::Error,
                        vec![
                            FieldPathSegment::Key("nodes".to_string()),
                            FieldPathSegment::Index(node_index),
                            FieldPathSegment::Key("protocol".to_string()),
                        ],
                        &format!("workflow node references missing protocol: {protocol_key}"),
                        "workspace.workflow.protocol_missing",
                        Some(json!({ "protocol": protocol_key })),
                    ));
                    continue;
                }
            };

            if let Some(index) = include_index.as_ref() {
                if !index.includes.contains(&protocol_key) {
                    issues.push(issue(
                        "workspace_error",
                        IssueSeverity::Error,
                        vec![
                            FieldPathSegment::Key("nodes".to_string()),
                            FieldPathSegment::Index(node_index),
                            FieldPathSegment::Key("protocol".to_string()),
                        ],
                        &format!(
                            "node protocol `{protocol_key}` is not included by workflow required pack"
                        ),
                        "workspace.workflow.protocol_not_in_pack",
                        required_pack
                            .as_ref()
                            .map(|pack| json!({ "pack": pack, "protocol": protocol_key })),
                    ));
                } else if let Some(node_chain) = chain.as_deref() {
                    if let Some(include) = index.by_protocol.get(&protocol_key) {
                        if let Some(scope) = include_chain_scope(include) {
                            if !scope.contains(&node_chain) {
                                issues.push(issue(
                                    "workspace_error",
                                    IssueSeverity::Error,
                                    vec![
                                        FieldPathSegment::Key("nodes".to_string()),
                                        FieldPathSegment::Index(node_index),
                                        FieldPathSegment::Key("chain".to_string()),
                                    ],
                                    &format!(
                                        "node chain `{node_chain}` is outside pack chain_scope for `{protocol_key}`"
                                    ),
                                    "workspace.workflow.chain_scope_violation",
                                    required_pack.as_ref().map(|pack| {
                                        json!({
                                            "pack": pack,
                                            "protocol": protocol_key,
                                            "chain": node_chain
                                        })
                                    }),
                                ));
                            }
                        }
                    }
                }
            }

            match node_type.as_deref() {
                Some("action_ref") => {
                    if let Some(action) = leaf {
                        if !protocol.actions.contains_key(action.as_str()) {
                            issues.push(issue(
                                "workspace_error",
                                IssueSeverity::Error,
                                vec![
                                    FieldPathSegment::Key("nodes".to_string()),
                                    FieldPathSegment::Index(node_index),
                                    FieldPathSegment::Key("action".to_string()),
                                ],
                                &format!("action not found: {protocol_key}/{action}"),
                                "workspace.workflow.action_missing",
                                Some(json!({ "protocol": protocol_key, "action": action })),
                            ));
                        }
                    } else {
                        issues.push(issue(
                            "workspace_error",
                            IssueSeverity::Error,
                            vec![
                                FieldPathSegment::Key("nodes".to_string()),
                                FieldPathSegment::Index(node_index),
                                FieldPathSegment::Key("action".to_string()),
                            ],
                            "action_ref node must set `action`",
                            "workspace.workflow.action_required",
                            None,
                        ));
                    }
                }
                Some("query_ref") => {
                    if let Some(query) = leaf {
                        if !protocol.queries.contains_key(query.as_str()) {
                            issues.push(issue(
                                "workspace_error",
                                IssueSeverity::Error,
                                vec![
                                    FieldPathSegment::Key("nodes".to_string()),
                                    FieldPathSegment::Index(node_index),
                                    FieldPathSegment::Key("query".to_string()),
                                ],
                                &format!("query not found: {protocol_key}/{query}"),
                                "workspace.workflow.query_missing",
                                Some(json!({ "protocol": protocol_key, "query": query })),
                            ));
                        }
                    } else {
                        issues.push(issue(
                            "workspace_error",
                            IssueSeverity::Error,
                            vec![
                                FieldPathSegment::Key("nodes".to_string()),
                                FieldPathSegment::Index(node_index),
                                FieldPathSegment::Key("query".to_string()),
                            ],
                            "query_ref node must set `query`",
                            "workspace.workflow.query_required",
                            None,
                        ));
                    }
                }
                _ => {}
            }
        }
    }

    StructuredIssue::sort_stable(&mut issues);
    issues
}

#[derive(Debug, Clone)]
struct PackIncludeIndex<'a> {
    includes: HashSet<String>,
    by_protocol: HashMap<String, &'a Value>,
}

fn build_pack_include_index(pack: &PackDocument) -> PackIncludeIndex<'_> {
    let mut includes = HashSet::new();
    let mut by_protocol = HashMap::new();

    for include in &pack.includes {
        if let Some((protocol, version)) = include_protocol_key(include) {
            let key = format!("{protocol}@{version}");
            includes.insert(key.clone());
            by_protocol.entry(key).or_insert(include);
        }
    }

    PackIncludeIndex {
        includes,
        by_protocol,
    }
}

fn protocol_identity(protocol: &ProtocolDocument) -> Option<(String, String)> {
    let meta = protocol.meta.as_object()?;
    let protocol_id = meta.get("protocol")?.as_str()?.to_string();
    let version = meta.get("version")?.as_str()?.to_string();
    Some((protocol_id, version))
}

fn pack_identity(pack: &PackDocument) -> Option<(String, String)> {
    let meta_name = pack
        .meta
        .as_ref()
        .and_then(Value::as_object)
        .and_then(|meta| meta.get("name"))
        .and_then(Value::as_str);
    let meta_version = pack
        .meta
        .as_ref()
        .and_then(Value::as_object)
        .and_then(|meta| meta.get("version"))
        .and_then(Value::as_str);

    let name = meta_name.or(pack.name.as_deref())?;
    let version = meta_version.or(pack.version.as_deref())?;
    Some((name.to_string(), version.to_string()))
}

fn workflow_requires_pack(workflow: &WorkflowDocument) -> Option<String> {
    let requires_pack = workflow.requires_pack.as_ref()?.as_object()?;
    let name = requires_pack.get("name")?.as_str()?;
    let version = requires_pack.get("version")?.as_str()?;
    Some(format!("{name}@{version}"))
}

fn include_protocol_key(include: &Value) -> Option<(String, String)> {
    let include = include.as_object()?;
    let protocol = include.get("protocol")?.as_str()?.to_string();
    let version = include.get("version")?.as_str()?.to_string();
    Some((protocol, version))
}

fn include_chain_scope(include: &Value) -> Option<Vec<&str>> {
    let include = include.as_object()?;
    let scope = include.get("chain_scope")?.as_array()?;
    let mut out = Vec::with_capacity(scope.len());
    for item in scope {
        out.push(item.as_str()?);
    }
    Some(out)
}

fn extract_node_reference(
    node: &Value,
    default_chain: Option<&str>,
) -> Option<(String, Option<String>, Option<String>, Option<String>)> {
    let node = node.as_object()?;
    let node_type = node.get("type").and_then(Value::as_str).map(str::to_string);
    let chain = node
        .get("chain")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| default_chain.map(str::to_string));

    if let Some(protocol) = node.get("protocol").and_then(Value::as_str) {
        let (protocol_id, version) = protocol.split_once('@')?;
        let key = format!("{protocol_id}@{version}");
        let leaf = match node_type.as_deref() {
            Some("action_ref") => node.get("action").and_then(Value::as_str).map(str::to_string),
            Some("query_ref") => node.get("query").and_then(Value::as_str).map(str::to_string),
            _ => None,
        };
        return Some((key, node_type, leaf, chain));
    }

    if let Some(action_ref) = node.get("action_ref").and_then(Value::as_str) {
        let ((protocol_id, version), action) = split_reference(action_ref)?;
        return Some((
            format!("{protocol_id}@{version}"),
            Some("action_ref".to_string()),
            Some(action.to_string()),
            chain,
        ));
    }

    if let Some(query_ref) = node.get("query_ref").and_then(Value::as_str) {
        let ((protocol_id, version), query) = split_reference(query_ref)?;
        return Some((
            format!("{protocol_id}@{version}"),
            Some("query_ref".to_string()),
            Some(query.to_string()),
            chain,
        ));
    }

    None
}

fn split_reference(input: &str) -> Option<((&str, &str), &str)> {
    let (protocol_version, leaf) = input.split_once('/')?;
    let (protocol, version) = protocol_version.split_once('@')?;
    if protocol.is_empty() || version.is_empty() || leaf.is_empty() {
        return None;
    }
    Some(((protocol, version), leaf))
}

fn issue(
    kind: &str,
    severity: IssueSeverity,
    path: Vec<FieldPathSegment>,
    message: &str,
    reference: &str,
    related: Option<Value>,
) -> StructuredIssue {
    StructuredIssue {
        kind: kind.to_string(),
        severity,
        node_id: None,
        field_path: FieldPath::from_segments(path),
        message: message.to_string(),
        reference: Some(reference.to_string()),
        related,
    }
}

#[cfg(test)]
#[path = "workspace_test.rs"]
mod tests;
