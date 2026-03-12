use serde::{Deserialize, Serialize};

use ais_agent_control::ids::{RunId, SignerRequestId};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignerRequestStatus {
    Pending,
    Approved,
    Denied,
    Submitted,
    Expired,
    TimedOut,
    Reconciled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignerDecisionKind {
    Approved,
    Denied,
    Submitted,
    Expired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignerTimeout {
    pub requested_at_ms: u64,
    pub expires_at_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignerDecision {
    pub request_id: SignerRequestId,
    pub kind: SignerDecisionKind,
    pub decision_at_ms: Option<u64>,
    pub tx_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignerRequestState {
    pub request_id: SignerRequestId,
    pub run_id: RunId,
    pub node_id: Option<String>,
    pub chain: String,
    pub summary: String,
    pub status: SignerRequestStatus,
    pub timeout: Option<SignerTimeout>,
    pub last_decision: Option<SignerDecision>,
    pub submitted_tx_hash: Option<String>,
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
            status: SignerRequestStatus::Pending,
            timeout: None,
            last_decision: None,
            submitted_tx_hash: None,
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

    pub fn apply_decision(&mut self, decision: SignerDecision) {
        self.status = match decision.kind {
            SignerDecisionKind::Approved => SignerRequestStatus::Approved,
            SignerDecisionKind::Denied => SignerRequestStatus::Denied,
            SignerDecisionKind::Submitted => SignerRequestStatus::Submitted,
            SignerDecisionKind::Expired => SignerRequestStatus::Expired,
        };
        if matches!(decision.kind, SignerDecisionKind::Submitted) {
            self.submitted_tx_hash = decision.tx_hash.clone();
            self.reconcile_required = true;
        }
        self.last_decision = Some(decision);
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
