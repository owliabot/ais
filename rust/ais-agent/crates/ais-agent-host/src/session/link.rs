use serde::{Deserialize, Serialize};

use ais_agent_control::ids::RunId;

use crate::{
    inspect::{InspectSnapshot, RunPhase, RunStatus},
    session::HostSessionId,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInspectCursor {
    pub checkpoint_seq: u64,
    pub plan_epoch: u64,
    pub status: RunStatus,
    pub phase: RunPhase,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostRunLink {
    pub host_session_id: HostSessionId,
    pub run_id: RunId,
    pub mission_goal: String,
    #[serde(default)]
    pub allowed_chains: Vec<String>,
    pub inspect_cursor: Option<SessionInspectCursor>,
}

impl HostRunLink {
    pub fn new(
        host_session_id: HostSessionId,
        run_id: RunId,
        mission_goal: String,
        allowed_chains: Vec<String>,
    ) -> Self {
        Self {
            host_session_id,
            run_id,
            mission_goal,
            allowed_chains,
            inspect_cursor: None,
        }
    }

    pub fn apply_inspect(&mut self, inspect: &InspectSnapshot) {
        self.inspect_cursor = Some(SessionInspectCursor {
            checkpoint_seq: inspect.checkpoint_seq,
            plan_epoch: inspect.plan_epoch,
            status: inspect.status.clone(),
            phase: inspect.phase.clone(),
        });
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostSessionSnapshot {
    pub host_session_id: HostSessionId,
    pub active_run_id: Option<RunId>,
    #[serde(default)]
    pub linked_runs: Vec<HostRunLink>,
}
