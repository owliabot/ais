use super::{parse_document_with_options, AisDocument, DocumentFormat, ParseDocumentOptions};

#[test]
fn parse_json_plan_dispatches_by_schema() {
    let input = r#"{"schema":"ais-plan/0.0.3","nodes":[]}"#;
    let options = ParseDocumentOptions {
        format: DocumentFormat::Json,
        validate_schema: true,
    };

    let parsed = parse_document_with_options(input, options).expect("must parse");
    match parsed {
        AisDocument::Plan(_) => {}
        _ => panic!("expected plan document"),
    }
}

#[test]
fn parse_yaml_duplicate_keys_is_rejected() {
    let input = r#"
schema: ais-plan/0.0.3
nodes: []
meta:
  name: a
  name: b
"#;
    let options = ParseDocumentOptions {
        format: DocumentFormat::Yaml,
        validate_schema: false,
    };

    let issues = parse_document_with_options(input, options).expect_err("must reject");
    assert!(issues
        .iter()
        .any(|issue| issue.reference.as_deref() == Some("yaml.duplicate_key")));
}

#[test]
fn parse_unknown_schema_is_rejected() {
    let input = r#"{"schema":"ais-unknown/0.0.1"}"#;
    let options = ParseDocumentOptions {
        format: DocumentFormat::Json,
        validate_schema: false,
    };

    let issues = parse_document_with_options(input, options).expect_err("must reject");
    assert!(issues
        .iter()
        .any(|issue| issue.reference.as_deref() == Some("parse.unsupported_schema")));
}
