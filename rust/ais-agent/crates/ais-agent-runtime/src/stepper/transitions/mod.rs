//! Fine-grained runtime transitions.

mod broadcast;
mod complete;
mod derive;
mod evm_binding;
mod govern;
mod ingest;
mod observe;
mod recover;
mod signer;
mod simulate;
mod solana_binding;
mod verify;

use ais_agent_core::{
    action::{ActionGraph, ActionNode, ActionNodeStatus},
    actuation::{ActuationKind, ActuationRecord, ActuationStatus},
};

use crate::runtime::ActiveRun;

pub(crate) use self::{
    broadcast::apply_broadcast_transition, complete::apply_complete_transition,
    derive::apply_derive_transition, govern::apply_govern_transition,
    ingest::apply_ingest_transition, observe::apply_observe_transition,
    recover::apply_recover_transition, signer::apply_signer_transition,
    simulate::apply_simulate_transition, verify::apply_verify_transition,
};

#[cfg(test)]
pub(crate) use self::broadcast::apply_live_solana_broadcast_with_client;
#[cfg(test)]
pub(crate) use self::evm_binding::{
    resolve_evm_actuate_binding, resolve_evm_observe_binding, resolve_evm_simulate_binding,
    resolve_evm_verify_binding,
};
#[cfg(test)]
pub(crate) use self::observe::apply_live_evm_observe_with_provider;
#[cfg(test)]
pub(crate) use self::observe::apply_live_solana_observe_with_client;
#[cfg(test)]
pub(crate) use self::simulate::apply_live_evm_simulate_with_provider;
#[cfg(test)]
pub(crate) use self::simulate::apply_live_solana_simulate_with_client;
#[cfg(test)]
pub(crate) use self::solana_binding::{
    resolve_solana_actuate_binding, resolve_solana_observe_binding,
    resolve_solana_simulate_binding, resolve_solana_verify_binding,
};
#[cfg(test)]
pub(crate) use self::{
    broadcast::apply_live_evm_broadcast_with_provider, verify::apply_live_evm_verify_with_provider,
    verify::apply_live_solana_verify_with_client,
};

pub(crate) fn dependencies_satisfied(graph: &ActionGraph, node: &ActionNode) -> bool {
    node.depends_on.iter().all(|dependency_id| {
        graph.nodes.get(dependency_id).is_some_and(|dependency| {
            matches!(
                dependency.status,
                ActionNodeStatus::Succeeded | ActionNodeStatus::Skipped
            )
        })
    })
}

pub(crate) fn mark_node_status(
    runtime: &mut ActiveRun,
    node_id: &str,
    status: ActionNodeStatus,
) -> bool {
    let Some(node) = runtime.checkpoint.action_graph.nodes.get_mut(node_id) else {
        return false;
    };
    node.status = status.clone();
    if matches!(status, ActionNodeStatus::Succeeded) {
        runtime.checkpoint.last_completed_node_id = Some(node_id.to_owned());
    }
    true
}

pub(crate) fn add_actuation_record(
    runtime: &mut ActiveRun,
    node_id: &str,
    kind: ActuationKind,
    chain: Option<String>,
    tx_hash: Option<String>,
    summary: impl Into<String>,
) {
    let record_index = runtime.checkpoint.actuation_records.len() + 1;
    let record_id = format!("{node_id}:{}:{record_index}", actuation_kind_label(&kind));
    runtime.checkpoint.actuation_records.push(ActuationRecord {
        record_id,
        node_id: node_id.to_owned(),
        kind,
        status: ActuationStatus::Succeeded,
        chain,
        tx_hash,
        summary: summary.into(),
    });
}

fn actuation_kind_label(kind: &ActuationKind) -> &'static str {
    match kind {
        ActuationKind::EnvelopeBuilt => "envelope_built",
        ActuationKind::SignerRequested => "signer_requested",
        ActuationKind::BroadcastSubmitted => "broadcast_submitted",
        ActuationKind::ReceiptObserved => "receipt_observed",
        ActuationKind::ExternalJobSubmitted => "external_job_submitted",
    }
}

pub(crate) fn latest_broadcast_tx_hash_for_node(
    runtime: &ActiveRun,
    node_id: &str,
) -> Option<String> {
    runtime
        .checkpoint
        .actuation_records
        .iter()
        .rev()
        .find(|record| {
            record.node_id == node_id && matches!(record.kind, ActuationKind::BroadcastSubmitted)
        })
        .and_then(|record| record.tx_hash.clone())
}
