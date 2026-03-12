use crate::governor::{
    GovernorDecision, GovernorInput, GovernorRejection, GovernorRequirement, SimulationStatus,
};

#[derive(Debug, Clone)]
pub struct GovernorOutcome {
    pub decision: GovernorDecision,
    pub requirements: Vec<GovernorRequirement>,
    pub rejection: Option<GovernorRejection>,
}

pub fn decide_governor_outcome(input: &GovernorInput) -> GovernorOutcome {
    if let Some(rejection) = reject_for_budget(input) {
        return GovernorOutcome {
            decision: GovernorDecision::Reject,
            requirements: Vec::new(),
            rejection: Some(rejection),
        };
    }

    let requirements = collect_evidence_requirements(input);
    if !requirements.is_empty() {
        return GovernorOutcome {
            decision: GovernorDecision::RequireMoreEvidence,
            requirements,
            rejection: None,
        };
    }

    if let Some(rejection) = reject_for_write_contracts(input) {
        return GovernorOutcome {
            decision: GovernorDecision::Reject,
            requirements: Vec::new(),
            rejection: Some(rejection),
        };
    }

    if let Some(rejection) = reject_for_simulation(input) {
        return GovernorOutcome {
            decision: GovernorDecision::Reject,
            requirements: Vec::new(),
            rejection: Some(rejection),
        };
    }

    let decision = if input.action.is_write && input.action.requires_signer {
        GovernorDecision::AllowWithSigner
    } else {
        GovernorDecision::Allow
    };

    GovernorOutcome {
        decision,
        requirements: Vec::new(),
        rejection: None,
    }
}

fn reject_for_budget(input: &GovernorInput) -> Option<GovernorRejection> {
    if let Some(max_steps) = input.mission_budget.max_steps {
        if input.steps_executed >= max_steps {
            return Some(GovernorRejection {
                code: "budget_steps_exhausted".to_owned(),
                message: "mission step budget exhausted".to_owned(),
            });
        }
    }

    if let Some(max_signer_requests) = input.mission_budget.max_signer_requests {
        if input.action.requires_signer && input.signer.signer_requests_used >= max_signer_requests
        {
            return Some(GovernorRejection {
                code: "budget_signer_requests_exhausted".to_owned(),
                message: "mission signer-request budget exhausted".to_owned(),
            });
        }
    }

    if let Some(max_wall_clock_ms) = input.mission_budget.max_wall_clock_ms {
        if input.elapsed_wall_clock_ms > max_wall_clock_ms {
            return Some(GovernorRejection {
                code: "budget_wall_clock_exhausted".to_owned(),
                message: "mission wall-clock budget exhausted".to_owned(),
            });
        }
    }

    None
}

fn collect_evidence_requirements(input: &GovernorInput) -> Vec<GovernorRequirement> {
    input
        .evidence_requirements
        .iter()
        .map(|requirement| {
            let reason = if requirement.stale {
                format!("{} (stale evidence requires refresh)", requirement.reason)
            } else {
                requirement.reason.clone()
            };

            GovernorRequirement {
                reference: requirement.reference.clone(),
                reason,
            }
        })
        .collect()
}

fn reject_for_write_contracts(input: &GovernorInput) -> Option<GovernorRejection> {
    if input.action.is_write
        && (input.action.requires_effect_contract
            || input.mission_policy.require_effect_contract_for_writes)
        && input.effect_contract.is_none()
    {
        return Some(GovernorRejection {
            code: "missing_effect_contract".to_owned(),
            message: "write action requires an effect contract under current mission policy"
                .to_owned(),
        });
    }

    None
}

fn reject_for_simulation(input: &GovernorInput) -> Option<GovernorRejection> {
    if !input.action.is_write {
        return None;
    }

    let Some(simulation) = input.simulation.as_ref() else {
        return Some(GovernorRejection {
            code: "missing_simulation".to_owned(),
            message: "write action requires a simulation assessment".to_owned(),
        });
    };

    match simulation.status {
        SimulationStatus::Succeeded => None,
        SimulationStatus::NotRun => Some(GovernorRejection {
            code: "simulation_not_run".to_owned(),
            message: "write action cannot proceed before simulation runs".to_owned(),
        }),
        SimulationStatus::Failed => Some(GovernorRejection {
            code: "simulation_failed".to_owned(),
            message: simulation.summary.clone(),
        }),
    }
}
