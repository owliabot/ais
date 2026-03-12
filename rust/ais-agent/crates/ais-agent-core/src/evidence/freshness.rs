use serde::{Deserialize, Serialize};

/// Runtime-owned freshness metadata for an evidence record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct EvidenceFreshness {
    pub observed_at_ms: Option<u64>,
    pub expires_at_ms: Option<u64>,
    pub max_age_ms: Option<u64>,
}

impl EvidenceFreshness {
    pub fn is_stale_at(&self, now_ms: u64) -> bool {
        if let Some(expires_at_ms) = self.expires_at_ms {
            if now_ms > expires_at_ms {
                return true;
            }
        }

        match (self.observed_at_ms, self.max_age_ms) {
            (Some(observed_at_ms), Some(max_age_ms)) => {
                now_ms.saturating_sub(observed_at_ms) > max_age_ms
            }
            _ => false,
        }
    }
}
