use serde::{Deserialize, Serialize};

use ais_agent_core::{evidence::EvidenceGraph, mission::Mission};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StandardDriverRequest {
    pub mission: Mission,
    pub evidence: EvidenceGraph,
    pub action_selector: String,
}
