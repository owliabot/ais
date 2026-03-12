use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use ais_agent_control::ids::{RunId, SignerRequestId};
use ais_agent_core::runtime::{SignerDecision, SignerDecisionKind};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostSignerDecisionKind {
    Approved,
    Denied,
    Submitted,
    Expired,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostSignerDecision {
    pub run_id: RunId,
    pub request_id: SignerRequestId,
    pub decision: HostSignerDecisionKind,
    pub decided_at_ms: Option<u64>,
    pub tx_hash: Option<String>,
    #[serde(default)]
    pub details: BTreeMap<String, Value>,
}

impl HostSignerDecision {
    pub fn into_runtime_decision(self) -> SignerDecision {
        SignerDecision {
            request_id: self.request_id,
            kind: match self.decision {
                HostSignerDecisionKind::Approved => SignerDecisionKind::Approved,
                HostSignerDecisionKind::Denied => SignerDecisionKind::Denied,
                HostSignerDecisionKind::Submitted => SignerDecisionKind::Submitted,
                HostSignerDecisionKind::Expired => SignerDecisionKind::Expired,
            },
            decision_at_ms: self.decided_at_ms,
            tx_hash: self.tx_hash,
        }
    }
}
