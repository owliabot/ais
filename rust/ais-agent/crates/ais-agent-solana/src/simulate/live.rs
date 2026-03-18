//! Live Solana simulation port.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;
use solana_client::{nonblocking::rpc_client::RpcClient, rpc_config::RpcSimulateTransactionConfig};
use solana_sdk::{
    hash::Hash,
    message::{v0, Message, VersionedMessage},
    signature::Signature,
    transaction::VersionedTransaction,
};
use thiserror::Error;

use ais_agent_core::binding::solana::{SolanaConnectionSpec, SolanaTransactionRequest};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SolanaTransactionSimulationReport {
    pub accepted: bool,
    pub logs: Vec<String>,
    pub units_consumed: Option<u64>,
    pub error: Option<String>,
    pub replacement_blockhash: Option<String>,
    pub source_hint: String,
}

#[async_trait]
pub trait SolanaRpcSimulateClient: Send + Sync {
    async fn simulate_transaction(
        &self,
        transaction: &VersionedTransaction,
    ) -> Result<SolanaTransactionSimulationReport, String>;
}

#[async_trait]
impl SolanaRpcSimulateClient for RpcClient {
    async fn simulate_transaction(
        &self,
        transaction: &VersionedTransaction,
    ) -> Result<SolanaTransactionSimulationReport, String> {
        let config = RpcSimulateTransactionConfig {
            sig_verify: false,
            replace_recent_blockhash: true,
            ..Default::default()
        };
        let result = self
            .simulate_transaction_with_config(transaction, config)
            .await
            .map_err(|error| error.to_string())?;

        Ok(SolanaTransactionSimulationReport {
            accepted: result.value.err.is_none(),
            logs: result.value.logs.unwrap_or_default(),
            units_consumed: result.value.units_consumed,
            error: result.value.err.map(|error| format!("{error:?}")),
            replacement_blockhash: result
                .value
                .replacement_blockhash
                .map(|value| value.blockhash.to_string()),
            source_hint: "solana_rpc:simulate_transaction".to_owned(),
        })
    }
}

#[derive(Debug, Error)]
pub enum SolanaLiveSimulateError {
    #[error("solana simulate error: {0}")]
    Rpc(String),
    #[error("solana v0 simulation requires a payer")]
    MissingPayerForV0,
    #[error("failed to compile solana v0 message: {0}")]
    Compile(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SolanaLiveSimulatePort {
    connection: SolanaConnectionSpec,
}

impl SolanaLiveSimulatePort {
    pub fn new(connection: SolanaConnectionSpec) -> Self {
        Self { connection }
    }

    pub async fn simulate_transaction(
        &self,
        request: &SolanaTransactionRequest,
    ) -> Result<SolanaTransactionSimulationReport, SolanaLiveSimulateError> {
        let client = RpcClient::new(self.connection.http_url.clone());
        Self::simulate_with_client(&client, request).await
    }

    pub async fn simulate_with_client<C>(
        client: &C,
        request: &SolanaTransactionRequest,
    ) -> Result<SolanaTransactionSimulationReport, SolanaLiveSimulateError>
    where
        C: SolanaRpcSimulateClient,
    {
        let transaction = compile_transaction_request(request)?;
        client
            .simulate_transaction(&transaction)
            .await
            .map_err(SolanaLiveSimulateError::Rpc)
    }

    pub fn report_payload(report: &SolanaTransactionSimulationReport) -> serde_json::Value {
        json!({
            "accepted": report.accepted,
            "logs": report.logs,
            "units_consumed": report.units_consumed,
            "error": report.error,
            "replacement_blockhash": report.replacement_blockhash,
            "source_hint": report.source_hint,
        })
    }
}

pub fn compile_transaction_request(
    request: &SolanaTransactionRequest,
) -> Result<VersionedTransaction, SolanaLiveSimulateError> {
    match request {
        SolanaTransactionRequest::Legacy {
            recent_blockhash,
            payer,
            instructions,
        } => {
            let blockhash = recent_blockhash.unwrap_or_else(Hash::default);
            let message = Message::new_with_blockhash(instructions, payer.as_ref(), &blockhash);
            Ok(unsigned_transaction(VersionedMessage::Legacy(message)))
        }
        SolanaTransactionRequest::V0 {
            recent_blockhash,
            payer,
            instructions,
            address_lookup_tables,
        } => {
            let payer = payer.ok_or(SolanaLiveSimulateError::MissingPayerForV0)?;
            let blockhash = recent_blockhash.unwrap_or_else(Hash::default);
            let message =
                v0::Message::try_compile(&payer, instructions, address_lookup_tables, blockhash)
                    .map_err(|error| SolanaLiveSimulateError::Compile(error.to_string()))?;
            Ok(unsigned_transaction(VersionedMessage::V0(message)))
        }
    }
}

fn unsigned_transaction(message: VersionedMessage) -> VersionedTransaction {
    let required_signatures = message.header().num_required_signatures as usize;
    VersionedTransaction {
        signatures: vec![Signature::default(); required_signatures],
        message,
    }
}
