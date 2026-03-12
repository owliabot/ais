use serde::{Deserialize, Serialize};

use ais_agent_control::{events::RunEventEnvelope, ids::RunId};

use crate::{
    inspect::{InspectSnapshot, PauseBundle},
    session::HostSessionSnapshot,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostAcceptedResponse {
    pub run_id: Option<RunId>,
    pub message: Option<String>,
    pub session: Option<HostSessionSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostCommandError {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HostCommandResponse {
    Accepted(HostAcceptedResponse),
    Inspect(InspectSnapshot),
    Pause(PauseBundle),
    Session(HostSessionSnapshot),
    Error(HostCommandError),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostCommandOutcome {
    pub response: HostCommandResponse,
    #[serde(default)]
    pub events: Vec<RunEventEnvelope>,
}
