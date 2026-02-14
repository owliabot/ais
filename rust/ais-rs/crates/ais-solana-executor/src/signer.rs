use crate::types::SolanaInstructionRequest;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedSolanaTransaction {
    pub raw_tx: String,
    pub tx_hash: String,
}

pub trait SolanaTransactionSigner: Send + Sync {
    fn sign_instruction(
        &self,
        request: &SolanaInstructionRequest,
    ) -> Result<SignedSolanaTransaction, SignerError>;
}

#[derive(Debug, Clone)]
pub struct LocalPrivateKeySigner {
    private_key: String,
}

impl LocalPrivateKeySigner {
    pub fn from_config(private_key: impl Into<String>) -> Result<Self, SignerError> {
        let private_key = private_key.into();
        if private_key.trim().is_empty() {
            return Err(SignerError::InvalidKey("private key cannot be empty".to_string()));
        }
        Ok(Self { private_key })
    }
}

impl SolanaTransactionSigner for LocalPrivateKeySigner {
    fn sign_instruction(
        &self,
        request: &SolanaInstructionRequest,
    ) -> Result<SignedSolanaTransaction, SignerError> {
        let payload = json!({
            "private_key_marker": format!("len:{}", self.private_key.len()),
            "tx_version": request.tx_version,
            "program": request.program,
            "instruction": request.instruction,
            "accounts": request.accounts,
            "data": request.data,
            "compute_units": request.compute_units,
            "lookup_tables": request.lookup_tables,
        });
        let payload_text =
            serde_json::to_string(&payload).map_err(|error| SignerError::SigningFailed(error.to_string()))?;
        let digest = Sha256::digest(payload_text.as_bytes());
        let tx_hash = format!("0x{}", hex_encode(digest.as_slice()));
        let raw_tx = format!("base64:{}", hex_encode(payload_text.as_bytes()));
        Ok(SignedSolanaTransaction { raw_tx, tx_hash })
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SignerError {
    #[error("invalid key: {0}")]
    InvalidKey(String),
    #[error("signing failed: {0}")]
    SigningFailed(String),
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join("")
}

#[cfg(test)]
#[path = "signer_test.rs"]
mod tests;
