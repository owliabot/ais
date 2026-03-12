use serde::{Deserialize, Serialize};

use ais_agent_control::commands::RunCommand;

use crate::session::{HostRequestId, HostSessionId};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostCommandEnvelope<C> {
    pub host_session_id: HostSessionId,
    pub host_request_id: Option<HostRequestId>,
    pub command: C,
}

pub type HostedRunCommand = HostCommandEnvelope<RunCommand>;
