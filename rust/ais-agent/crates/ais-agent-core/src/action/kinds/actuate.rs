use crate::binding::{
    evm::{EvmActuateBinding, EvmConnectionSpec},
    solana::{SolanaActuateBinding, SolanaConnectionSpec},
};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActuateMode {
    DriverCall,
    ReflectedCall,
    ApiNativeEnvelope,
    RawEnvelope,
    ExternalJob,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvmActuateLiveBinding {
    #[serde(default)]
    pub connection: Option<EvmConnectionSpec>,
    pub binding: EvmActuateBinding,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SolanaActuateLiveBinding {
    #[serde(default)]
    pub connection: Option<SolanaConnectionSpec>,
    pub binding: SolanaActuateBinding,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "family", rename_all = "snake_case")]
pub enum ActuateLiveBinding {
    Evm(EvmActuateLiveBinding),
    Solana(SolanaActuateLiveBinding),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActuateAction {
    pub mode: ActuateMode,
    pub actuator_hint: String,
    pub chain: Option<String>,
    pub envelope_ref: Option<String>,
    pub requires_effect_contract: bool,
    #[serde(default)]
    pub live: Option<ActuateLiveBinding>,
}

impl ActuateAction {
    pub fn evm_live(&self) -> Option<&EvmActuateLiveBinding> {
        match &self.live {
            Some(ActuateLiveBinding::Evm(live)) => Some(live),
            _ => None,
        }
    }

    pub fn solana_live(&self) -> Option<&SolanaActuateLiveBinding> {
        match &self.live {
            Some(ActuateLiveBinding::Solana(live)) => Some(live),
            _ => None,
        }
    }
}
