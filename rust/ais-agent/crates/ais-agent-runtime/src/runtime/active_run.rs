//! Hot runtime aggregate that combines current execution truth.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use ais_agent_control::{
    events::RunEventEnvelope,
    ids::{CommandId, RunId},
};
use ais_agent_core::{
    checkpoint::CheckpointSnapshot, envelope::RuntimeEnvelope, mission::Mission,
    runtime::SignerRequestState,
};

/// Runtime-owned hot state.
///
/// This object is intentionally separate from checkpoint snapshots:
/// - checkpoints are persistence/recovery boundaries
/// - `ActiveRun` is the currently loaded execution state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveRun {
    pub run_id: RunId,
    pub mission: Mission,
    pub checkpoint: CheckpointSnapshot,
    pub pending_signer_state: Option<SignerRequestState>,
    #[serde(default)]
    pub envelopes: BTreeMap<String, RuntimeEnvelope>,
    #[serde(default)]
    pub event_log: Vec<RunEventEnvelope>,
    pub event_seq: u64,
    pub last_command_id: Option<CommandId>,
    pub last_updated_at_ms: Option<u64>,
    pub revision: u64,
}

impl ActiveRun {
    pub fn new(mission: Mission, checkpoint: CheckpointSnapshot) -> Self {
        Self {
            run_id: checkpoint.lifecycle.run_id.clone(),
            mission,
            checkpoint,
            pending_signer_state: None,
            envelopes: BTreeMap::new(),
            event_log: Vec::new(),
            event_seq: 0,
            last_command_id: None,
            last_updated_at_ms: None,
            revision: 0,
        }
    }

    pub fn checkpoint_seq(&self) -> u64 {
        self.checkpoint.checkpoint_seq
    }

    pub fn plan_epoch(&self) -> u64 {
        self.checkpoint.plan_epoch
    }

    pub fn set_pending_signer_state(&mut self, pending_signer_state: Option<SignerRequestState>) {
        self.pending_signer_state = pending_signer_state;
    }

    pub fn record_command(&mut self, command_id: CommandId, updated_at_ms: Option<u64>) {
        self.last_command_id = Some(command_id);
        self.last_updated_at_ms = updated_at_ms;
    }

    pub fn next_event_seq(&mut self) -> u64 {
        self.event_seq = self.event_seq.saturating_add(1);
        self.event_seq
    }

    pub fn record_event(&mut self, event: RunEventEnvelope) {
        self.event_log.push(event);
    }

    pub fn latest_event_seq(&self) -> Option<u64> {
        self.event_log.last().map(|event| event.event_seq)
    }

    pub fn bump_revision(&mut self) {
        self.revision = self.revision.saturating_add(1);
    }

    pub fn touch_transition(&mut self) {
        self.checkpoint.lifecycle.bump_checkpoint();
        self.checkpoint.checkpoint_seq = self.checkpoint.lifecycle.checkpoint_seq;
        self.checkpoint.plan_epoch = self.checkpoint.lifecycle.plan_epoch;
        self.bump_revision();
    }
}
