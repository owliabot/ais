use super::{
    OffchainApyExecutor, OffchainApyExecutorConfig, OffchainApyHttpRequest, OffchainHttpClient,
};
use ais_engine::{CheckpointSideEffectRecord, Executor};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
struct StubClient {
    responses: Arc<Mutex<VecDeque<Result<Value, String>>>>,
}

impl StubClient {
    fn from_responses(responses: Vec<Result<Value, String>>) -> Self {
        Self {
            responses: Arc::new(Mutex::new(VecDeque::from(responses))),
        }
    }
}

impl OffchainHttpClient for StubClient {
    fn send_json(&self, _request: &OffchainApyHttpRequest) -> Result<Value, String> {
        self.responses
            .lock()
            .expect("lock")
            .pop_front()
            .unwrap_or_else(|| Err("no more responses".to_string()))
    }
}

fn test_config() -> OffchainApyExecutorConfig {
    OffchainApyExecutorConfig {
        chain: "eip155:1".to_string(),
        allowed_domains: vec!["api.example.com".to_string(), "*.trusted.org".to_string()],
        timeout_ms: 2000,
        max_retries: 1,
        retry_backoff_ms: 0,
    }
}

#[test]
fn rejects_endpoint_outside_allowlist() {
    let executor = OffchainApyExecutor::with_client(
        test_config(),
        Box::new(StubClient::from_responses(vec![Ok(json!({"apy":"0.031"}))])),
    );
    let node = json!({
        "id": "n1",
        "chain": "eip155:1",
        "execution": {
            "type": "offchain_apy_query",
            "endpoint": "https://evil.example.net/rates"
        }
    });
    let mut runtime = json!({});
    let error = executor
        .execute(&node, &mut runtime)
        .expect_err("must reject");
    assert!(error.contains("not in allowlist"));
}

#[test]
fn accepts_wildcard_allowlist_and_projects_outputs() {
    let executor = OffchainApyExecutor::with_client(
        test_config(),
        Box::new(StubClient::from_responses(vec![Ok(json!({
            "supply_apy": "0.031",
            "market": "aave-v3"
        }))])),
    );
    let node = json!({
        "id": "n1",
        "chain": "eip155:1",
        "execution": {
            "type": "offchain_apy_query",
            "endpoint": "https://api.trusted.org/v1/apy",
            "method": "POST",
            "args": {
                "asset": {"lit": "USDC"}
            }
        }
    });
    let mut runtime = json!({});
    let output = executor.execute(&node, &mut runtime).expect("must execute");
    assert_eq!(
        output.result.get("execution_type").and_then(Value::as_str),
        Some("offchain_apy_query")
    );
    assert_eq!(
        output
            .result
            .get("outputs")
            .and_then(Value::as_object)
            .and_then(|obj| obj.get("supply_apy"))
            .and_then(Value::as_str),
        Some("0.031")
    );
    assert!(output.side_effects.is_empty());
}

#[test]
fn retries_once_before_success() {
    let executor = OffchainApyExecutor::with_client(
        test_config(),
        Box::new(StubClient::from_responses(vec![
            Err("temporary network error".to_string()),
            Ok(json!({"supply_apy":"0.031"})),
        ])),
    );
    let node = json!({
        "id": "n1",
        "chain": "eip155:1",
        "execution": {
            "type": "offchain_apy_query",
            "endpoint": "https://api.example.com/apy"
        }
    });
    let mut runtime = json!({});
    let output = executor.execute(&node, &mut runtime).expect("must execute");
    assert_eq!(
        output.result.get("attempt").and_then(Value::as_u64),
        Some(2)
    );
}

#[test]
fn maps_side_effects_from_offchain_outputs() {
    let executor = OffchainApyExecutor::with_client(
        test_config(),
        Box::new(StubClient::from_responses(vec![Ok(json!({
            "supply_apy":"0.031",
            "side_effects":[
                {
                    "effect_type":"tx",
                    "tx_hash":"0xabc",
                    "status":"confirmed"
                },
                {
                    "effect_type":"approval",
                    "idempotency_key":"approval:n1:spender",
                    "status":"sent",
                    "provider_ref":"mock-provider"
                }
            ]
        }))])),
    );
    let node = json!({
        "id": "n1",
        "chain": "eip155:1",
        "execution": {
            "type": "offchain_apy_query",
            "endpoint": "https://api.example.com/apy"
        }
    });
    let mut runtime = json!({});
    let output = executor.execute(&node, &mut runtime).expect("must execute");
    assert_eq!(output.side_effects.len(), 2);
    assert_eq!(output.side_effects[0].idempotency_key, "tx:n1:0xabc");
    assert_eq!(output.side_effects[0].status, "confirmed");
    assert_eq!(
        output.side_effects[0].execution_type.as_deref(),
        Some("offchain_apy_query")
    );
    assert_eq!(
        output.side_effects[1].idempotency_key,
        "approval:n1:spender"
    );
    assert_eq!(output.side_effects[1].effect_type, "approval");
    assert_eq!(
        output.side_effects[1].provider_ref.as_deref(),
        Some("mock-provider")
    );
}

#[test]
fn reconcile_sent_offchain_side_effect_marks_not_supported_reason_code() {
    let executor = OffchainApyExecutor::with_client(
        test_config(),
        Box::new(StubClient::from_responses(vec![Ok(json!({"ok": true}))])),
    );
    let reconciled = executor
        .reconcile_side_effect(&CheckpointSideEffectRecord {
            schema: Some("ais-side-effect-record/0.1.0".to_string()),
            idempotency_key: "tx:n1:0xabc".to_string(),
            node_id: "n1".to_string(),
            effect_type: "tx".to_string(),
            chain: Some("eip155:1".to_string()),
            execution_type: Some("offchain_apy_query".to_string()),
            tx_hash: Some("0xabc".to_string()),
            nonce: None,
            provider_ref: None,
            reason_code: None,
            details: None,
            status: "sent".to_string(),
            observed_at: "2026-02-25T00:00:00Z".to_string(),
        })
        .expect("reconcile should not fail")
        .expect("must reconcile offchain side-effect");
    assert_eq!(reconciled.status, "sent");
    assert_eq!(
        reconciled.reason_code.as_deref(),
        Some("side_effect_reconcile_not_supported")
    );
}
