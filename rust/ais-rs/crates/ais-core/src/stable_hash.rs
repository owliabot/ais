use crate::stable_json::{stable_json_bytes, StableJsonOptions};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub fn stable_hash_hex(value: &Value, options: &StableJsonOptions) -> serde_json::Result<String> {
    let bytes = stable_json_bytes(value, options)?;
    let digest = Sha256::digest(bytes);
    Ok(format!("{digest:x}"))
}

#[cfg(test)]
#[path = "stable_hash_test.rs"]
mod tests;
