use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub const DEFAULT_RPC_TIMEOUT_MS: u64 = 30_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CommitmentLevel {
    #[default]
    Confirmed,
    Processed,
    Finalized,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SolanaRpcEndpoint {
    pub chain: String,
    pub rpc_url: String,
    #[serde(default)]
    pub commitment: CommitmentLevel,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
}

impl SolanaRpcEndpoint {
    pub fn new(chain: impl Into<String>, rpc_url: impl Into<String>) -> Result<Self, ProviderError> {
        let endpoint = Self {
            chain: chain.into(),
            rpc_url: rpc_url.into(),
            commitment: CommitmentLevel::Confirmed,
            timeout_ms: DEFAULT_RPC_TIMEOUT_MS,
        };
        endpoint.validate()?;
        Ok(endpoint)
    }

    pub fn with_commitment(mut self, commitment: CommitmentLevel) -> Result<Self, ProviderError> {
        self.commitment = commitment;
        self.validate()?;
        Ok(self)
    }

    pub fn with_timeout_ms(mut self, timeout_ms: u64) -> Result<Self, ProviderError> {
        self.timeout_ms = timeout_ms;
        self.validate()?;
        Ok(self)
    }

    fn validate(&self) -> Result<(), ProviderError> {
        if !self.chain.starts_with("solana:") {
            return Err(ProviderError::InvalidChain(self.chain.clone()));
        }
        if !(self.rpc_url.starts_with("http://") || self.rpc_url.starts_with("https://")) {
            return Err(ProviderError::InvalidRpcUrl(self.rpc_url.clone()));
        }
        if self.timeout_ms == 0 {
            return Err(ProviderError::InvalidTimeout(self.timeout_ms));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SolanaInstructionAccount {
    pub name: String,
    pub pubkey: String,
    pub signer: bool,
    pub writable: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SolanaInstructionRequest {
    #[serde(default = "default_tx_version")]
    pub tx_version: String,
    pub program: String,
    pub instruction: String,
    pub accounts: Vec<SolanaInstructionAccount>,
    pub data: String,
    #[serde(default)]
    pub compute_units: Option<u64>,
    #[serde(default)]
    pub lookup_tables: Option<Value>,
}

pub trait SolanaRpcClient: Send + Sync {
    fn get_balance(&self, pubkey: &str) -> Result<u64, ProviderError>;
    fn get_account_info(&self, pubkey: &str) -> Result<Value, ProviderError>;
    fn get_token_account_balance(&self, pubkey: &str) -> Result<Value, ProviderError>;
    fn get_signature_statuses(&self, signatures: &[String]) -> Result<Value, ProviderError>;
    fn send_signed_transaction(
        &self,
        request: &SolanaInstructionRequest,
        signed_tx: &str,
    ) -> Result<String, ProviderError>;
}

pub trait SolanaRpcClientFactory: Send + Sync {
    fn build_client(&self, endpoint: &SolanaRpcEndpoint) -> Result<Box<dyn SolanaRpcClient>, ProviderError>;
}

pub struct SolanaProviderRegistry {
    endpoints: BTreeMap<String, SolanaRpcEndpoint>,
}

impl SolanaProviderRegistry {
    pub fn new() -> Self {
        Self {
            endpoints: BTreeMap::new(),
        }
    }

    pub fn from_endpoints(endpoints: Vec<SolanaRpcEndpoint>) -> Result<Self, ProviderError> {
        let mut registry = Self::new();
        for endpoint in endpoints {
            registry.register_endpoint(endpoint)?;
        }
        Ok(registry)
    }

    pub fn register_endpoint(&mut self, endpoint: SolanaRpcEndpoint) -> Result<(), ProviderError> {
        endpoint.validate()?;
        let chain = endpoint.chain.clone();
        if self.endpoints.insert(chain.clone(), endpoint).is_some() {
            return Err(ProviderError::DuplicateChain(chain));
        }
        Ok(())
    }

    pub fn endpoint(&self, chain: &str) -> Result<&SolanaRpcEndpoint, ProviderError> {
        self.endpoints
            .get(chain)
            .ok_or_else(|| ProviderError::ChainNotConfigured(chain.to_string()))
    }

    pub fn build_client_for_chain(
        &self,
        chain: &str,
        factory: &dyn SolanaRpcClientFactory,
    ) -> Result<Box<dyn SolanaRpcClient>, ProviderError> {
        let endpoint = self.endpoint(chain)?;
        factory.build_client(endpoint)
    }
}

impl Default for SolanaProviderRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ProviderError {
    #[error("invalid chain, expected `solana:*`: {0}")]
    InvalidChain(String),
    #[error("invalid rpc url, expected http(s): {0}")]
    InvalidRpcUrl(String),
    #[error("invalid timeout_ms, expected > 0: {0}")]
    InvalidTimeout(u64),
    #[error("duplicate chain endpoint configured: {0}")]
    DuplicateChain(String),
    #[error("chain endpoint not configured: {0}")]
    ChainNotConfigured(String),
    #[error("provider transport error: {0}")]
    Transport(String),
}

const fn default_timeout_ms() -> u64 {
    DEFAULT_RPC_TIMEOUT_MS
}

fn default_tx_version() -> String {
    "legacy".to_string()
}

#[cfg(test)]
#[path = "types_test.rs"]
mod tests;
