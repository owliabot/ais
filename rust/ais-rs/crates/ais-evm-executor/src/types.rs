use crate::provider::ProviderError;
use alloy_primitives::{Address, Bytes, U256};
use serde_json::Value;

pub const EVM_RPC_ALLOWLIST: &[&str] = &[
    "eth_getBalance",
    "eth_blockNumber",
    "eth_getLogs",
    "eth_call",
    "eth_getTransactionReceipt",
    "eth_simulateV1",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmCallExecutionConfig {
    pub wait_for_receipt: bool,
    pub poll_interval_ms: u64,
    pub max_poll_attempts: u32,
}

impl Default for EvmCallExecutionConfig {
    fn default() -> Self {
        Self {
            wait_for_receipt: true,
            poll_interval_ms: 1_500,
            max_poll_attempts: 20,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum EvmExecutorError {
    #[error("{0}")]
    Message(String),
    #[error(transparent)]
    Provider(#[from] ProviderError),
}

pub struct EvmCallSendResult {
    pub tx_hash: String,
    pub nonce: Option<u64>,
    pub gas_limit: Option<u64>,
    pub max_fee_per_gas: Option<u128>,
    pub max_priority_fee_per_gas: Option<u128>,
    pub receipt: Option<Value>,
}

pub struct EvmReadRequest {
    pub chain: String,
    pub rpc_url: String,
    pub timeout_ms: u64,
    pub to: Address,
    pub data: Bytes,
}

pub struct EvmRpcRequest {
    pub chain: String,
    pub rpc_url: String,
    pub timeout_ms: u64,
    pub method: String,
    pub params: Value,
}

pub trait EvmReadRpcSender: Send + Sync {
    fn eth_call(&self, request: EvmReadRequest) -> Result<Bytes, String>;
    fn rpc_request(&self, request: EvmRpcRequest) -> Result<Value, String>;
}

pub struct EvmCallSendRequest {
    pub chain: String,
    pub rpc_url: String,
    pub timeout_ms: u64,
    pub from: Address,
    pub private_key_hex: String,
    pub to: Address,
    pub data: Bytes,
    pub value: U256,
    pub nonce: Option<u64>,
    pub gas_limit: Option<u64>,
    pub max_fee_per_gas: Option<u128>,
    pub max_priority_fee_per_gas: Option<u128>,
    pub wait_for_receipt: bool,
}

pub trait EvmCallSender: Send + Sync {
    fn send(&self, request: EvmCallSendRequest) -> Result<EvmCallSendResult, String>;
}
