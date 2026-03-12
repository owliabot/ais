//! Host ingest application transition.

use ais_agent_core::{
    evidence::{EvidenceUsage, EvidenceUsageKind},
    runtime::RunPhase,
};

use crate::{
    runtime::ActiveRun,
    stepper::{StepTransition, StepTransitionKind},
};

pub(crate) fn apply_ingest_transition(runtime: &mut ActiveRun) -> Option<StepTransition> {
    let requirement_index = runtime
        .checkpoint
        .evidence_graph
        .requirements
        .iter()
        .position(|requirement| requirement.satisfied_by_evidence_id.is_none())?;

    let reference = runtime.checkpoint.evidence_graph.requirements[requirement_index]
        .reference
        .clone();
    let evidence_id = runtime
        .checkpoint
        .evidence_graph
        .records
        .keys()
        .find(|evidence_id| matches_reference(reference.as_str(), evidence_id.as_str()))?
        .clone();

    let required_by_node_id = {
        let requirement = &mut runtime.checkpoint.evidence_graph.requirements[requirement_index];
        requirement.satisfied_by_evidence_id = Some(evidence_id.clone());
        requirement.required_by_node_id.clone()
    };

    if let Some(node_id) = required_by_node_id {
        runtime
            .checkpoint
            .evidence_graph
            .usages
            .push(EvidenceUsage {
                evidence_id: evidence_id.clone(),
                node_id,
                kind: EvidenceUsageKind::SatisfiedRequirement,
                detail: Some(reference.clone()),
            });
    }

    runtime.checkpoint.pending_requests.pending_evidence_refs = runtime
        .checkpoint
        .evidence_graph
        .requirements
        .iter()
        .filter(|requirement| requirement.satisfied_by_evidence_id.is_none())
        .map(|requirement| requirement.reference.clone())
        .collect();

    if runtime
        .checkpoint
        .pending_requests
        .pending_evidence_refs
        .is_empty()
        && matches!(
            runtime.checkpoint.lifecycle.status,
            ais_agent_core::runtime::RunStatus::AwaitingEvidence
        )
    {
        runtime
            .checkpoint
            .lifecycle
            .mark_running(RunPhase::Planning);
    }

    runtime.touch_transition();

    Some(StepTransition {
        kind: StepTransitionKind::Ingest,
        node_id: None,
        summary: format!("ingested evidence for {reference} via {evidence_id}"),
    })
}

fn matches_reference(reference: &str, evidence_id: &str) -> bool {
    reference == evidence_id
        || reference
            .strip_prefix("evidence.")
            .is_some_and(|trimmed| trimmed == evidence_id)
}
