use serde::{Deserialize, Serialize};

use crate::action::kinds::{
    actuate::ActuateAction, derive::DeriveAction, observe::ObserveAction, recover::RecoverAction,
    simulate::SimulateAction, verify::VerifyAction,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionNodeKind {
    Observe,
    Derive,
    Simulate,
    Actuate,
    Verify,
    Recover,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionNodeStatus {
    Pending,
    Ready,
    Running,
    Blocked,
    Succeeded,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionOrigin {
    DriverFragment,
    ReflectionPath,
    ApiNativePath,
    RawEnvelopePath,
    RecoveryRuntime,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionInputRef {
    pub reference: String,
    pub optional: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ActionPayload {
    Observe(ObserveAction),
    Derive(DeriveAction),
    Simulate(SimulateAction),
    Actuate(ActuateAction),
    Verify(VerifyAction),
    Recover(RecoverAction),
}

/// Runtime-owned action node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionNode {
    pub node_id: String,
    pub kind: ActionNodeKind,
    pub origin: ActionOrigin,
    pub status: ActionNodeStatus,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub inputs: Vec<ActionInputRef>,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
    pub payload: ActionPayload,
    pub implementation_hint: Option<String>,
    pub expected_effect_ref: Option<String>,
}
