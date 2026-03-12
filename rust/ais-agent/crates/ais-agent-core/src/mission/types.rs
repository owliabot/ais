use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::mission::{MissionBudget, MissionPolicy};

/// Runtime-owned mission object.
///
/// This is the normalized object the runtime executes against; it is distinct
/// from host-submitted mission DTOs at the command boundary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mission {
    pub mission_id: String,
    pub goal: String,
    #[serde(default)]
    pub allowed_chains: Vec<String>,
    pub budget: MissionBudget,
    pub policy: MissionPolicy,
    #[serde(default)]
    pub constraints: BTreeMap<String, Value>,
    #[serde(default)]
    pub metadata: BTreeMap<String, Value>,
}
