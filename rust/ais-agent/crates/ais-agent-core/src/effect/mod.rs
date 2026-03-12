//! Effect-contract domain objects.

mod contract;
mod delta;
mod verifier;

pub use contract::{EffectAssertion, EffectContract, EffectContractKind};
pub use delta::{EffectDelta, EffectDeltaStatus};
pub use verifier::{verify_effect_contract, EffectObservationBundle, EffectVerificationResult};

#[cfg(test)]
mod tests;
