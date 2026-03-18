//! Host-facing signer request and resolution contract.

mod request;
mod resolution;

pub use request::{HostSignerRequest, HostSignerTimeoutPolicy};
pub use resolution::{HostSignerResolution, HostSignerResolutionKind};

#[cfg(test)]
mod tests;
