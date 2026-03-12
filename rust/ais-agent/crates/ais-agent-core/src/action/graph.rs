use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::action::ActionNode;

/// Runtime-owned action graph.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ActionGraph {
    pub graph_id: Option<String>,
    #[serde(default)]
    pub roots: Vec<String>,
    #[serde(default)]
    pub terminals: Vec<String>,
    #[serde(default)]
    pub nodes: BTreeMap<String, ActionNode>,
}
