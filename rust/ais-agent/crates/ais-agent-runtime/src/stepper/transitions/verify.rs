//! Effect verification transition.

use ais_agent_control::recovery::{
    InterruptionClass, ProviderFailureInfo, RunFailureCode, RunFailureStage,
};
use ais_agent_core::{
    action::{
        kinds::verify::{VerifyAction, VerifyLiveBinding},
        ActionNode, ActionNodeKind, ActionNodeStatus, ActionPayload,
    },
    actuation::ActuationKind,
    binding::evm::EvmVerifyBinding,
    effect::{verify_effect_contract, EffectDeltaStatus, EffectObservationBundle},
    evidence::{EvidenceFreshness, EvidenceKind, EvidenceProvenance, EvidenceRecord},
    recovery::{classify_side_effect_phase, provider_interruption_class},
    runtime::RunPhase,
};
use ais_agent_evm::{read::live::EvmAlloyReadPort, receipt::live::EvmAlloyReceiptPort};
use ais_agent_solana::receipt::live::SolanaLiveReceiptPort;
#[cfg(test)]
use ais_agent_solana::receipt::live::SolanaRpcReceiptClient;
#[cfg(test)]
use alloy::providers::Provider;
use serde_json::{json, Value};

use super::{
    evm_binding::resolve_evm_verify_binding, latest_broadcast_submission_id_for_node,
    solana_binding::resolve_solana_verify_binding,
};

use crate::{
    runtime::ActiveRun,
    stepper::{
        transitions::{add_actuation_record, dependencies_satisfied, mark_node_status},
        StepTransition, StepTransitionKind,
    },
};

pub(crate) async fn apply_verify_transition(runtime: &mut ActiveRun) -> Option<StepTransition> {
    let node_id = runtime
        .checkpoint
        .action_graph
        .nodes
        .iter()
        .find(|(_, node)| {
            node.kind == ActionNodeKind::Verify
                && matches!(
                    node.status,
                    ActionNodeStatus::Pending | ActionNodeStatus::Ready
                )
                && dependencies_satisfied(&runtime.checkpoint.action_graph, node)
        })
        .map(|(node_id, _)| node_id.clone())?;

    let node = runtime.checkpoint.action_graph.nodes.get(&node_id)?.clone();

    if let Some(binding) = resolve_evm_verify_binding(&node) {
        let ActionPayload::Verify(verify) = &node.payload else {
            return fail_verify(runtime, &node_id, "verify payload missing for evm binding");
        };
        let Some(VerifyLiveBinding::Evm(live)) = &verify.live else {
            return fail_verify(runtime, &node_id, "verify payload missing evm live binding");
        };
        let Some(connection) = &live.connection else {
            return fail_verify(runtime, &node_id, "evm verify binding missing connection");
        };
        let Some(submission_id) = resolve_confirmation_submission_id(runtime, &node) else {
            return fail_verify(
                runtime,
                &node_id,
                "evm verify binding missing broadcast submission id",
            );
        };

        let receipt = match EvmAlloyReceiptPort::new(connection.http_url.clone())
            .get_transaction_receipt(&submission_id)
            .await
        {
            Ok(receipt) => receipt,
            Err(error) => {
                return pause_confirmation_retryable(
                    runtime,
                    &node,
                    &node_id,
                    "evm.rpc",
                    "eth_getTransactionReceipt",
                    format!("evm verify {binding:?} failed: {error}"),
                );
            }
        };

        return apply_evm_receipt_observation(
            runtime,
            &node,
            verify,
            binding,
            submission_id,
            receipt.payload,
            receipt.observed,
            receipt.success,
        )
        .await;
    }

    if let Some(binding) = resolve_solana_verify_binding(&node) {
        let ActionPayload::Verify(verify) = &node.payload else {
            return fail_verify(
                runtime,
                &node_id,
                "verify payload missing for solana binding",
            );
        };
        let Some(VerifyLiveBinding::Solana(live)) = &verify.live else {
            return fail_verify(
                runtime,
                &node_id,
                "verify payload missing solana live binding",
            );
        };
        let Some(connection) = &live.connection else {
            return fail_verify(
                runtime,
                &node_id,
                "solana verify binding missing connection",
            );
        };
        let Some(submission_id) = resolve_confirmation_submission_id(runtime, &node) else {
            return fail_verify(
                runtime,
                &node_id,
                "solana verify binding missing broadcast submission id",
            );
        };

        let receipt = match SolanaLiveReceiptPort::new(connection.clone())
            .get_signature_receipt(&submission_id)
            .await
        {
            Ok(receipt) => receipt,
            Err(error) => {
                return pause_confirmation_retryable(
                    runtime,
                    &node,
                    &node_id,
                    "solana.rpc",
                    "get_signature_status",
                    format!("solana verify {binding:?} failed: {error}"),
                );
            }
        };

        return apply_solana_receipt_observation(
            runtime,
            &node,
            verify,
            submission_id,
            receipt.payload,
            receipt.observed,
            receipt.success,
        )
        .await;
    }

    mark_node_status(runtime, node_id.as_str(), ActionNodeStatus::Succeeded);
    runtime
        .checkpoint
        .lifecycle
        .mark_running(RunPhase::Verifying);
    runtime.touch_transition();

    Some(StepTransition {
        kind: StepTransitionKind::Verify,
        node_id: Some(node_id.clone()),
        summary: format!("verified node {node_id}"),
    })
}

async fn apply_evm_receipt_observation(
    runtime: &mut ActiveRun,
    node: &ActionNode,
    verify: &VerifyAction,
    binding: EvmVerifyBinding,
    submission_id: String,
    receipt_payload: Value,
    observed: bool,
    success: Option<bool>,
) -> Option<StepTransition> {
    if !observed {
        runtime.checkpoint.pending_requests.pending_submission_id = Some(submission_id.clone());
        runtime
            .checkpoint
            .lifecycle
            .await_confirmation(format!("waiting for chain receipt {submission_id}"));
        runtime.checkpoint.lifecycle.failure = None;
        runtime.touch_transition();

        return Some(StepTransition {
            kind: StepTransitionKind::Verify,
            node_id: Some(node.node_id.clone()),
            summary: format!(
                "receipt not yet observed for {submission_id}; waiting for confirmation"
            ),
        });
    }

    let node_id = node.node_id.as_str();
    let receipt_payload = normalized_receipt_payload(receipt_payload, &submission_id, success);
    let receipt_evidence_id = format!("receipt.{node_id}");
    insert_observation_record(
        runtime,
        &receipt_evidence_id,
        "evm.alloy.receipt",
        node_id,
        receipt_payload.clone(),
    );
    add_actuation_record(
        runtime,
        node_id,
        ActuationKind::ReceiptObserved,
        runtime.mission.allowed_chains.first().cloned(),
        Some(submission_id.clone()),
        format!("receipt observed for {submission_id}"),
    );

    if is_effect_contract_binding(&binding) {
        return apply_effect_contract_verification(
            runtime,
            node,
            verify,
            receipt_payload,
            submission_id,
        )
        .await;
    }

    if success != Some(true) {
        return pause_verify_mismatch(
            runtime,
            node,
            format!("receipt observed for {submission_id} but execution failed"),
            Some(submission_id),
        );
    }

    runtime.checkpoint.pending_requests.pending_submission_id = None;
    runtime
        .checkpoint
        .lifecycle
        .resolve_confirmation_wait(RunPhase::Verifying);
    mark_node_status(runtime, node_id, ActionNodeStatus::Succeeded);
    runtime.touch_transition();

    Some(StepTransition {
        kind: StepTransitionKind::Verify,
        node_id: Some(node_id.to_owned()),
        summary: format!("verified live evm receipt for node {node_id}"),
    })
}

async fn apply_effect_contract_verification(
    runtime: &mut ActiveRun,
    node: &ActionNode,
    verify: &VerifyAction,
    receipt_payload: Value,
    submission_id: String,
) -> Option<StepTransition> {
    let node_id = node.node_id.as_str();
    let Some(effect_id) = node.expected_effect_ref.clone() else {
        return fail_verify(runtime, node_id, "verify node missing expected effect ref");
    };
    let Some(effect_contract) = runtime.checkpoint.effect_contracts.get(&effect_id).cloned() else {
        return fail_verify(
            runtime,
            node_id,
            format!("missing effect contract `{effect_id}`"),
        );
    };
    let Some(VerifyLiveBinding::Evm(live)) = &verify.live else {
        return fail_verify(runtime, node_id, "effect verify missing evm live binding");
    };
    let Some(connection) = &live.connection else {
        return fail_verify(runtime, node_id, "effect verify missing evm connection");
    };

    let post_payload = if let Some(request) = &live.post_request {
        match EvmAlloyReadPort::new(connection.http_url.clone())
            .observe(request)
            .await
        {
            Ok(payload) => {
                let evidence_id = verify
                    .post_observation_ref
                    .clone()
                    .unwrap_or_else(|| format!("post.{node_id}"));
                insert_observation_record(
                    runtime,
                    &evidence_id,
                    "evm.alloy.post_state",
                    node_id,
                    payload.clone(),
                );
                Some(payload)
            }
            Err(error) => {
                return pause_verify_provider_failure(
                    runtime,
                    node,
                    node_id,
                    "evm.rpc",
                    "eth_call",
                    format!("evm post-state observe failed: {error}"),
                    Some(submission_id.clone()),
                );
            }
        }
    } else {
        verify
            .post_observation_ref
            .as_ref()
            .and_then(|reference| runtime.checkpoint.evidence_graph.records.get(reference))
            .map(|record| record.payload.clone())
    };

    apply_effect_contract_verdict(
        runtime,
        node,
        verify,
        &effect_id,
        effect_contract,
        receipt_payload,
        post_payload,
        submission_id,
    )
}

async fn apply_solana_receipt_observation(
    runtime: &mut ActiveRun,
    node: &ActionNode,
    verify: &VerifyAction,
    submission_id: String,
    receipt_payload: Value,
    observed: bool,
    success: Option<bool>,
) -> Option<StepTransition> {
    if !observed {
        runtime.checkpoint.pending_requests.pending_submission_id = Some(submission_id.clone());
        runtime
            .checkpoint
            .lifecycle
            .await_confirmation(format!("waiting for chain receipt {submission_id}"));
        runtime.checkpoint.lifecycle.failure = None;
        runtime.touch_transition();

        return Some(StepTransition {
            kind: StepTransitionKind::Verify,
            node_id: Some(node.node_id.clone()),
            summary: format!(
                "receipt not yet observed for {submission_id}; waiting for confirmation"
            ),
        });
    }

    let node_id = node.node_id.as_str();
    let receipt_payload =
        normalized_solana_receipt_payload(receipt_payload, &submission_id, success);
    let receipt_evidence_id = format!("receipt.{node_id}");
    insert_observation_record(
        runtime,
        &receipt_evidence_id,
        "solana.rpc.signature_status",
        node_id,
        receipt_payload.clone(),
    );
    add_actuation_record(
        runtime,
        node_id,
        ActuationKind::ReceiptObserved,
        runtime.mission.allowed_chains.first().cloned(),
        Some(submission_id.clone()),
        format!("receipt observed for {submission_id}"),
    );

    let Some(VerifyLiveBinding::Solana(live)) = &verify.live else {
        return fail_verify(
            runtime,
            node_id,
            "solana verify payload missing live binding",
        );
    };
    let is_effect = matches!(
        live.binding,
        ais_agent_core::binding::solana::SolanaVerifyBinding::EffectContractFromSignatureStatus
    );
    if is_effect {
        return apply_solana_effect_contract_verification(
            runtime,
            node,
            verify,
            receipt_payload,
            submission_id,
        )
        .await;
    }

    if success != Some(true) {
        return pause_verify_mismatch(
            runtime,
            node,
            format!("receipt observed for {submission_id} but execution failed"),
            Some(submission_id),
        );
    }

    runtime.checkpoint.pending_requests.pending_submission_id = None;
    runtime
        .checkpoint
        .lifecycle
        .resolve_confirmation_wait(RunPhase::Verifying);
    mark_node_status(runtime, node_id, ActionNodeStatus::Succeeded);
    runtime.touch_transition();

    Some(StepTransition {
        kind: StepTransitionKind::Verify,
        node_id: Some(node_id.to_owned()),
        summary: format!("verified live solana receipt for node {node_id}"),
    })
}

async fn apply_solana_effect_contract_verification(
    runtime: &mut ActiveRun,
    node: &ActionNode,
    verify: &VerifyAction,
    receipt_payload: Value,
    submission_id: String,
) -> Option<StepTransition> {
    let node_id = node.node_id.as_str();
    let Some(effect_id) = node.expected_effect_ref.clone() else {
        return fail_verify(runtime, node_id, "verify node missing expected effect ref");
    };
    let Some(effect_contract) = runtime.checkpoint.effect_contracts.get(&effect_id).cloned() else {
        return fail_verify(
            runtime,
            node_id,
            format!("missing effect contract `{effect_id}`"),
        );
    };
    let post_payload = verify
        .post_observation_ref
        .as_ref()
        .and_then(|reference| runtime.checkpoint.evidence_graph.records.get(reference))
        .map(|record| record.payload.clone());

    apply_effect_contract_verdict(
        runtime,
        node,
        verify,
        &effect_id,
        effect_contract,
        receipt_payload,
        post_payload,
        submission_id,
    )
}

fn apply_effect_contract_verdict(
    runtime: &mut ActiveRun,
    node: &ActionNode,
    verify: &VerifyAction,
    effect_id: &str,
    effect_contract: ais_agent_core::effect::EffectContract,
    receipt_payload: Value,
    post_payload: Option<Value>,
    submission_id: String,
) -> Option<StepTransition> {
    let node_id = node.node_id.as_str();
    let pre_payload = verify
        .pre_observation_ref
        .as_ref()
        .and_then(|reference| runtime.checkpoint.evidence_graph.records.get(reference))
        .map(|record| record.payload.clone());

    let result = verify_effect_contract(
        &effect_contract,
        &EffectObservationBundle {
            pre: pre_payload,
            post: post_payload,
            receipt: Some(receipt_payload),
            expected: None,
            context: Some(json!({
                "node_id": node_id,
                "effect_id": effect_id,
                "submission_id": submission_id,
            })),
        },
    );

    insert_observation_record(
        runtime,
        &format!("effect.{node_id}"),
        "runtime.effect_verifier",
        node_id,
        serde_json::to_value(&result).unwrap_or_else(|_| {
            json!({
                "final_status": "violated",
                "final_summary": "failed to serialize effect verification result",
            })
        }),
    );

    match result.final_status {
        EffectDeltaStatus::Satisfied => {
            runtime.checkpoint.pending_requests.pending_submission_id = None;
            runtime
                .checkpoint
                .lifecycle
                .resolve_confirmation_wait(RunPhase::Verifying);
            mark_node_status(runtime, node_id, ActionNodeStatus::Succeeded);
            runtime.touch_transition();

            Some(StepTransition {
                kind: StepTransitionKind::Verify,
                node_id: Some(node_id.to_owned()),
                summary: format!("effect contract satisfied for node {node_id}"),
            })
        }
        EffectDeltaStatus::Violated => {
            pause_verify_mismatch(runtime, node, result.final_summary, Some(submission_id))
        }
        EffectDeltaStatus::UnknownDueToMissingObservation | EffectDeltaStatus::Pending => {
            let mut missing = result
                .deltas
                .iter()
                .flat_map(|delta| delta.missing_bindings.clone())
                .collect::<Vec<_>>();
            missing.sort();
            missing.dedup();
            runtime
                .checkpoint
                .lifecycle
                .await_evidence(result.final_summary.clone(), missing.clone());
            runtime.checkpoint.pending_requests.pending_evidence_refs = missing;
            runtime.touch_transition();

            Some(StepTransition {
                kind: StepTransitionKind::Verify,
                node_id: Some(node_id.to_owned()),
                summary: result.final_summary,
            })
        }
    }
}

fn is_effect_contract_binding(binding: &EvmVerifyBinding) -> bool {
    matches!(
        binding,
        EvmVerifyBinding::EffectContractFromReceipt
            | EvmVerifyBinding::EffectContractFromPostState
            | EvmVerifyBinding::EffectContractFromReceiptAndPostState
    )
}

fn resolve_confirmation_submission_id(runtime: &ActiveRun, node: &ActionNode) -> Option<String> {
    runtime
        .checkpoint
        .pending_requests
        .pending_submission_id
        .clone()
        .or_else(|| {
            node.depends_on.iter().rev().find_map(|dependency_id| {
                latest_broadcast_submission_id_for_node(runtime, dependency_id)
            })
        })
        .or_else(|| latest_broadcast_submission_id_for_node(runtime, node.node_id.as_str()))
}

fn insert_observation_record(
    runtime: &mut ActiveRun,
    evidence_id: &str,
    source: &str,
    node_id: &str,
    payload: Value,
) {
    runtime.checkpoint.evidence_graph.records.insert(
        evidence_id.to_owned(),
        EvidenceRecord {
            evidence_id: evidence_id.to_owned(),
            kind: EvidenceKind::ExternalObservation,
            provenance: EvidenceProvenance {
                source: source.to_owned(),
                chain_scope: runtime.mission.allowed_chains.first().cloned(),
                trace_hint: Some(node_id.to_owned()),
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
}

fn normalized_receipt_payload(raw: Value, tx_hash: &str, success: Option<bool>) -> Value {
    json!({
        "tx_hash": tx_hash,
        "status": success,
        "raw": raw,
    })
}

fn normalized_solana_receipt_payload(raw: Value, signature: &str, success: Option<bool>) -> Value {
    json!({
        "signature": signature,
        "status": success,
        "raw": raw,
    })
}

fn pause_confirmation_retryable(
    runtime: &mut ActiveRun,
    node: &ActionNode,
    node_id: &str,
    provider: &str,
    operation: &str,
    reason: impl Into<String>,
) -> Option<StepTransition> {
    let reason = reason.into();
    let submission_id = resolve_confirmation_submission_id(runtime, node);
    let actuation_ref = latest_actuation_ref_for_node(runtime, node);
    let failure_code = classify_confirmation_failure(reason.as_str());
    let interruption_class = classify_confirmation_interruption_class(&failure_code);
    if let Some(submission_id) = submission_id.clone() {
        runtime.checkpoint.pending_requests.pending_submission_id = Some(submission_id);
    }
    runtime.checkpoint.lifecycle.await_confirmation(format!(
        "waiting for chain receipt {}",
        runtime
            .checkpoint
            .pending_requests
            .pending_submission_id
            .clone()
            .unwrap_or_else(|| "unknown".to_owned())
    ));
    runtime.checkpoint.lifecycle.failure =
        Some(ais_agent_control::recovery::RunFailureContext::new(
            failure_code.clone(),
            RunFailureStage::Confirm,
            runtime.checkpoint.lifecycle.checkpoint_seq,
            runtime.checkpoint.lifecycle.plan_epoch,
            Some(ais_agent_control::recovery::StableBoundaryKind::Confirmation),
            reason.clone(),
        ));
    if let Some(failure) = runtime.checkpoint.lifecycle.failure.as_mut() {
        failure.node_refs.push(node_id.to_owned());
        if let Some(effect_ref) = node.expected_effect_ref.clone() {
            failure.effect_refs.push(effect_ref);
        }
        if let Some(actuation_ref) = actuation_ref.clone().or_else(|| submission_id.clone()) {
            failure.actuation_refs.push(actuation_ref);
        }
        if let Some(submission_id) = submission_id.clone() {
            failure.confirmation_refs.push(submission_id);
        }
        if matches!(failure_code, RunFailureCode::ProviderUnavailable) {
            failure.provider_error = Some(ProviderFailureInfo {
                provider: provider.to_owned(),
                operation: operation.to_owned(),
                code: None,
                message: reason.clone(),
                retryable: true,
            });
        }
    }
    runtime.checkpoint.lifecycle.record_interruption(
        interruption_class,
        Some(RunFailureStage::Confirm),
        classify_side_effect_phase(&runtime.checkpoint),
        reason.clone(),
    );
    runtime.touch_transition();

    Some(StepTransition {
        kind: StepTransitionKind::Verify,
        node_id: Some(node_id.to_owned()),
        summary: format!("verification paused for retry on node {node_id}: {reason}"),
    })
}

fn pause_verify_provider_failure(
    runtime: &mut ActiveRun,
    node: &ActionNode,
    node_id: &str,
    provider: &str,
    operation: &str,
    reason: impl Into<String>,
    submission_id: Option<String>,
) -> Option<StepTransition> {
    let reason = reason.into();
    let actuation_ref = latest_actuation_ref_for_node(runtime, node);
    if let Some(submission_id_value) = submission_id.clone() {
        runtime.checkpoint.pending_requests.pending_submission_id = Some(submission_id_value);
    }
    runtime.checkpoint.lifecycle.await_confirmation(format!(
        "waiting for chain receipt {}",
        runtime
            .checkpoint
            .pending_requests
            .pending_submission_id
            .clone()
            .unwrap_or_else(|| "unknown".to_owned())
    ));
    runtime.checkpoint.lifecycle.failure =
        Some(ais_agent_control::recovery::RunFailureContext::new(
            RunFailureCode::ProviderUnavailable,
            RunFailureStage::Verify,
            runtime.checkpoint.lifecycle.checkpoint_seq,
            runtime.checkpoint.lifecycle.plan_epoch,
            Some(ais_agent_control::recovery::StableBoundaryKind::Confirmation),
            reason.clone(),
        ));
    if let Some(failure) = runtime.checkpoint.lifecycle.failure.as_mut() {
        failure.node_refs.push(node_id.to_owned());
        if let Some(effect_ref) = node.expected_effect_ref.clone() {
            failure.effect_refs.push(effect_ref);
        }
        if let Some(actuation_ref) = actuation_ref.clone().or_else(|| submission_id.clone()) {
            failure.actuation_refs.push(actuation_ref);
        }
        if let Some(submission_id) = submission_id {
            failure.confirmation_refs.push(submission_id);
        }
        failure.provider_error = Some(ProviderFailureInfo {
            provider: provider.to_owned(),
            operation: operation.to_owned(),
            code: None,
            message: reason.clone(),
            retryable: true,
        });
    }
    runtime.checkpoint.lifecycle.record_interruption(
        provider_interruption_class(&reason),
        Some(RunFailureStage::Verify),
        classify_side_effect_phase(&runtime.checkpoint),
        reason.clone(),
    );
    runtime.touch_transition();

    Some(StepTransition {
        kind: StepTransitionKind::Verify,
        node_id: Some(node_id.to_owned()),
        summary: format!("verification provider failure on node {node_id}: {reason}"),
    })
}

fn pause_verify_mismatch(
    runtime: &mut ActiveRun,
    node: &ActionNode,
    reason: impl Into<String>,
    submission_id: Option<String>,
) -> Option<StepTransition> {
    let reason = reason.into();
    let node_id = node.node_id.as_str();
    let actuation_ref = latest_actuation_ref_for_node(runtime, node);
    mark_node_status(runtime, node_id, ActionNodeStatus::Failed);
    runtime.checkpoint.pending_requests.pending_submission_id = None;
    runtime.checkpoint.lifecycle.pause_with_failure(
        RunFailureStage::Verify,
        RunFailureCode::VerifyMismatch,
        reason.as_str(),
    );
    if let Some(failure) = runtime.checkpoint.lifecycle.failure.as_mut() {
        failure.node_refs.push(node_id.to_owned());
        if let Some(effect_ref) = node.expected_effect_ref.clone() {
            failure.effect_refs.push(effect_ref);
        }
        if let Some(actuation_ref) = actuation_ref.or_else(|| submission_id.clone()) {
            failure.actuation_refs.push(actuation_ref);
        }
        if let Some(submission_id) = submission_id {
            failure.confirmation_refs.push(submission_id);
        }
        failure.evidence_refs.extend(
            [
                node.expected_effect_ref
                    .as_ref()
                    .map(|_| format!("effect.{node_id}")),
                Some(format!("receipt.{node_id}")),
                verify_observation_ref(node, true),
                verify_observation_ref(node, false),
            ]
            .into_iter()
            .flatten(),
        );
        failure.evidence_refs.sort();
        failure.evidence_refs.dedup();
    }
    runtime.touch_transition();

    Some(StepTransition {
        kind: StepTransitionKind::Verify,
        node_id: Some(node_id.to_owned()),
        summary: format!("verification mismatch on node {node_id}: {reason}"),
    })
}

fn classify_confirmation_failure(reason: &str) -> RunFailureCode {
    let reason = reason.to_ascii_lowercase();
    if [
        "timeout",
        "timed out",
        "deadline exceeded",
        "context deadline exceeded",
    ]
    .iter()
    .any(|needle| reason.contains(needle))
    {
        return RunFailureCode::ConfirmationTimeout;
    }
    RunFailureCode::ProviderUnavailable
}

fn classify_confirmation_interruption_class(failure_code: &RunFailureCode) -> InterruptionClass {
    match failure_code {
        RunFailureCode::ConfirmationTimeout => InterruptionClass::ConfirmationWaitTimeout,
        RunFailureCode::ProviderUnavailable => InterruptionClass::ProviderUnavailable,
        _ => InterruptionClass::ProviderUnavailable,
    }
}

fn latest_actuation_ref_for_node(runtime: &ActiveRun, node: &ActionNode) -> Option<String> {
    node.depends_on
        .iter()
        .rev()
        .find_map(|dependency_id| {
            runtime
                .checkpoint
                .actuation_records
                .iter()
                .rev()
                .find(|record| {
                    record.node_id == *dependency_id
                        && matches!(record.kind, ActuationKind::BroadcastSubmitted)
                })
                .map(|record| record.record_id.clone())
        })
        .or_else(|| {
            runtime
                .checkpoint
                .actuation_records
                .iter()
                .rev()
                .find(|record| {
                    record.node_id == node.node_id
                        && matches!(record.kind, ActuationKind::BroadcastSubmitted)
                })
                .map(|record| record.record_id.clone())
        })
}

fn verify_observation_ref(node: &ActionNode, pre: bool) -> Option<String> {
    let ActionPayload::Verify(verify) = &node.payload else {
        return None;
    };
    if pre {
        verify.pre_observation_ref.clone()
    } else {
        verify.post_observation_ref.clone()
    }
}

fn fail_verify(
    runtime: &mut ActiveRun,
    node_id: &str,
    reason: impl Into<String>,
) -> Option<StepTransition> {
    let reason = reason.into();
    mark_node_status(runtime, node_id, ActionNodeStatus::Failed);
    runtime.checkpoint.lifecycle.fail(
        RunFailureStage::Verify,
        RunFailureCode::RuntimeInvariantViolation,
        reason.as_str(),
    );
    runtime.touch_transition();

    Some(StepTransition {
        kind: StepTransitionKind::Verify,
        node_id: Some(node_id.to_owned()),
        summary: format!("failed verify node {node_id}: {reason}"),
    })
}

fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
pub(crate) async fn apply_live_solana_verify_with_client<C>(
    runtime: &mut ActiveRun,
    client: &C,
) -> Option<StepTransition>
where
    C: SolanaRpcReceiptClient,
{
    let node_id = runtime
        .checkpoint
        .action_graph
        .nodes
        .iter()
        .find(|(_, node)| {
            node.kind == ActionNodeKind::Verify
                && matches!(
                    node.status,
                    ActionNodeStatus::Pending | ActionNodeStatus::Ready
                )
                && dependencies_satisfied(&runtime.checkpoint.action_graph, node)
        })
        .map(|(node_id, _)| node_id.clone())?;

    let node = runtime.checkpoint.action_graph.nodes.get(&node_id)?.clone();
    let ActionPayload::Verify(verify) = &node.payload else {
        return fail_verify(
            runtime,
            &node_id,
            "verify payload missing for solana binding",
        );
    };
    let Some(submission_id) = resolve_confirmation_submission_id(runtime, &node) else {
        return fail_verify(
            runtime,
            &node_id,
            "solana verify binding missing broadcast submission id",
        );
    };
    let receipt = match SolanaLiveReceiptPort::get_signature_receipt_with_client(
        client,
        &submission_id.parse().ok()?,
    )
    .await
    {
        Ok(receipt) => receipt,
        Err(error) => {
            return pause_confirmation_retryable(
                runtime,
                &node,
                &node_id,
                "solana.rpc",
                "get_signature_status",
                format!("solana verify via client failed: {error}"),
            );
        }
    };

    apply_solana_receipt_observation(
        runtime,
        &node,
        verify,
        submission_id,
        receipt.payload,
        receipt.observed,
        receipt.success,
    )
    .await
}

#[cfg(test)]
pub(crate) async fn apply_live_evm_verify_with_provider<P>(
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
            node.kind == ActionNodeKind::Verify
                && matches!(
                    node.status,
                    ActionNodeStatus::Pending | ActionNodeStatus::Ready
                )
                && dependencies_satisfied(&runtime.checkpoint.action_graph, node)
        })
        .map(|(node_id, _)| node_id.clone())?;

    let node = runtime.checkpoint.action_graph.nodes.get(&node_id)?.clone();
    let ActionPayload::Verify(verify) = &node.payload else {
        return fail_verify(runtime, &node_id, "verify payload missing for evm binding");
    };
    let Some(binding) = resolve_evm_verify_binding(&node) else {
        return fail_verify(runtime, &node_id, "missing evm verify binding");
    };
    let Some(submission_id) = resolve_confirmation_submission_id(runtime, &node) else {
        return fail_verify(
            runtime,
            &node_id,
            "evm verify binding missing broadcast submission id",
        );
    };
    let parsed_submission_id = match submission_id.parse() {
        Ok(submission_id) => submission_id,
        Err(error) => {
            return fail_verify(
                runtime,
                &node_id,
                format!("invalid submission id for verify: {error}"),
            );
        }
    };

    let receipt = match EvmAlloyReceiptPort::get_transaction_receipt_with_provider(
        provider,
        parsed_submission_id,
    )
    .await
    {
        Ok(receipt) => receipt,
        Err(error) => {
            return pause_confirmation_retryable(
                runtime,
                &node,
                &node_id,
                "evm.rpc",
                "eth_getTransactionReceipt",
                format!("evm verify via provider failed: {error}"),
            );
        }
    };

    if !receipt.observed {
        let submission_id = format!("{:#x}", receipt.tx_hash);
        runtime.checkpoint.pending_requests.pending_submission_id = Some(submission_id.clone());
        runtime
            .checkpoint
            .lifecycle
            .await_confirmation(format!("waiting for chain receipt {submission_id}"));
        runtime.checkpoint.lifecycle.failure = None;
        runtime.touch_transition();
        return Some(StepTransition {
            kind: StepTransitionKind::Verify,
            node_id: Some(node_id.clone()),
            summary: format!(
                "receipt not yet observed for {submission_id}; waiting for confirmation"
            ),
        });
    }

    let submission_id = format!("{:#x}", receipt.tx_hash);
    let raw_receipt_payload = receipt.payload.clone();
    let success = receipt.success;
    let observed = receipt.observed;

    if is_effect_contract_binding(&binding) {
        let receipt_payload =
            normalized_receipt_payload(raw_receipt_payload.clone(), &submission_id, success);
        let receipt_evidence_id = format!("receipt.{node_id}");
        insert_observation_record(
            runtime,
            &receipt_evidence_id,
            "evm.alloy.receipt",
            &node_id,
            receipt_payload.clone(),
        );
        add_actuation_record(
            runtime,
            node_id.as_str(),
            ActuationKind::ReceiptObserved,
            runtime.mission.allowed_chains.first().cloned(),
            Some(submission_id.clone()),
            format!("receipt observed for {submission_id}"),
        );

        let Some(VerifyLiveBinding::Evm(live)) = &verify.live else {
            return fail_verify(runtime, &node_id, "effect verify missing evm live binding");
        };
        let post_payload = if let Some(request) = &live.post_request {
            match EvmAlloyReadPort::observe_with_provider(provider, request).await {
                Ok(payload) => {
                    let evidence_id = verify
                        .post_observation_ref
                        .clone()
                        .unwrap_or_else(|| format!("post.{node_id}"));
                    insert_observation_record(
                        runtime,
                        &evidence_id,
                        "evm.alloy.post_state",
                        &node_id,
                        payload.clone(),
                    );
                    Some(payload)
                }
                Err(error) => {
                    return pause_verify_provider_failure(
                        runtime,
                        &node,
                        &node_id,
                        "evm.rpc",
                        "eth_call",
                        format!("evm post-state observe via provider failed: {error}"),
                        Some(submission_id.clone()),
                    );
                }
            }
        } else {
            verify
                .post_observation_ref
                .as_ref()
                .and_then(|reference| runtime.checkpoint.evidence_graph.records.get(reference))
                .map(|record| record.payload.clone())
        };

        let Some(effect_id) = node.expected_effect_ref.clone() else {
            return fail_verify(runtime, &node_id, "verify node missing expected effect ref");
        };
        let Some(effect_contract) = runtime.checkpoint.effect_contracts.get(&effect_id).cloned()
        else {
            return fail_verify(
                runtime,
                &node_id,
                format!("missing effect contract `{effect_id}`"),
            );
        };
        return apply_effect_contract_verdict(
            runtime,
            &node,
            verify,
            &effect_id,
            effect_contract,
            receipt_payload,
            post_payload,
            submission_id,
        );
    }

    apply_evm_receipt_observation(
        runtime,
        &node,
        verify,
        binding,
        submission_id,
        raw_receipt_payload,
        observed,
        success,
    )
    .await
}
