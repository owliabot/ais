use crate::abi::{build_calldata, decode_outputs, parse_abi_function};
use crate::client_pool::AlloyRpcClientPool;
use crate::provider::EvmProviderRegistry;
use crate::signer::EvmTransactionSigner;
use crate::types::EVM_RPC_ALLOWLIST;
pub use crate::types::{
    EvmCallExecutionConfig, EvmCallSendRequest, EvmCallSendResult, EvmCallSender, EvmExecutorError,
    EvmReadRequest, EvmReadRpcSender, EvmRpcRequest,
};
use crate::utils::{
    lit_or_value, normalize_evm_rpc_params, optional_u128_field, optional_u64_field, parse_address,
    parse_u256, parse_u256_quantity, value_or_lit_as_str,
};
use ais_engine::{
    canonical_side_effect_status, is_pending_side_effect_status, CheckpointSideEffectRecord,
    Executor, ExecutorOutput, SIDE_EFFECT_RECORD_SCHEMA_0_1_0, SIDE_EFFECT_STATUS_CONFIRMED,
    SIDE_EFFECT_STATUS_REVERTED, SIDE_EFFECT_STATUS_SENT,
};
use alloy::{
    consensus::Transaction,
    network::{EthereumWallet, TransactionBuilder},
    providers::{Provider, ProviderBuilder},
    rpc::types::TransactionRequest,
    signers::local::PrivateKeySigner,
};
use alloy_primitives::hex;
use alloy_primitives::{Bytes, U256};
use serde_json::{json, Map, Value};
use std::borrow::Cow;
use std::sync::Arc;
use std::time::Duration;

pub struct EvmExecutor {
    providers: EvmProviderRegistry,
    signer: Option<Box<dyn EvmTransactionSigner>>,
    call_config: EvmCallExecutionConfig,
    call_sender: Box<dyn EvmCallSender>,
    read_rpc_sender: Box<dyn EvmReadRpcSender>,
}

impl EvmExecutor {
    pub fn new(providers: EvmProviderRegistry) -> Self {
        let client_pool = Arc::new(AlloyRpcClientPool::new());
        Self {
            providers,
            signer: None,
            call_config: EvmCallExecutionConfig::default(),
            call_sender: Box::new(AlloyEvmCallSender::new(client_pool.clone())),
            read_rpc_sender: Box::new(AlloyEvmReadRpcSender::new(client_pool)),
        }
    }

    pub fn with_signer(mut self, signer: Box<dyn EvmTransactionSigner>) -> Self {
        self.signer = Some(signer);
        self
    }

    pub fn with_call_config(mut self, call_config: EvmCallExecutionConfig) -> Self {
        self.call_config = call_config;
        self
    }

    pub fn with_call_sender(mut self, call_sender: Box<dyn EvmCallSender>) -> Self {
        self.call_sender = call_sender;
        self
    }

    pub fn with_read_rpc_sender(mut self, read_rpc_sender: Box<dyn EvmReadRpcSender>) -> Self {
        self.read_rpc_sender = read_rpc_sender;
        self
    }

    pub fn supports(&self, chain: &str, execution_type: &str) -> bool {
        if !chain.starts_with("eip155:") {
            return false;
        }
        if self.providers.endpoint(chain).is_err() {
            return false;
        }
        matches!(execution_type, "evm_read" | "evm_call" | "evm_rpc")
    }
}

impl Executor for EvmExecutor {
    fn execute(&self, node: &Value, _runtime: &mut Value) -> Result<ExecutorOutput, String> {
        let node_id = node
            .as_object()
            .and_then(|object| object.get("id"))
            .and_then(Value::as_str)
            .ok_or_else(|| "node.id must be string".to_string())?;
        let chain = node
            .as_object()
            .and_then(|object| object.get("chain"))
            .and_then(Value::as_str)
            .ok_or_else(|| "node.chain must be string".to_string())?;
        let execution = node
            .as_object()
            .and_then(|object| object.get("execution"))
            .and_then(Value::as_object)
            .ok_or_else(|| "node.execution must be object".to_string())?;
        let execution_type = execution
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| "execution.type must be string".to_string())?;

        if !self.supports(chain, execution_type) {
            return Err(format!(
                "evm executor does not support chain `{chain}` + execution type `{execution_type}`"
            ));
        }

        match execution_type {
            "evm_read" => self.execute_evm_read(chain, execution),
            "evm_call" => self.execute_evm_call(node_id, chain, execution),
            "evm_rpc" => self.execute_evm_rpc(chain, execution),
            other => Err(format!(
                "unsupported execution type for evm executor: {other}"
            )),
        }
    }

    fn reconcile_side_effect(
        &self,
        record: &CheckpointSideEffectRecord,
    ) -> Result<Option<CheckpointSideEffectRecord>, String> {
        if record.execution_type.as_deref() != Some("evm_call") {
            return Ok(None);
        }
        let Some(chain) = record.chain.as_deref() else {
            return Ok(None);
        };
        if !self.supports(chain, "evm_call") {
            return Ok(None);
        }
        if !is_pending_side_effect_status(record.status.as_str()) {
            return Ok(Some(record.clone()));
        }

        let mut updated = record.clone();
        updated.observed_at = "1970-01-01T00:00:00Z".to_string();

        let Some(tx_hash) = updated.tx_hash.as_deref() else {
            updated.reason_code = Some("side_effect_reconcile_missing_tx_hash".to_string());
            return Ok(Some(updated));
        };
        if !looks_like_evm_tx_hash(tx_hash) {
            updated.reason_code = Some("side_effect_reconcile_invalid_tx_hash".to_string());
            return Ok(Some(updated));
        }

        let endpoint = match self.providers.endpoint(chain) {
            Ok(endpoint) => endpoint,
            Err(error) => {
                updated.reason_code =
                    Some("side_effect_reconcile_provider_unavailable".to_string());
                updated.details = Some(json!({ "error": error.to_string() }));
                return Ok(Some(updated));
            }
        };
        let receipt = match self.read_rpc_sender.rpc_request(EvmRpcRequest {
            chain: chain.to_string(),
            rpc_url: endpoint.rpc_url.clone(),
            timeout_ms: endpoint.timeout_ms,
            method: "eth_getTransactionReceipt".to_string(),
            params: json!([tx_hash]),
        }) {
            Ok(value) => value,
            Err(error) => {
                updated.reason_code = Some("side_effect_reconcile_rpc_error".to_string());
                updated.details = Some(json!({ "error": error }));
                return Ok(Some(updated));
            }
        };

        if receipt.is_null() {
            updated.reason_code = Some("side_effect_reconcile_pending".to_string());
            return Ok(Some(updated));
        }

        let receipt_success = is_receipt_success(&receipt);
        updated.details = Some(json!({ "receipt": receipt }));
        updated.reason_code = Some("side_effect_reconciled".to_string());
        if receipt_success {
            updated.status = SIDE_EFFECT_STATUS_CONFIRMED.to_string();
        } else {
            updated.status = SIDE_EFFECT_STATUS_REVERTED.to_string();
        }
        Ok(Some(updated))
    }
}

impl EvmExecutor {
    fn execute_evm_read(
        &self,
        chain: &str,
        execution: &Map<String, Value>,
    ) -> Result<ExecutorOutput, String> {
        let target = parse_address(value_or_lit_as_str(execution, "to")?)
            .map_err(|error| format!("invalid evm_read.to: {error}"))?;
        let abi = execution
            .get("abi")
            .and_then(Value::as_object)
            .ok_or_else(|| "evm_read.abi must be object".to_string())?;
        let function = parse_abi_function(abi)?;
        let args = execution
            .get("args")
            .and_then(Value::as_object)
            .ok_or_else(|| "evm_read.args must be object".to_string())?;

        let call_data = build_calldata(&function, args)?;
        let endpoint = self
            .providers
            .endpoint(chain)
            .map_err(|error| format!("provider unavailable: {error}"))?;
        let raw = self.read_rpc_sender.eth_call(EvmReadRequest {
            chain: chain.to_string(),
            rpc_url: endpoint.rpc_url.clone(),
            timeout_ms: endpoint.timeout_ms,
            to: target,
            data: call_data.clone(),
        })?;
        let decoded = decode_outputs(&raw, &function)?;
        let mut payload = Map::<String, Value>::new();
        payload.insert(
            "execution_type".to_string(),
            Value::String("evm_read".to_string()),
        );
        payload.insert("chain".to_string(), Value::String(chain.to_string()));
        payload.insert("to".to_string(), Value::String(format!("{target:#x}")));
        payload.insert("method".to_string(), Value::String(function.name));
        payload.insert(
            "call_data".to_string(),
            Value::String(format!("0x{}", hex::encode(call_data.as_ref()))),
        );
        payload.insert(
            "raw".to_string(),
            Value::String(format!("0x{}", hex::encode(raw.as_ref()))),
        );
        payload.insert("outputs".to_string(), Value::Object(decoded));

        Ok(ExecutorOutput {
            result: Value::Object(payload),
            writes: Map::new(),
            side_effects: Vec::new(),
        })
    }

    fn execute_evm_call(
        &self,
        node_id: &str,
        chain: &str,
        execution: &Map<String, Value>,
    ) -> Result<ExecutorOutput, String> {
        let target = parse_address(value_or_lit_as_str(execution, "to")?)
            .map_err(|error| format!("invalid evm_call.to: {error}"))?;
        let abi = execution
            .get("abi")
            .and_then(Value::as_object)
            .ok_or_else(|| "evm_call.abi must be object".to_string())?;
        let function = parse_abi_function(abi)?;
        let args = execution
            .get("args")
            .and_then(Value::as_object)
            .ok_or_else(|| "evm_call.args must be object".to_string())?;
        let call_data = build_calldata(&function, args)?;
        let value = execution
            .get("value")
            .map(lit_or_value)
            .map(parse_u256)
            .transpose()
            .map_err(|error| format!("invalid evm_call.value: {error}"))?
            .unwrap_or(U256::ZERO);

        let Some(signer) = self.signer.as_ref() else {
            return Err(format!(
                "need_user_confirm: missing signer for evm_call summary={}",
                json!({
                    "chain": chain,
                    "to": format!("{target:#x}"),
                    "method": function.name,
                    "value": value.to_string(),
                    "data": format!("0x{}", hex::encode(call_data.as_ref()))
                })
            ));
        };

        let from = signer
            .address()
            .ok_or_else(|| "evm_call signer must expose address".to_string())?;
        let private_key_hex = signer.private_key_hex().ok_or_else(|| {
            "evm_call signer must expose private_key_hex for alloy wallet".to_string()
        })?;
        let endpoint = self
            .providers
            .endpoint(chain)
            .map_err(|error| format!("provider unavailable: {error}"))?;
        let send_result = self.call_sender.send(EvmCallSendRequest {
            chain: chain.to_string(),
            rpc_url: endpoint.rpc_url.clone(),
            timeout_ms: endpoint.timeout_ms,
            from,
            private_key_hex,
            to: target,
            data: call_data.clone(),
            value,
            nonce: optional_u64_field(execution, "nonce")?,
            gas_limit: optional_u64_field(execution, "gas_limit")?,
            max_fee_per_gas: optional_u128_field(execution, "max_fee_per_gas")?,
            max_priority_fee_per_gas: optional_u128_field(execution, "max_priority_fee_per_gas")?,
            wait_for_receipt: self.call_config.wait_for_receipt,
        })?;

        let status = match send_result.receipt.as_ref() {
            Some(receipt) => {
                if is_receipt_success(receipt) {
                    SIDE_EFFECT_STATUS_CONFIRMED
                } else {
                    SIDE_EFFECT_STATUS_REVERTED
                }
            }
            None => SIDE_EFFECT_STATUS_SENT,
        };
        let tx_hash = send_result.tx_hash.clone();
        let side_effect = CheckpointSideEffectRecord {
            schema: Some(SIDE_EFFECT_RECORD_SCHEMA_0_1_0.to_string()),
            idempotency_key: format!("tx:{node_id}:{tx_hash}"),
            node_id: node_id.to_string(),
            effect_type: "tx".to_string(),
            chain: Some(chain.to_string()),
            execution_type: Some("evm_call".to_string()),
            tx_hash: Some(tx_hash),
            nonce: send_result.nonce,
            provider_ref: None,
            reason_code: None,
            details: None,
            status: canonical_side_effect_status(status).to_string(),
            observed_at: "1970-01-01T00:00:00Z".to_string(),
        };

        Ok(ExecutorOutput {
            result: json!({
                "execution_type": "evm_call",
                "chain": chain,
                "tx_hash": send_result.tx_hash,
                "tx": {
                    "to": format!("{target:#x}"),
                    "from": format!("{from:#x}"),
                    "value": value.to_string(),
                    "data": format!("0x{}", hex::encode(call_data.as_ref())),
                    "nonce": send_result.nonce,
                    "gas_limit": send_result.gas_limit,
                    "max_fee_per_gas": send_result.max_fee_per_gas.map(|n| n.to_string()),
                    "max_priority_fee_per_gas": send_result.max_priority_fee_per_gas.map(|n| n.to_string()),
                },
                "receipt": send_result.receipt,
            }),
            writes: Map::new(),
            side_effects: vec![side_effect],
        })
    }

    fn execute_evm_rpc(
        &self,
        chain: &str,
        execution: &Map<String, Value>,
    ) -> Result<ExecutorOutput, String> {
        let method = execution
            .get("method")
            .and_then(Value::as_str)
            .ok_or_else(|| "evm_rpc.method must be string".to_string())?;
        if !EVM_RPC_ALLOWLIST.contains(&method) {
            return Err(format!(
                "evm_rpc method not allowed: `{method}` (allowlist={})",
                EVM_RPC_ALLOWLIST.join(",")
            ));
        }
        let params = execution
            .get("params")
            .map(lit_or_value)
            .cloned()
            .unwrap_or(Value::Null);
        let params = normalize_evm_rpc_params(method, params)?;
        let endpoint = self
            .providers
            .endpoint(chain)
            .map_err(|error| format!("provider unavailable: {error}"))?;
        let result = self.read_rpc_sender.rpc_request(EvmRpcRequest {
            chain: chain.to_string(),
            rpc_url: endpoint.rpc_url.clone(),
            timeout_ms: endpoint.timeout_ms,
            method: method.to_string(),
            params: params.clone(),
        })?;
        let mut payload = Map::<String, Value>::new();
        payload.insert(
            "execution_type".to_string(),
            Value::String("evm_rpc".to_string()),
        );
        payload.insert("chain".to_string(), Value::String(chain.to_string()));
        payload.insert("method".to_string(), Value::String(method.to_string()));
        payload.insert("result".to_string(), result.clone());
        if method == "eth_getBalance" {
            if let Some(raw) = result.as_str() {
                if let Ok(balance) = parse_u256_quantity(raw) {
                    payload.insert("balance".to_string(), Value::String(balance.to_string()));
                }
            }
        }

        Ok(ExecutorOutput {
            result: Value::Object(payload),
            writes: Map::new(),
            side_effects: Vec::new(),
        })
    }
}

fn is_receipt_success(receipt: &Value) -> bool {
    receipt
        .as_object()
        .and_then(|object| object.get("status"))
        .map_or(true, |status| match status {
            Value::String(value) => {
                let normalized = value.trim().to_ascii_lowercase();
                normalized == "0x1" || normalized == "1" || normalized == "true"
            }
            Value::Number(value) => value.as_u64() == Some(1),
            Value::Bool(value) => *value,
            _ => false,
        })
}

fn looks_like_evm_tx_hash(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 66 || !(bytes.starts_with(b"0x") || bytes.starts_with(b"0X")) {
        return false;
    }
    bytes[2..]
        .iter()
        .all(|ch| (*ch as char).is_ascii_hexdigit())
}

pub struct AlloyEvmCallSender {
    client_pool: Arc<AlloyRpcClientPool>,
}
pub struct AlloyEvmReadRpcSender {
    client_pool: Arc<AlloyRpcClientPool>,
}

impl AlloyEvmCallSender {
    fn new(client_pool: Arc<AlloyRpcClientPool>) -> Self {
        Self { client_pool }
    }
}

impl AlloyEvmReadRpcSender {
    fn new(client_pool: Arc<AlloyRpcClientPool>) -> Self {
        Self { client_pool }
    }
}

impl EvmCallSender for AlloyEvmCallSender {
    fn send(&self, request: EvmCallSendRequest) -> Result<EvmCallSendResult, String> {
        send_evm_call_with_fillers(request, self.client_pool.clone())
    }
}

impl EvmReadRpcSender for AlloyEvmReadRpcSender {
    fn eth_call(&self, request: EvmReadRequest) -> Result<Bytes, String> {
        let client = self.client_pool.client(
            request.chain.as_str(),
            request.rpc_url.as_str(),
            request.timeout_ms,
        )?;
        self.client_pool.runtime.block_on(async move {
            let provider = ProviderBuilder::new().on_client(client);
            let tx = TransactionRequest::default()
                .with_to(request.to)
                .with_input(request.data);
            tokio::time::timeout(
                Duration::from_millis(request.timeout_ms),
                provider.call(&tx),
            )
            .await
            .map_err(|_| format!("eth_call timeout after {}ms", request.timeout_ms))?
            .map_err(|error| format!("eth_call failed: {error}"))
        })
    }

    fn rpc_request(&self, request: EvmRpcRequest) -> Result<Value, String> {
        let client = self.client_pool.client(
            request.chain.as_str(),
            request.rpc_url.as_str(),
            request.timeout_ms,
        )?;
        self.client_pool.runtime.block_on(async move {
            let provider = ProviderBuilder::new().on_client(client);
            let raw_params = serde_json::value::to_raw_value(&request.params).map_err(|error| {
                format!("rpc `{}` params encode failed: {error}", request.method)
            })?;
            let raw_result = tokio::time::timeout(
                Duration::from_millis(request.timeout_ms),
                provider.raw_request_dyn(Cow::Owned(request.method.clone()), &raw_params),
            )
            .await
            .map_err(|_| {
                format!(
                    "rpc `{}` timeout after {}ms",
                    request.method, request.timeout_ms
                )
            })?
            .map_err(|error| format!("evm_rpc `{}` failed: {error}", request.method))?;
            serde_json::from_str::<Value>(raw_result.get())
                .map_err(|error| format!("rpc `{}` result decode failed: {error}", request.method))
        })
    }
}

fn send_evm_call_with_fillers(
    request: EvmCallSendRequest,
    client_pool: Arc<AlloyRpcClientPool>,
) -> Result<EvmCallSendResult, String> {
    let client = client_pool.client(
        request.chain.as_str(),
        request.rpc_url.as_str(),
        request.timeout_ms,
    )?;
    client_pool.runtime.block_on(async move {
        let local_signer: PrivateKeySigner = request
            .private_key_hex
            .parse()
            .map_err(|error| format!("invalid signer private key: {error}"))?;
        let wallet = EthereumWallet::new(local_signer);
        let provider = ProviderBuilder::new()
            .with_recommended_fillers()
            .wallet(wallet)
            .on_client(client);

        let mut tx = TransactionRequest::default()
            .with_from(request.from)
            .with_to(request.to)
            .with_input(request.data)
            .with_value(request.value);
        if let Some(nonce) = request.nonce {
            tx = tx.with_nonce(nonce);
        }
        if let Some(gas_limit) = request.gas_limit {
            tx = tx.with_gas_limit(gas_limit);
        }
        if let Some(max_fee_per_gas) = request.max_fee_per_gas {
            tx = tx.with_max_fee_per_gas(max_fee_per_gas);
        }
        if let Some(max_priority_fee_per_gas) = request.max_priority_fee_per_gas {
            tx = tx.with_max_priority_fee_per_gas(max_priority_fee_per_gas);
        }

        let pending = provider.send_transaction(tx);
        let pending = tokio::time::timeout(Duration::from_millis(request.timeout_ms), pending)
            .await
            .map_err(|_| format!("send transaction timeout after {}ms", request.timeout_ms))?
            .map_err(|error| format!("send transaction failed: {error}"))?;
        let tx_hash = *pending.tx_hash();
        let pending_tx = tokio::time::timeout(
            Duration::from_millis(request.timeout_ms),
            provider.get_transaction_by_hash(tx_hash),
        )
        .await
        .map_err(|_| format!("get transaction timeout after {}ms", request.timeout_ms))?
        .map_err(|error| format!("get transaction by hash failed: {error}"))?;
        let receipt = if request.wait_for_receipt {
            Some(
                serde_json::to_value(
                    tokio::time::timeout(
                        Duration::from_millis(request.timeout_ms),
                        pending.get_receipt(),
                    )
                    .await
                    .map_err(|_| format!("get receipt timeout after {}ms", request.timeout_ms))?
                    .map_err(|error| format!("get receipt failed: {error}"))?,
                )
                .map_err(|error| format!("receipt json encode failed: {error}"))?,
            )
        } else {
            None
        };

        Ok(EvmCallSendResult {
            tx_hash: format!("{tx_hash:#x}"),
            nonce: pending_tx.as_ref().map(|tx| tx.nonce()),
            gas_limit: pending_tx.as_ref().map(|tx| tx.gas_limit()),
            max_fee_per_gas: pending_tx.as_ref().map(|tx| tx.max_fee_per_gas()),
            max_priority_fee_per_gas: pending_tx
                .as_ref()
                .and_then(|tx| tx.max_priority_fee_per_gas()),
            receipt,
        })
    })
}

#[cfg(test)]
#[path = "executor_test.rs"]
mod tests;
