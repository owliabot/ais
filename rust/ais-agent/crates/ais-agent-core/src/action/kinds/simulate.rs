use crate::binding::{
    evm::{EvmCallRequest, EvmConnectionSpec, EvmSimulateBinding},
    solana::{SolanaConnectionSpec, SolanaSimulateBinding, SolanaTransactionRequest},
};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SimulateKind {
    Call,
    Bundle,
    StateDelta,
    GasEstimate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvmSimulateLiveBinding {
    #[serde(default)]
    pub connection: Option<EvmConnectionSpec>,
    pub binding: EvmSimulateBinding,
    pub request: EvmCallRequest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SolanaSimulateLiveBinding {
    #[serde(default)]
    pub connection: Option<SolanaConnectionSpec>,
    pub binding: SolanaSimulateBinding,
    pub request: SolanaTransactionRequest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "family", rename_all = "snake_case")]
pub enum SimulateLiveBinding {
    Evm(EvmSimulateLiveBinding),
    Solana(SolanaSimulateLiveBinding),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimulateAction {
    pub simulate_kind: SimulateKind,
    pub simulator_hint: String,
    #[serde(default)]
    pub live: Option<SimulateLiveBinding>,
}

impl SimulateAction {
    pub fn evm_live(&self) -> Option<&EvmSimulateLiveBinding> {
        match &self.live {
            Some(SimulateLiveBinding::Evm(live)) => Some(live),
            _ => None,
        }
    }

    pub fn solana_live(&self) -> Option<&SolanaSimulateLiveBinding> {
        match &self.live {
            Some(SimulateLiveBinding::Solana(live)) => Some(live),
            _ => None,
        }
    }
}
