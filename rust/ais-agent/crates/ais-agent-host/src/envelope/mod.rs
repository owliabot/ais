//! Host-facing raw envelope contract.

mod submission;

pub use submission::{HostEnvelopeKind, HostEnvelopeSubmission};

#[cfg(test)]
mod tests;
