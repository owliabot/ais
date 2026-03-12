use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    ids::RunId,
    recovery::{RecoveryDisposition, RunFailureCode},
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanPatchSubmission {
    pub patch_id: String,
    pub run_id: RunId,
    pub basis_checkpoint_seq: u64,
    pub basis_plan_epoch: u64,
    pub reason_code: RunFailureCode,
    pub target: PlanPatchTarget,
    #[serde(default)]
    pub operations: Vec<PlanPatchOperation>,
    pub expected_outcome: Option<PatchOutcome>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PlanPatchTarget {
    ActiveFrontier,
    NodeSet {
        #[serde(default)]
        node_ids: Vec<String>,
    },
    FailedFragment {
        #[serde(default)]
        node_ids: Vec<String>,
    },
    PendingVerifyBranch {
        #[serde(default)]
        effect_refs: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PlanPatchOperation {
    ReplaceFragment {
        fragment: Value,
        #[serde(default)]
        preserved_effect_refs: Vec<String>,
    },
    AppendFragment {
        fragment: Value,
        #[serde(default)]
        preserved_effect_refs: Vec<String>,
    },
    DropBranch {
        #[serde(default)]
        node_ids: Vec<String>,
    },
    TightenConstraints {
        #[serde(default)]
        constraints: BTreeMap<String, Value>,
    },
    ReplaceEffectContract {
        effect_ref: String,
        contract: Value,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PatchOutcome {
    pub next_recovery_disposition: Option<RecoveryDisposition>,
    #[serde(default)]
    pub touched_node_refs: Vec<String>,
    #[serde(default)]
    pub preserved_effect_refs: Vec<String>,
}
