//! Host session / run linkage and idempotency hooks.

mod context;
mod idempotency;
mod ids;
mod link;
mod store;

#[cfg(test)]
mod tests;

pub use context::{HostCommandEnvelope, HostedRunCommand};
pub use idempotency::{IdempotencyRecord, IdempotencyResolution};
pub use ids::{HostRequestId, HostSessionId};
pub use link::{HostRunLink, HostSessionSnapshot, SessionInspectCursor};
pub use store::{HostSessionStore, InMemoryHostSessionStore};
