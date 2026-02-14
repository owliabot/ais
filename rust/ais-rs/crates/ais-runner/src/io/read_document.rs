use ais_core::{FieldPath, IssueSeverity, StructuredIssue};
use ais_sdk::{
    parse_document_with_options, AisDocument, DocumentFormat, PackDocument, ParseDocumentOptions,
    PlanDocument, ProtocolDocument, WorkflowDocument,
};
use serde_json::{json, Map, Value};
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

            match parse_document_with_options(
                text.as_str(),
                ParseDocumentOptions {
                    format: DocumentFormat::Auto,
                    validate_schema: true,
                },
            ) {
                Ok(document) => match document {
                    AisDocument::Protocol(protocol) => loaded.protocols.push(protocol),
                    AisDocument::Pack(pack) => loaded.packs.push(pack),
                    AisDocument::Workflow(workflow) => loaded.workflows.push(workflow),
                    AisDocument::Plan(plan) => loaded.plans.push(plan),
                    AisDocument::Catalog(_) | AisDocument::PlanSkeleton(_) => {}
                },
                Err(parse_issues) => issues.extend(
                    parse_issues
                        .into_iter()
                        .map(|issue| attach_issue_file(issue, path.as_path())),
                ),
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

fn is_document_candidate(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some("json") | Some("yaml") | Some("yml")
    )
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
