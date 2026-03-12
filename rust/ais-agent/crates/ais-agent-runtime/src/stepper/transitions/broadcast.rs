//! Broadcast transition.

use ais_agent_control::recovery::{
    InterruptionClass, ProviderFailureInfo, RunFailureCode, RunFailureStage, SideEffectPhase,
};
use ais_agent_core::{
    action::{kinds::actuate::ActuateLiveBinding, ActionNodeKind, ActionNodeStatus, ActionPayload},
    actuation::ActuationKind,
    envelope::RuntimeEnvelopeKind,
    recovery::{classify_side_effect_phase, provider_interruption_class},
    runtime::RunPhase,
};
use ais_agent_evm::broadcast::live::EvmAlloyBroadcastPort;
#[cfg(test)]
use ais_agent_solana::broadcast::live::SolanaRpcBroadcastClient;
use ais_agent_solana::broadcast::live::{extract_signed_transaction, SolanaLiveBroadcastPort};
#[cfg(test)]
use alloy::providers::Provider;

use super::{
    evm_binding::resolve_evm_actuate_binding, solana_binding::resolve_solana_actuate_binding,
};

use crate::{
    runtime::ActiveRun,
    stepper::{
        transitions::{add_actuation_record, dependencies_satisfied, mark_node_status},
        StepTransition, StepTransitionKind,
    },
};

pub(crate) async fn apply_broadcast_transition(runtime: &mut ActiveRun) -> Option<StepTransition> {
    if runtime.pending_signer_state.is_some()
        || runtime
            .checkpoint
            .pending_requests
            .pending_confirmation_id
            .is_some()
    {
        return None;
    }

    let node_id = runtime
        .checkpoint
        .action_graph
        .nodes
        .iter()
        .find(|(_, node)| {
            node.kind == ActionNodeKind::Actuate
                && node.status == ActionNodeStatus::Ready
                && dependencies_satisfied(&runtime.checkpoint.action_graph, node)
        })
        .map(|(node_id, _)| node_id.clone())?;

    let node = runtime.checkpoint.action_graph.nodes.get(&node_id)?.clone();

    if let Some(binding) = resolve_evm_actuate_binding(&node) {
        let ActionPayload::Actuate(actuate) = &node.payload else {
            return fail_broadcast(runtime, &node_id, "actuate payload missing for evm binding");
        };
        let Some(ActuateLiveBinding::Evm(live)) = &actuate.live else {
            return fail_broadcast(
                runtime,
                &node_id,
                "actuate payload missing evm live binding",
            );
        };
        let Some(connection) = &live.connection else {
            return fail_broadcast(
                runtime,
                &node_id,
                "evm broadcast binding missing connection",
            );
        };
        let Some(envelope_ref) = &actuate.envelope_ref else {
            return fail_envelope(
                runtime,
                &node_id,
                None,
                &node,
                "evm broadcast binding missing envelope_ref",
            );
        };
        let Some(envelope) = runtime.envelopes.get(envelope_ref) else {
            return fail_envelope(
                runtime,
                &node_id,
                Some(envelope_ref.as_str()),
                &node,
                format!("missing runtime envelope `{envelope_ref}` for broadcast"),
            );
        };
        if envelope.kind != RuntimeEnvelopeKind::EvmEnvelope {
            return fail_envelope(
                runtime,
                &node_id,
                Some(envelope_ref.as_str()),
                &node,
                format!("envelope `{envelope_ref}` is not an evm envelope"),
            );
        }
        let Some(raw_tx) = extract_raw_tx_hex(&envelope.payload) else {
            return fail_envelope(
                runtime,
                &node_id,
                Some(envelope_ref.as_str()),
                &node,
                format!("envelope `{envelope_ref}` missing `raw_tx` payload"),
            );
        };

        let submission = match EvmAlloyBroadcastPort::new(connection.rpc_url.clone())
            .send_raw_transaction_hex(&raw_tx)
            .await
        {
            Ok(submission) => submission,
            Err(error) => {
                return fail_broadcast_with_context(
                    runtime,
                    &node_id,
                    &node,
                    "evm.rpc",
                    "eth_sendRawTransaction",
                    format!("evm broadcast {binding:?} failed: {error}"),
                );
            }
        };

        let tx_hash = format!("{:#x}", submission.tx_hash);
        add_actuation_record(
            runtime,
            node_id.as_str(),
            ActuationKind::BroadcastSubmitted,
            actuate
                .chain
                .clone()
                .or_else(|| Some(envelope.chain.clone())),
            Some(tx_hash.clone()),
            format!("broadcast submitted for {node_id}"),
        );
        mark_node_status(runtime, node_id.as_str(), ActionNodeStatus::Succeeded);
        runtime.checkpoint.pending_requests.pending_confirmation_id = Some(tx_hash.clone());
        runtime.checkpoint.lifecycle.await_confirmation(format!(
            "waiting for chain receipt after broadcast {tx_hash}"
        ));
        runtime.touch_transition();

        return Some(StepTransition {
            kind: StepTransitionKind::Broadcast,
            node_id: Some(node_id.clone()),
            summary: format!("broadcast submitted for node {node_id}; awaiting receipt"),
        });
    }

    if let Some(binding) = resolve_solana_actuate_binding(&node) {
        let ActionPayload::Actuate(actuate) = &node.payload else {
            return fail_broadcast(
                runtime,
                &node_id,
                "actuate payload missing for solana binding",
            );
        };
        let Some(ActuateLiveBinding::Solana(live)) = &actuate.live else {
            return fail_broadcast(
                runtime,
                &node_id,
                "actuate payload missing solana live binding",
            );
        };
        let Some(connection) = &live.connection else {
            return fail_broadcast(
                runtime,
                &node_id,
                "solana broadcast binding missing connection",
            );
        };
        let Some(envelope_ref) = &actuate.envelope_ref else {
            return fail_envelope(
                runtime,
                &node_id,
                None,
                &node,
                "solana broadcast binding missing envelope_ref",
            );
        };
        let Some(envelope) = runtime.envelopes.get(envelope_ref) else {
            return fail_envelope(
                runtime,
                &node_id,
                Some(envelope_ref.as_str()),
                &node,
                format!("missing runtime envelope `{envelope_ref}` for broadcast"),
            );
        };
        if envelope.kind != RuntimeEnvelopeKind::SolanaEnvelope {
            return fail_envelope(
                runtime,
                &node_id,
                Some(envelope_ref.as_str()),
                &node,
                format!("envelope `{envelope_ref}` is not a solana envelope"),
            );
        }
        let transaction = match extract_signed_transaction(&envelope.payload) {
            Ok(transaction) => transaction,
            Err(error) => {
                return fail_envelope(
                    runtime,
                    &node_id,
                    Some(envelope_ref.as_str()),
                    &node,
                    format!("solana broadcast {binding:?} invalid envelope: {error}"),
                );
            }
        };

        let submission = match SolanaLiveBroadcastPort::new(connection.clone())
            .send_signed_transaction(&transaction)
            .await
        {
            Ok(submission) => submission,
            Err(error) => {
                return fail_broadcast_with_context(
                    runtime,
                    &node_id,
                    &node,
                    "solana.rpc",
                    "send_transaction",
                    format!("solana broadcast {binding:?} failed: {error}"),
                );
            }
        };

        let tx_hash = submission.signature.to_string();
        add_actuation_record(
            runtime,
            node_id.as_str(),
            ActuationKind::BroadcastSubmitted,
            actuate
                .chain
                .clone()
                .or_else(|| Some(envelope.chain.clone())),
            Some(tx_hash.clone()),
            format!("broadcast submitted for {node_id}"),
        );
        mark_node_status(runtime, node_id.as_str(), ActionNodeStatus::Succeeded);
        runtime.checkpoint.pending_requests.pending_confirmation_id = Some(tx_hash.clone());
        runtime.checkpoint.lifecycle.await_confirmation(format!(
            "waiting for chain receipt after broadcast {tx_hash}"
        ));
        runtime.touch_transition();

        return Some(StepTransition {
            kind: StepTransitionKind::Broadcast,
            node_id: Some(node_id.clone()),
            summary: format!("broadcast submitted for node {node_id}; awaiting receipt"),
        });
    }

    let chain = runtime
        .checkpoint
        .action_graph
        .nodes
        .get(node_id.as_str())
        .and_then(|node| match &node.payload {
            ActionPayload::Actuate(actuate) => actuate.chain.clone(),
            _ => None,
        });

    add_actuation_record(
        runtime,
        node_id.as_str(),
        ActuationKind::BroadcastSubmitted,
        chain,
        None,
        format!("broadcast submitted for {node_id}"),
    );
    mark_node_status(runtime, node_id.as_str(), ActionNodeStatus::Succeeded);
    runtime
        .checkpoint
        .lifecycle
        .mark_running(RunPhase::Broadcasting);
    runtime.touch_transition();

    Some(StepTransition {
        kind: StepTransitionKind::Broadcast,
        node_id: Some(node_id.clone()),
        summary: format!("broadcast submitted for node {node_id}"),
    })
}

fn fail_broadcast(
    runtime: &mut ActiveRun,
    node_id: &str,
    reason: impl Into<String>,
) -> Option<StepTransition> {
    let reason = reason.into();
    mark_node_status(runtime, node_id, ActionNodeStatus::Failed);
    runtime.checkpoint.lifecycle.fail(
        RunFailureStage::Broadcast,
        RunFailureCode::BroadcastRejected,
        reason.as_str(),
    );
    runtime.touch_transition();

    Some(StepTransition {
        kind: StepTransitionKind::Broadcast,
        node_id: Some(node_id.to_owned()),
        summary: format!("failed broadcast node {node_id}: {reason}"),
    })
}

fn fail_broadcast_with_context(
    runtime: &mut ActiveRun,
    node_id: &str,
    node: &ais_agent_core::action::ActionNode,
    provider: &str,
    operation: &str,
    reason: impl Into<String>,
) -> Option<StepTransition> {
    let reason = reason.into();
    let failure_code = classify_broadcast_failure(reason.as_str());
    match failure_code {
        RunFailureCode::BroadcastRejected => fail_broadcast(runtime, node_id, reason),
        RunFailureCode::BroadcastUncertain | RunFailureCode::ProviderUnavailable => {
            mark_node_status(runtime, node_id, ActionNodeStatus::Blocked);
            runtime.checkpoint.lifecycle.pause_with_failure(
                RunFailureStage::Broadcast,
                failure_code.clone(),
                reason.as_str(),
            );
            if let Some(failure) = runtime.checkpoint.lifecycle.failure.as_mut() {
                failure.node_refs.push(node_id.to_owned());
                if let Some(effect_ref) = node.expected_effect_ref.clone() {
                    failure.effect_refs.push(effect_ref);
                }
                failure.actuation_refs = actuation_refs_for_node(node);
                failure.provider_error = Some(ProviderFailureInfo {
                    provider: provider.to_owned(),
                    operation: operation.to_owned(),
                    code: None,
                    message: reason.clone(),
                    retryable: matches!(failure_code, RunFailureCode::ProviderUnavailable),
                });
            }
            runtime.checkpoint.lifecycle.record_interruption(
                match failure_code {
                    RunFailureCode::BroadcastUncertain => {
                        InterruptionClass::BroadcastOutcomeUncertain
                    }
                    RunFailureCode::ProviderUnavailable => provider_interruption_class(&reason),
                    _ => unreachable!("filtered by match arm"),
                },
                Some(RunFailureStage::Broadcast),
                match failure_code {
                    RunFailureCode::BroadcastUncertain => Some(SideEffectPhase::BroadcastSubmitted),
                    _ => classify_side_effect_phase(&runtime.checkpoint),
                },
                reason.clone(),
            );
            runtime.touch_transition();
            Some(StepTransition {
                kind: StepTransitionKind::Broadcast,
                node_id: Some(node_id.to_owned()),
                summary: format!("broadcast uncertainty for node {node_id}: {reason}"),
            })
        }
        other => fail_broadcast(runtime, node_id, format!("{other:?}: {reason}")),
    }
}

fn fail_envelope(
    runtime: &mut ActiveRun,
    node_id: &str,
    envelope_ref: Option<&str>,
    node: &ais_agent_core::action::ActionNode,
    reason: impl Into<String>,
) -> Option<StepTransition> {
    let reason = reason.into();
    let blocking_refs = envelope_ref
        .map(|reference| vec![reference.to_owned()])
        .unwrap_or_default();

    if blocking_refs.is_empty() {
        mark_node_status(runtime, node_id, ActionNodeStatus::Failed);
    } else {
        mark_node_status(runtime, node_id, ActionNodeStatus::Blocked);
    }

    runtime.checkpoint.pending_requests.pending_envelope_refs = blocking_refs.clone();
    runtime.checkpoint.lifecycle.pause_with_failure(
        RunFailureStage::Broadcast,
        RunFailureCode::EnvelopeInvalid,
        reason.as_str(),
    );
    if let Some(boundary) = runtime.checkpoint.lifecycle.active_boundary.as_mut() {
        boundary.blocking_refs = blocking_refs.clone();
    }
    if let Some(failure) = runtime.checkpoint.lifecycle.failure.as_mut() {
        failure.node_refs.push(node_id.to_owned());
        if let Some(effect_ref) = node.expected_effect_ref.clone() {
            failure.effect_refs.push(effect_ref);
        }
    }
    runtime.touch_transition();

    Some(StepTransition {
        kind: StepTransitionKind::Broadcast,
        node_id: Some(node_id.to_owned()),
        summary: format!("invalid envelope for broadcast node {node_id}: {reason}"),
    })
}

fn extract_raw_tx_hex(payload: &serde_json::Value) -> Option<String> {
    payload
        .get("raw_tx")
        .or_else(|| payload.get("raw_transaction"))
        .or_else(|| payload.get("signed_tx"))
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned)
}

fn classify_broadcast_failure(reason: &str) -> RunFailureCode {
    let reason = reason.to_ascii_lowercase();
    if [
        "already known",
        "already processed",
        "known transaction",
        "status unknown",
        "uncertain",
        "timed out",
        "timeout",
        "deadline exceeded",
        "nonce too low",
    ]
    .iter()
    .any(|needle| reason.contains(needle))
    {
        return RunFailureCode::BroadcastUncertain;
    }

    if [
        "429",
        "rate limit",
        "too many requests",
        "temporarily unavailable",
        "service unavailable",
        "connection refused",
        "connection reset",
        "network is unreachable",
        "dns",
        "transport",
        "rpc",
        "provider",
        "http error",
        "unavailable",
    ]
    .iter()
    .any(|needle| reason.contains(needle))
    {
        return RunFailureCode::ProviderUnavailable;
    }

    RunFailureCode::BroadcastRejected
}

fn actuation_refs_for_node(node: &ais_agent_core::action::ActionNode) -> Vec<String> {
    match &node.payload {
        ActionPayload::Actuate(actuate) => actuate.envelope_ref.clone().into_iter().collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
pub(crate) async fn apply_live_evm_broadcast_with_provider<P>(
    runtime: &mut ActiveRun,
    provider: &P,
) -> Option<StepTransition>
where
    P: Provider,
{
    if runtime.pending_signer_state.is_some()
        || runtime
            .checkpoint
            .pending_requests
            .pending_confirmation_id
            .is_some()
    {
        return None;
    }

    let node_id = runtime
        .checkpoint
        .action_graph
        .nodes
        .iter()
        .find(|(_, node)| {
            node.kind == ActionNodeKind::Actuate
                && node.status == ActionNodeStatus::Ready
                && dependencies_satisfied(&runtime.checkpoint.action_graph, node)
        })
        .map(|(node_id, _)| node_id.clone())?;

    let node = runtime.checkpoint.action_graph.nodes.get(&node_id)?.clone();
    let ActionPayload::Actuate(actuate) = &node.payload else {
        return fail_broadcast(runtime, &node_id, "actuate payload missing for evm binding");
    };
    let Some(ActuateLiveBinding::Evm(_live)) = &actuate.live else {
        return fail_broadcast(
            runtime,
            &node_id,
            "actuate payload missing evm live binding",
        );
    };
    let Some(envelope_ref) = &actuate.envelope_ref else {
        return fail_broadcast(
            runtime,
            &node_id,
            "evm broadcast binding missing envelope_ref",
        );
    };
    let Some(envelope) = runtime.envelopes.get(envelope_ref) else {
        return fail_broadcast(
            runtime,
            &node_id,
            format!("missing runtime envelope `{envelope_ref}` for broadcast"),
        );
    };
    let Some(raw_tx) = extract_raw_tx_hex(&envelope.payload) else {
        return fail_broadcast(
            runtime,
            &node_id,
            format!("envelope `{envelope_ref}` missing `raw_tx` payload"),
        );
    };
    let raw_tx = match ais_agent_evm::broadcast::live::parse_raw_transaction_hex(&raw_tx) {
        Ok(raw_tx) => raw_tx,
        Err(error) => {
            return fail_broadcast(
                runtime,
                &node_id,
                format!("evm broadcast via provider failed: {error}"),
            );
        }
    };
    let submission =
        match EvmAlloyBroadcastPort::send_raw_transaction_with_provider(provider, &raw_tx).await {
            Ok(submission) => submission,
            Err(error) => {
                return fail_broadcast_with_context(
                    runtime,
                    &node_id,
                    &node,
                    "evm.rpc",
                    "eth_sendRawTransaction",
                    format!("evm broadcast via provider failed: {error}"),
                );
            }
        };

    let tx_hash = format!("{:#x}", submission.tx_hash);
    add_actuation_record(
        runtime,
        node_id.as_str(),
        ActuationKind::BroadcastSubmitted,
        actuate
            .chain
            .clone()
            .or_else(|| Some(envelope.chain.clone())),
        Some(tx_hash.clone()),
        format!("broadcast submitted for {node_id}"),
    );
    mark_node_status(runtime, node_id.as_str(), ActionNodeStatus::Succeeded);
    runtime.checkpoint.pending_requests.pending_confirmation_id = Some(tx_hash.clone());
    runtime.checkpoint.lifecycle.await_confirmation(format!(
        "waiting for chain receipt after broadcast {tx_hash}"
    ));
    runtime.touch_transition();

    Some(StepTransition {
        kind: StepTransitionKind::Broadcast,
        node_id: Some(node_id.clone()),
        summary: format!("broadcast submitted for node {node_id}; awaiting receipt"),
    })
}

#[cfg(test)]
pub(crate) async fn apply_live_solana_broadcast_with_client<C>(
    runtime: &mut ActiveRun,
    client: &C,
) -> Option<StepTransition>
where
    C: SolanaRpcBroadcastClient,
{
    if runtime.pending_signer_state.is_some()
        || runtime
            .checkpoint
            .pending_requests
            .pending_confirmation_id
            .is_some()
    {
        return None;
    }

    let node_id = runtime
        .checkpoint
        .action_graph
        .nodes
        .iter()
        .find(|(_, node)| {
            node.kind == ActionNodeKind::Actuate
                && node.status == ActionNodeStatus::Ready
                && dependencies_satisfied(&runtime.checkpoint.action_graph, node)
        })
        .map(|(node_id, _)| node_id.clone())?;

    let node = runtime.checkpoint.action_graph.nodes.get(&node_id)?.clone();
    let ActionPayload::Actuate(actuate) = &node.payload else {
        return fail_broadcast(
            runtime,
            &node_id,
            "actuate payload missing for solana binding",
        );
    };
    let Some(ActuateLiveBinding::Solana(_live)) = &actuate.live else {
        return fail_broadcast(
            runtime,
            &node_id,
            "actuate payload missing solana live binding",
        );
    };
    let Some(envelope_ref) = &actuate.envelope_ref else {
        return fail_broadcast(
            runtime,
            &node_id,
            "solana broadcast binding missing envelope_ref",
        );
    };
    let Some(envelope) = runtime.envelopes.get(envelope_ref) else {
        return fail_broadcast(
            runtime,
            &node_id,
            format!("missing runtime envelope `{envelope_ref}` for broadcast"),
        );
    };
    let transaction = match extract_signed_transaction(&envelope.payload) {
        Ok(transaction) => transaction,
        Err(error) => {
            return fail_broadcast(
                runtime,
                &node_id,
                format!("solana broadcast via client invalid envelope: {error}"),
            );
        }
    };

    let submission = match SolanaLiveBroadcastPort::send_with_client(client, &transaction).await {
        Ok(submission) => submission,
        Err(error) => {
            return fail_broadcast_with_context(
                runtime,
                &node_id,
                &node,
                "solana.rpc",
                "send_transaction",
                format!("solana broadcast via client failed: {error}"),
            );
        }
    };

    let tx_hash = submission.signature.to_string();
    add_actuation_record(
        runtime,
        node_id.as_str(),
        ActuationKind::BroadcastSubmitted,
        actuate
            .chain
            .clone()
            .or_else(|| Some(envelope.chain.clone())),
        Some(tx_hash.clone()),
        format!("broadcast submitted for {node_id}"),
    );
    mark_node_status(runtime, node_id.as_str(), ActionNodeStatus::Succeeded);
    runtime.checkpoint.pending_requests.pending_confirmation_id = Some(tx_hash.clone());
    runtime.checkpoint.lifecycle.await_confirmation(format!(
        "waiting for chain receipt after broadcast {tx_hash}"
    ));
    runtime.touch_transition();

    Some(StepTransition {
        kind: StepTransitionKind::Broadcast,
        node_id: Some(node_id.clone()),
        summary: format!("broadcast submitted for node {node_id}; awaiting receipt"),
    })
}
