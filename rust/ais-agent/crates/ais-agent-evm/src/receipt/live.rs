//! Alloy-backed live EVM receipt polling port.

use alloy::{
    primitives::TxHash,
    providers::{Provider, ProviderBuilder},
    rpc::types::TransactionReceipt,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EvmReceiptObservation {
    pub tx_hash: TxHash,
    pub observed: bool,
    pub success: Option<bool>,
    pub block_number: Option<u64>,
    pub confirmation_depth: Option<u64>,
    pub source_hint: String,
    pub payload: Value,
}

#[derive(Debug, Error)]
pub enum EvmLiveReceiptError {
    #[error("invalid rpc url: {0}")]
    InvalidRpcUrl(String),
    #[error("invalid tx hash: {0}")]
    InvalidTxHash(String),
    #[error("alloy provider error: {0}")]
    Provider(String),
    #[error("receipt serialization failed: {0}")]
    Serialize(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmAlloyReceiptPort {
    rpc_url: String,
}

impl EvmAlloyReceiptPort {
    pub fn new(rpc_url: impl Into<String>) -> Self {
        Self {
            rpc_url: rpc_url.into(),
        }
    }

    pub async fn get_transaction_receipt(
        &self,
        tx_hash: &str,
    ) -> Result<EvmReceiptObservation, EvmLiveReceiptError> {
        let provider = ProviderBuilder::new()
            .connect(self.rpc_url.as_str())
            .await
            .map_err(|error| EvmLiveReceiptError::InvalidRpcUrl(error.to_string()))?;
        let tx_hash = tx_hash
            .parse::<TxHash>()
            .map_err(|error| EvmLiveReceiptError::InvalidTxHash(error.to_string()))?;
        Self::get_transaction_receipt_with_provider(&provider, tx_hash).await
    }

    pub async fn get_transaction_receipt_with_provider<P>(
        provider: &P,
        tx_hash: TxHash,
    ) -> Result<EvmReceiptObservation, EvmLiveReceiptError>
    where
        P: Provider,
    {
        let receipt = provider
            .get_transaction_receipt(tx_hash)
            .await
            .map_err(|error| EvmLiveReceiptError::Provider(error.to_string()))?;

        match receipt {
            Some(receipt) => build_observed_receipt(provider, receipt).await,
            None => Ok(EvmReceiptObservation {
                tx_hash,
                observed: false,
                success: None,
                block_number: None,
                confirmation_depth: None,
                source_hint: "alloy_provider:get_transaction_receipt".to_owned(),
                payload: serde_json::json!({
                    "observed": false,
                    "tx_hash": format!("{:#x}", tx_hash),
                }),
            }),
        }
    }
}

async fn build_observed_receipt<P>(
    provider: &P,
    receipt: TransactionReceipt,
) -> Result<EvmReceiptObservation, EvmLiveReceiptError>
where
    P: Provider,
{
    let latest_block_number = provider
        .get_block_number()
        .await
        .map_err(|error| EvmLiveReceiptError::Provider(error.to_string()))?;
    let confirmation_depth = receipt.block_number.map(|block_number| {
        latest_block_number
            .saturating_sub(block_number)
            .saturating_add(1)
    });
    let payload = serde_json::to_value(&receipt)
        .map_err(|error| EvmLiveReceiptError::Serialize(error.to_string()))?;

    Ok(EvmReceiptObservation {
        tx_hash: receipt.transaction_hash,
        observed: true,
        success: Some(receipt.status()),
        block_number: receipt.block_number,
        confirmation_depth,
        source_hint: "alloy_provider:get_transaction_receipt".to_owned(),
        payload,
    })
}

#[cfg(test)]
mod tests {
    use alloy::{
        consensus::{Receipt, ReceiptEnvelope},
        primitives::{address, b256, TxHash},
        providers::ProviderBuilder,
        rpc::types::TransactionReceipt,
        transports::mock::Asserter,
    };

    use super::EvmAlloyReceiptPort;

    #[tokio::test]
    async fn alloy_live_receipt_port_returns_missing_when_unconfirmed() {
        let asserter = Asserter::new();
        let provider = ProviderBuilder::new().connect_mocked_client(asserter.clone());
        let tx_hash = b256!("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
        asserter.push_success::<Option<TransactionReceipt>>(&None);

        let observation =
            EvmAlloyReceiptPort::get_transaction_receipt_with_provider(&provider, tx_hash)
                .await
                .expect("receipt");

        assert!(!observation.observed);
        assert_eq!(observation.tx_hash, tx_hash);
    }

    #[tokio::test]
    async fn alloy_live_receipt_port_returns_observed_receipt() {
        let asserter = Asserter::new();
        let provider = ProviderBuilder::new().connect_mocked_client(asserter.clone());
        let tx_hash = b256!("cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc");
        asserter.push_success(&sample_receipt(tx_hash, 100));
        asserter.push_success(&105u64);

        let observation =
            EvmAlloyReceiptPort::get_transaction_receipt_with_provider(&provider, tx_hash)
                .await
                .expect("receipt");

        assert!(observation.observed);
        assert_eq!(observation.success, Some(true));
        assert_eq!(observation.block_number, Some(100));
        assert_eq!(observation.confirmation_depth, Some(6));
    }

    fn sample_receipt(tx_hash: TxHash, block_number: u64) -> Option<TransactionReceipt> {
        Some(TransactionReceipt {
            inner: ReceiptEnvelope::Eip1559(
                Receipt {
                    status: true.into(),
                    cumulative_gas_used: 21_000,
                    logs: Vec::new(),
                }
                .with_bloom(),
            ),
            transaction_hash: tx_hash,
            transaction_index: Some(0),
            block_hash: Some(b256!(
                "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"
            )),
            block_number: Some(block_number),
            gas_used: 21_000,
            effective_gas_price: 1,
            blob_gas_used: None,
            blob_gas_price: None,
            from: address!("1111111111111111111111111111111111111111"),
            to: Some(address!("2222222222222222222222222222222222222222")),
            contract_address: None,
        })
    }
}
