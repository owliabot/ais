use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use ais_agent_control::ids::{ChainSubmissionId, RunId, SignerRequestId};
use ais_agent_core::runtime::{SignerResolution, SignerResolutionKind};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostSignerResolutionKind {
    Denied,
    Submitted,
    Signed,
    Expired,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostSignerResolution {
    pub run_id: RunId,
    pub request_id: SignerRequestId,
    pub kind: HostSignerResolutionKind,
    pub resolved_at_ms: Option<u64>,
    pub submission_id: Option<ChainSubmissionId>,
    #[serde(default)]
    pub signed_payload: Option<Value>,
    #[serde(default)]
    pub details: BTreeMap<String, Value>,
}

impl HostSignerResolution {
    pub fn into_runtime_resolution(self) -> SignerResolution {
        SignerResolution {
            request_id: self.request_id,
            kind: match self.kind {
                HostSignerResolutionKind::Denied => SignerResolutionKind::Denied,
                HostSignerResolutionKind::Submitted => SignerResolutionKind::Submitted,
                HostSignerResolutionKind::Signed => SignerResolutionKind::Signed,
                HostSignerResolutionKind::Expired => SignerResolutionKind::Expired,
            },
            resolved_at_ms: self.resolved_at_ms,
            submission_id: self.submission_id.map(|value| value.0),
            signed_payload: self.signed_payload,
        }
    }
}
