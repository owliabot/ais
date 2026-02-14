use super::{IssueSeverity, StructuredIssue};
use crate::FieldPath;

#[test]
fn issues_are_sorted_stably() {
    let mut issues = vec![
        StructuredIssue {
            kind: "validation".to_string(),
            severity: IssueSeverity::Warning,
            node_id: None,
            field_path: "$.b".parse().expect("must parse"),
            message: "second".to_string(),
            reference: None,
            related: None,
        },
        StructuredIssue {
            kind: "validation".to_string(),
            severity: IssueSeverity::Error,
            node_id: Some("n1".to_string()),
            field_path: FieldPath::root(),
            message: "first".to_string(),
            reference: None,
            related: None,
        },
    ];

    StructuredIssue::sort_stable(&mut issues);

    assert_eq!(issues[0].severity, IssueSeverity::Error);
    assert_eq!(issues[1].severity, IssueSeverity::Warning);
}
