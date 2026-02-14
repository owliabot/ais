use super::{
    EvmCallExecutionConfig, EvmCallSendRequest, EvmCallSendResult, EvmCallSender, EvmExecutor,
    EvmReadRequest, EvmReadRpcSender, EvmRpcRequest,
};
use crate::provider::{EvmProviderRegistry, EvmRpcEndpoint};
use crate::signer::EvmTransactionSigner;
use ais_engine::Executor;
use alloy_dyn_abi::{DynSolValue, FunctionExt as DynFunctionExt};
use alloy_json_abi::Function;
use alloy_primitives::{hex, Address, Bytes};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

#[test]
fn supports_requires_exact_configured_chain_and_supported_type() {
    let providers = EvmProviderRegistry::from_endpoints(vec![
        EvmRpcEndpoint::new("eip155:1", "https://eth.example").expect("valid endpoint"),
    ])
    .expect("valid registry");
    let executor = EvmExecutor::new(providers);

    assert!(executor.supports("eip155:1", "evm_read"));
    assert!(!executor.supports("eip155:137", "evm_read"));
    assert!(!executor.supports("solana:mainnet", "evm_read"));
    assert!(!executor.supports("eip155:1", "solana_read"));
}

#[test]
fn execute_rejects_mismatched_execution_type() {
    let providers = EvmProviderRegistry::from_endpoints(vec![
        EvmRpcEndpoint::new("eip155:1", "https://eth.example").expect("valid endpoint"),
    ])
    .expect("valid registry");
    let executor = EvmExecutor::new(providers);
    let mut runtime = json!({});

    let error = executor
        .execute(
            &json!({
                "id": "n1",
                "chain": "eip155:1",
                "execution": {"type": "solana_instruction"}
            }),
            &mut runtime,
        )
        .expect_err("must reject unsupported type");
    assert!(error.contains("does not support"));
}

#[test]
fn evm_read_executes_eth_call_and_decodes_uint_output() {
    let read_requests = Arc::new(Mutex::new(Vec::<EvmReadRequest>::new()));
    let providers = EvmProviderRegistry::from_endpoints(vec![
        EvmRpcEndpoint::new("eip155:1", "https://eth.example").expect("valid endpoint"),
    ])
    .expect("valid registry");
    let executor = EvmExecutor::new(providers).with_read_rpc_sender(Box::new(MockReadRpcSender {
        read_requests: read_requests.clone(),
        read_response: Bytes::from(hex_bytes(
            "0000000000000000000000000000000000000000000000000000000000000064",
        )),
        rpc_calls: Arc::new(Mutex::new(Vec::new())),
        rpc_responses: BTreeMap::new(),
    }));
    let mut runtime = json!({});

    let result = executor
        .execute(
            &json!({
                "id": "quote-1",
                "chain": "eip155:1",
                "execution": {
                    "type": "evm_read",
                    "to": {"lit": "0x0000000000000000000000000000000000000001"},
                    "abi": {
                        "type": "function",
                        "name": "balanceOf",
                        "inputs": [{"name": "owner", "type": "address"}],
                        "outputs": [{"name": "balance", "type": "uint256"}]
                    },
                    "args": {
                        "owner": {"lit": "0x0000000000000000000000000000000000000002"}
                    }
                }
            }),
            &mut runtime,
        )
        .expect("evm_read must succeed");

    let outputs = result
        .result
        .get("outputs")
        .and_then(Value::as_object)
        .expect("outputs must exist");
    assert_eq!(
        outputs.get("balance"),
        Some(&Value::String("100".to_string()))
    );

    let requests = read_requests.lock().expect("must lock");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].timeout_ms, 30_000);
    assert_eq!(
        format!("{:#x}", requests[0].to),
        "0x0000000000000000000000000000000000000001"
    );
    assert!(requests[0].data.len() >= 4);
}

#[test]
fn evm_read_decodes_dynamic_string_output() {
    let function: Function = serde_json::from_value(json!({
        "type":"function",
        "name":"symbol",
        "inputs":[],
        "outputs":[{"name":"symbol","type":"string"}]
    }))
    .expect("abi");
    let encoded = function
        .abi_encode_output(&[DynSolValue::String("USDC".to_string())])
        .expect("encode output");
    let providers = EvmProviderRegistry::from_endpoints(vec![
        EvmRpcEndpoint::new("eip155:1", "https://eth.example").expect("valid endpoint"),
    ])
    .expect("valid registry");
    let executor = EvmExecutor::new(providers).with_read_rpc_sender(Box::new(MockReadRpcSender {
        read_requests: Arc::new(Mutex::new(Vec::new())),
        read_response: Bytes::from(encoded),
        rpc_calls: Arc::new(Mutex::new(Vec::new())),
        rpc_responses: BTreeMap::new(),
    }));
    let mut runtime = json!({});

    let result = executor
        .execute(
            &json!({
                "id":"read-string",
                "chain":"eip155:1",
                "execution":{
                    "type":"evm_read",
                    "to":{"lit":"0x0000000000000000000000000000000000000001"},
                    "abi":{
                        "type":"function",
                        "name":"symbol",
                        "inputs":[],
                        "outputs":[{"name":"symbol","type":"string"}]
                    },
                    "args": {}
                }
            }),
            &mut runtime,
        )
        .expect("evm_read must succeed");

    let outputs = result
        .result
        .get("outputs")
        .and_then(Value::as_object)
        .expect("outputs must be object");
    assert_eq!(
        outputs.get("symbol"),
        Some(&Value::String("USDC".to_string()))
    );
}

#[test]
fn evm_read_supports_tuple_object_args() {
    let read_requests = Arc::new(Mutex::new(Vec::<EvmReadRequest>::new()));
    let providers = EvmProviderRegistry::from_endpoints(vec![
        EvmRpcEndpoint::new("eip155:1", "https://eth.example").expect("valid endpoint"),
    ])
    .expect("valid registry");
    let executor = EvmExecutor::new(providers).with_read_rpc_sender(Box::new(MockReadRpcSender {
        read_requests: read_requests.clone(),
        read_response: Bytes::from(hex_bytes(
            "0000000000000000000000000000000000000000000000000000000000000001",
        )),
        rpc_calls: Arc::new(Mutex::new(Vec::new())),
        rpc_responses: BTreeMap::new(),
    }));
    let mut runtime = json!({});

    executor
        .execute(
            &json!({
                "id":"read-tuple",
                "chain":"eip155:1",
                "execution":{
                    "type":"evm_read",
                    "to":{"lit":"0x0000000000000000000000000000000000000001"},
                    "abi":{
                        "type":"function",
                        "name":"quote",
                        "inputs":[
                            {
                                "name":"pair",
                                "type":"tuple",
                                "components":[
                                    {"name":"owner","type":"address"},
                                    {"name":"amount","type":"uint256"}
                                ]
                            }
                        ],
                        "outputs":[{"name":"ok","type":"uint256"}]
                    },
                    "args":{
                        "pair":{
                            "owner":{"lit":"0x0000000000000000000000000000000000000002"},
                            "amount":{"lit":"42"}
                        }
                    }
                }
            }),
            &mut runtime,
        )
        .expect("tuple args must encode");

    let requests = read_requests.lock().expect("must lock");
    assert_eq!(requests.len(), 1);
    assert!(requests[0].data.len() >= 4 + 64);
}

#[test]
fn evm_call_without_signer_returns_need_user_confirm() {
    let providers = EvmProviderRegistry::from_endpoints(vec![
        EvmRpcEndpoint::new("eip155:1", "https://eth.example").expect("valid endpoint"),
    ])
    .expect("valid registry");
    let executor = EvmExecutor::new(providers);
    let mut runtime = json!({});

    let error = executor
        .execute(
            &json!({
                "id": "call-1",
                "chain": "eip155:1",
                "execution": {
                    "type": "evm_call",
                    "to": {"lit": "0x0000000000000000000000000000000000000001"},
                    "abi": {
                        "type": "function",
                        "name": "transfer",
                        "inputs": [
                            {"name":"to","type":"address"},
                            {"name":"amount","type":"uint256"}
                        ]
                    },
                    "args": {
                        "to": {"lit": "0x0000000000000000000000000000000000000002"},
                        "amount": {"lit": "10"}
                    }
                }
            }),
            &mut runtime,
        )
        .expect_err("must require signer");
    assert!(error.contains("need_user_confirm"));
}

#[test]
fn evm_call_with_signer_sends_and_returns_receipt() {
    let providers = EvmProviderRegistry::from_endpoints(vec![
        EvmRpcEndpoint::new("eip155:1", "https://eth.example").expect("valid endpoint"),
    ])
    .expect("valid registry");
    let call_requests = Arc::new(Mutex::new(Vec::<EvmCallSendRequest>::new()));
    let executor = EvmExecutor::new(providers)
        .with_signer(Box::new(MockSigner))
        .with_call_sender(Box::new(MockCallSender {
            requests: call_requests.clone(),
            result: EvmCallSendResult {
                tx_hash: "0x02".to_string(),
                nonce: Some(3),
                gas_limit: Some(21_000),
                max_fee_per_gas: Some(5_000_000_000),
                max_priority_fee_per_gas: Some(1_000_000_000),
                receipt: Some(json!({"status":"0x1"})),
            },
        }))
        .with_call_config(EvmCallExecutionConfig {
            wait_for_receipt: true,
            poll_interval_ms: 1,
            max_poll_attempts: 1,
        });
    let mut runtime = json!({});

    let result = executor
        .execute(
            &json!({
                "id": "call-2",
                "chain": "eip155:1",
                "execution": {
                    "type": "evm_call",
                    "to": {"lit": "0x0000000000000000000000000000000000000001"},
                    "abi": {
                        "type": "function",
                        "name": "transfer",
                        "inputs": [
                            {"name":"to","type":"address"},
                            {"name":"amount","type":"uint256"}
                        ]
                    },
                    "args": {
                        "to": {"lit": "0x0000000000000000000000000000000000000002"},
                        "amount": {"lit": "10"}
                    }
                }
            }),
            &mut runtime,
        )
        .expect("evm_call must succeed");

    assert_eq!(result.result.get("tx_hash"), Some(&Value::String("0x02".to_string())));
    assert_eq!(result.result.pointer("/tx/nonce"), Some(&Value::Number(3u64.into())));
    assert!(result.result.get("receipt").is_some());
    assert_eq!(call_requests.lock().expect("lock").len(), 1);
}

#[test]
fn evm_call_uses_filler_sender_for_nonce_and_fee_fields() {
    let call_requests = Arc::new(Mutex::new(Vec::<EvmCallSendRequest>::new()));
    let providers = EvmProviderRegistry::from_endpoints(vec![
        EvmRpcEndpoint::new("eip155:1", "https://eth.example").expect("valid endpoint"),
    ])
    .expect("valid registry");
    let executor = EvmExecutor::new(providers)
        .with_signer(Box::new(MockSigner))
        .with_call_sender(Box::new(MockCallSender {
            requests: call_requests.clone(),
            result: EvmCallSendResult {
                tx_hash: "0x03".to_string(),
                nonce: Some(8),
                gas_limit: Some(42_000),
                max_fee_per_gas: Some(9_000_000_000),
                max_priority_fee_per_gas: Some(2_000_000_000),
                receipt: None,
            },
        }))
        .with_call_config(EvmCallExecutionConfig {
            wait_for_receipt: false,
            poll_interval_ms: 1,
            max_poll_attempts: 1,
        });
    let mut runtime = json!({});

    let result = executor
        .execute(
            &json!({
                "id": "call-estimate-1",
                "chain": "eip155:1",
                "execution": {
                    "type": "evm_call",
                    "to": {"lit": "0x0000000000000000000000000000000000000001"},
                    "abi": {
                        "type": "function",
                        "name": "transfer",
                        "inputs": [
                            {"name":"to","type":"address"},
                            {"name":"amount","type":"uint256"}
                        ]
                    },
                    "args": {
                        "to": {"lit": "0x0000000000000000000000000000000000000002"},
                        "amount": {"lit": "10"}
                    }
                }
            }),
            &mut runtime,
        )
        .expect("evm_call must succeed");

    let requests = call_requests.lock().expect("must lock");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].timeout_ms, 30_000);
    assert_eq!(requests[0].nonce, None);
    assert_eq!(requests[0].gas_limit, None);
    assert_eq!(requests[0].max_fee_per_gas, None);
    assert_eq!(requests[0].max_priority_fee_per_gas, None);
    assert_eq!(
        result.result.pointer("/tx/gas_limit"),
        Some(&Value::Number(42_000u64.into()))
    );
    assert_eq!(
        result.result.pointer("/tx/max_priority_fee_per_gas"),
        Some(&Value::String("2000000000".to_string()))
    );
    assert_eq!(
        result.result.pointer("/tx/max_fee_per_gas"),
        Some(&Value::String("9000000000".to_string()))
    );
}

#[test]
fn evm_rpc_rejects_non_allowlisted_method() {
    let providers = EvmProviderRegistry::from_endpoints(vec![
        EvmRpcEndpoint::new("eip155:1", "https://eth.example").expect("valid endpoint"),
    ])
    .expect("valid registry");
    let executor = EvmExecutor::new(providers);
    let mut runtime = json!({});

    let error = executor
        .execute(
            &json!({
                "id": "rpc-1",
                "chain": "eip155:1",
                "execution": {
                    "type": "evm_rpc",
                    "method": "eth_sendRawTransaction",
                    "params": []
                }
            }),
            &mut runtime,
        )
        .expect_err("must reject");
    assert!(error.contains("not allowed"));
}

#[test]
fn evm_rpc_allowlisted_method_calls_provider() {
    let rpc_calls = Arc::new(Mutex::new(Vec::<(String, Value)>::new()));
    let providers = EvmProviderRegistry::from_endpoints(vec![
        EvmRpcEndpoint::new("eip155:1", "https://eth.example").expect("valid endpoint"),
    ])
    .expect("valid registry");
    let executor = EvmExecutor::new(providers).with_read_rpc_sender(Box::new(MockReadRpcSender {
        read_requests: Arc::new(Mutex::new(Vec::new())),
        read_response: Bytes::new(),
        rpc_calls: rpc_calls.clone(),
        rpc_responses: BTreeMap::from([(
            "eth_blockNumber".to_string(),
            Value::String("0x2a".to_string()),
        )]),
    }));
    let mut runtime = json!({});

    let result = executor
        .execute(
            &json!({
                "id": "rpc-2",
                "chain": "eip155:1",
                "execution": {
                    "type": "evm_rpc",
                    "method": "eth_blockNumber",
                    "params": []
                }
            }),
            &mut runtime,
        )
        .expect("must pass");

    assert_eq!(result.result.get("method"), Some(&Value::String("eth_blockNumber".to_string())));
    assert_eq!(rpc_calls.lock().expect("lock").len(), 1);
}

#[test]
fn evm_rpc_get_balance_object_params_are_normalized() {
    let rpc_calls = Arc::new(Mutex::new(Vec::<(String, Value)>::new()));
    let providers = EvmProviderRegistry::from_endpoints(vec![
        EvmRpcEndpoint::new("eip155:31338", "https://eth.example").expect("valid endpoint"),
    ])
    .expect("valid registry");
    let executor = EvmExecutor::new(providers).with_read_rpc_sender(Box::new(MockReadRpcSender {
        read_requests: Arc::new(Mutex::new(Vec::new())),
        read_response: Bytes::new(),
        rpc_calls: rpc_calls.clone(),
        rpc_responses: BTreeMap::from([(
            "eth_getBalance".to_string(),
            Value::String("0x2a".to_string()),
        )]),
    }));
    let mut runtime = json!({});

    let result = executor
        .execute(
            &json!({
                "id": "rpc-3",
                "chain": "eip155:31338",
                "execution": {
                    "type": "evm_rpc",
                    "method": "eth_getBalance",
                    "params": {
                        "address": {"lit": "0x0000000000000000000000000000000000000001"},
                        "block": "latest"
                    }
                }
            }),
            &mut runtime,
        )
        .expect("must pass");

    let calls = rpc_calls.lock().expect("lock");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "eth_getBalance");
    assert_eq!(
        calls[0].1,
        json!(["0x0000000000000000000000000000000000000001", "latest"])
    );
    assert_eq!(
        result.result.get("balance"),
        Some(&Value::String("42".to_string()))
    );
}

#[test]
fn evm_rpc_get_balance_array_wrapper_params_are_normalized() {
    let rpc_calls = Arc::new(Mutex::new(Vec::<(String, Value)>::new()));
    let providers = EvmProviderRegistry::from_endpoints(vec![
        EvmRpcEndpoint::new("eip155:31338", "https://eth.example").expect("valid endpoint"),
    ])
    .expect("valid registry");
    let executor = EvmExecutor::new(providers).with_read_rpc_sender(Box::new(MockReadRpcSender {
        read_requests: Arc::new(Mutex::new(Vec::new())),
        read_response: Bytes::new(),
        rpc_calls: rpc_calls.clone(),
        rpc_responses: BTreeMap::new(),
    }));
    let mut runtime = json!({});

    executor
        .execute(
            &json!({
                "id": "rpc-4",
                "chain": "eip155:31338",
                "execution": {
                    "type": "evm_rpc",
                    "method": "eth_getBalance",
                    "params": {
                        "array": [
                            {"lit": "0x0000000000000000000000000000000000000001"},
                            "latest"
                        ]
                    }
                }
            }),
            &mut runtime,
        )
        .expect("must pass");

    let calls = rpc_calls.lock().expect("lock");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "eth_getBalance");
    assert_eq!(
        calls[0].1,
        json!(["0x0000000000000000000000000000000000000001", "latest"])
    );
}

struct MockReadRpcSender {
    read_requests: Arc<Mutex<Vec<EvmReadRequest>>>,
    read_response: Bytes,
    rpc_calls: Arc<Mutex<Vec<(String, Value)>>>,
    rpc_responses: BTreeMap<String, Value>,
}

impl EvmReadRpcSender for MockReadRpcSender {
    fn eth_call(&self, request: EvmReadRequest) -> Result<Bytes, String> {
        self.read_requests
            .lock()
            .expect("must lock")
            .push(EvmReadRequest {
                chain: request.chain,
                rpc_url: request.rpc_url,
                timeout_ms: request.timeout_ms,
                to: request.to,
                data: request.data,
            });
        Ok(self.read_response.clone())
    }

    fn rpc_request(&self, request: EvmRpcRequest) -> Result<Value, String> {
        self.rpc_calls
            .lock()
            .expect("must lock")
            .push((request.method.clone(), request.params));
        Ok(self
            .rpc_responses
            .get(request.method.as_str())
            .cloned()
            .unwrap_or_else(|| json!({"ok": true})))
    }
}

struct MockSigner;

impl EvmTransactionSigner for MockSigner {
    fn private_key_hex(&self) -> Option<String> {
        Some("0x1111111111111111111111111111111111111111111111111111111111111111".to_string())
    }

    fn address(&self) -> Option<Address> {
        Some(
            "0x19E7E376E7C213B7E7E7E46CC70A5DD086DAFF2A"
                .parse()
                .expect("valid address"),
        )
    }
}

struct MockCallSender {
    requests: Arc<Mutex<Vec<EvmCallSendRequest>>>,
    result: EvmCallSendResult,
}

impl EvmCallSender for MockCallSender {
    fn send(&self, request: EvmCallSendRequest) -> Result<EvmCallSendResult, String> {
        self.requests.lock().expect("must lock").push(request);
        Ok(EvmCallSendResult {
            tx_hash: self.result.tx_hash.clone(),
            nonce: self.result.nonce,
            gas_limit: self.result.gas_limit,
            max_fee_per_gas: self.result.max_fee_per_gas,
            max_priority_fee_per_gas: self.result.max_priority_fee_per_gas,
            receipt: self.result.receipt.clone(),
        })
    }
}

fn hex_bytes(input: &str) -> Vec<u8> {
    hex::decode(input.trim()).expect("valid hex")
}
