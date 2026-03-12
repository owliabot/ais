use crate::control::HostCommandOutcome;
use serde::{Deserialize, Serialize};

use ais_agent_control::ids::{CommandId, IdempotencyKey, RunId};

use crate::session::HostSessionId;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdempotencyRecord {
    pub host_session_id: HostSessionId,
    pub key: IdempotencyKey,
    pub command_id: CommandId,
    pub run_id: Option<RunId>,
    pub outcome: Option<HostCommandOutcome>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdempotencyResolution {
    Accepted,
    Replay {
        existing_command_id: CommandId,
        run_id: Option<RunId>,
        outcome: Option<HostCommandOutcome>,
    },
    Conflict {
        existing_command_id: CommandId,
        run_id: Option<RunId>,
    },
}
