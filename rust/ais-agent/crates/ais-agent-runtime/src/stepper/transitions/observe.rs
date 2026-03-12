//! Observation transition.

use ais_agent_control::recovery::{RunFailureCode, RunFailureStage};
use ais_agent_core::{
    action::{kinds::observe::ObserveLiveBinding, ActionNodeKind, ActionNodeStatus, ActionPayload},
    evidence::{EvidenceFreshness, EvidenceKind, EvidenceProvenance, EvidenceRecord},
    recovery::{classify_side_effect_phase, provider_interruption_class},
    runtime::RunPhase,
};
use ais_agent_evm::read::live::EvmAlloyReadPort;
use ais_agent_solana::read::live::SolanaLiveReadPort;
#[cfg(test)]
use ais_agent_solana::read::live::SolanaRpcReadClient;
#[cfg(test)]
use alloy::providers::Provider;

use super::{
    evm_binding::resolve_evm_observe_binding, solana_binding::resolve_solana_observe_binding,
};

use crate::{
    runtime::ActiveRun,
    stepper::{
        transitions::{dependencies_satisfied, mark_node_status},
        StepTransition, StepTransitionKind,
    },
};

pub(crate) async fn apply_observe_transition(runtime: &mut ActiveRun) -> Option<StepTransition> {
    let node_id = runtime
        .checkpoint
        .action_graph
        .nodes
        .iter()
        .find(|(_, node)| {
            node.kind == ActionNodeKind::Observe
                && matches!(
                    node.status,
                    ActionNodeStatus::Pending | ActionNodeStatus::Ready
                )
                && dependencies_satisfied(&runtime.checkpoint.action_graph, node)
        })
        .map(|(node_id, _)| node_id.clone())?;

    let node = runtime.checkpoint.action_graph.nodes.get(&node_id)?.clone();

    if let Some(binding) = resolve_evm_observe_binding(&node) {
        let ActionPayload::Observe(observe) = &node.payload else {
            return fail_observe(runtime, &node_id, "observe payload missing for evm binding");
        };
        let Some(ObserveLiveBinding::Evm(live)) = &observe.live else {
            return fail_observe(
                runtime,
                &node_id,
                "observe payload missing evm live binding",
            );
        };
        let Some(connection) = &live.connection else {
            return fail_observe(runtime, &node_id, "evm observe binding missing connection");
        };

        let payload = match EvmAlloyReadPort::new(connection.rpc_url.clone())
            .observe(&live.request)
            .await
        {
            Ok(payload) => payload,
            Err(error) => {
                return fail_observe(
                    runtime,
                    &node_id,
                    format!("evm observe {binding:?} failed: {error}"),
                );
            }
        };

        let evidence_id = observe
            .output_key
            .clone()
            .unwrap_or_else(|| format!("observe.{node_id}"));
        runtime.checkpoint.evidence_graph.records.insert(
            evidence_id.clone(),
            EvidenceRecord {
                evidence_id: evidence_id.clone(),
                kind: EvidenceKind::ExternalObservation,
                provenance: EvidenceProvenance {
                    source: "evm.alloy.live_read".to_owned(),
                    chain_scope: runtime.mission.allowed_chains.first().cloned(),
                    trace_hint: Some(node_id.clone()),
                },
                freshness: EvidenceFreshness {
                    observed_at_ms: Some(current_time_ms()),
                    expires_at_ms: None,
                    max_age_ms: None,
                },
                confidence_ppm: Some(1_000_000),
                payload,
            },
        );
        mark_node_status(runtime, node_id.as_str(), ActionNodeStatus::Succeeded);
        runtime
            .checkpoint
            .lifecycle
            .mark_running(RunPhase::Planning);
        runtime.touch_transition();

        return Some(StepTransition {
            kind: StepTransitionKind::Observe,
            node_id: Some(node_id.clone()),
            summary: format!("completed live evm observe node {node_id} -> evidence {evidence_id}"),
        });
    }

    if let Some(binding) = resolve_solana_observe_binding(&node) {
        let ActionPayload::Observe(observe) = &node.payload else {
            return fail_observe(
                runtime,
                &node_id,
                "observe payload missing for solana binding",
            );
        };
        let Some(ObserveLiveBinding::Solana(live)) = &observe.live else {
            return fail_observe(
                runtime,
                &node_id,
                "observe payload missing solana live binding",
            );
        };
        let Some(connection) = &live.connection else {
            return fail_observe(
                runtime,
                &node_id,
                "solana observe binding missing connection",
            );
        };

        let payload = match SolanaLiveReadPort::new(connection.clone())
            .observe(&live.request)
            .await
        {
            Ok(payload) => payload,
            Err(error) => {
                return fail_observe(
                    runtime,
                    &node_id,
                    format!("solana observe {binding:?} failed: {error}"),
                );
            }
        };

        let evidence_id = observe
            .output_key
            .clone()
            .unwrap_or_else(|| format!("observe.{node_id}"));
        runtime.checkpoint.evidence_graph.records.insert(
            evidence_id.clone(),
            EvidenceRecord {
                evidence_id: evidence_id.clone(),
                kind: EvidenceKind::ExternalObservation,
                provenance: EvidenceProvenance {
                    source: "solana.rpc.live_read".to_owned(),
                    chain_scope: runtime.mission.allowed_chains.first().cloned(),
                    trace_hint: Some(node_id.clone()),
                },
                freshness: EvidenceFreshness {
                    observed_at_ms: Some(current_time_ms()),
                    expires_at_ms: None,
                    max_age_ms: None,
                },
                confidence_ppm: Some(1_000_000),
                payload,
            },
        );
        mark_node_status(runtime, node_id.as_str(), ActionNodeStatus::Succeeded);
        runtime
            .checkpoint
            .lifecycle
            .mark_running(RunPhase::Planning);
        runtime.touch_transition();

        return Some(StepTransition {
            kind: StepTransitionKind::Observe,
            node_id: Some(node_id.clone()),
            summary: format!(
                "completed live solana observe node {node_id} -> evidence {evidence_id}"
            ),
        });
    }

    mark_node_status(runtime, node_id.as_str(), ActionNodeStatus::Succeeded);
    runtime
        .checkpoint
        .lifecycle
        .mark_running(RunPhase::Planning);
    runtime.touch_transition();

    Some(StepTransition {
        kind: StepTransitionKind::Observe,
        node_id: Some(node_id.clone()),
        summary: format!("completed observe node {node_id}"),
    })
}

fn fail_observe(
    runtime: &mut ActiveRun,
    node_id: &str,
    reason: impl Into<String>,
) -> Option<StepTransition> {
    let reason = reason.into();
    mark_node_status(runtime, node_id, ActionNodeStatus::Blocked);
    runtime.checkpoint.lifecycle.pause_with_failure(
        RunFailureStage::Observe,
        RunFailureCode::ProviderUnavailable,
        reason.as_str(),
    );
    if let Some(failure) = runtime.checkpoint.lifecycle.failure.as_mut() {
        failure.node_refs.push(node_id.to_owned());
        failure.provider_error = Some(ais_agent_control::recovery::ProviderFailureInfo {
            provider: "live.read".to_owned(),
            operation: "observe".to_owned(),
            code: None,
            message: reason.clone(),
            retryable: true,
        });
    }
    runtime.checkpoint.lifecycle.record_interruption(
        provider_interruption_class(&reason),
        Some(RunFailureStage::Observe),
        classify_side_effect_phase(&runtime.checkpoint),
        reason.clone(),
    );
    runtime.touch_transition();

    Some(StepTransition {
        kind: StepTransitionKind::Observe,
        node_id: Some(node_id.to_owned()),
        summary: format!("observe interrupted for node {node_id}: {reason}"),
    })
}

fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
pub(crate) async fn apply_live_evm_observe_with_provider<P>(
    runtime: &mut ActiveRun,
    provider: &P,
) -> Option<StepTransition>
where
    P: Provider,
{
    let node_id = runtime
        .checkpoint
        .action_graph
        .nodes
        .iter()
        .find(|(_, node)| {
            node.kind == ActionNodeKind::Observe
                && matches!(
                    node.status,
                    ActionNodeStatus::Pending | ActionNodeStatus::Ready
                )
                && dependencies_satisfied(&runtime.checkpoint.action_graph, node)
        })
        .map(|(node_id, _)| node_id.clone())?;

    let node = runtime.checkpoint.action_graph.nodes.get(&node_id)?.clone();
    let ActionPayload::Observe(observe) = &node.payload else {
        return fail_observe(runtime, &node_id, "observe payload missing for evm binding");
    };
    let Some(ObserveLiveBinding::Evm(live)) = &observe.live else {
        return fail_observe(
            runtime,
            &node_id,
            "observe payload missing evm live binding",
        );
    };

    let payload = match EvmAlloyReadPort::observe_with_provider(provider, &live.request).await {
        Ok(payload) => payload,
        Err(error) => {
            return fail_observe(
                runtime,
                &node_id,
                format!("evm observe via provider failed: {error}"),
            );
        }
    };

    let evidence_id = observe
        .output_key
        .clone()
        .unwrap_or_else(|| format!("observe.{node_id}"));
    runtime.checkpoint.evidence_graph.records.insert(
        evidence_id.clone(),
        EvidenceRecord {
            evidence_id: evidence_id.clone(),
            kind: EvidenceKind::ExternalObservation,
            provenance: EvidenceProvenance {
                source: "evm.alloy.live_read".to_owned(),
                chain_scope: runtime.mission.allowed_chains.first().cloned(),
                trace_hint: Some(node_id.clone()),
            },
            freshness: EvidenceFreshness {
                observed_at_ms: Some(current_time_ms()),
                expires_at_ms: None,
                max_age_ms: None,
            },
            confidence_ppm: Some(1_000_000),
            payload,
        },
    );
    mark_node_status(runtime, node_id.as_str(), ActionNodeStatus::Succeeded);
    runtime
        .checkpoint
        .lifecycle
        .mark_running(RunPhase::Planning);
    runtime.touch_transition();

    Some(StepTransition {
        kind: StepTransitionKind::Observe,
        node_id: Some(node_id.clone()),
        summary: format!("completed live evm observe node {node_id} -> evidence {evidence_id}"),
    })
}

#[cfg(test)]
pub(crate) async fn apply_live_solana_observe_with_client<C>(
    runtime: &mut ActiveRun,
    client: &C,
) -> Option<StepTransition>
where
    C: SolanaRpcReadClient,
{
    let node_id = runtime
        .checkpoint
        .action_graph
        .nodes
        .iter()
        .find(|(_, node)| {
            node.kind == ActionNodeKind::Observe
                && matches!(
                    node.status,
                    ActionNodeStatus::Pending | ActionNodeStatus::Ready
                )
                && dependencies_satisfied(&runtime.checkpoint.action_graph, node)
        })
        .map(|(node_id, _)| node_id.clone())?;

    let node = runtime.checkpoint.action_graph.nodes.get(&node_id)?.clone();
    let ActionPayload::Observe(observe) = &node.payload else {
        return fail_observe(
            runtime,
            &node_id,
            "observe payload missing for solana binding",
        );
    };
    let Some(ObserveLiveBinding::Solana(live)) = &observe.live else {
        return fail_observe(
            runtime,
            &node_id,
            "observe payload missing solana live binding",
        );
    };

    let payload = match SolanaLiveReadPort::observe_with_client(client, &live.request).await {
        Ok(payload) => payload,
        Err(error) => {
            return fail_observe(
                runtime,
                &node_id,
                format!("solana observe via client failed: {error}"),
            );
        }
    };

    let evidence_id = observe
        .output_key
        .clone()
        .unwrap_or_else(|| format!("observe.{node_id}"));
    runtime.checkpoint.evidence_graph.records.insert(
        evidence_id.clone(),
        EvidenceRecord {
            evidence_id: evidence_id.clone(),
            kind: EvidenceKind::ExternalObservation,
            provenance: EvidenceProvenance {
                source: "solana.rpc.live_read".to_owned(),
                chain_scope: runtime.mission.allowed_chains.first().cloned(),
                trace_hint: Some(node_id.clone()),
            },
            freshness: EvidenceFreshness {
                observed_at_ms: Some(current_time_ms()),
                expires_at_ms: None,
                max_age_ms: None,
            },
            confidence_ppm: Some(1_000_000),
            payload,
        },
    );
    mark_node_status(runtime, node_id.as_str(), ActionNodeStatus::Succeeded);
    runtime
        .checkpoint
        .lifecycle
        .mark_running(RunPhase::Planning);
    runtime.touch_transition();

    Some(StepTransition {
        kind: StepTransitionKind::Observe,
        node_id: Some(node_id.clone()),
        summary: format!("completed live solana observe node {node_id} -> evidence {evidence_id}"),
    })
}
