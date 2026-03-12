use crate::{
    action::kinds::actuate::ActuateMode,
    effect::{EffectAssertion, EffectContract, EffectContractKind},
    governor::{
        decide_governor_outcome, ActionGovernanceInput, EvidenceRequirementInput, GovernorDecision,
        GovernorInput, SignerBoundaryInput, SimulationAssessment, SimulationStatus,
    },
    mission::{MissionBudget, MissionPolicy},
};

#[test]
fn governor_requires_more_evidence_when_any_requirement_is_missing_or_stale() {
    let outcome = decide_governor_outcome(&GovernorInput {
        mission_budget: MissionBudget::default(),
        mission_policy: MissionPolicy::default(),
        action: sample_write_action(true),
        evidence_requirements: vec![EvidenceRequirementInput {
            reference: "evidence.quote".to_owned(),
            reason: "best route quote required".to_owned(),
            stale: true,
        }],
        simulation: Some(successful_simulation()),
        effect_contract: Some(sample_effect_contract()),
        signer: SignerBoundaryInput::default(),
        elapsed_wall_clock_ms: 0,
        steps_executed: 0,
    });

    assert_eq!(outcome.decision, GovernorDecision::RequireMoreEvidence);
    assert_eq!(outcome.requirements.len(), 1);
    assert!(outcome.requirements[0].reason.contains("stale"));
    assert!(outcome.rejection.is_none());
}

#[test]
fn governor_rejects_write_without_effect_contract_when_policy_requires_it() {
    let outcome = decide_governor_outcome(&GovernorInput {
        mission_budget: MissionBudget::default(),
        mission_policy: MissionPolicy {
            require_effect_contract_for_writes: true,
            ..MissionPolicy::default()
        },
        action: sample_write_action(false),
        evidence_requirements: Vec::new(),
        simulation: Some(successful_simulation()),
        effect_contract: None,
        signer: SignerBoundaryInput::default(),
        elapsed_wall_clock_ms: 0,
        steps_executed: 0,
    });

    assert_eq!(outcome.decision, GovernorDecision::Reject);
    assert_eq!(
        outcome
            .rejection
            .as_ref()
            .map(|rejection| rejection.code.as_str()),
        Some("missing_effect_contract")
    );
}

#[test]
fn governor_rejects_write_when_simulation_failed() {
    let outcome = decide_governor_outcome(&GovernorInput {
        mission_budget: MissionBudget::default(),
        mission_policy: MissionPolicy::default(),
        action: sample_write_action(false),
        evidence_requirements: Vec::new(),
        simulation: Some(SimulationAssessment {
            status: SimulationStatus::Failed,
            summary: "slippage would exceed mission bound".to_owned(),
        }),
        effect_contract: Some(sample_effect_contract()),
        signer: SignerBoundaryInput::default(),
        elapsed_wall_clock_ms: 0,
        steps_executed: 0,
    });

    assert_eq!(outcome.decision, GovernorDecision::Reject);
    assert_eq!(
        outcome
            .rejection
            .as_ref()
            .map(|rejection| rejection.code.as_str()),
        Some("simulation_failed")
    );
}

#[test]
fn governor_allows_with_signer_for_safe_write_after_checks_pass() {
    let outcome = decide_governor_outcome(&GovernorInput {
        mission_budget: MissionBudget {
            max_signer_requests: Some(3),
            ..MissionBudget::default()
        },
        mission_policy: MissionPolicy::default(),
        action: sample_write_action(true),
        evidence_requirements: Vec::new(),
        simulation: Some(successful_simulation()),
        effect_contract: Some(sample_effect_contract()),
        signer: SignerBoundaryInput {
            signer_requests_used: 1,
        },
        elapsed_wall_clock_ms: 200,
        steps_executed: 1,
    });

    assert_eq!(outcome.decision, GovernorDecision::AllowWithSigner);
    assert!(outcome.rejection.is_none());
}

#[test]
fn governor_rejects_when_step_budget_is_exhausted() {
    let outcome = decide_governor_outcome(&GovernorInput {
        mission_budget: MissionBudget {
            max_steps: Some(2),
            ..MissionBudget::default()
        },
        mission_policy: MissionPolicy::default(),
        action: sample_write_action(false),
        evidence_requirements: Vec::new(),
        simulation: Some(successful_simulation()),
        effect_contract: Some(sample_effect_contract()),
        signer: SignerBoundaryInput::default(),
        elapsed_wall_clock_ms: 0,
        steps_executed: 2,
    });

    assert_eq!(outcome.decision, GovernorDecision::Reject);
    assert_eq!(
        outcome
            .rejection
            .as_ref()
            .map(|rejection| rejection.code.as_str()),
        Some("budget_steps_exhausted")
    );
}

fn sample_write_action(requires_signer: bool) -> ActionGovernanceInput {
    ActionGovernanceInput {
        action_id: "swap".to_owned(),
        mode: Some(ActuateMode::RawEnvelope),
        is_write: true,
        requires_signer,
        requires_effect_contract: true,
    }
}

fn successful_simulation() -> SimulationAssessment {
    SimulationAssessment {
        status: SimulationStatus::Succeeded,
        summary: "simulation succeeded".to_owned(),
    }
}

fn sample_effect_contract() -> EffectContract {
    EffectContract {
        effect_id: "effect-1".to_owned(),
        kind: EffectContractKind::AssetDelta,
        assertions: vec![EffectAssertion {
            expression: "post.usdc < pre.usdc".to_owned(),
            description: "spend input asset".to_owned(),
        }],
        tolerance_hint: Some("tight".to_owned()),
    }
}
