//! Alloy-backed live EVM read port.

use ais_agent_core::binding::evm::{EvmCallRequest, EvmObserveRequest};
use alloy::{
    network::TransactionBuilder,
    primitives::{Address, Bytes, U256},
    providers::{Provider, ProviderBuilder},
    rpc::types::TransactionRequest,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EvmBlockObservation {
    pub block_number: u64,
    pub source_hint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EvmStorageObservation {
    pub address: Address,
    pub slot: U256,
    pub value: U256,
    pub source_hint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EvmBalanceObservation {
    pub address: Address,
    pub balance: U256,
    pub source_hint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EvmCallObservation {
    pub to: Address,
    pub data: Bytes,
    pub return_data: Bytes,
    pub source_hint: String,
}

#[derive(Debug, Error)]
pub enum EvmLiveReadError {
    #[error("invalid rpc url: {0}")]
    InvalidRpcUrl(String),
    #[error("alloy provider error: {0}")]
    Provider(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmAlloyReadPort {
    rpc_url: String,
}

impl EvmAlloyReadPort {
    pub fn new(rpc_url: impl Into<String>) -> Self {
        Self {
            rpc_url: rpc_url.into(),
        }
    }

    pub fn rpc_url(&self) -> &str {
        self.rpc_url.as_str()
    }

    pub async fn get_block_number(&self) -> Result<EvmBlockObservation, EvmLiveReadError> {
        let provider = ProviderBuilder::new()
            .connect(self.rpc_url.as_str())
            .await
            .map_err(|error| EvmLiveReadError::InvalidRpcUrl(error.to_string()))?;

        Self::get_block_number_with_provider(&provider).await
    }

    pub async fn get_storage_at(
        &self,
        address: Address,
        slot: U256,
    ) -> Result<EvmStorageObservation, EvmLiveReadError> {
        let provider = ProviderBuilder::new()
            .connect(self.rpc_url.as_str())
            .await
            .map_err(|error| EvmLiveReadError::InvalidRpcUrl(error.to_string()))?;

        Self::get_storage_at_with_provider(&provider, address, slot).await
    }

    pub async fn get_native_balance(
        &self,
        address: Address,
    ) -> Result<EvmBalanceObservation, EvmLiveReadError> {
        let provider = ProviderBuilder::new()
            .connect(self.rpc_url.as_str())
            .await
            .map_err(|error| EvmLiveReadError::InvalidRpcUrl(error.to_string()))?;

        Self::get_native_balance_with_provider(&provider, address).await
    }

    pub async fn eth_call(
        &self,
        request: &EvmCallRequest,
    ) -> Result<EvmCallObservation, EvmLiveReadError> {
        let provider = ProviderBuilder::new()
            .connect(self.rpc_url.as_str())
            .await
            .map_err(|error| EvmLiveReadError::InvalidRpcUrl(error.to_string()))?;

        Self::eth_call_with_provider(&provider, request).await
    }

    pub async fn get_block_number_with_provider<P>(
        provider: &P,
    ) -> Result<EvmBlockObservation, EvmLiveReadError>
    where
        P: Provider,
    {
        let block_number = provider
            .get_block_number()
            .await
            .map_err(|error| EvmLiveReadError::Provider(error.to_string()))?;

        Ok(EvmBlockObservation {
            block_number,
            source_hint: "alloy_provider:get_block_number".to_owned(),
        })
    }

    pub async fn get_storage_at_with_provider<P>(
        provider: &P,
        address: Address,
        slot: U256,
    ) -> Result<EvmStorageObservation, EvmLiveReadError>
    where
        P: Provider,
    {
        let value = provider
            .get_storage_at(address, slot)
            .await
            .map_err(|error| EvmLiveReadError::Provider(error.to_string()))?;

        Ok(EvmStorageObservation {
            address,
            slot,
            value,
            source_hint: "alloy_provider:get_storage_at".to_owned(),
        })
    }

    pub async fn get_native_balance_with_provider<P>(
        provider: &P,
        address: Address,
    ) -> Result<EvmBalanceObservation, EvmLiveReadError>
    where
        P: Provider,
    {
        let balance = provider
            .get_balance(address)
            .await
            .map_err(|error| EvmLiveReadError::Provider(error.to_string()))?;

        Ok(EvmBalanceObservation {
            address,
            balance,
            source_hint: "alloy_provider:get_balance".to_owned(),
        })
    }

    pub async fn eth_call_with_provider<P>(
        provider: &P,
        request: &EvmCallRequest,
    ) -> Result<EvmCallObservation, EvmLiveReadError>
    where
        P: Provider,
    {
        let tx = Self::into_transaction_request(request);
        let return_data = provider
            .call(tx)
            .await
            .map_err(|error| EvmLiveReadError::Provider(error.to_string()))?;

        Ok(EvmCallObservation {
            to: request.to,
            data: request.data.clone(),
            return_data,
            source_hint: "alloy_provider:eth_call".to_owned(),
        })
    }

    pub async fn observe(
        &self,
        request: &EvmObserveRequest,
    ) -> Result<serde_json::Value, EvmLiveReadError> {
        let provider = ProviderBuilder::new()
            .connect(self.rpc_url.as_str())
            .await
            .map_err(|error| EvmLiveReadError::InvalidRpcUrl(error.to_string()))?;

        Self::observe_with_provider(&provider, request).await
    }

    pub async fn observe_with_provider<P>(
        provider: &P,
        request: &EvmObserveRequest,
    ) -> Result<serde_json::Value, EvmLiveReadError>
    where
        P: Provider,
    {
        match request {
            EvmObserveRequest::BlockNumber => {
                let observation = Self::get_block_number_with_provider(provider).await?;
                Ok(Self::block_observation_payload(&observation))
            }
            EvmObserveRequest::NativeBalance { address } => {
                let observation =
                    Self::get_native_balance_with_provider(provider, *address).await?;
                Ok(Self::balance_observation_payload(&observation))
            }
            EvmObserveRequest::StorageSlot { address, slot } => {
                let observation =
                    Self::get_storage_at_with_provider(provider, *address, *slot).await?;
                Ok(Self::storage_observation_payload(&observation))
            }
            EvmObserveRequest::Erc20BalanceOf { token, owner } => {
                let observation = Self::eth_call_with_provider(
                    provider,
                    &EvmCallRequest {
                        from: None,
                        to: *token,
                        data: encode_erc20_balance_of(*owner),
                        value: None,
                    },
                )
                .await?;
                Ok(Self::erc20_amount_payload(
                    &observation,
                    "balance_of",
                    json!({
                        "token": token,
                        "owner": owner,
                    }),
                ))
            }
            EvmObserveRequest::Erc20Allowance {
                token,
                owner,
                spender,
            } => {
                let observation = Self::eth_call_with_provider(
                    provider,
                    &EvmCallRequest {
                        from: None,
                        to: *token,
                        data: encode_erc20_allowance(*owner, *spender),
                        value: None,
                    },
                )
                .await?;
                Ok(Self::erc20_amount_payload(
                    &observation,
                    "allowance",
                    json!({
                        "token": token,
                        "owner": owner,
                        "spender": spender,
                    }),
                ))
            }
            EvmObserveRequest::ContractStateRead { to, data } => {
                let observation = Self::eth_call_with_provider(
                    provider,
                    &EvmCallRequest {
                        from: None,
                        to: *to,
                        data: data.clone(),
                        value: None,
                    },
                )
                .await?;
                Ok(Self::call_observation_payload(&observation))
            }
        }
    }

    pub fn block_observation_payload(observation: &EvmBlockObservation) -> serde_json::Value {
        json!({
            "block_number": observation.block_number,
            "source_hint": observation.source_hint,
        })
    }

    pub fn storage_observation_payload(observation: &EvmStorageObservation) -> serde_json::Value {
        json!({
            "address": observation.address,
            "slot": observation.slot.to_string(),
            "value": observation.value.to_string(),
            "source_hint": observation.source_hint,
        })
    }

    pub fn balance_observation_payload(observation: &EvmBalanceObservation) -> serde_json::Value {
        json!({
            "address": observation.address,
            "balance": observation.balance.to_string(),
            "source_hint": observation.source_hint,
        })
    }

    pub fn call_observation_payload(observation: &EvmCallObservation) -> serde_json::Value {
        json!({
            "to": observation.to,
            "data": observation.data,
            "return_data": observation.return_data,
            "source_hint": observation.source_hint,
        })
    }

    fn into_transaction_request(request: &EvmCallRequest) -> TransactionRequest {
        let tx = TransactionRequest::default()
            .with_to(request.to)
            .with_input(request.data.clone());
        let tx = if let Some(from) = request.from {
            tx.with_from(from)
        } else {
            tx
        };
        if let Some(value) = request.value {
            tx.with_value(value)
        } else {
            tx
        }
    }

    fn erc20_amount_payload(
        observation: &EvmCallObservation,
        method: &'static str,
        target: serde_json::Value,
    ) -> serde_json::Value {
        let decoded = decode_u256_return(&observation.return_data).map(|value| value.to_string());
        json!({
            "method": method,
            "target": target,
            "return_data": observation.return_data,
            "decoded_u256": decoded,
            "source_hint": observation.source_hint,
        })
    }
}

fn encode_erc20_balance_of(owner: Address) -> Bytes {
    let mut payload = Vec::with_capacity(4 + 32);
    payload.extend_from_slice(&[0x70, 0xa0, 0x82, 0x31]);
    payload.extend_from_slice(&pad_address(owner));
    Bytes::from(payload)
}

fn encode_erc20_allowance(owner: Address, spender: Address) -> Bytes {
    let mut payload = Vec::with_capacity(4 + 64);
    payload.extend_from_slice(&[0xdd, 0x62, 0xed, 0x3e]);
    payload.extend_from_slice(&pad_address(owner));
    payload.extend_from_slice(&pad_address(spender));
    Bytes::from(payload)
}

fn pad_address(address: Address) -> [u8; 32] {
    let mut padded = [0u8; 32];
    padded[12..].copy_from_slice(address.as_slice());
    padded
}

fn decode_u256_return(bytes: &Bytes) -> Option<U256> {
    if bytes.len() < 32 {
        return None;
    }

    let mut word = [0u8; 32];
    word.copy_from_slice(&bytes[bytes.len() - 32..]);
    Some(U256::from_be_bytes(word))
}

#[cfg(test)]
mod tests {
    use alloy::{
        primitives::{address, bytes, U256},
        providers::ProviderBuilder,
        transports::mock::Asserter,
    };

    use super::EvmAlloyReadPort;
    use ais_agent_core::binding::evm::EvmCallRequest;

    #[tokio::test]
    async fn alloy_live_read_port_reads_block_number_from_provider_stack() {
        let asserter = Asserter::new();
        let provider = ProviderBuilder::new().connect_mocked_client(asserter.clone());
        asserter.push_success(&1234u64);

        let observation = EvmAlloyReadPort::get_block_number_with_provider(&provider)
            .await
            .expect("block number");

        assert_eq!(observation.block_number, 1234);
        assert_eq!(observation.source_hint, "alloy_provider:get_block_number");
    }

    #[tokio::test]
    async fn alloy_live_read_port_reads_storage_with_native_alloy_types() {
        let asserter = Asserter::new();
        let provider = ProviderBuilder::new().connect_mocked_client(asserter.clone());
        let storage = U256::from(0x42);
        asserter.push_success(&storage);

        let observation = EvmAlloyReadPort::get_storage_at_with_provider(
            &provider,
            address!("1111111111111111111111111111111111111111"),
            U256::from(0),
        )
        .await
        .expect("storage");

        assert_eq!(observation.value, storage);
        assert_eq!(observation.source_hint, "alloy_provider:get_storage_at");
    }

    #[tokio::test]
    async fn alloy_live_read_port_reads_native_balance_with_alloy_types() {
        let asserter = Asserter::new();
        let provider = ProviderBuilder::new().connect_mocked_client(asserter.clone());
        let balance = U256::from(123456u64);
        asserter.push_success(&balance);

        let observation = EvmAlloyReadPort::get_native_balance_with_provider(
            &provider,
            address!("1111111111111111111111111111111111111111"),
        )
        .await
        .expect("balance");

        assert_eq!(observation.balance, balance);
        assert_eq!(observation.source_hint, "alloy_provider:get_balance");
    }

    #[tokio::test]
    async fn alloy_live_read_port_executes_eth_call_with_native_alloy_types() {
        let asserter = Asserter::new();
        let provider = ProviderBuilder::new().connect_mocked_client(asserter.clone());
        let return_data =
            bytes!("0000000000000000000000000000000000000000000000000000000000000042");
        asserter.push_success(&return_data);

        let observation = EvmAlloyReadPort::eth_call_with_provider(
            &provider,
            &EvmCallRequest {
                from: None,
                to: address!("1111111111111111111111111111111111111111"),
                data: bytes!("06fdde03"),
                value: None,
            },
        )
        .await
        .expect("eth_call");

        assert_eq!(observation.return_data, return_data);
        assert_eq!(observation.source_hint, "alloy_provider:eth_call");
    }
}
