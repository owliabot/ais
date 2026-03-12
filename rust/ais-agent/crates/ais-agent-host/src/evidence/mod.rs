//! Host-facing evidence ingest contract.

mod submission;

#[cfg(test)]
mod tests;

pub use submission::HostEvidenceSubmission;
