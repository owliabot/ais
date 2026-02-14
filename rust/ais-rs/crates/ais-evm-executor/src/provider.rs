use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const DEFAULT_RPC_TIMEOUT_MS: u64 = 30_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvmRpcTransport {
    Http,
    Ws,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmRpcEndpoint {
    pub chain: String,
    pub rpc_url: String,
    pub timeout_ms: u64,
}

impl EvmRpcEndpoint {
    pub fn new(chain: impl Into<String>, rpc_url: impl Into<String>) -> Result<Self, ProviderError> {
        let endpoint = Self {
            chain: chain.into(),
            rpc_url: rpc_url.into(),
            timeout_ms: DEFAULT_RPC_TIMEOUT_MS,
        };
        endpoint.validate()?;
        Ok(endpoint)
    }

    pub fn with_timeout_ms(mut self, timeout_ms: u64) -> Result<Self, ProviderError> {
        self.timeout_ms = timeout_ms;
        self.validate()?;
        Ok(self)
    }

    fn validate(&self) -> Result<(), ProviderError> {
        if !self.chain.starts_with("eip155:") {
            return Err(ProviderError::InvalidChain(self.chain.clone()));
        }
        if self.transport().is_err() {
            return Err(ProviderError::InvalidRpcUrl(self.rpc_url.clone()));
        }
        if self.timeout_ms == 0 {
            return Err(ProviderError::InvalidTimeout(self.timeout_ms));
        }
        Ok(())
    }

    pub fn transport(&self) -> Result<EvmRpcTransport, ProviderError> {
        if self.rpc_url.starts_with("http://") || self.rpc_url.starts_with("https://") {
            return Ok(EvmRpcTransport::Http);
        }
        if self.rpc_url.starts_with("ws://") || self.rpc_url.starts_with("wss://") {
            return Ok(EvmRpcTransport::Ws);
        }
        Err(ProviderError::InvalidRpcUrl(self.rpc_url.clone()))
    }
}

#[derive(Debug)]
pub struct EvmProviderRegistry {
    endpoints: BTreeMap<String, EvmRpcEndpoint>,
}

impl EvmProviderRegistry {
    pub fn new() -> Self {
        Self {
            endpoints: BTreeMap::new(),
        }
    }

    pub fn from_endpoints(endpoints: Vec<EvmRpcEndpoint>) -> Result<Self, ProviderError> {
        let mut registry = Self::new();
        for endpoint in endpoints {
            registry.register_endpoint(endpoint)?;
        }
        Ok(registry)
    }

    pub fn register_endpoint(&mut self, endpoint: EvmRpcEndpoint) -> Result<(), ProviderError> {
        endpoint.validate()?;
        let chain = endpoint.chain.clone();
        if self.endpoints.insert(chain.clone(), endpoint).is_some() {
            return Err(ProviderError::DuplicateChain(chain));
        }
        Ok(())
    }

    pub fn endpoint(&self, chain: &str) -> Result<&EvmRpcEndpoint, ProviderError> {
        self.endpoints
            .get(chain)
            .ok_or_else(|| ProviderError::ChainNotConfigured(chain.to_string()))
    }

    pub fn chains(&self) -> Vec<String> {
        self.endpoints.keys().cloned().collect()
    }
}

impl Default for EvmProviderRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProviderError {
    #[error("invalid chain, expected `eip155:*`: {0}")]
    InvalidChain(String),
    #[error("invalid rpc url, expected http(s) or ws(s): {0}")]
    InvalidRpcUrl(String),
    #[error("invalid timeout_ms, expected > 0: {0}")]
    InvalidTimeout(u64),
    #[error("duplicate chain endpoint configured: {0}")]
    DuplicateChain(String),
    #[error("chain endpoint not configured: {0}")]
    ChainNotConfigured(String),
}

#[cfg(test)]
#[path = "provider_test.rs"]
mod tests;
