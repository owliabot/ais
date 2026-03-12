use serde::{Deserialize, Serialize};

use crate::{
    action::kinds::actuate::ActuateMode,
    effect::EffectContract,
    mission::{MissionBudget, MissionPolicy},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionGovernanceInput {
    pub action_id: String,
    pub mode: Option<ActuateMode>,
    pub is_write: bool,
    pub requires_signer: bool,
    pub requires_effect_contract: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceRequirementInput {
    pub reference: String,
    pub reason: String,
    pub stale: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SimulationStatus {
    NotRun,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimulationAssessment {
    pub status: SimulationStatus,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SignerBoundaryInput {
    pub signer_requests_used: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GovernorInput {
    pub mission_budget: MissionBudget,
    pub mission_policy: MissionPolicy,
    pub action: ActionGovernanceInput,
    #[serde(default)]
    pub evidence_requirements: Vec<EvidenceRequirementInput>,
    pub simulation: Option<SimulationAssessment>,
    pub effect_contract: Option<EffectContract>,
    pub signer: SignerBoundaryInput,
    pub elapsed_wall_clock_ms: u64,
    pub steps_executed: u32,
}
