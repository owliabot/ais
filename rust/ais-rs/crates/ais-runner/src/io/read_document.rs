use ais_core::{FieldPath, IssueSeverity, StructuredIssue};
use ais_sdk::{
    parse_document_with_options, validate_document_semantics, AisDocument, DocumentFormat,
    PackDocument, ParseDocumentOptions, PlanDocument, ProtocolDocument, WorkflowDocument,
};
use serde_json::{json, Map, Value};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default)]
pub struct LoadedWorkspaceDocuments {
    pub protocols: Vec<ProtocolDocument>,
    pub packs: Vec<PackDocument>,
    pub workflows: Vec<WorkflowDocument>,
    pub plans: Vec<PlanDocument>,
}

pub fn load_workspace_documents(
    workspace_root: impl AsRef<Path>,
) -> Result<LoadedWorkspaceDocuments, Vec<StructuredIssue>> {
    load_workspace_documents_excluding(workspace_root, &[])
}

pub fn load_workspace_documents_excluding(
    workspace_root: impl AsRef<Path>,
    exclude_files: &[PathBuf],
) -> Result<LoadedWorkspaceDocuments, Vec<StructuredIssue>> {
    let root = workspace_root.as_ref();
    let excluded = canonicalize_excludes(exclude_files);
    let mut issues = Vec::<StructuredIssue>::new();
    let mut loaded = LoadedWorkspaceDocuments::default();

    let mut pending = vec![root.to_path_buf()];
    while let Some(current) = pending.pop() {
        let entries = match fs::read_dir(&current) {
            Ok(entries) => entries,
            Err(error) => {
                issues.push(issue_with_path(
                    "workspace_io_error",
                    FieldPath::root(),
                    format!("read_dir failed: {error}"),
                    "runner.workspace.read_dir_failed",
                    current.as_path(),
                ));
                continue;
            }
        };

        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    issues.push(issue_with_path(
                        "workspace_io_error",
                        FieldPath::root(),
                        format!("read_dir entry failed: {error}"),
                        "runner.workspace.read_dir_entry_failed",
                        current.as_path(),
                    ));
                    continue;
                }
            };

            let path = entry.path();
            if path.is_dir() {
                if should_skip_directory(root, path.as_path()) {
                    continue;
                }
                pending.push(path);
                continue;
            }
            if !is_document_candidate(path.as_path()) || should_skip(path.as_path(), &excluded) {
                continue;
            }

            let text = match fs::read_to_string(&path) {
                Ok(text) => text,
                Err(error) => {
                    issues.push(issue_with_path(
                        "workspace_io_error",
                        FieldPath::root(),
                        format!("read file failed: {error}"),
                        "runner.workspace.read_file_failed",
                        path.as_path(),
                    ));
                    continue;
                }
            };
            if !looks_like_workspace_ais_document(text.as_str(), path.as_path()) {
                continue;
            }

            match parse_document_with_options(
                text.as_str(),
                ParseDocumentOptions {
                    format: DocumentFormat::Auto,
                    validate_schema: true,
                },
            ) {
                Ok(document) => {
                    let semantic_issues = validate_document_semantics(&document);
                    if !semantic_issues.is_empty() {
                        issues.extend(
                            semantic_issues
                                .into_iter()
                                .map(|issue| attach_issue_file(issue, path.as_path())),
                        );
                        continue;
                    }

                    match document {
                        AisDocument::Protocol(protocol) => loaded.protocols.push(protocol),
                        AisDocument::Pack(pack) => loaded.packs.push(pack),
                        AisDocument::Workflow(workflow) => loaded.workflows.push(workflow),
                        AisDocument::Plan(plan) => loaded.plans.push(plan),
                        AisDocument::Catalog(_)
                        | AisDocument::PlanSkeleton(_)
                        | AisDocument::PlanSketch(_) => {}
                    }
                }
                Err(parse_issues) => issues.extend(
                    parse_issues
                        .into_iter()
                        .map(|issue| attach_issue_file(issue, path.as_path())),
                ),
            }
        }
    }

    let registry_protocols =
        load_registry_protocol_includes(root, &loaded.packs, &loaded.protocols);
    match registry_protocols {
        Ok(protocols) => loaded.protocols.extend(protocols),
        Err(mut registry_issues) => issues.append(&mut registry_issues),
    }

    if issues.is_empty() {
        Ok(loaded)
    } else {
        StructuredIssue::sort_stable(&mut issues);
        Err(issues)
    }
}

fn is_document_candidate(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some("json") | Some("yaml") | Some("yml")
    )
}

fn looks_like_workspace_ais_document(text: &str, path: &Path) -> bool {
    if let Some(schema) = parse_schema_field(text, path) {
        return is_workspace_document_schema(schema.as_str());
    }
    raw_text_mentions_ais_schema(text)
}

fn parse_schema_field(text: &str, path: &Path) -> Option<String> {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("json") => serde_json::from_str::<Value>(text).ok(),
        Some("yaml") | Some("yml") => serde_yaml::from_str::<Value>(text).ok(),
        _ => serde_yaml::from_str::<Value>(text)
            .ok()
            .or_else(|| serde_json::from_str::<Value>(text).ok()),
    }
    .and_then(|value| {
        value
            .as_object()
            .and_then(|object| object.get("schema"))
            .and_then(Value::as_str)
            .map(str::to_string)
    })
}

fn is_workspace_document_schema(schema: &str) -> bool {
    matches!(
        schema,
        value if value.starts_with("ais/")
            || value.starts_with("ais-pack/")
            || value.starts_with("ais-flow/")
            || value.starts_with("ais-plan/")
            || value.starts_with("ais-catalog/")
            || value.starts_with("ais-plan-sketch/")
            || value.starts_with("ais-plan-skeleton/")
    )
}

fn raw_text_mentions_ais_schema(text: &str) -> bool {
    text.lines().any(|line| {
        let trimmed = line.trim_start();
        (trimmed.starts_with("schema:")
            && trimmed["schema:".len()..].trim_start().starts_with("ais"))
            || (trimmed.starts_with("\"schema\"") && trimmed.contains("\"ais"))
    })
}

fn canonicalize_excludes(exclude_files: &[PathBuf]) -> Vec<PathBuf> {
    exclude_files
        .iter()
        .map(|path| fs::canonicalize(path).unwrap_or_else(|_| path.clone()))
        .collect()
}

fn should_skip(path: &Path, excludes: &[PathBuf]) -> bool {
    let target = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    excludes.iter().any(|excluded| excluded == &target)
}

fn should_skip_directory(workspace_root: &Path, path: &Path) -> bool {
    let registry_root = workspace_root.join(".ais").join("registry");
    path == registry_root || path.starts_with(&registry_root)
}

fn load_registry_protocol_includes(
    workspace_root: &Path,
    packs: &[PackDocument],
    workspace_protocols: &[ProtocolDocument],
) -> Result<Vec<ProtocolDocument>, Vec<StructuredIssue>> {
    let mut issues = Vec::<StructuredIssue>::new();
    let mut loaded = Vec::<ProtocolDocument>::new();
    let mut seen = workspace_protocols
        .iter()
        .filter_map(protocol_identity)
        .collect::<HashSet<_>>();

    for pack in packs {
        for include in &pack.includes {
            let Some(include_object) = include.as_object() else {
                continue;
            };
            if include_object.get("source").and_then(Value::as_str) != Some("registry") {
                continue;
            }

            let Some(protocol) = include_object.get("protocol").and_then(Value::as_str) else {
                continue;
            };
            let Some(version) = include_object.get("version").and_then(Value::as_str) else {
                continue;
            };
            let key = format!("{protocol}@{version}");
            if seen.contains(&key) {
                continue;
            }

            let Some(path) = find_registry_protocol_snapshot(workspace_root, protocol, version)
            else {
                issues.push(registry_issue(
                    "runner.workspace.registry_protocol_missing",
                    format!("registry include references missing protocol snapshot: {key}"),
                    workspace_root,
                    Some(json!({
                        "protocol": key,
                        "source": "registry",
                        "registry_root": workspace_root.join(".ais").join("registry").display().to_string(),
                    })),
                ));
                continue;
            };

            let text = match fs::read_to_string(&path) {
                Ok(text) => text,
                Err(error) => {
                    issues.push(issue_with_path(
                        "workspace_io_error",
                        FieldPath::root(),
                        format!("read registry protocol failed: {error}"),
                        "runner.workspace.registry_read_failed",
                        path.as_path(),
                    ));
                    continue;
                }
            };

            let document = match parse_document_with_options(
                text.as_str(),
                ParseDocumentOptions {
                    format: DocumentFormat::Auto,
                    validate_schema: true,
                },
            ) {
                Ok(document) => document,
                Err(parse_issues) => {
                    issues.extend(
                        parse_issues
                            .into_iter()
                            .map(|issue| attach_issue_file(issue, path.as_path())),
                    );
                    continue;
                }
            };

            let protocol_document = match document {
                AisDocument::Protocol(protocol_document) => protocol_document,
                _ => {
                    issues.push(issue_with_path(
                        "workspace_error",
                        FieldPath::root(),
                        format!("registry snapshot for `{key}` must be an AIS protocol document"),
                        "runner.workspace.registry_not_protocol_document",
                        path.as_path(),
                    ));
                    continue;
                }
            };

            match protocol_identity(&protocol_document) {
                Some(identity) if identity == key => {
                    seen.insert(identity);
                    loaded.push(protocol_document);
                }
                Some(identity) => issues.push(issue_with_path(
                    "workspace_error",
                    FieldPath::root(),
                    format!(
                        "registry snapshot identity mismatch: expected `{key}`, found `{identity}`"
                    ),
                    "runner.workspace.registry_identity_mismatch",
                    path.as_path(),
                )),
                None => issues.push(issue_with_path(
                    "workspace_error",
                    FieldPath::root(),
                    format!("registry snapshot for `{key}` is missing protocol identity"),
                    "runner.workspace.registry_identity_missing",
                    path.as_path(),
                )),
            }
        }
    }

    if issues.is_empty() {
        Ok(loaded)
    } else {
        StructuredIssue::sort_stable(&mut issues);
        Err(issues)
    }
}

fn find_registry_protocol_snapshot(
    workspace_root: &Path,
    protocol: &str,
    version: &str,
) -> Option<PathBuf> {
    let base = workspace_root
        .join(".ais")
        .join("registry")
        .join("protocols")
        .join(protocol);
    [
        base.join(format!("{version}.json")),
        base.join(format!("{version}.yaml")),
        base.join(format!("{version}.yml")),
        base.join(version).join("protocol.json"),
        base.join(version).join("protocol.yaml"),
        base.join(version).join("protocol.yml"),
    ]
    .into_iter()
    .find(|candidate| candidate.is_file())
}

fn protocol_identity(protocol: &ProtocolDocument) -> Option<String> {
    let meta = protocol.meta.as_object()?;
    let protocol_id = meta.get("protocol")?.as_str()?;
    let version = meta.get("version")?.as_str()?;
    Some(format!("{protocol_id}@{version}"))
}

fn issue_with_path(
    kind: &str,
    field_path: FieldPath,
    message: String,
    reference: &str,
    path: &Path,
) -> StructuredIssue {
    StructuredIssue {
        kind: kind.to_string(),
        severity: IssueSeverity::Error,
        node_id: None,
        field_path,
        message,
        reference: Some(reference.to_string()),
        related: Some(json!({ "file": path.display().to_string() })),
    }
}

fn registry_issue(
    reference: &str,
    message: String,
    workspace_root: &Path,
    related: Option<Value>,
) -> StructuredIssue {
    StructuredIssue {
        kind: "workspace_error".to_string(),
        severity: IssueSeverity::Error,
        node_id: None,
        field_path: FieldPath::root(),
        message,
        reference: Some(reference.to_string()),
        related: Some(match related {
            Some(Value::Object(mut object)) => {
                object.insert(
                    "workspace_root".to_string(),
                    Value::String(workspace_root.display().to_string()),
                );
                Value::Object(object)
            }
            Some(other) => {
                let mut object = Map::new();
                object.insert(
                    "workspace_root".to_string(),
                    Value::String(workspace_root.display().to_string()),
                );
                object.insert("details".to_string(), other);
                Value::Object(object)
            }
            None => {
                let mut object = Map::new();
                object.insert(
                    "workspace_root".to_string(),
                    Value::String(workspace_root.display().to_string()),
                );
                Value::Object(object)
            }
        }),
    }
}

fn attach_issue_file(mut issue: StructuredIssue, path: &Path) -> StructuredIssue {
    let file = Value::String(path.display().to_string());
    issue.related = Some(match issue.related.take() {
        Some(Value::Object(mut object)) => {
            object.insert("file".to_string(), file);
            Value::Object(object)
        }
        Some(other) => {
            let mut object = Map::new();
            object.insert("file".to_string(), file);
            object.insert("details".to_string(), other);
            Value::Object(object)
        }
        None => {
            let mut object = Map::new();
            object.insert("file".to_string(), file);
            Value::Object(object)
        }
    });
    issue
}

#[cfg(test)]
#[path = "read_document_test.rs"]
mod tests;
