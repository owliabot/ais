//! Live Solana broadcast port.

use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use serde::{Deserialize, Serialize};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::{signature::Signature, transaction::VersionedTransaction};
use thiserror::Error;

use ais_agent_core::binding::solana::{SolanaConnectionSpec, SolanaSignedEnvelope};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SolanaBroadcastSubmission {
    pub signature: Signature,
    pub source_hint: String,
}

#[async_trait]
pub trait SolanaRpcBroadcastClient: Send + Sync {
    async fn send_transaction(
        &self,
        transaction: &VersionedTransaction,
    ) -> Result<Signature, String>;
}

#[async_trait]
impl SolanaRpcBroadcastClient for RpcClient {
    async fn send_transaction(
        &self,
        transaction: &VersionedTransaction,
    ) -> Result<Signature, String> {
        self.send_transaction(transaction)
            .await
            .map_err(|error| error.to_string())
    }
}

#[derive(Debug, Error)]
pub enum SolanaLiveBroadcastError {
    #[error("solana broadcast rpc error: {0}")]
    Rpc(String),
    #[error("solana envelope payload missing signed transaction")]
    MissingSignedTransaction,
    #[error("invalid base64 signed transaction: {0}")]
    InvalidBase64(String),
    #[error("invalid serialized signed transaction: {0}")]
    InvalidTransaction(String),
    #[error("invalid transaction json payload: {0}")]
    InvalidEnvelopeJson(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SolanaLiveBroadcastPort {
    connection: SolanaConnectionSpec,
}

impl SolanaLiveBroadcastPort {
    pub fn new(connection: SolanaConnectionSpec) -> Self {
        Self { connection }
    }

    pub async fn send_signed_transaction(
        &self,
        transaction: &VersionedTransaction,
    ) -> Result<SolanaBroadcastSubmission, SolanaLiveBroadcastError> {
        let client = RpcClient::new(self.connection.http_url.clone());
        Self::send_with_client(&client, transaction).await
    }

    pub async fn send_with_client<C>(
        client: &C,
        transaction: &VersionedTransaction,
    ) -> Result<SolanaBroadcastSubmission, SolanaLiveBroadcastError>
    where
        C: SolanaRpcBroadcastClient,
    {
        let signature = client
            .send_transaction(transaction)
            .await
            .map_err(SolanaLiveBroadcastError::Rpc)?;

        Ok(SolanaBroadcastSubmission {
            signature,
            source_hint: "solana_rpc:send_transaction".to_owned(),
        })
    }
}

pub fn extract_signed_transaction(
    payload: &serde_json::Value,
) -> Result<VersionedTransaction, SolanaLiveBroadcastError> {
    if let Some(value) = payload.get("transaction") {
        return serde_json::from_value::<VersionedTransaction>(value.clone())
            .map_err(|error| SolanaLiveBroadcastError::InvalidEnvelopeJson(error.to_string()));
    }
    if let Some(value) = payload.get("signed_transaction") {
        return serde_json::from_value::<VersionedTransaction>(value.clone())
            .map_err(|error| SolanaLiveBroadcastError::InvalidEnvelopeJson(error.to_string()));
    }
    if let Some(value) = payload.get("solana_signed_envelope") {
        let envelope = serde_json::from_value::<SolanaSignedEnvelope>(value.clone())
            .map_err(|error| SolanaLiveBroadcastError::InvalidEnvelopeJson(error.to_string()))?;
        return Ok(envelope.transaction);
    }
    if let Some(encoded) = payload
        .get("signed_tx_base64")
        .or_else(|| payload.get("transaction_base64"))
        .and_then(|value| value.as_str())
    {
        let bytes = BASE64
            .decode(encoded)
            .map_err(|error| SolanaLiveBroadcastError::InvalidBase64(error.to_string()))?;
        return bincode::serde::decode_from_slice(&bytes, bincode::config::standard())
            .map(|(transaction, _)| transaction)
            .map_err(|error| SolanaLiveBroadcastError::InvalidTransaction(error.to_string()));
    }

    Err(SolanaLiveBroadcastError::MissingSignedTransaction)
}
