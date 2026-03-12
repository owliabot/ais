//! Driver registry and routing heuristics.

mod candidate;
mod capability;
mod router;

pub use candidate::DriverCandidate;
pub use capability::{DriverCapability, DriverPathKind};
pub use router::{route_driver_candidates, DriverRegistry};

#[cfg(test)]
mod tests;
