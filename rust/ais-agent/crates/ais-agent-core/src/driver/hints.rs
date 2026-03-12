use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    action::ActionNodeKind,
    binding::evm::{
        EvmActuateBinding, EvmCallRequest, EvmObserveBinding, EvmObserveRequest,
        EvmSimulateBinding, EvmVerifyBinding,
    },
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DriverEvmObserveHint {
    pub binding: EvmObserveBinding,
    pub request: EvmObserveRequest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DriverEvmSimulateHint {
    pub binding: EvmSimulateBinding,
    pub request: EvmCallRequest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DriverEvmActuateHint {
    pub binding: EvmActuateBinding,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DriverEvmVerifyHint {
    pub binding: EvmVerifyBinding,
    #[serde(default)]
    pub post_evm_request: Option<EvmObserveRequest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DriverNodeLiveBindingHint {
    EvmObserve(DriverEvmObserveHint),
    EvmSimulate(DriverEvmSimulateHint),
    EvmActuate(DriverEvmActuateHint),
    EvmVerify(DriverEvmVerifyHint),
}

impl DriverNodeLiveBindingHint {
    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::EvmObserve(_) => "evm_observe",
            Self::EvmSimulate(_) => "evm_simulate",
            Self::EvmActuate(_) => "evm_actuate",
            Self::EvmVerify(_) => "evm_verify",
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DriverFragmentBindingError {
    #[error("driver fragment live-binding hint references missing node `{node_id}`")]
    NodeNotFound { node_id: String },
    #[error(
        "driver fragment live-binding hint `{hint_kind}` does not match node `{node_id}` of kind `{node_kind:?}`"
    )]
    KindMismatch {
        node_id: String,
        node_kind: ActionNodeKind,
        hint_kind: String,
    },
}
