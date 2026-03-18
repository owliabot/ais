//! Governor application transition.

use alloy::hex;
use serde_json::{json, Value};

use ais_agent_control::{
    execution_artifact::ExecutionChainFamily,
    ids::SignerRequestId,
    recovery::{RunFailureCode, RunFailureStage},
};
use ais_agent_core::{
    action::{kinds::actuate::ActuateMode, ActionNodeKind, ActionNodeStatus},
    actuation::ActuationKind,
    binding::evm::EvmCallRequest,
    checkpoint::{PendingSignerRequestSnapshot, PendingSignerTimeoutSnapshot},
    governor::{
        decide_governor_outcome, ActionGovernanceInput, EvidenceRequirementInput, GovernorDecision,
        GovernorInput, SignerBoundaryInput, SimulationAssessment, SimulationStatus,
    },
    runtime::{RunPhase, SignerRequestState},
};

use crate::{
    runtime::ActiveRun,
    stepper::{
        transitions::{add_actuation_record, dependencies_satisfied, mark_node_status},
        StepTransition, StepTransitionKind,
    },
};

pub(crate) fn apply_govern_transition(runtime: &mut ActiveRun) -> Option<StepTransition> {
    if runtime.pending_signer_state.is_some() {
        return None;
    }

    let node_id = runtime
        .checkpoint
        .action_graph
        .nodes
        .iter()
        .find(|(_, node)| {
            node.kind == ActionNodeKind::Actuate
                && matches!(
                    node.status,
                    ActionNodeStatus::Pending | ActionNodeStatus::Blocked
                )
                && dependencies_satisfied(&runtime.checkpoint.action_graph, node)
        })
        .map(|(node_id, _)| node_id.clone())?;

    let Some(node) = runtime.checkpoint.action_graph.nodes.get(node_id.as_str()) else {
        return None;
    };

    let (mode, chain, requires_effect_contract, actuator_hint) = match &node.payload {
        ais_agent_core::action::ActionPayload::Actuate(actuate) => (
            Some(actuate.mode.clone()),
            actuate.chain.clone(),
            actuate.requires_effect_contract,
            Some(actuate.actuator_hint.clone()),
        ),
        _ => return None,
    };

    let outcome = decide_governor_outcome(&GovernorInput {
        mission_budget: runtime.mission.budget.clone(),
        mission_policy: runtime.mission.policy.clone(),
        action: ActionGovernanceInput {
            action_id: node_id.clone(),
            mode: mode.clone(),
            is_write: true,
            requires_signer: requires_signer(mode.as_ref()),
            requires_effect_contract,
        },
        evidence_requirements: node
            .evidence_refs
            .iter()
            .filter_map(|reference| governor_requirement_input(runtime, reference))
            .collect(),
        simulation: Some(simulation_assessment_for(runtime, node)),
        effect_contract: node.expected_effect_ref.as_ref().map(|effect_id| {
            ais_agent_core::effect::EffectContract {
                effect_id: effect_id.clone(),
                kind: ais_agent_core::effect::EffectContractKind::StateTransition,
                assertions: Vec::new(),
                tolerance_hint: None,
            }
        }),
        signer: SignerBoundaryInput {
            signer_requests_used: runtime
                .checkpoint
                .actuation_records
                .iter()
                .filter(|record| record.kind == ActuationKind::SignerRequested)
                .count() as u32,
        },
        elapsed_wall_clock_ms: runtime.last_updated_at_ms.unwrap_or_default(),
        steps_executed: runtime.checkpoint.checkpoint_seq as u32,
    });

    match outcome.decision {
        GovernorDecision::Allow => {
            mark_node_status(runtime, node_id.as_str(), ActionNodeStatus::Ready);
            runtime
                .checkpoint
                .lifecycle
                .mark_running(RunPhase::Governing);
            runtime.touch_transition();
            Some(StepTransition {
                kind: StepTransitionKind::Govern,
                node_id: Some(node_id.clone()),
                summary: format!("governor allowed node {node_id}"),
            })
        }
        GovernorDecision::AllowWithSigner => {
            let request_id = SignerRequestId(format!(
                "{}:signer:{}",
                runtime.run_id.0,
                runtime.event_seq.saturating_add(1)
            ));
            let signer_payload = build_signer_request_payload(runtime, node);
            let request = SignerRequestState::new_pending(
                request_id.clone(),
                runtime.run_id.clone(),
                chain.unwrap_or_else(|| "unknown".to_owned()),
                actuator_hint.unwrap_or_else(|| format!("sign action {node_id}")),
            )
            .with_node_id(node_id.clone());
            let request = match signer_payload.clone() {
                Some(payload) => request.with_payload(payload),
                None => request,
            };

            runtime
                .checkpoint
                .pending_requests
                .pending_signer_request_id = Some(request_id.0.clone());
            runtime.checkpoint.pending_requests.pending_signer_request =
                Some(PendingSignerRequestSnapshot {
                    request_id: request_id.0.clone(),
                    node_id: Some(node_id.clone()),
                    chain: Some(request.chain.clone()),
                    summary: request.summary.clone(),
                    payload: signer_payload,
                    timeout_policy: request.timeout.as_ref().map(|timeout| {
                        PendingSignerTimeoutSnapshot {
                            requested_at_ms: timeout.requested_at_ms,
                            expires_at_ms: timeout.expires_at_ms,
                        }
                    }),
                });
            runtime.checkpoint.lifecycle.await_signer_request(&request);
            runtime.pending_signer_state = Some(request);
            mark_node_status(runtime, node_id.as_str(), ActionNodeStatus::Blocked);
            add_actuation_record(
                runtime,
                node_id.as_str(),
                ActuationKind::SignerRequested,
                None,
                None,
                format!("signer requested for {node_id}"),
            );
            runtime.touch_transition();
            Some(StepTransition {
                kind: StepTransitionKind::Govern,
                node_id: Some(node_id.clone()),
                summary: format!("governor paused at signer boundary for {node_id}"),
            })
        }
        GovernorDecision::RequireMoreEvidence => {
            let blocking_refs: Vec<String> = outcome
                .requirements
                .iter()
                .map(|requirement| requirement.reference.clone())
                .collect();
            let failure_code = classify_evidence_failure_code(runtime, &blocking_refs);
            runtime.checkpoint.pending_requests.pending_evidence_refs = blocking_refs.clone();
            runtime.checkpoint.lifecycle.await_evidence_with_failure(
                RunFailureStage::Govern,
                failure_code,
                format!("node {node_id} requires additional evidence"),
                blocking_refs,
            );
            mark_node_status(runtime, node_id.as_str(), ActionNodeStatus::Blocked);
            if let Some(failure) = runtime.checkpoint.lifecycle.failure.as_mut() {
                failure.node_refs.push(node_id.clone());
                failure.evidence_refs = runtime
                    .checkpoint
                    .pending_requests
                    .pending_evidence_refs
                    .clone();
            }
            runtime.touch_transition();
            Some(StepTransition {
                kind: StepTransitionKind::Govern,
                node_id: Some(node_id.clone()),
                summary: format!("governor requires more evidence for {node_id}"),
            })
        }
        GovernorDecision::Reject => {
            mark_node_status(runtime, node_id.as_str(), ActionNodeStatus::Failed);
            let rejection =
                outcome
                    .rejection
                    .unwrap_or(ais_agent_core::governor::GovernorRejection {
                        code: "governor_rejected".to_owned(),
                        message: format!("governor rejected node {node_id}"),
                    });
            runtime.checkpoint.lifecycle.pause_with_failure(
                RunFailureStage::Govern,
                map_governor_rejection_code(rejection.code.as_str()),
                rejection.message.clone(),
            );
            if let Some(failure) = runtime.checkpoint.lifecycle.failure.as_mut() {
                failure.node_refs.push(node_id.clone());
                failure.governor_decision_ref = Some(format!(
                    "{}:governor:{}",
                    runtime.run_id.0, runtime.checkpoint.lifecycle.checkpoint_seq
                ));
            }
            runtime.touch_transition();
            Some(StepTransition {
                kind: StepTransitionKind::Govern,
                node_id: Some(node_id.clone()),
                summary: format!("governor rejected node {node_id}: {}", rejection.message),
            })
        }
    }
}

fn build_signer_request_payload(
    runtime: &ActiveRun,
    node: &ais_agent_core::action::ActionNode,
) -> Option<Value> {
    let ais_agent_core::action::ActionPayload::Actuate(actuate) = &node.payload else {
        return None;
    };

    if let Some(envelope_ref) = actuate.envelope_ref.as_ref() {
        if let Some(envelope) = runtime.envelopes.get(envelope_ref) {
            return Some(json!({
                "kind": match envelope.kind {
                    ais_agent_core::envelope::RuntimeEnvelopeKind::EvmEnvelope => "evm_envelope",
                    ais_agent_core::envelope::RuntimeEnvelopeKind::SolanaEnvelope => "solana_envelope",
                    ais_agent_core::envelope::RuntimeEnvelopeKind::ExternalJob => "external_job",
                },
                "chain": envelope.chain,
                "envelope_ref": envelope.envelope_id,
                "payload": envelope.payload,
            }));
        }
    }

    find_evm_simulate_request(runtime, node).map(|request| {
        json!({
            "kind": "evm_transaction_request",
            "chain_family": ExecutionChainFamily::Evm,
            "chain": actuate.chain,
            "from": request.from.map(|from| format!("{from:#x}")),
            "to": format!("{:#x}", request.to),
            "data": format!("0x{}", hex::encode(request.data.as_ref())),
            "value": request.value.map(|value| value.to_string()),
        })
    })
}

fn find_evm_simulate_request(
    runtime: &ActiveRun,
    node: &ais_agent_core::action::ActionNode,
) -> Option<EvmCallRequest> {
    node.depends_on.iter().find_map(|dependency_id| {
        let dependency = runtime.checkpoint.action_graph.nodes.get(dependency_id)?;
        let ais_agent_core::action::ActionPayload::Simulate(simulate) = &dependency.payload else {
            return None;
        };
        let live = simulate.evm_live()?;
        Some(live.request.clone())
    })
}

fn map_governor_rejection_code(code: &str) -> RunFailureCode {
    match code {
        "budget_steps_exhausted"
        | "budget_signer_requests_exhausted"
        | "budget_wall_clock_exhausted" => RunFailureCode::BudgetExhausted,
        _ => RunFailureCode::GovernorDenied,
    }
}

fn requires_signer(mode: Option<&ActuateMode>) -> bool {
    matches!(
        mode,
        Some(
            ActuateMode::DriverCall
                | ActuateMode::ReflectedCall
                | ActuateMode::ApiNativeEnvelope
                | ActuateMode::RawEnvelope
        )
    )
}

fn governor_requirement_input(
    runtime: &ActiveRun,
    reference: &str,
) -> Option<EvidenceRequirementInput> {
    let requirement = runtime
        .checkpoint
        .evidence_graph
        .requirements
        .iter()
        .find(|requirement| requirement.reference == reference)?;
    let stale = requirement
        .satisfied_by_evidence_id
        .as_ref()
        .and_then(|evidence_id| runtime.checkpoint.evidence_graph.records.get(evidence_id))
        .is_some_and(|record| record.freshness.is_stale_at(current_time_ms()));

    if requirement.satisfied_by_evidence_id.is_some() && !stale {
        return None;
    }

    Some(EvidenceRequirementInput {
        reference: requirement.reference.clone(),
        reason: requirement.reason.clone(),
        stale,
    })
}

fn classify_evidence_failure_code(runtime: &ActiveRun, blocking_refs: &[String]) -> RunFailureCode {
    if blocking_refs.iter().any(|reference| {
        runtime
            .checkpoint
            .evidence_graph
            .requirements
            .iter()
            .find(|requirement| requirement.reference == *reference)
            .and_then(|requirement| requirement.satisfied_by_evidence_id.as_ref())
            .and_then(|evidence_id| runtime.checkpoint.evidence_graph.records.get(evidence_id))
            .is_some_and(|record| record.freshness.is_stale_at(current_time_ms()))
    }) {
        RunFailureCode::StaleEvidence
    } else {
        RunFailureCode::MissingEvidence
    }
}

fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn simulation_assessment_for(
    runtime: &ActiveRun,
    node: &ais_agent_core::action::ActionNode,
) -> SimulationAssessment {
    let dependency_status = node
        .depends_on
        .iter()
        .filter_map(|dependency_id| runtime.checkpoint.action_graph.nodes.get(dependency_id))
        .find(|dependency| dependency.kind == ActionNodeKind::Simulate)
        .map(|dependency| dependency.status.clone());

    match dependency_status {
        Some(ActionNodeStatus::Succeeded) => SimulationAssessment {
            status: SimulationStatus::Succeeded,
            summary: "simulation completed".to_owned(),
        },
        Some(ActionNodeStatus::Failed) => SimulationAssessment {
            status: SimulationStatus::Failed,
            summary: "simulation failed".to_owned(),
        },
        _ => SimulationAssessment {
            status: SimulationStatus::NotRun,
            summary: "simulation has not run".to_owned(),
        },
    }
}
