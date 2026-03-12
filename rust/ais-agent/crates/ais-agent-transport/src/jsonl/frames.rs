use serde::{Deserialize, Serialize};

use ais_agent_control::events::RunEventEnvelope;
use ais_agent_host::{
    control::HostCommandResponse,
    events::HostRunEventBatch,
    session::{HostRequestId, HostedRunCommand},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum JsonlInboundFrame {
    Command { command: HostedRunCommand },
    PollEvents { query: HostRunEventQuery },
}

use ais_agent_host::events::HostRunEventQuery;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonlResponseFrame {
    pub request_id: Option<HostRequestId>,
    pub response: HostCommandResponse,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonlServerErrorFrame {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum JsonlOutboundFrame {
    Response(JsonlResponseFrame),
    Event { event: RunEventEnvelope },
    EventBatch { batch: HostRunEventBatch },
    ServerError(JsonlServerErrorFrame),
}
