//! Alloy-backed stateless EVM simulation port.

use ais_agent_core::binding::evm::EvmCallRequest;
use alloy::{
    network::TransactionBuilder,
    primitives::Bytes,
    providers::{Provider, ProviderBuilder},
    rpc::types::TransactionRequest,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EvmCallSimulationReport {
    pub accepted: bool,
    pub return_data: Bytes,
    pub source_hint: String,
}

#[derive(Debug, Error)]
pub enum EvmLiveSimulateError {
    #[error("invalid rpc url: {0}")]
    InvalidRpcUrl(String),
    #[error("alloy provider error: {0}")]
    Provider(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmAlloySimulatePort {
    rpc_url: String,
}

impl EvmAlloySimulatePort {
    pub fn new(rpc_url: impl Into<String>) -> Self {
        Self {
            rpc_url: rpc_url.into(),
        }
    }

    pub async fn eth_call(
        &self,
        request: &EvmCallRequest,
    ) -> Result<EvmCallSimulationReport, EvmLiveSimulateError> {
        let provider = ProviderBuilder::new()
            .connect(self.rpc_url.as_str())
            .await
            .map_err(|error| EvmLiveSimulateError::InvalidRpcUrl(error.to_string()))?;

        Self::eth_call_with_provider(&provider, request).await
    }

    pub async fn eth_call_with_provider<P>(
        provider: &P,
        request: &EvmCallRequest,
    ) -> Result<EvmCallSimulationReport, EvmLiveSimulateError>
    where
        P: Provider,
    {
        let tx = into_transaction_request(request);
        let return_data = provider
            .call(tx)
            .await
            .map_err(|error| EvmLiveSimulateError::Provider(error.to_string()))?;

        Ok(EvmCallSimulationReport {
            accepted: true,
            return_data,
            source_hint: "alloy_provider:eth_call".to_owned(),
        })
    }

    pub fn report_payload(report: &EvmCallSimulationReport) -> serde_json::Value {
        json!({
            "accepted": report.accepted,
            "return_data": report.return_data,
            "source_hint": report.source_hint,
        })
    }
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

#[cfg(test)]
mod tests {
    use ais_agent_core::binding::evm::EvmCallRequest;
    use alloy::{
        primitives::{address, bytes},
        providers::ProviderBuilder,
        transports::mock::Asserter,
    };

    use super::EvmAlloySimulatePort;

    #[tokio::test]
    async fn alloy_live_simulate_port_executes_eth_call() {
        let asserter = Asserter::new();
        let provider = ProviderBuilder::new().connect_mocked_client(asserter.clone());
        let return_data =
            bytes!("0000000000000000000000000000000000000000000000000000000000000001");
        asserter.push_success(&return_data);

        let report = EvmAlloySimulatePort::eth_call_with_provider(
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

        assert!(report.accepted);
        assert_eq!(report.return_data, return_data);
        assert_eq!(report.source_hint, "alloy_provider:eth_call");
    }
}
