use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ActionStatusCountsView {
    pub pending: u32,
    pub ready: u32,
    pub running: u32,
    pub blocked: u32,
    pub succeeded: u32,
    pub failed: u32,
    pub skipped: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgressView {
    pub graph_id: Option<String>,
    pub total_nodes: u32,
    pub roots: u32,
    pub terminals: u32,
    pub status_counts: ActionStatusCountsView,
    #[serde(default)]
    pub active_node_ids: Vec<String>,
    #[serde(default)]
    pub blocked_node_ids: Vec<String>,
    pub last_completed_node_id: Option<String>,
    pub required_evidence_count: u32,
    pub actuation_record_count: u32,
}
