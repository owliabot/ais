//! Unified host ingest surface for runtime re-entry.

mod submission;

#[cfg(test)]
mod tests;

pub use submission::{HostIngestKind, HostIngestSubmission};
