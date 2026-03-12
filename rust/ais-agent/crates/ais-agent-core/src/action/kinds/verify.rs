use crate::binding::{
    evm::{EvmConnectionSpec, EvmObserveRequest, EvmVerifyBinding},
    solana::{SolanaConnectionSpec, SolanaObserveRequest, SolanaVerifyBinding},
};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerifyKind {
    EffectContract,
    ReceiptObserved,
    StateDelta,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvmVerifyLiveBinding {
    #[serde(default)]
    pub connection: Option<EvmConnectionSpec>,
    pub binding: EvmVerifyBinding,
    #[serde(default)]
    pub post_request: Option<EvmObserveRequest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SolanaVerifyLiveBinding {
    #[serde(default)]
    pub connection: Option<SolanaConnectionSpec>,
    pub binding: SolanaVerifyBinding,
    #[serde(default)]
    pub post_request: Option<SolanaObserveRequest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "family", rename_all = "snake_case")]
pub enum VerifyLiveBinding {
    Evm(EvmVerifyLiveBinding),
    Solana(SolanaVerifyLiveBinding),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifyAction {
    pub verify_kind: VerifyKind,
    pub verifier_hint: String,
    #[serde(default)]
    pub pre_observation_ref: Option<String>,
    #[serde(default)]
    pub post_observation_ref: Option<String>,
    #[serde(default)]
    pub live: Option<VerifyLiveBinding>,
}

impl VerifyAction {
    pub fn evm_live(&self) -> Option<&EvmVerifyLiveBinding> {
        match &self.live {
            Some(VerifyLiveBinding::Evm(live)) => Some(live),
            _ => None,
        }
    }

    pub fn solana_live(&self) -> Option<&SolanaVerifyLiveBinding> {
        match &self.live {
            Some(VerifyLiveBinding::Solana(live)) => Some(live),
            _ => None,
        }
    }
}
