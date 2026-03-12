use serde::{Deserialize, Serialize};

use crate::{
    action::ActionGraph, actuation::ActuationRecord, effect::EffectContract,
    evidence::EvidenceGraph, runtime::RunLifecycleState,
};
use std::collections::BTreeMap;

/// Runtime-owned checkpoint snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointSnapshot {
    pub run_id: String,
    pub mission_id: String,
    pub checkpoint_seq: u64,
    pub plan_epoch: u64,
    pub lifecycle: RunLifecycleState,
    pub action_graph: ActionGraph,
    pub evidence_graph: EvidenceGraph,
    #[serde(default)]
    pub effect_contracts: BTreeMap<String, EffectContract>,
    pub pending_requests: PendingRequestsSnapshot,
    pub last_completed_node_id: Option<String>,
    #[serde(default)]
    pub actuation_records: Vec<ActuationRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PendingRequestsSnapshot {
    #[serde(default)]
    pub pending_evidence_refs: Vec<String>,
    #[serde(default)]
    pub pending_envelope_refs: Vec<String>,
    pub pending_signer_request_id: Option<String>,
    pub pending_confirmation_id: Option<String>,
}
