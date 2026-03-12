//! Live Solana signature-status / receipt port.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::signature::Signature;
use thiserror::Error;

use crate::read::live::{SolanaRpcReadClient, SolanaSignatureStatusSnapshot};
use ais_agent_core::binding::solana::SolanaConnectionSpec;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SolanaReceiptObservation {
    pub signature: Signature,
    pub observed: bool,
    pub success: Option<bool>,
    pub slot: Option<u64>,
    pub confirmation_depth: Option<u64>,
    pub confirmation_status: Option<String>,
    pub source_hint: String,
    pub payload: serde_json::Value,
}

#[async_trait]
pub trait SolanaRpcReceiptClient: Send + Sync {
    async fn get_slot(&self) -> Result<u64, String>;
    async fn get_signature_status(
        &self,
        signature: &Signature,
    ) -> Result<SolanaSignatureStatusSnapshot, String>;
}

#[async_trait]
impl SolanaRpcReceiptClient for RpcClient {
    async fn get_slot(&self) -> Result<u64, String> {
        SolanaRpcReadClient::get_slot(self).await
    }

    async fn get_signature_status(
        &self,
        signature: &Signature,
    ) -> Result<SolanaSignatureStatusSnapshot, String> {
        SolanaRpcReadClient::get_signature_status(self, signature).await
    }
}

#[derive(Debug, Error)]
pub enum SolanaLiveReceiptError {
    #[error("solana receipt rpc error: {0}")]
    Rpc(String),
    #[error("invalid signature: {0}")]
    InvalidSignature(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SolanaLiveReceiptPort {
    connection: SolanaConnectionSpec,
}

impl SolanaLiveReceiptPort {
    pub fn new(connection: SolanaConnectionSpec) -> Self {
        Self { connection }
    }

    pub async fn get_signature_receipt(
        &self,
        signature: &str,
    ) -> Result<SolanaReceiptObservation, SolanaLiveReceiptError> {
        let client = RpcClient::new(self.connection.rpc_url.clone());
        let signature = signature
            .parse::<Signature>()
            .map_err(|error| SolanaLiveReceiptError::InvalidSignature(error.to_string()))?;
        Self::get_signature_receipt_with_client(&client, &signature).await
    }

    pub async fn get_signature_receipt_with_client<C>(
        client: &C,
        signature: &Signature,
    ) -> Result<SolanaReceiptObservation, SolanaLiveReceiptError>
    where
        C: SolanaRpcReceiptClient,
    {
        let status = client
            .get_signature_status(signature)
            .await
            .map_err(SolanaLiveReceiptError::Rpc)?;

        let Some(slot) = status.slot else {
            return Ok(SolanaReceiptObservation {
                signature: *signature,
                observed: false,
                success: None,
                slot: None,
                confirmation_depth: None,
                confirmation_status: None,
                source_hint: "solana_rpc:get_signature_statuses".to_owned(),
                payload: serde_json::json!({
                    "observed": false,
                    "signature": signature,
                }),
            });
        };

        let current_slot = client
            .get_slot()
            .await
            .map_err(SolanaLiveReceiptError::Rpc)?;
        let confirmation_depth = Some(current_slot.saturating_sub(slot).saturating_add(1));
        let success = status.error.is_none();

        Ok(SolanaReceiptObservation {
            signature: *signature,
            observed: true,
            success: Some(success),
            slot: Some(slot),
            confirmation_depth,
            confirmation_status: status.confirmation_status.clone(),
            source_hint: "solana_rpc:get_signature_statuses".to_owned(),
            payload: serde_json::json!({
                "signature": signature,
                "slot": slot,
                "confirmations": status.confirmations,
                "confirmation_status": status.confirmation_status,
                "error": status.error,
            }),
        })
    }
}
