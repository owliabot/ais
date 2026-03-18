use serde::{Deserialize, Serialize};
use serde_json::Value;

use ais_agent_control::ids::{RunId, SignerRequestId};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignerRequestStatus {
    Pending,
    Denied,
    Submitted,
    Signed,
    Expired,
    TimedOut,
    Reconciled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignerResolutionKind {
    Denied,
    Submitted,
    Signed,
    Expired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignerTimeout {
    pub requested_at_ms: u64,
    pub expires_at_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignerResolution {
    pub request_id: SignerRequestId,
    pub kind: SignerResolutionKind,
    pub resolved_at_ms: Option<u64>,
    pub tx_hash: Option<String>,
    pub signed_payload: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignerRequestState {
    pub request_id: SignerRequestId,
    pub run_id: RunId,
    pub node_id: Option<String>,
    pub chain: String,
    pub summary: String,
    #[serde(default)]
    pub payload: Option<Value>,
    pub status: SignerRequestStatus,
    pub timeout: Option<SignerTimeout>,
    pub last_resolution: Option<SignerResolution>,
    pub submitted_tx_hash: Option<String>,
    #[serde(default)]
    pub signed_payload: Option<Value>,
    pub reconcile_required: bool,
}

impl SignerRequestState {
    pub fn new_pending(
        request_id: SignerRequestId,
        run_id: RunId,
        chain: impl Into<String>,
        summary: impl Into<String>,
    ) -> Self {
        Self {
            request_id,
            run_id,
            node_id: None,
            chain: chain.into(),
            summary: summary.into(),
            payload: None,
            status: SignerRequestStatus::Pending,
            timeout: None,
            last_resolution: None,
            submitted_tx_hash: None,
            signed_payload: None,
            reconcile_required: false,
        }
    }

    pub fn with_node_id(mut self, node_id: impl Into<String>) -> Self {
        self.node_id = Some(node_id.into());
        self
    }

    pub fn with_timeout(mut self, requested_at_ms: u64, expires_at_ms: Option<u64>) -> Self {
        self.timeout = Some(SignerTimeout {
            requested_at_ms,
            expires_at_ms,
        });
        self
    }

    pub fn with_payload(mut self, payload: Value) -> Self {
        self.payload = Some(payload);
        self
    }

    pub fn apply_resolution(&mut self, resolution: SignerResolution) {
        self.status = match resolution.kind {
            SignerResolutionKind::Denied => SignerRequestStatus::Denied,
            SignerResolutionKind::Submitted => SignerRequestStatus::Submitted,
            SignerResolutionKind::Signed => SignerRequestStatus::Signed,
            SignerResolutionKind::Expired => SignerRequestStatus::Expired,
        };
        if matches!(resolution.kind, SignerResolutionKind::Submitted) {
            self.submitted_tx_hash = resolution.tx_hash.clone();
            self.reconcile_required = true;
        }
        if matches!(resolution.kind, SignerResolutionKind::Signed) {
            self.signed_payload = resolution.signed_payload.clone();
        }
        self.last_resolution = Some(resolution);
    }

    pub fn mark_timed_out(&mut self, now_ms: u64) -> bool {
        let Some(timeout) = &self.timeout else {
            return false;
        };
        let Some(expires_at_ms) = timeout.expires_at_ms else {
            return false;
        };
        if now_ms > expires_at_ms && matches!(self.status, SignerRequestStatus::Pending) {
            self.status = SignerRequestStatus::TimedOut;
            return true;
        }
        false
    }

    pub fn mark_reconciled(&mut self, observed_tx_hash: Option<String>) {
        if let Some(tx_hash) = observed_tx_hash {
            self.submitted_tx_hash = Some(tx_hash);
        }
        self.status = SignerRequestStatus::Reconciled;
        self.reconcile_required = false;
    }
}
