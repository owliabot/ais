//! Alloy-backed live EVM broadcast port.

use alloy::{
    hex,
    primitives::{Bytes, TxHash},
    providers::{Provider, ProviderBuilder},
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EvmBroadcastSubmission {
    pub tx_hash: TxHash,
    pub source_hint: String,
}

#[derive(Debug, Error)]
pub enum EvmLiveBroadcastError {
    #[error("invalid rpc url: {0}")]
    InvalidRpcUrl(String),
    #[error("invalid raw transaction payload: {0}")]
    InvalidRawTransaction(String),
    #[error("alloy provider error: {0}")]
    Provider(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmAlloyBroadcastPort {
    rpc_url: String,
}

impl EvmAlloyBroadcastPort {
    pub fn new(rpc_url: impl Into<String>) -> Self {
        Self {
            rpc_url: rpc_url.into(),
        }
    }

    pub async fn send_raw_transaction_hex(
        &self,
        encoded_tx: &str,
    ) -> Result<EvmBroadcastSubmission, EvmLiveBroadcastError> {
        let provider = ProviderBuilder::new()
            .connect(self.rpc_url.as_str())
            .await
            .map_err(|error| EvmLiveBroadcastError::InvalidRpcUrl(error.to_string()))?;
        let bytes = parse_raw_transaction_hex(encoded_tx)?;
        Self::send_raw_transaction_with_provider(&provider, &bytes).await
    }

    pub async fn send_raw_transaction_with_provider<P>(
        provider: &P,
        encoded_tx: &[u8],
    ) -> Result<EvmBroadcastSubmission, EvmLiveBroadcastError>
    where
        P: Provider,
    {
        let pending = provider
            .send_raw_transaction(encoded_tx)
            .await
            .map_err(|error| EvmLiveBroadcastError::Provider(error.to_string()))?;

        Ok(EvmBroadcastSubmission {
            tx_hash: *pending.tx_hash(),
            source_hint: "alloy_provider:eth_sendRawTransaction".to_owned(),
        })
    }
}

pub fn parse_raw_transaction_hex(encoded_tx: &str) -> Result<Bytes, EvmLiveBroadcastError> {
    let encoded_tx = encoded_tx.strip_prefix("0x").unwrap_or(encoded_tx);
    let decoded = hex::decode(encoded_tx)
        .map_err(|error| EvmLiveBroadcastError::InvalidRawTransaction(error.to_string()))?;
    Ok(Bytes::from(decoded))
}

#[cfg(test)]
mod tests {
    use alloy::{
        primitives::{b256, bytes},
        providers::ProviderBuilder,
        transports::mock::Asserter,
    };

    use super::{parse_raw_transaction_hex, EvmAlloyBroadcastPort};

    #[test]
    fn parse_raw_transaction_hex_accepts_prefixed_payload() {
        let parsed = parse_raw_transaction_hex("0x010203").expect("parse raw tx");
        assert_eq!(parsed, bytes!("010203"));
    }

    #[tokio::test]
    async fn alloy_live_broadcast_port_returns_tx_hash_from_provider() {
        let asserter = Asserter::new();
        let provider = ProviderBuilder::new().connect_mocked_client(asserter.clone());
        let tx_hash = b256!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        asserter.push_success(&tx_hash);

        let submission =
            EvmAlloyBroadcastPort::send_raw_transaction_with_provider(&provider, &bytes!("0102"))
                .await
                .expect("broadcast");

        assert_eq!(submission.tx_hash, tx_hash);
        assert_eq!(
            submission.source_hint,
            "alloy_provider:eth_sendRawTransaction"
        );
    }
}
