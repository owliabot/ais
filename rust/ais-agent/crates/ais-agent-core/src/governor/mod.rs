//! Governor decision engine for write-safety and evidence gating.

mod decision;
mod engine;
mod input;

pub use decision::{GovernorDecision, GovernorRejection, GovernorRequirement};
pub use engine::{decide_governor_outcome, GovernorOutcome};
pub use input::{
    ActionGovernanceInput, EvidenceRequirementInput, GovernorInput, SignerBoundaryInput,
    SimulationAssessment, SimulationStatus,
};

#[cfg(test)]
mod tests;
