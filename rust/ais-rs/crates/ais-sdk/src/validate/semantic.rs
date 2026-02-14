use crate::documents::{
    CatalogDocument, PackDocument, PlanDocument, PlanSkeletonDocument, ProtocolDocument,
    WorkflowDocument,
};
use crate::parse::AisDocument;
use ais_core::{FieldPath, FieldPathSegment, IssueSeverity, StructuredIssue};
use ais_schema::versions::{
    SCHEMA_CATALOG_0_0_1, SCHEMA_PACK_0_0_2, SCHEMA_PLAN_0_0_3, SCHEMA_PLAN_SKELETON_0_0_1,
    SCHEMA_PROTOCOL_0_0_2, SCHEMA_WORKFLOW_0_0_3,
};
use regex::Regex;
use serde_json::Value;

const REF_PATTERN: &str = r"^[a-z0-9-]+@\d+\.\d+\.\d+/[A-Za-z0-9._:-]+$";
const PROTOCOL_ID_PATTERN: &str = r"^[a-z0-9-]+$";
const SEMVER_PATTERN: &str = r"^\d+\.\d+\.\d+$";
const EXECUTION_TYPE_PATTERN: &str = r"^[a-z][a-z0-9_:-]*$";

pub fn validate_document_semantics(document: &AisDocument) -> Vec<StructuredIssue> {
    let mut issues = Vec::new();

    match document {
        AisDocument::Protocol(protocol) => validate_protocol(protocol, &mut issues),
        AisDocument::Pack(pack) => validate_pack(pack, &mut issues),
        AisDocument::Workflow(workflow) => validate_workflow(workflow, &mut issues),
        AisDocument::Plan(plan) => validate_plan(plan, &mut issues),
        AisDocument::Catalog(catalog) => validate_catalog(catalog, &mut issues),
        AisDocument::PlanSkeleton(skeleton) => validate_plan_skeleton(skeleton, &mut issues),
    }

    StructuredIssue::sort_stable(&mut issues);
    issues
}

fn validate_protocol(protocol: &ProtocolDocument, issues: &mut Vec<StructuredIssue>) {
    validate_schema_field(
        &protocol.schema,
        SCHEMA_PROTOCOL_0_0_2,
        vec![FieldPathSegment::Key("schema".to_string())],
        issues,
    );

    if !protocol.meta.is_object() {
        issues.push(issue(
            "semantic_error",
            vec![FieldPathSegment::Key("meta".to_string())],
            "protocol meta must be an object",
            "protocol.meta.object",
        ));
    }

    if protocol.deployments.is_empty() {
        issues.push(issue(
            "semantic_error",
            vec![FieldPathSegment::Key("deployments".to_string())],
            "protocol deployments must not be empty",
            "protocol.deployments.non_empty",
        ));
    }

    validate_capabilities_list(
        &protocol.capabilities_required,
        vec![FieldPathSegment::Key("capabilities_required".to_string())],
        issues,
    );

    for (action_id, action_spec) in &protocol.actions {
        let path = vec![
            FieldPathSegment::Key("actions".to_string()),
            FieldPathSegment::Key(action_id.clone()),
        ];
        validate_operation_spec(action_spec, path, issues);
    }

    for (query_id, query_spec) in &protocol.queries {
        let path = vec![
            FieldPathSegment::Key("queries".to_string()),
            FieldPathSegment::Key(query_id.clone()),
        ];
        validate_operation_spec(query_spec, path, issues);
    }
}

fn validate_pack(pack: &PackDocument, issues: &mut Vec<StructuredIssue>) {
    validate_schema_field(
        &pack.schema,
        SCHEMA_PACK_0_0_2,
        vec![FieldPathSegment::Key("schema".to_string())],
        issues,
    );

    let protocol_pattern = Regex::new(PROTOCOL_ID_PATTERN).expect("valid regex");
    let semver_pattern = Regex::new(SEMVER_PATTERN).expect("valid regex");

    if pack.includes.is_empty() {
        issues.push(issue(
            "semantic_error",
            vec![FieldPathSegment::Key("includes".to_string())],
            "pack includes must not be empty",
            "pack.includes.non_empty",
        ));
    }

    for (index, include) in pack.includes.iter().enumerate() {
        let base_path = vec![
            FieldPathSegment::Key("includes".to_string()),
            FieldPathSegment::Index(index),
        ];

        let Some(include_object) = include.as_object() else {
            issues.push(issue(
                "semantic_error",
                base_path,
                "pack include must be an object",
                "pack.includes.object",
            ));
            continue;
        };

        match include_object.get("protocol").and_then(Value::as_str) {
            Some(protocol) if protocol_pattern.is_match(protocol) => {}
            _ => issues.push(issue(
                "semantic_error",
                path_with_key(&base_path, "protocol"),
                "pack include protocol must match ^[a-z0-9-]+$",
                "pack.includes.protocol",
            )),
        }

        match include_object.get("version").and_then(Value::as_str) {
            Some(version) if semver_pattern.is_match(version) => {}
            _ => issues.push(issue(
                "semantic_error",
                path_with_key(&base_path, "version"),
                "pack include version must match semver x.y.z",
                "pack.includes.version",
            )),
        }
    }
}

fn validate_workflow(workflow: &WorkflowDocument, issues: &mut Vec<StructuredIssue>) {
    validate_schema_field(
        &workflow.schema,
        SCHEMA_WORKFLOW_0_0_3,
        vec![FieldPathSegment::Key("schema".to_string())],
        issues,
    );

    if !workflow.meta.is_object() {
        issues.push(issue(
            "semantic_error",
            vec![FieldPathSegment::Key("meta".to_string())],
            "workflow meta must be an object",
            "workflow.meta.object",
        ));
    }

    if workflow.nodes.is_empty() {
        issues.push(issue(
            "semantic_error",
            vec![FieldPathSegment::Key("nodes".to_string())],
            "workflow nodes must not be empty",
            "workflow.nodes.non_empty",
        ));
    }

    let ref_pattern = Regex::new(REF_PATTERN).expect("valid regex");

    for (index, node) in workflow.nodes.iter().enumerate() {
        let base_path = vec![
            FieldPathSegment::Key("nodes".to_string()),
            FieldPathSegment::Index(index),
        ];

        let Some(node_object) = node.as_object() else {
            issues.push(issue(
                "semantic_error",
                base_path,
                "workflow node must be an object",
                "workflow.nodes.object",
            ));
            continue;
        };

        let action_ref = node_object.get("action_ref").and_then(Value::as_str);
        let query_ref = node_object.get("query_ref").and_then(Value::as_str);
        if action_ref.is_none() && query_ref.is_none() {
            issues.push(issue(
                "semantic_error",
                path_with_key(&base_path, "action_ref"),
                "workflow node must contain either action_ref or query_ref",
                "workflow.nodes.ref_required",
            ));
        }

        if let Some(value) = action_ref {
            if !ref_pattern.is_match(value) {
                issues.push(issue(
                    "semantic_error",
                    path_with_key(&base_path, "action_ref"),
                    "action_ref must match protocol@version/action",
                    "workflow.nodes.action_ref",
                ));
            }
        }

        if let Some(value) = query_ref {
            if !ref_pattern.is_match(value) {
                issues.push(issue(
                    "semantic_error",
                    path_with_key(&base_path, "query_ref"),
                    "query_ref must match protocol@version/query",
                    "workflow.nodes.query_ref",
                ));
            }
        }

        if let Some(deps) = node_object.get("deps") {
            match deps.as_array() {
                Some(items) => {
                    for (dep_index, dep) in items.iter().enumerate() {
                        if dep.as_str().map(str::trim).filter(|it| !it.is_empty()).is_none() {
                            issues.push(issue(
                                "semantic_error",
                                path_with_key_index(&base_path, "deps", dep_index),
                                "workflow node dep must be a non-empty string",
                                "workflow.nodes.deps",
                            ));
                        }
                    }
                }
                None => issues.push(issue(
                    "semantic_error",
                    path_with_key(&base_path, "deps"),
                    "workflow node deps must be an array",
                    "workflow.nodes.deps_type",
                )),
            }
        }
    }
}

fn validate_plan(plan: &PlanDocument, issues: &mut Vec<StructuredIssue>) {
    validate_schema_field(
        &plan.schema,
        SCHEMA_PLAN_0_0_3,
        vec![FieldPathSegment::Key("schema".to_string())],
        issues,
    );

    if plan.nodes.is_empty() {
        issues.push(issue(
            "semantic_error",
            vec![FieldPathSegment::Key("nodes".to_string())],
            "plan nodes must not be empty",
            "plan.nodes.non_empty",
        ));
    }
}

fn validate_catalog(catalog: &CatalogDocument, issues: &mut Vec<StructuredIssue>) {
    validate_schema_field(
        &catalog.schema,
        SCHEMA_CATALOG_0_0_1,
        vec![FieldPathSegment::Key("schema".to_string())],
        issues,
    );
}

fn validate_plan_skeleton(skeleton: &PlanSkeletonDocument, issues: &mut Vec<StructuredIssue>) {
    validate_schema_field(
        &skeleton.schema,
        SCHEMA_PLAN_SKELETON_0_0_1,
        vec![FieldPathSegment::Key("schema".to_string())],
        issues,
    );

    if skeleton.nodes.is_empty() {
        issues.push(issue(
            "semantic_error",
            vec![FieldPathSegment::Key("nodes".to_string())],
            "plan skeleton nodes must not be empty",
            "plan_skeleton.nodes.non_empty",
        ));
    }
}

fn validate_operation_spec(
    spec: &Value,
    base_path: Vec<FieldPathSegment>,
    issues: &mut Vec<StructuredIssue>,
) {
    let Some(object) = spec.as_object() else {
        issues.push(issue(
            "semantic_error",
            base_path,
            "operation spec must be an object",
            "protocol.operation.object",
        ));
        return;
    };

    if let Some(capabilities) = object.get("capabilities_required") {
        match capabilities.as_array() {
            Some(items) => {
                for (index, item) in items.iter().enumerate() {
                    if item.as_str().map(str::trim).filter(|it| !it.is_empty()).is_none() {
                        issues.push(issue(
                            "semantic_error",
                            path_with_key_index(&base_path, "capabilities_required", index),
                            "capability must be a non-empty string",
                            "protocol.capabilities.non_empty",
                        ));
                    }
                }
            }
            None => issues.push(issue(
                "semantic_error",
                path_with_key(&base_path, "capabilities_required"),
                "capabilities_required must be an array",
                "protocol.capabilities.array",
            )),
        }
    }

    if let Some(execution) = object.get("execution") {
        let Some(execution_map) = execution.as_object() else {
            issues.push(issue(
                "semantic_error",
                path_with_key(&base_path, "execution"),
                "execution must be an object",
                "protocol.execution.object",
            ));
            return;
        };

        for (chain_key, exec_spec) in execution_map {
            let exec_path = path_with_key(&path_with_key(&base_path, "execution"), chain_key);
            let Some(exec_object) = exec_spec.as_object() else {
                issues.push(issue(
                    "semantic_error",
                    exec_path,
                    "execution spec must be an object",
                    "protocol.execution.entry_object",
                ));
                continue;
            };

            let Some(exec_type) = exec_object.get("type").and_then(Value::as_str) else {
                issues.push(issue(
                    "semantic_error",
                    path_with_key(&exec_path, "type"),
                    "execution spec must contain string field `type`",
                    "protocol.execution.type_required",
                ));
                continue;
            };

            if !is_valid_execution_type(exec_type) {
                issues.push(issue(
                    "semantic_error",
                    path_with_key(&exec_path, "type"),
                    "execution type must be lower_snake/plugin style",
                    "protocol.execution.type_format",
                ));
            }
        }
    }
}

fn validate_capabilities_list(
    capabilities: &[String],
    base_path: Vec<FieldPathSegment>,
    issues: &mut Vec<StructuredIssue>,
) {
    for (index, capability) in capabilities.iter().enumerate() {
        if capability.trim().is_empty() {
            issues.push(issue(
                "semantic_error",
                path_with_index(&base_path, index),
                "capability must be a non-empty string",
                "capabilities.non_empty",
            ));
        }
    }
}

fn validate_schema_field(
    actual: &str,
    expected: &str,
    path: Vec<FieldPathSegment>,
    issues: &mut Vec<StructuredIssue>,
) {
    if actual != expected {
        issues.push(issue(
            "semantic_error",
            path,
            &format!("schema mismatch: expected `{expected}`, got `{actual}`"),
            "document.schema_mismatch",
        ));
    }
}

fn is_valid_execution_type(value: &str) -> bool {
    let pattern = Regex::new(EXECUTION_TYPE_PATTERN).expect("valid regex");
    pattern.is_match(value)
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

fn path_with_index(path: &[FieldPathSegment], index: usize) -> Vec<FieldPathSegment> {
    let mut out = path.to_vec();
    out.push(FieldPathSegment::Index(index));
    out
}

fn path_with_key_index(path: &[FieldPathSegment], key: &str, index: usize) -> Vec<FieldPathSegment> {
    let mut out = path.to_vec();
    out.push(FieldPathSegment::Key(key.to_string()));
    out.push(FieldPathSegment::Index(index));
    out
}

#[cfg(test)]
#[path = "semantic_test.rs"]
mod tests;
