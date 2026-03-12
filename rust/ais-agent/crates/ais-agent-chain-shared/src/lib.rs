//! Shared chain-layer contracts for the greenfield `ais-agent`.

mod capabilities;
mod chain_id;
mod confirmations;
mod error;
mod family;
mod io;
pub mod reflect;
mod traits;

pub use capabilities::{CapabilityKind, ChainCapability};
pub use chain_id::ChainId;
pub use confirmations::{ConfirmationDepth, FinalityLevel};
pub use error::ChainCapabilityError;
pub use family::ChainFamily;
pub use io::{
    BroadcastRequest, BroadcastResponse, ReadRequest, ReadResponse, ReceiptQuery, ReceiptView,
    SimulationRequest, SimulationResponse, StateQuery, StateView,
};
pub use reflect::{
    ReflectionArtifactKind, ReflectionDriver, ReflectionDriverError, ReflectionDriverOutput,
    ReflectionRequest,
};
pub use traits::{
    BroadcastCapability, ChainFamilySurface, ReadCapability, ReceiptCapability,
    SimulationCapability, StateCapability,
};
