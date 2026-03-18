//! Simulation transition.

use ais_agent_control::recovery::{RunFailureCode, RunFailureStage};
use ais_agent_core::{
    action::{
        kinds::simulate::SimulateLiveBinding, ActionNodeKind, ActionNodeStatus, ActionPayload,
    },
    evidence::{EvidenceFreshness, EvidenceKind, EvidenceProvenance, EvidenceRecord},
    runtime::RunPhase,
};
use ais_agent_evm::simulate::live::EvmAlloySimulatePort;
use ais_agent_solana::simulate::live::SolanaLiveSimulatePort;
#[cfg(test)]
use ais_agent_solana::simulate::live::SolanaRpcSimulateClient;
#[cfg(test)]
use alloy::providers::Provider;

use super::{
    evm_binding::resolve_evm_simulate_binding, solana_binding::resolve_solana_simulate_binding,
};

use crate::{
    runtime::ActiveRun,
    stepper::{
        transitions::{dependencies_satisfied, mark_node_status},
        StepTransition, StepTransitionKind,
    },
};

pub(crate) async fn apply_simulate_transition(runtime: &mut ActiveRun) -> Option<StepTransition> {
    let node_id = runtime
        .checkpoint
        .action_graph
        .nodes
        .iter()
        .find(|(_, node)| {
            node.kind == ActionNodeKind::Simulate
                && matches!(
                    node.status,
                    ActionNodeStatus::Pending | ActionNodeStatus::Ready
                )
                && dependencies_satisfied(&runtime.checkpoint.action_graph, node)
        })
        .map(|(node_id, _)| node_id.clone())?;

    let node = runtime.checkpoint.action_graph.nodes.get(&node_id)?.clone();

    if let Some(binding) = resolve_evm_simulate_binding(&node) {
        let ActionPayload::Simulate(simulate) = &node.payload else {
            return fail_simulate(
                runtime,
                &node_id,
                "simulate payload missing for evm binding",
            );
        };
        let Some(SimulateLiveBinding::Evm(live)) = &simulate.live else {
            return fail_simulate(
                runtime,
                &node_id,
                "simulate payload missing evm live binding",
            );
        };
        let Some(connection) = &live.connection else {
            return fail_simulate(runtime, &node_id, "evm simulate binding missing connection");
        };

        let report = match EvmAlloySimulatePort::new(connection.http_url.clone())
            .eth_call(&live.request)
            .await
        {
            Ok(report) => report,
            Err(error) => {
                return fail_simulate(
                    runtime,
                    &node_id,
                    format!("evm simulate {binding:?} failed: {error}"),
                );
            }
        };

        let evidence_id = format!("simulation.{node_id}");
        runtime.checkpoint.evidence_graph.records.insert(
            evidence_id.clone(),
            EvidenceRecord {
                evidence_id,
                kind: EvidenceKind::ExternalObservation,
                provenance: EvidenceProvenance {
                    source: "evm.alloy.simulate".to_owned(),
                    chain_scope: runtime.mission.allowed_chains.first().cloned(),
                    trace_hint: Some(node_id.clone()),
                },
                freshness: EvidenceFreshness {
                    observed_at_ms: Some(current_time_ms()),
                    expires_at_ms: None,
                    max_age_ms: None,
                },
                confidence_ppm: Some(1_000_000),
                payload: EvmAlloySimulatePort::report_payload(&report),
            },
        );

        if !report.accepted {
            return fail_simulate(runtime, &node_id, "evm simulate report rejected request");
        }

        mark_node_status(runtime, node_id.as_str(), ActionNodeStatus::Succeeded);
        runtime
            .checkpoint
            .lifecycle
            .mark_running(RunPhase::Simulating);
        runtime.touch_transition();

        return Some(StepTransition {
            kind: StepTransitionKind::Simulate,
            node_id: Some(node_id.clone()),
            summary: format!("completed live evm simulate node {node_id}"),
        });
    }

    if let Some(binding) = resolve_solana_simulate_binding(&node) {
        let ActionPayload::Simulate(simulate) = &node.payload else {
            return fail_simulate(
                runtime,
                &node_id,
                "simulate payload missing for solana binding",
            );
        };
        let Some(SimulateLiveBinding::Solana(live)) = &simulate.live else {
            return fail_simulate(
                runtime,
                &node_id,
                "simulate payload missing solana live binding",
            );
        };
        let Some(connection) = &live.connection else {
            return fail_simulate(
                runtime,
                &node_id,
                "solana simulate binding missing connection",
            );
        };

        let report = match SolanaLiveSimulatePort::new(connection.clone())
            .simulate_transaction(&live.request)
            .await
        {
            Ok(report) => report,
            Err(error) => {
                return fail_simulate(
                    runtime,
                    &node_id,
                    format!("solana simulate {binding:?} failed: {error}"),
                );
            }
        };

        let evidence_id = format!("simulation.{node_id}");
        runtime.checkpoint.evidence_graph.records.insert(
            evidence_id.clone(),
            EvidenceRecord {
                evidence_id,
                kind: EvidenceKind::ExternalObservation,
                provenance: EvidenceProvenance {
                    source: "solana.rpc.simulate".to_owned(),
                    chain_scope: runtime.mission.allowed_chains.first().cloned(),
                    trace_hint: Some(node_id.clone()),
                },
                freshness: EvidenceFreshness {
                    observed_at_ms: Some(current_time_ms()),
                    expires_at_ms: None,
                    max_age_ms: None,
                },
                confidence_ppm: Some(1_000_000),
                payload: SolanaLiveSimulatePort::report_payload(&report),
            },
        );

        if !report.accepted {
            return fail_simulate(runtime, &node_id, "solana simulate report rejected request");
        }

        mark_node_status(runtime, node_id.as_str(), ActionNodeStatus::Succeeded);
        runtime
            .checkpoint
            .lifecycle
            .mark_running(RunPhase::Simulating);
        runtime.touch_transition();

        return Some(StepTransition {
            kind: StepTransitionKind::Simulate,
            node_id: Some(node_id.clone()),
            summary: format!("completed live solana simulate node {node_id}"),
        });
    }

    mark_node_status(runtime, node_id.as_str(), ActionNodeStatus::Succeeded);
    runtime
        .checkpoint
        .lifecycle
        .mark_running(RunPhase::Simulating);
    runtime.touch_transition();

    Some(StepTransition {
        kind: StepTransitionKind::Simulate,
        node_id: Some(node_id.clone()),
        summary: format!("completed simulate node {node_id}"),
    })
}

fn fail_simulate(
    runtime: &mut ActiveRun,
    node_id: &str,
    reason: impl Into<String>,
) -> Option<StepTransition> {
    let reason = reason.into();
    mark_node_status(runtime, node_id, ActionNodeStatus::Failed);
    runtime.checkpoint.lifecycle.pause_with_failure(
        RunFailureStage::Simulate,
        RunFailureCode::SimulationRejected,
        reason.as_str(),
    );
    if let Some(failure) = runtime.checkpoint.lifecycle.failure.as_mut() {
        failure.node_refs.push(node_id.to_owned());
    }
    runtime.touch_transition();

    Some(StepTransition {
        kind: StepTransitionKind::Simulate,
        node_id: Some(node_id.to_owned()),
        summary: format!("failed simulate node {node_id}: {reason}"),
    })
}

fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
pub(crate) async fn apply_live_evm_simulate_with_provider<P>(
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
            node.kind == ActionNodeKind::Simulate
                && matches!(
                    node.status,
                    ActionNodeStatus::Pending | ActionNodeStatus::Ready
                )
                && dependencies_satisfied(&runtime.checkpoint.action_graph, node)
        })
        .map(|(node_id, _)| node_id.clone())?;

    let node = runtime.checkpoint.action_graph.nodes.get(&node_id)?.clone();
    let ActionPayload::Simulate(simulate) = &node.payload else {
        return fail_simulate(
            runtime,
            &node_id,
            "simulate payload missing for evm binding",
        );
    };
    let Some(SimulateLiveBinding::Evm(live)) = &simulate.live else {
        return fail_simulate(
            runtime,
            &node_id,
            "simulate payload missing evm live binding",
        );
    };

    let report = match EvmAlloySimulatePort::eth_call_with_provider(provider, &live.request).await {
        Ok(report) => report,
        Err(error) => {
            return fail_simulate(
                runtime,
                &node_id,
                format!("evm simulate via provider failed: {error}"),
            );
        }
    };

    let evidence_id = format!("simulation.{node_id}");
    runtime.checkpoint.evidence_graph.records.insert(
        evidence_id.clone(),
        EvidenceRecord {
            evidence_id,
            kind: EvidenceKind::ExternalObservation,
            provenance: EvidenceProvenance {
                source: "evm.alloy.simulate".to_owned(),
                chain_scope: runtime.mission.allowed_chains.first().cloned(),
                trace_hint: Some(node_id.clone()),
            },
            freshness: EvidenceFreshness {
                observed_at_ms: Some(current_time_ms()),
                expires_at_ms: None,
                max_age_ms: None,
            },
            confidence_ppm: Some(1_000_000),
            payload: EvmAlloySimulatePort::report_payload(&report),
        },
    );

    if !report.accepted {
        return fail_simulate(runtime, &node_id, "evm simulate report rejected request");
    }

    mark_node_status(runtime, node_id.as_str(), ActionNodeStatus::Succeeded);
    runtime
        .checkpoint
        .lifecycle
        .mark_running(RunPhase::Simulating);
    runtime.touch_transition();

    Some(StepTransition {
        kind: StepTransitionKind::Simulate,
        node_id: Some(node_id.clone()),
        summary: format!("completed live evm simulate node {node_id}"),
    })
}

#[cfg(test)]
pub(crate) async fn apply_live_solana_simulate_with_client<C>(
    runtime: &mut ActiveRun,
    client: &C,
) -> Option<StepTransition>
where
    C: SolanaRpcSimulateClient,
{
    let node_id = runtime
        .checkpoint
        .action_graph
        .nodes
        .iter()
        .find(|(_, node)| {
            node.kind == ActionNodeKind::Simulate
                && matches!(
                    node.status,
                    ActionNodeStatus::Pending | ActionNodeStatus::Ready
                )
                && dependencies_satisfied(&runtime.checkpoint.action_graph, node)
        })
        .map(|(node_id, _)| node_id.clone())?;

    let node = runtime.checkpoint.action_graph.nodes.get(&node_id)?.clone();
    let ActionPayload::Simulate(simulate) = &node.payload else {
        return fail_simulate(
            runtime,
            &node_id,
            "simulate payload missing for solana binding",
        );
    };
    let Some(SimulateLiveBinding::Solana(live)) = &simulate.live else {
        return fail_simulate(
            runtime,
            &node_id,
            "simulate payload missing solana live binding",
        );
    };

    let report = match SolanaLiveSimulatePort::simulate_with_client(client, &live.request).await {
        Ok(report) => report,
        Err(error) => {
            return fail_simulate(
                runtime,
                &node_id,
                format!("solana simulate via client failed: {error}"),
            );
        }
    };

    let evidence_id = format!("simulation.{node_id}");
    runtime.checkpoint.evidence_graph.records.insert(
        evidence_id.clone(),
        EvidenceRecord {
            evidence_id,
            kind: EvidenceKind::ExternalObservation,
            provenance: EvidenceProvenance {
                source: "solana.rpc.simulate".to_owned(),
                chain_scope: runtime.mission.allowed_chains.first().cloned(),
                trace_hint: Some(node_id.clone()),
            },
            freshness: EvidenceFreshness {
                observed_at_ms: Some(current_time_ms()),
                expires_at_ms: None,
                max_age_ms: None,
            },
            confidence_ppm: Some(1_000_000),
            payload: SolanaLiveSimulatePort::report_payload(&report),
        },
    );

    if !report.accepted {
        return fail_simulate(runtime, &node_id, "solana simulate report rejected request");
    }

    mark_node_status(runtime, node_id.as_str(), ActionNodeStatus::Succeeded);
    runtime
        .checkpoint
        .lifecycle
        .mark_running(RunPhase::Simulating);
    runtime.touch_transition();

    Some(StepTransition {
        kind: StepTransitionKind::Simulate,
        node_id: Some(node_id.clone()),
        summary: format!("completed live solana simulate node {node_id}"),
    })
}
