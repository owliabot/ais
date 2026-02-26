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

#[test]
fn parse_json_plan_sketch_dispatches_by_schema() {
    let input = r#"{
      "schema":"ais-plan-sketch/0.1.0",
      "intent":"check and transfer",
      "pack_snapshot":{"hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
      "catalog_snapshot":{"schema":"ais-catalog/0.0.1","hash":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"},
      "segments":[{"segment_id":"s1","cursor_in":"c0","cursor_out":"c1","done":false,"steps":[{"id":"step1","kind":"query","candidate_ref":"erc20@0.0.2/balance-of","inputs":{}}]}]
    }"#;
    let options = ParseDocumentOptions {
        format: DocumentFormat::Json,
        validate_schema: true,
    };

    let parsed = parse_document_with_options(input, options).expect("must parse");
    match parsed {
        AisDocument::PlanSketch(_) => {}
        _ => panic!("expected plan sketch document"),
    }
}
