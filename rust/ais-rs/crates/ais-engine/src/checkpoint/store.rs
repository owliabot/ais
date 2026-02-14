use super::{decode_checkpoint_json, encode_checkpoint_json, CheckpointDocument};
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum CheckpointStoreError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

pub fn save_checkpoint_to_path(
    path: impl AsRef<Path>,
    document: &CheckpointDocument,
) -> Result<(), CheckpointStoreError> {
    let encoded = encode_checkpoint_json(document)?;
    std::fs::write(path, encoded)?;
    Ok(())
}

pub fn load_checkpoint_from_path(path: impl AsRef<Path>) -> Result<CheckpointDocument, CheckpointStoreError> {
    let content = std::fs::read_to_string(path)?;
    Ok(decode_checkpoint_json(&content)?)
}

#[cfg(test)]
#[path = "store_test.rs"]
mod tests;
