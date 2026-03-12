use crate::binding::{
    evm::{EvmConnectionSpec, EvmObserveBinding, EvmObserveRequest},
    solana::{SolanaConnectionSpec, SolanaObserveBinding, SolanaObserveRequest},
};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObserveSourceKind {
    ChainRead,
    OffchainRead,
    WalletState,
    MetadataFetch,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvmObserveLiveBinding {
    #[serde(default)]
    pub connection: Option<EvmConnectionSpec>,
    pub binding: EvmObserveBinding,
    pub request: EvmObserveRequest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SolanaObserveLiveBinding {
    #[serde(default)]
    pub connection: Option<SolanaConnectionSpec>,
    pub binding: SolanaObserveBinding,
    pub request: SolanaObserveRequest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "family", rename_all = "snake_case")]
pub enum ObserveLiveBinding {
    Evm(EvmObserveLiveBinding),
    Solana(SolanaObserveLiveBinding),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObserveAction {
    pub source_kind: ObserveSourceKind,
    pub source_hint: String,
    pub output_key: Option<String>,
    #[serde(default)]
    pub live: Option<ObserveLiveBinding>,
}

impl ObserveAction {
    pub fn evm_live(&self) -> Option<&EvmObserveLiveBinding> {
        match &self.live {
            Some(ObserveLiveBinding::Evm(live)) => Some(live),
            _ => None,
        }
    }

    pub fn solana_live(&self) -> Option<&SolanaObserveLiveBinding> {
        match &self.live {
            Some(ObserveLiveBinding::Solana(live)) => Some(live),
            _ => None,
        }
    }
}
