use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::evidence::EvidenceRecord;

/// Runtime-owned evidence inventory and usage graph.
///
/// This is the canonical layer for:
/// - what evidence exists
/// - which evidence is still required
/// - which action nodes consumed which evidence
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EvidenceGraph {
    #[serde(default)]
    pub records: BTreeMap<String, EvidenceRecord>,
    #[serde(default)]
    pub requirements: Vec<EvidenceRequirement>,
    #[serde(default)]
    pub usages: Vec<EvidenceUsage>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceUsageKind {
    Read,
    DerivedFrom,
    SatisfiedRequirement,
    ConsumedForExecution,
    ConsumedForVerification,
}

/// A missing or pending evidence requirement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceRequirement {
    pub requirement_id: String,
    pub reference: String,
    pub reason: String,
    pub required_by_node_id: Option<String>,
    pub satisfied_by_evidence_id: Option<String>,
}

/// A trace edge from evidence to an action node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceUsage {
    pub evidence_id: String,
    pub node_id: String,
    pub kind: EvidenceUsageKind,
    pub detail: Option<String>,
}
