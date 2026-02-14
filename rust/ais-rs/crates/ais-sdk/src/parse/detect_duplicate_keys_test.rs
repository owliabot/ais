use super::detect_yaml_duplicate_keys;

#[test]
fn duplicate_key_is_detected() {
    let input = r#"
a:
  b: 1
  b: 2
"#;
    let issues = detect_yaml_duplicate_keys(input);
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].field_path.to_string(), "$.a.b");
}

#[test]
fn same_key_under_different_parents_is_not_duplicate() {
    let input = r#"
a:
  b: 1
c:
  b: 2
"#;
    let issues = detect_yaml_duplicate_keys(input);
    assert!(issues.is_empty());
}

#[test]
fn unique_keys_produce_no_issues() {
    let input = r#"
a:
  b: 1
  c: 2
"#;
    let issues = detect_yaml_duplicate_keys(input);
    assert!(issues.is_empty());
}

#[test]
fn same_key_across_array_items_is_not_duplicate() {
    let input = r#"
nodes:
  - id: n1
    action: transfer
  - id: n2
    action: approve
"#;
    let issues = detect_yaml_duplicate_keys(input);
    assert!(issues.is_empty());
}

#[test]
fn duplicate_key_inside_same_array_item_is_detected() {
    let input = r#"
nodes:
  - id: n1
    action: transfer
    action: approve
"#;
    let issues = detect_yaml_duplicate_keys(input);
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].reference.as_deref(), Some("yaml.duplicate_key"));
}
