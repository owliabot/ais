use super::stable_hash_hex;
use crate::StableJsonOptions;
use serde_json::json;

#[test]
fn stable_hash_ignores_ordering() {
    let left = json!({"b":2,"a":1});
    let right = json!({"a":1,"b":2});
    let options = StableJsonOptions::default();
    let left_hash = stable_hash_hex(&left, &options).expect("hash");
    let right_hash = stable_hash_hex(&right, &options).expect("hash");
    assert_eq!(left_hash, right_hash);
}
