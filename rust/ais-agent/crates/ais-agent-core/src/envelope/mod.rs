//! Runtime envelope objects and raw-envelope binding rules.

mod gate;
mod types;

pub use gate::{bind_raw_envelope_action, ensure_raw_envelope_broadcastable, RawEnvelopeGateError};
pub use types::{RuntimeEnvelope, RuntimeEnvelopeKind};

#[cfg(test)]
mod tests;
