mod detect_duplicate_keys;
mod json;
mod yaml;

use crate::documents::{
    CatalogDocument, PackDocument, PlanDocument, PlanSkeletonDocument, ProtocolDocument,
    WorkflowDocument,
};
use ais_core::{FieldPath, IssueSeverity, StructuredIssue};
use ais_schema::validate_schema_instance;
use ais_schema::versions::{
    SCHEMA_CATALOG_0_0_1, SCHEMA_PACK_0_0_2, SCHEMA_PLAN_0_0_3, SCHEMA_PLAN_SKELETON_0_0_1,
    SCHEMA_PROTOCOL_0_0_2, SCHEMA_WORKFLOW_0_0_3,
};
use serde::de::DeserializeOwned;
use serde_json::Value;

pub use detect_duplicate_keys::detect_yaml_duplicate_keys;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentFormat {
    Auto,
    Json,
    Yaml,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParseDocumentOptions {
    pub format: DocumentFormat,
    pub validate_schema: bool,
}

impl Default for ParseDocumentOptions {
    fn default() -> Self {
        Self {
            format: DocumentFormat::Auto,
            validate_schema: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum AisDocument {
    Protocol(ProtocolDocument),
    Pack(PackDocument),
    Workflow(WorkflowDocument),
    Plan(PlanDocument),
    Catalog(CatalogDocument),
    PlanSkeleton(PlanSkeletonDocument),
}

impl AisDocument {
    pub fn schema_id(&self) -> &'static str {
        match self {
            AisDocument::Protocol(_) => SCHEMA_PROTOCOL_0_0_2,
            AisDocument::Pack(_) => SCHEMA_PACK_0_0_2,
            AisDocument::Workflow(_) => SCHEMA_WORKFLOW_0_0_3,
            AisDocument::Plan(_) => SCHEMA_PLAN_0_0_3,
            AisDocument::Catalog(_) => SCHEMA_CATALOG_0_0_1,
            AisDocument::PlanSkeleton(_) => SCHEMA_PLAN_SKELETON_0_0_1,
        }
    }

    pub fn to_value(&self) -> Value {
        match self {
            AisDocument::Protocol(value) => serde_json::to_value(value).expect("serializable"),
            AisDocument::Pack(value) => serde_json::to_value(value).expect("serializable"),
            AisDocument::Workflow(value) => serde_json::to_value(value).expect("serializable"),
            AisDocument::Plan(value) => serde_json::to_value(value).expect("serializable"),
            AisDocument::Catalog(value) => serde_json::to_value(value).expect("serializable"),
            AisDocument::PlanSkeleton(value) => serde_json::to_value(value).expect("serializable"),
        }
    }
}

pub fn parse_document(input: &str) -> Result<AisDocument, Vec<StructuredIssue>> {
    parse_document_with_options(input, ParseDocumentOptions::default())
}

pub fn parse_document_with_options(
    input: &str,
    options: ParseDocumentOptions,
) -> Result<AisDocument, Vec<StructuredIssue>> {
    let value = match options.format {
        DocumentFormat::Auto => {
            if looks_like_json(input) {
                json::parse_json(input)
            } else {
                yaml::parse_yaml(input)
            }
        }
        DocumentFormat::Json => json::parse_json(input),
        DocumentFormat::Yaml => yaml::parse_yaml(input),
    }?;

    let schema_id = extract_schema_id(&value)?;

    if options.validate_schema {
        let mut issues = validate_schema_instance(schema_id.as_str(), &value);
        if !issues.is_empty() {
            StructuredIssue::sort_stable(&mut issues);
            return Err(issues);
        }
    }

    let document = match schema_id.as_str() {
        SCHEMA_PROTOCOL_0_0_2 => {
            AisDocument::Protocol(parse_typed_document::<ProtocolDocument>(value, &schema_id)?)
        }
        SCHEMA_PACK_0_0_2 => {
            AisDocument::Pack(parse_typed_document::<PackDocument>(value, &schema_id)?)
        }
        SCHEMA_WORKFLOW_0_0_3 => {
            AisDocument::Workflow(parse_typed_document::<WorkflowDocument>(value, &schema_id)?)
        }
        SCHEMA_PLAN_0_0_3 => {
            AisDocument::Plan(parse_typed_document::<PlanDocument>(value, &schema_id)?)
        }
        SCHEMA_CATALOG_0_0_1 => {
            AisDocument::Catalog(parse_typed_document::<CatalogDocument>(value, &schema_id)?)
        }
        SCHEMA_PLAN_SKELETON_0_0_1 => {
            AisDocument::PlanSkeleton(parse_typed_document::<PlanSkeletonDocument>(value, &schema_id)?)
        }
        _ => {
            return Err(vec![StructuredIssue {
                kind: "parse_error".to_string(),
                severity: IssueSeverity::Error,
                node_id: None,
                field_path: "$.schema".parse().expect("field path must parse"),
                message: format!("unsupported AIS schema: {schema_id}"),
                reference: Some("parse.unsupported_schema".to_string()),
                related: None,
            }]);
        }
    };

    Ok(document)
}

fn looks_like_json(input: &str) -> bool {
    let trimmed = input.trim_start();
    trimmed.starts_with('{') || trimmed.starts_with('[')
}

fn extract_schema_id(value: &Value) -> Result<String, Vec<StructuredIssue>> {
    let schema_id = value
        .as_object()
        .and_then(|obj| obj.get("schema"))
        .and_then(Value::as_str)
        .map(str::to_string);

    match schema_id {
        Some(schema) => Ok(schema),
        None => Err(vec![StructuredIssue {
            kind: "parse_error".to_string(),
            severity: IssueSeverity::Error,
            node_id: None,
            field_path: FieldPath::root(),
            message: "document must contain string field `schema`".to_string(),
            reference: Some("parse.schema_required".to_string()),
            related: None,
        }]),
    }
}

fn parse_typed_document<T: DeserializeOwned>(
    value: Value,
    schema_id: &str,
) -> Result<T, Vec<StructuredIssue>> {
    serde_json::from_value::<T>(value).map_err(|err| {
        vec![StructuredIssue {
            kind: "parse_error".to_string(),
            severity: IssueSeverity::Error,
            node_id: None,
            field_path: "$".parse().expect("field path must parse"),
            message: format!("typed parse failed for schema {schema_id}: {err}"),
            reference: Some("parse.typed_deserialize_error".to_string()),
            related: None,
        }]
    })
}

#[cfg(test)]
#[path = "mod_test.rs"]
mod tests;
