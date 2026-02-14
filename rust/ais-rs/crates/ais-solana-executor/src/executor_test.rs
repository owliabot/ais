use super::{SolanaExecutor, SolanaInstructionExecutionConfig};
use crate::signer::{SignedSolanaTransaction, SolanaTransactionSigner};
use crate::types::{
    ProviderError, SolanaInstructionRequest, SolanaProviderRegistry, SolanaRpcClient,
    SolanaRpcClientFactory, SolanaRpcEndpoint,
};
use ais_engine::Executor;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

#[test]
fn supports_requires_exact_configured_chain_and_types() {
    let providers = SolanaProviderRegistry::from_endpoints(vec![
        SolanaRpcEndpoint::new("solana:mainnet", "https://rpc.example").expect("valid endpoint"),
    ])
    .expect("must build registry");
    let executor = SolanaExecutor::new(providers, Box::new(MockFactory::default()));

    assert!(executor.supports("solana:mainnet", "solana_read"));
    assert!(executor.supports("solana:mainnet", "solana_instruction"));
    assert!(!executor.supports("solana:devnet", "solana_read"));
    assert!(!executor.supports("eip155:1", "solana_read"));
}

#[test]
fn solana_read_get_balance_dispatches_to_client() {
    let calls = Arc::new(Mutex::new(Vec::<String>::new()));
    let factory = MockFactory {
        calls: calls.clone(),
        instruction_requests: Arc::new(Mutex::new(Vec::new())),
    };
    let providers = SolanaProviderRegistry::from_endpoints(vec![
        SolanaRpcEndpoint::new("solana:mainnet", "https://rpc.example").expect("valid endpoint"),
    ])
    .expect("must build registry");
    let executor = SolanaExecutor::new(providers, Box::new(factory));
    let mut runtime = json!({});

    let output = executor
        .execute(
            &json!({
                "id": "read-1",
                "chain": "solana:mainnet",
                "execution": {
                    "type": "solana_read",
                    "method": "getBalance",
                    "params": [{"lit":"Fh3z...pubkey"}]
                }
            }),
            &mut runtime,
        )
        .expect("must execute");

    assert_eq!(output.result.get("method"), Some(&json!("getBalance")));
    assert_eq!(output.result.get("result"), Some(&json!(1000)));
    assert_eq!(calls.lock().expect("lock").as_slice(), ["getBalance"]);
}

#[test]
fn solana_read_rejects_method_outside_allowlist() {
    let providers = SolanaProviderRegistry::from_endpoints(vec![
        SolanaRpcEndpoint::new("solana:mainnet", "https://rpc.example").expect("valid endpoint"),
    ])
    .expect("must build registry");
    let executor = SolanaExecutor::new(providers, Box::new(MockFactory::default()));
    let mut runtime = json!({});
    let error = executor
        .execute(
            &json!({
                "id": "read-2",
                "chain": "solana:mainnet",
                "execution": {
                    "type": "solana_read",
                    "method": "requestAirdrop",
                    "params": []
                }
            }),
            &mut runtime,
        )
        .expect_err("must reject");
    assert!(error.contains("not allowed"));
}

#[test]
fn solana_instruction_dispatches_to_client() {
    let requests = Arc::new(Mutex::new(Vec::<SolanaInstructionRequest>::new()));
    let factory = MockFactory {
        calls: Arc::new(Mutex::new(Vec::new())),
        instruction_requests: requests.clone(),
    };
    let providers = SolanaProviderRegistry::from_endpoints(vec![
        SolanaRpcEndpoint::new("solana:mainnet", "https://rpc.example").expect("valid endpoint"),
    ])
    .expect("must build registry");
    let executor = SolanaExecutor::new(providers, Box::new(factory)).with_signer(Box::new(MockSigner));
    let mut runtime = json!({});

    let output = executor
        .execute(
            &json!({
                "id": "ix-1",
                "chain": "solana:mainnet",
                "execution": {
                    "type": "solana_instruction",
                    "program": {"lit":"JUP6Lkb..."},
                    "instruction": "swap",
                    "accounts": [{
                        "name":"payer",
                        "pubkey":{"lit":"Fh3z...pubkey"},
                        "signer":{"lit":true},
                        "writable":{"lit":true}
                    }],
                    "data":{"lit":"base64:AAA="}
                }
            }),
            &mut runtime,
        )
        .expect("must execute");

    assert_eq!(output.result.get("signature"), Some(&json!("solana_sig_1")));
    assert_eq!(output.result.get("signed_tx_hash"), Some(&json!("0x1111")));
    assert_eq!(requests.lock().expect("lock").len(), 1);
}

#[test]
fn solana_instruction_without_signer_returns_need_user_confirm() {
    let providers = SolanaProviderRegistry::from_endpoints(vec![
        SolanaRpcEndpoint::new("solana:mainnet", "https://rpc.example").expect("valid endpoint"),
    ])
    .expect("must build registry");
    let executor = SolanaExecutor::new(providers, Box::new(MockFactory::default()));
    let mut runtime = json!({});

    let error = executor
        .execute(
            &json!({
                "id":"ix-2",
                "chain":"solana:mainnet",
                "execution":{
                    "type":"solana_instruction",
                    "program":{"lit":"JUP6Lkb..."},
                    "instruction":"swap",
                    "accounts":[{"name":"payer","pubkey":{"lit":"abc"},"signer":{"lit":true},"writable":{"lit":true}}],
                    "data":{"lit":"base64:AAA="}
                }
            }),
            &mut runtime,
        )
        .expect_err("must require signer");
    assert!(error.contains("need_user_confirm"));
}

#[test]
fn solana_instruction_with_lookup_tables_requires_v0() {
    let providers = SolanaProviderRegistry::from_endpoints(vec![
        SolanaRpcEndpoint::new("solana:mainnet", "https://rpc.example").expect("valid endpoint"),
    ])
    .expect("must build registry");
    let executor = SolanaExecutor::new(providers, Box::new(MockFactory::default()))
        .with_signer(Box::new(MockSigner));
    let mut runtime = json!({});
    let error = executor
        .execute(
            &json!({
                "id":"ix-3",
                "chain":"solana:mainnet",
                "execution":{
                    "type":"solana_instruction",
                    "tx_version":"legacy",
                    "program":{"lit":"JUP6Lkb..."},
                    "instruction":"swap",
                    "accounts":[{"name":"payer","pubkey":{"lit":"abc"},"signer":{"lit":true},"writable":{"lit":true}}],
                    "data":{"lit":"base64:AAA="},
                    "lookup_tables":{"lit":[{"address":"table-1"}]}
                }
            }),
            &mut runtime,
        )
        .expect_err("must reject");
    assert!(error.contains("tx_version=v0"));
}

#[test]
fn solana_instruction_with_signer_confirms_signature() {
    let requests = Arc::new(Mutex::new(Vec::<SolanaInstructionRequest>::new()));
    let factory = MockFactory {
        calls: Arc::new(Mutex::new(Vec::new())),
        instruction_requests: requests,
    };
    let providers = SolanaProviderRegistry::from_endpoints(vec![
        SolanaRpcEndpoint::new("solana:mainnet", "https://rpc.example").expect("valid endpoint"),
    ])
    .expect("must build registry");
    let executor = SolanaExecutor::new(providers, Box::new(factory))
        .with_signer(Box::new(MockSigner))
        .with_instruction_config(SolanaInstructionExecutionConfig {
            wait_for_confirmation: true,
            poll_interval_ms: 1,
            max_poll_attempts: 1,
        });
    let mut runtime = json!({});
    let output = executor
        .execute(
            &json!({
                "id":"ix-4",
                "chain":"solana:mainnet",
                "execution":{
                    "type":"solana_instruction",
                    "tx_version":"v0",
                    "program":{"lit":"JUP6Lkb..."},
                    "instruction":"swap",
                    "accounts":[{"name":"payer","pubkey":{"lit":"abc"},"signer":{"lit":true},"writable":{"lit":true}}],
                    "data":{"lit":"base64:AAA="},
                    "lookup_tables":{"lit":[{"address":"table-1"}]}
                }
            }),
            &mut runtime,
        )
        .expect("must execute");
    assert_eq!(output.result.get("tx_version"), Some(&json!("v0")));
    assert!(output.result.get("confirmation_status").is_some());
}

#[derive(Default)]
struct MockFactory {
    calls: Arc<Mutex<Vec<String>>>,
    instruction_requests: Arc<Mutex<Vec<SolanaInstructionRequest>>>,
}

impl SolanaRpcClientFactory for MockFactory {
    fn build_client(
        &self,
        _endpoint: &SolanaRpcEndpoint,
    ) -> Result<Box<dyn SolanaRpcClient>, ProviderError> {
        Ok(Box::new(MockClient {
            calls: self.calls.clone(),
            instruction_requests: self.instruction_requests.clone(),
        }))
    }
}

struct MockClient {
    calls: Arc<Mutex<Vec<String>>>,
    instruction_requests: Arc<Mutex<Vec<SolanaInstructionRequest>>>,
}

impl SolanaRpcClient for MockClient {
    fn get_balance(&self, _pubkey: &str) -> Result<u64, ProviderError> {
        self.calls.lock().expect("lock").push("getBalance".to_string());
        Ok(1000)
    }

    fn get_account_info(&self, _pubkey: &str) -> Result<Value, ProviderError> {
        self.calls
            .lock()
            .expect("lock")
            .push("getAccountInfo".to_string());
        Ok(json!({"owner":"1111"}))
    }

    fn get_token_account_balance(&self, _pubkey: &str) -> Result<Value, ProviderError> {
        self.calls
            .lock()
            .expect("lock")
            .push("getTokenAccountBalance".to_string());
        Ok(json!({"amount":"10"}))
    }

    fn get_signature_statuses(&self, _signatures: &[String]) -> Result<Value, ProviderError> {
        self.calls
            .lock()
            .expect("lock")
            .push("getSignatureStatuses".to_string());
        Ok(json!([{"confirmationStatus":"confirmed"}]))
    }

    fn send_signed_transaction(
        &self,
        request: &SolanaInstructionRequest,
        _signed_tx: &str,
    ) -> Result<String, ProviderError> {
        self.instruction_requests
            .lock()
            .expect("lock")
            .push(request.clone());
        Ok("solana_sig_1".to_string())
    }
}

struct MockSigner;

impl SolanaTransactionSigner for MockSigner {
    fn sign_instruction(
        &self,
        _request: &SolanaInstructionRequest,
    ) -> Result<SignedSolanaTransaction, crate::signer::SignerError> {
        Ok(SignedSolanaTransaction {
            raw_tx: "base64:deadbeef".to_string(),
            tx_hash: "0x1111".to_string(),
        })
    }
}
