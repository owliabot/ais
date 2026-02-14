use super::{
    ProviderError, SolanaProviderRegistry, SolanaRpcClient, SolanaRpcClientFactory,
    SolanaRpcEndpoint,
};
use serde_json::Value;

#[test]
fn endpoint_rejects_non_solana_chain() {
    let error = SolanaRpcEndpoint::new("eip155:1", "https://rpc.example")
        .expect_err("must reject non solana chain");
    assert_eq!(error, ProviderError::InvalidChain("eip155:1".to_string()));
}

#[test]
fn endpoint_rejects_invalid_rpc_url() {
    let error = SolanaRpcEndpoint::new("solana:mainnet", "ws://rpc.example")
        .expect_err("must reject non-http url");
    assert_eq!(error, ProviderError::InvalidRpcUrl("ws://rpc.example".to_string()));
}

#[test]
fn registry_rejects_duplicate_chain() {
    let first =
        SolanaRpcEndpoint::new("solana:mainnet", "https://rpc-a.example").expect("valid endpoint");
    let second =
        SolanaRpcEndpoint::new("solana:mainnet", "https://rpc-b.example").expect("valid endpoint");
    let mut registry = SolanaProviderRegistry::new();
    registry.register_endpoint(first).expect("must register");
    let error = registry
        .register_endpoint(second)
        .expect_err("must reject duplicate");
    assert_eq!(
        error,
        ProviderError::DuplicateChain("solana:mainnet".to_string())
    );
}

#[test]
fn registry_builds_client_for_chain() {
    let endpoint =
        SolanaRpcEndpoint::new("solana:devnet", "https://rpc.example").expect("valid endpoint");
    let registry =
        SolanaProviderRegistry::from_endpoints(vec![endpoint]).expect("must build registry");
    let client = registry
        .build_client_for_chain("solana:devnet", &MockFactory)
        .expect("must build client");
    assert_eq!(client.get_balance("pubkey").expect("balance"), 1);
}

struct MockFactory;

impl SolanaRpcClientFactory for MockFactory {
    fn build_client(
        &self,
        _endpoint: &SolanaRpcEndpoint,
    ) -> Result<Box<dyn SolanaRpcClient>, ProviderError> {
        Ok(Box::new(MockClient))
    }
}

struct MockClient;

impl SolanaRpcClient for MockClient {
    fn get_balance(&self, _pubkey: &str) -> Result<u64, ProviderError> {
        Ok(1)
    }

    fn get_account_info(&self, _pubkey: &str) -> Result<Value, ProviderError> {
        Ok(Value::Null)
    }

    fn get_token_account_balance(&self, _pubkey: &str) -> Result<Value, ProviderError> {
        Ok(Value::Null)
    }

    fn get_signature_statuses(&self, _signatures: &[String]) -> Result<Value, ProviderError> {
        Ok(Value::Null)
    }

    fn send_signed_transaction(
        &self,
        _request: &super::SolanaInstructionRequest,
        _signed_tx: &str,
    ) -> Result<String, ProviderError> {
        Ok("sig".to_string())
    }
}
