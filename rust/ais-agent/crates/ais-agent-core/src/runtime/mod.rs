//! Runtime lifecycle and stable-boundary state.

mod boundaries;
mod lifecycle;
mod phases;
mod signer;

pub use boundaries::{BoundaryKind, StableBoundary};
pub use lifecycle::{RunLifecycleState, RunStatus, RuntimeFailure};
pub use phases::RunPhase;
pub use signer::{
    SignerRequestState, SignerRequestStatus, SignerResolution, SignerResolutionKind, SignerTimeout,
};

#[cfg(test)]
mod tests;
