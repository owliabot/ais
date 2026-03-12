//! Alloy/Anvil-backed EVM simulation environment.

use std::path::PathBuf;

use alloy::{
    node_bindings::{Anvil, AnvilInstance, NodeError},
    primitives::{keccak256, Address, B256, U256},
    providers::{ext::AnvilApi, DynProvider, Provider, ProviderBuilder},
    rpc::types::anvil::{Metadata, NodeInfo},
    sol_types::SolValue,
    transports::TransportError,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Runtime configuration for an Anvil-backed simulation environment.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct EvmAnvilConfig {
    pub anvil_path: Option<PathBuf>,
    pub fork_url: Option<String>,
    pub fork_block_number: Option<u64>,
    pub chain_id: Option<u64>,
    pub block_time_secs: Option<u64>,
    pub auto_impersonate: bool,
}

impl EvmAnvilConfig {
    pub fn local(chain_id: u64) -> Self {
        Self {
            chain_id: Some(chain_id),
            ..Self::default()
        }
    }

    pub fn forked(fork_url: impl Into<String>) -> Self {
        Self {
            fork_url: Some(fork_url.into()),
            ..Self::default()
        }
    }

    fn into_anvil(self) -> Anvil {
        let mut anvil = Anvil::new();

        if let Some(path) = self.anvil_path {
            anvil = anvil.path(path);
        }
        if let Some(fork_url) = self.fork_url {
            anvil = anvil.fork(fork_url);
        }
        if let Some(fork_block_number) = self.fork_block_number {
            anvil = anvil.fork_block_number(fork_block_number);
        }
        if let Some(chain_id) = self.chain_id {
            anvil = anvil.chain_id(chain_id);
        }
        if let Some(block_time_secs) = self.block_time_secs {
            anvil = anvil.block_time(block_time_secs);
        }
        if self.auto_impersonate {
            anvil = anvil.arg("--auto-impersonate");
        }

        anvil
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvmAnvilSnapshot {
    pub id: U256,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvmAnvilStatePatchResult {
    pub rpc_url: String,
    pub target: Address,
    pub operation: &'static str,
}

#[derive(Debug)]
pub struct EvmAnvilSimulationEnv {
    config: EvmAnvilConfig,
    rpc_url: String,
    provider: DynProvider,
    instance: AnvilInstance,
}

impl EvmAnvilSimulationEnv {
    pub fn spawn(config: EvmAnvilConfig) -> Result<Self, EvmAnvilError> {
        let instance = config.clone().into_anvil().try_spawn()?;
        let endpoint = instance.endpoint_url();
        let provider = ProviderBuilder::new()
            .connect_http(endpoint.clone())
            .erased();

        Ok(Self {
            config,
            rpc_url: endpoint.to_string(),
            provider,
            instance,
        })
    }

    pub fn config(&self) -> &EvmAnvilConfig {
        &self.config
    }

    pub fn rpc_url(&self) -> &str {
        &self.rpc_url
    }

    pub fn provider(&self) -> &DynProvider {
        &self.provider
    }

    pub fn instance(&self) -> &AnvilInstance {
        &self.instance
    }

    pub async fn node_info(&self) -> Result<NodeInfo, EvmAnvilError> {
        Ok(self.provider.anvil_node_info().await?)
    }

    pub async fn metadata(&self) -> Result<Metadata, EvmAnvilError> {
        Ok(self.provider.anvil_metadata().await?)
    }

    pub async fn set_balance(
        &self,
        address: Address,
        balance: U256,
    ) -> Result<EvmAnvilStatePatchResult, EvmAnvilError> {
        self.provider.anvil_set_balance(address, balance).await?;
        Ok(self.patch_result(address, "set_balance"))
    }

    pub async fn set_storage_at(
        &self,
        address: Address,
        slot: U256,
        value: B256,
    ) -> Result<EvmAnvilStatePatchResult, EvmAnvilError> {
        let ok = self
            .provider
            .anvil_set_storage_at(address, slot, value)
            .await?;
        if !ok {
            return Err(EvmAnvilError::StatePatchRejected {
                operation: "set_storage_at",
                target: address,
            });
        }
        Ok(self.patch_result(address, "set_storage_at"))
    }

    pub async fn set_erc20_balance(
        &self,
        token: Address,
        owner: Address,
        mapping_slot_index: U256,
        balance: U256,
    ) -> Result<EvmAnvilStatePatchResult, EvmAnvilError> {
        let slot = Self::mapping_slot_for_address(owner, mapping_slot_index);
        self.set_storage_at(token, slot, Self::u256_to_b256(balance))
            .await
    }

    pub async fn impersonate_account(
        &self,
        address: Address,
    ) -> Result<EvmAnvilStatePatchResult, EvmAnvilError> {
        self.provider.anvil_impersonate_account(address).await?;
        Ok(self.patch_result(address, "impersonate_account"))
    }

    pub async fn stop_impersonating_account(
        &self,
        address: Address,
    ) -> Result<EvmAnvilStatePatchResult, EvmAnvilError> {
        self.provider
            .anvil_stop_impersonating_account(address)
            .await?;
        Ok(self.patch_result(address, "stop_impersonating_account"))
    }

    pub async fn snapshot(&self) -> Result<EvmAnvilSnapshot, EvmAnvilError> {
        Ok(EvmAnvilSnapshot {
            id: self.provider.anvil_snapshot().await?,
        })
    }

    pub async fn revert(&self, snapshot: &EvmAnvilSnapshot) -> Result<bool, EvmAnvilError> {
        Ok(self.provider.anvil_revert(snapshot.id).await?)
    }

    pub async fn increase_time(&self, seconds: u64) -> Result<i64, EvmAnvilError> {
        Ok(self.provider.anvil_increase_time(seconds).await?)
    }

    pub async fn set_next_block_timestamp(&self, timestamp: u64) -> Result<(), EvmAnvilError> {
        Ok(self
            .provider
            .anvil_set_next_block_timestamp(timestamp)
            .await?)
    }

    pub fn mapping_slot_for_address(key: Address, slot_index: U256) -> U256 {
        U256::from_be_bytes(keccak256((key, slot_index).abi_encode()).0)
    }

    pub fn u256_to_b256(value: U256) -> B256 {
        B256::from(value.to_be_bytes::<32>())
    }

    fn patch_result(&self, target: Address, operation: &'static str) -> EvmAnvilStatePatchResult {
        EvmAnvilStatePatchResult {
            rpc_url: self.rpc_url.clone(),
            target,
            operation,
        }
    }
}

#[derive(Debug, Error)]
pub enum EvmAnvilError {
    #[error("failed to spawn anvil instance: {0}")]
    Spawn(#[from] NodeError),
    #[error("anvil rpc transport error: {0}")]
    Transport(#[from] TransportError),
    #[error("anvil state patch was rejected: operation={operation} target={target}")]
    StatePatchRejected {
        operation: &'static str,
        target: Address,
    },
}

#[cfg(test)]
mod tests {
    use alloy::primitives::{address, U256};

    use super::{EvmAnvilConfig, EvmAnvilSimulationEnv};

    #[test]
    fn mapping_slot_for_address_matches_standard_solidity_layout() {
        let owner = address!("1111111111111111111111111111111111111111");
        let slot = EvmAnvilSimulationEnv::mapping_slot_for_address(owner, U256::from(3));

        assert_eq!(
            format!("{slot:#x}"),
            "0xfc40ea33816453f766ebc0872d4b5152b468882abe7b6b35528069db4d6e41c4"
        );
    }

    #[test]
    fn local_config_captures_chain_specific_defaults() {
        let config = EvmAnvilConfig::local(31337);
        assert_eq!(config.chain_id, Some(31337));
        assert!(config.fork_url.is_none());
        assert!(!config.auto_impersonate);
    }

    #[tokio::test]
    async fn local_anvil_env_supports_snapshot_and_balance_patch() {
        if !anvil_available() {
            eprintln!("skipping anvil integration test because `anvil` is not in PATH");
            return;
        }

        let env = EvmAnvilSimulationEnv::spawn(EvmAnvilConfig {
            chain_id: Some(31337),
            auto_impersonate: true,
            ..EvmAnvilConfig::default()
        })
        .expect("spawn local anvil");

        let account = address!("1111111111111111111111111111111111111111");
        let snapshot = env.snapshot().await.expect("snapshot");
        env.set_balance(account, U256::from(12345))
            .await
            .expect("set balance");
        let reverted = env.revert(&snapshot).await.expect("revert");

        assert!(reverted);
    }

    fn anvil_available() -> bool {
        std::process::Command::new("anvil")
            .arg("--version")
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }
}
