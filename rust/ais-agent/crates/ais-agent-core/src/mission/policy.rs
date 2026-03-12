use serde::{Deserialize, Serialize};

/// Runtime-owned mission policy knobs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MissionPolicy {
    pub policy_mode: Option<String>,
    pub allow_raw_envelopes: bool,
    pub require_effect_contract_for_writes: bool,
}

impl Default for MissionPolicy {
    fn default() -> Self {
        Self {
            policy_mode: None,
            allow_raw_envelopes: true,
            require_effect_contract_for_writes: true,
        }
    }
}
