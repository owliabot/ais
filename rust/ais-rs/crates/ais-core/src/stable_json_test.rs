use super::{stable_json_bytes, StableJsonOptions};
use serde_json::json;

#[test]
fn object_keys_are_stable() {
    let value = json!({"b": 2, "a": 1, "nested": {"z": 2, "x": 1}});
    let bytes = stable_json_bytes(&value, &StableJsonOptions::default()).expect("must encode");
    let text = String::from_utf8(bytes).expect("must be utf8 json");
    assert_eq!(text, r#"{"a":1,"b":2,"nested":{"x":1,"z":2}}"#);
}

#[test]
fn ignored_object_keys_are_removed_everywhere() {
    let value = json!({"created_at": "t1", "nested": {"created_at": "t2", "x": 1}});
    let mut options = StableJsonOptions::default();
    options.ignore_object_keys.insert("created_at".to_string());

    let bytes = stable_json_bytes(&value, &options).expect("must encode");
    let text = String::from_utf8(bytes).expect("must be utf8 json");
    assert_eq!(text, r#"{"nested":{"x":1}}"#);
}
