use std::collections::BTreeMap;

use ais_agent_control::execution_artifact::{
    ExecutionArtifactLaunchSpec, ExecutionOutputKey, ExecutionPackageEntry, ExecutionStageId,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    action::ActionGraph, actuation::ActuationRecord, effect::EffectContract,
    evidence::EvidenceGraph, runtime::RunLifecycleState,
};

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
    #[serde(default)]
    pub execution_artifact: Option<ExecutionArtifactRuntimeSnapshot>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionArtifactRuntimeSnapshot {
    pub launch_spec: ExecutionArtifactLaunchSpec,
    pub active_stage_id: Option<ExecutionStageId>,
    #[serde(default)]
    pub planned_stage_graphs: BTreeMap<ExecutionStageId, ActionGraph>,
    #[serde(default)]
    pub exported_outputs: BTreeMap<ExecutionOutputKey, Value>,
    #[serde(default)]
    pub awaiting_continuation: Option<ArtifactContinuationSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactContinuationSnapshot {
    pub stage_id: ExecutionStageId,
    #[serde(default)]
    pub required_outputs: Vec<ExecutionOutputKey>,
    pub package_entry: ExecutionPackageEntry,
    #[serde(default)]
    pub next_stage_id: Option<ExecutionStageId>,
}
