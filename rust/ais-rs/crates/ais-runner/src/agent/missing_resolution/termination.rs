use super::policy::MissingResolutionDecision;
use ais_core::{stable_hash_hex, StableJsonOptions};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct MissingResolutionTerminationPolicy {
    pub max_no_progress_rounds: usize,
    pub max_same_decision_hash_rounds: usize,
    pub max_total_attempts: usize,
}

impl Default for MissingResolutionTerminationPolicy {
    fn default() -> Self {
        Self {
            max_no_progress_rounds: 1,
            max_same_decision_hash_rounds: 2,
            max_total_attempts: 6,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub(crate) struct MissingResolutionTerminationState {
    pub no_progress_rounds: usize,
    pub same_decision_hash_rounds: usize,
    pub total_attempts: usize,
    pub last_decision_hash: Option<String>,
}

pub(crate) fn observe_missing_resolution_round_progress(
    state: &mut MissingResolutionTerminationState,
    policy: MissingResolutionTerminationPolicy,
    decisions: &[MissingResolutionDecision],
    made_progress: bool,
    attempts_in_round: usize,
) -> Option<String> {
    state.total_attempts = state.total_attempts.saturating_add(attempts_in_round);
    if policy.max_total_attempts > 0 && state.total_attempts >= policy.max_total_attempts {
        return Some("max_total_attempts".to_string());
    }

    let decision_hash = decision_hash(decisions);
    if made_progress {
        state.no_progress_rounds = 0;
        state.same_decision_hash_rounds = 0;
        state.last_decision_hash = Some(decision_hash);
        return None;
    }

    if state.last_decision_hash.as_deref() == Some(decision_hash.as_str()) {
        state.same_decision_hash_rounds = state.same_decision_hash_rounds.saturating_add(1);
    } else {
        state.same_decision_hash_rounds = 1;
        state.last_decision_hash = Some(decision_hash);
    }
    if policy.max_same_decision_hash_rounds > 0
        && state.same_decision_hash_rounds >= policy.max_same_decision_hash_rounds
    {
        return Some("same_decision_hash_limit".to_string());
    }

    state.no_progress_rounds = state.no_progress_rounds.saturating_add(1);
    if policy.max_no_progress_rounds > 0
        && state.no_progress_rounds >= policy.max_no_progress_rounds
    {
        return Some("max_no_progress_rounds".to_string());
    }
    None
}

fn decision_hash(decisions: &[MissingResolutionDecision]) -> String {
    let payload = serde_json::to_value(decisions).unwrap_or_default();
    stable_hash_hex(&payload, &StableJsonOptions::default()).unwrap_or_default()
}

#[cfg(test)]
#[path = "../tests/missing_resolution_termination.rs"]
mod tests;
