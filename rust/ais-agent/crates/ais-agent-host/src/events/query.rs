use serde::{Deserialize, Serialize};

use ais_agent_control::{events::RunEventEnvelope, ids::RunId};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostRunEventQuery {
    pub run_id: RunId,
    pub after_event_seq: Option<u64>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostRunEventBatch {
    pub run_id: RunId,
    pub after_event_seq: Option<u64>,
    pub latest_event_seq: Option<u64>,
    pub next_after_event_seq: Option<u64>,
    pub truncated: bool,
    #[serde(default)]
    pub events: Vec<RunEventEnvelope>,
}
