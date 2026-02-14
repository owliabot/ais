use alloy_primitives::{Address, B256};
use k256::ecdsa::SigningKey;
use std::str::FromStr;

pub trait EvmTransactionSigner: Send + Sync {
    fn private_key_hex(&self) -> Option<String> {
        None
    }

    fn address(&self) -> Option<Address> {
        None
    }
}

#[derive(Debug, Clone)]
pub struct LocalPrivateKeySigner {
    private_key: B256,
    address: Address,
}

impl LocalPrivateKeySigner {
    pub fn from_hex(private_key_hex: &str) -> Result<Self, SignerError> {
        let private_key =
            B256::from_str(private_key_hex).map_err(|error| SignerError::InvalidKey(error.to_string()))?;
        let signing_key = SigningKey::from_slice(private_key.as_slice())
            .map_err(|error| SignerError::InvalidKey(error.to_string()))?;
        let address = Address::from_public_key(signing_key.verifying_key());
        Ok(Self {
            private_key,
            address,
        })
    }

    pub fn address(&self) -> Address {
        self.address
    }

    pub fn private_key_hex(&self) -> String {
        format!("{:#x}", self.private_key)
    }
}

impl EvmTransactionSigner for LocalPrivateKeySigner {
    fn private_key_hex(&self) -> Option<String> {
        Some(self.private_key_hex())
    }

    fn address(&self) -> Option<Address> {
        Some(self.address)
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SignerError {
    #[error("invalid private key hex: {0}")]
    InvalidKey(String),
}

#[cfg(test)]
#[path = "signer_test.rs"]
mod tests;
