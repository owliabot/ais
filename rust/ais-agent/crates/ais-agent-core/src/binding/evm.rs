use alloy::primitives::{Address, Bytes, U256};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvmConnectionSpec {
    pub http_url: String,
    #[serde(default)]
    pub ws_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvmObserveBinding {
    BlockNumber,
    NativeBalance,
    StorageSlot,
    Erc20BalanceOf,
    Erc20Allowance,
    ContractStateRead,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EvmObserveRequest {
    BlockNumber,
    NativeBalance {
        address: Address,
    },
    StorageSlot {
        address: Address,
        slot: U256,
    },
    Erc20BalanceOf {
        token: Address,
        owner: Address,
    },
    Erc20Allowance {
        token: Address,
        owner: Address,
        spender: Address,
    },
    ContractStateRead {
        to: Address,
        data: Bytes,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvmSimulateBinding {
    EthCall,
    TraceCall,
    TraceCallMany,
    AnvilStatefulCall,
    AnvilStatefulBundle,
    StateDeltaEstimate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct EvmCallRequest {
    #[serde(default)]
    pub from: Option<Address>,
    pub to: Address,
    #[serde(default)]
    pub data: Bytes,
    #[serde(default)]
    pub value: Option<U256>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvmActuateBinding {
    BroadcastSignedEnvelope,
    BroadcastTypedTransaction,
    BroadcastRawTransaction,
    SubmitExternalEnvelope,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvmVerifyBinding {
    ReceiptStatus,
    EffectContractFromReceipt,
    EffectContractFromPostState,
    EffectContractFromReceiptAndPostState,
}
