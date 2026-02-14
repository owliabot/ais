use super::{FieldPath, FieldPathParseError, FieldPathSegment};
use std::str::FromStr;

#[test]
fn root_path_roundtrip() {
    let parsed = FieldPath::from_str("$").expect("must parse root");
    assert!(parsed.is_root());
    assert_eq!(parsed.to_string(), "$");
}

#[test]
fn dotted_path_roundtrip() {
    let parsed = FieldPath::from_str("$.nodes[0].outputs.value").expect("must parse");
    assert_eq!(
        parsed.segments(),
        &[
            FieldPathSegment::Key("nodes".to_string()),
            FieldPathSegment::Index(0),
            FieldPathSegment::Key("outputs".to_string()),
            FieldPathSegment::Key("value".to_string()),
        ]
    );
    assert_eq!(parsed.to_string(), "$.nodes[0].outputs.value");
}

#[test]
fn plain_identifier_is_accepted() {
    let parsed = FieldPath::from_str("inputs.amount").expect("must parse");
    assert_eq!(parsed.to_string(), "$.inputs.amount");
}

#[test]
fn invalid_index_rejected() {
    let err = FieldPath::from_str("$.nodes[]").expect_err("must reject");
    assert_eq!(err, FieldPathParseError::InvalidIndex);
}
