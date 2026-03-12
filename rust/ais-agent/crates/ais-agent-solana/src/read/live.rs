//! Live Solana RPC-backed read port.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::{
    account::Account, pubkey::Pubkey, signature::Signature, transaction::TransactionError,
};
use thiserror::Error;

use ais_agent_core::binding::solana::{SolanaConnectionSpec, SolanaObserveRequest};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SolanaTokenBalanceSnapshot {
    pub amount: String,
    pub decimals: u8,
    pub ui_amount_string: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SolanaSignatureStatusSnapshot {
    pub slot: Option<u64>,
    pub confirmations: Option<usize>,
    pub confirmation_status: Option<String>,
    pub error: Option<String>,
}

#[async_trait]
pub trait SolanaRpcReadClient: Send + Sync {
    async fn get_slot(&self) -> Result<u64, String>;
    async fn get_balance(&self, address: &Pubkey) -> Result<u64, String>;
    async fn get_token_balance(
        &self,
        token_account: &Pubkey,
    ) -> Result<SolanaTokenBalanceSnapshot, String>;
    async fn get_account(&self, address: &Pubkey) -> Result<Account, String>;
    async fn get_signature_status(
        &self,
        signature: &Signature,
    ) -> Result<SolanaSignatureStatusSnapshot, String>;
}

#[async_trait]
impl SolanaRpcReadClient for RpcClient {
    async fn get_slot(&self) -> Result<u64, String> {
        self.get_slot().await.map_err(|error| error.to_string())
    }

    async fn get_balance(&self, address: &Pubkey) -> Result<u64, String> {
        self.get_balance(address)
            .await
            .map_err(|error| error.to_string())
    }

    async fn get_token_balance(
        &self,
        token_account: &Pubkey,
    ) -> Result<SolanaTokenBalanceSnapshot, String> {
        let balance = self
            .get_token_account_balance(token_account)
            .await
            .map_err(|error| error.to_string())?;
        Ok(SolanaTokenBalanceSnapshot {
            amount: balance.amount,
            decimals: balance.decimals,
            ui_amount_string: balance.ui_amount_string,
        })
    }

    async fn get_account(&self, address: &Pubkey) -> Result<Account, String> {
        self.get_account(address)
            .await
            .map_err(|error| error.to_string())
    }

    async fn get_signature_status(
        &self,
        signature: &Signature,
    ) -> Result<SolanaSignatureStatusSnapshot, String> {
        let statuses = self
            .get_signature_statuses(&[*signature])
            .await
            .map_err(|error| error.to_string())?;
        let Some(status) = statuses.value.into_iter().next().flatten() else {
            return Ok(SolanaSignatureStatusSnapshot {
                slot: None,
                confirmations: None,
                confirmation_status: None,
                error: None,
            });
        };
        Ok(SolanaSignatureStatusSnapshot {
            slot: Some(status.slot),
            confirmations: status.confirmations.map(|value| value as usize),
            confirmation_status: status
                .confirmation_status
                .map(|value| format!("{value:?}").to_lowercase()),
            error: status.err.map(transaction_error_to_string),
        })
    }
}

#[derive(Debug, Error)]
pub enum SolanaLiveReadError {
    #[error("solana rpc error: {0}")]
    Rpc(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SolanaLiveReadPort {
    connection: SolanaConnectionSpec,
}

impl SolanaLiveReadPort {
    pub fn new(connection: SolanaConnectionSpec) -> Self {
        Self { connection }
    }

    pub fn connection(&self) -> &SolanaConnectionSpec {
        &self.connection
    }

    pub async fn observe(
        &self,
        request: &SolanaObserveRequest,
    ) -> Result<serde_json::Value, SolanaLiveReadError> {
        let client = RpcClient::new(self.connection.rpc_url.clone());
        Self::observe_with_client(&client, request).await
    }

    pub async fn observe_with_client<C>(
        client: &C,
        request: &SolanaObserveRequest,
    ) -> Result<serde_json::Value, SolanaLiveReadError>
    where
        C: SolanaRpcReadClient,
    {
        match request {
            SolanaObserveRequest::Slot => {
                let slot = client.get_slot().await.map_err(SolanaLiveReadError::Rpc)?;
                Ok(json!({
                    "slot": slot,
                    "source_hint": "solana_rpc:get_slot",
                }))
            }
            SolanaObserveRequest::AccountLamports { address } => {
                let lamports = client
                    .get_balance(address)
                    .await
                    .map_err(SolanaLiveReadError::Rpc)?;
                Ok(json!({
                    "address": address,
                    "lamports": lamports,
                    "source_hint": "solana_rpc:get_balance",
                }))
            }
            SolanaObserveRequest::SplTokenBalance { token_account } => {
                let balance = client
                    .get_token_balance(token_account)
                    .await
                    .map_err(SolanaLiveReadError::Rpc)?;
                Ok(json!({
                    "token_account": token_account,
                    "amount": balance.amount,
                    "decimals": balance.decimals,
                    "ui_amount_string": balance.ui_amount_string,
                    "source_hint": "solana_rpc:get_token_account_balance",
                }))
            }
            SolanaObserveRequest::AccountData { address } => {
                let account = client
                    .get_account(address)
                    .await
                    .map_err(SolanaLiveReadError::Rpc)?;
                Ok(json!({
                    "address": address,
                    "lamports": account.lamports,
                    "owner": account.owner,
                    "executable": account.executable,
                    "rent_epoch": account.rent_epoch,
                    "data": account.data,
                    "source_hint": "solana_rpc:get_account",
                }))
            }
            SolanaObserveRequest::SignatureStatus { signature } => {
                let status = client
                    .get_signature_status(signature)
                    .await
                    .map_err(SolanaLiveReadError::Rpc)?;
                Ok(json!({
                    "signature": signature,
                    "slot": status.slot,
                    "confirmations": status.confirmations,
                    "confirmation_status": status.confirmation_status,
                    "error": status.error,
                    "source_hint": "solana_rpc:get_signature_statuses",
                }))
            }
        }
    }
}

fn transaction_error_to_string(error: TransactionError) -> String {
    format!("{error:?}")
}
