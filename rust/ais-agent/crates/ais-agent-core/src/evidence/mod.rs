//! Evidence-layer domain objects.

mod freshness;
mod graph;
mod provenance;
mod record;

pub use freshness::EvidenceFreshness;
pub use graph::{EvidenceGraph, EvidenceRequirement, EvidenceUsage, EvidenceUsageKind};
pub use provenance::EvidenceProvenance;
pub use record::{EvidenceKind, EvidenceRecord};
