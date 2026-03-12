//! Shared capability descriptors.

use serde::{Deserialize, Serialize};

use crate::ChainFamily;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityKind {
    Read,
    Simulate,
    Broadcast,
    Receipt,
    State,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChainCapability {
    pub family: ChainFamily,
    pub kind: CapabilityKind,
    pub implementation: &'static str,
}
