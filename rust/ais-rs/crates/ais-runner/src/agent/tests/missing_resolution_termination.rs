use super::*;
use crate::agent::missing_resolution::policy::MissingResolutionDecision;
use crate::agent::ref_model::RefPath;

fn sample_decisions() -> Vec<MissingResolutionDecision> {
    vec![MissingResolutionDecision::RunProducer {
        target: RefPath::Input {
            slot: "token.decimals".to_string(),
        },
        query_ref: "erc20@0.0.2/decimals".to_string(),
    }]
}

#[test]
fn observe_round_progress_stops_on_same_hash_limit_only_without_progress() {
    let policy = MissingResolutionTerminationPolicy {
        max_no_progress_rounds: 3,
        max_same_decision_hash_rounds: 2,
        max_total_attempts: 100,
    };
    let mut state = MissingResolutionTerminationState::default();
    assert!(observe_missing_resolution_round_progress(
        &mut state,
        policy,
        sample_decisions().as_slice(),
        false,
        1
    )
    .is_none());
    let reason = observe_missing_resolution_round_progress(
        &mut state,
        policy,
        sample_decisions().as_slice(),
        false,
        1,
    );
    assert_eq!(reason, Some("same_decision_hash_limit".to_string()));
}

#[test]
fn observe_round_progress_tracks_no_progress_limit() {
    let policy = MissingResolutionTerminationPolicy {
        max_no_progress_rounds: 1,
        max_same_decision_hash_rounds: 9,
        max_total_attempts: 100,
    };
    let mut state = MissingResolutionTerminationState::default();
    let reason = observe_missing_resolution_round_progress(
        &mut state,
        policy,
        sample_decisions().as_slice(),
        false,
        1,
    );
    assert_eq!(reason, Some("max_no_progress_rounds".to_string()));
}

#[test]
fn observe_round_progress_tracks_total_attempt_limit() {
    let policy = MissingResolutionTerminationPolicy {
        max_no_progress_rounds: 9,
        max_same_decision_hash_rounds: 9,
        max_total_attempts: 4,
    };
    let mut state = MissingResolutionTerminationState::default();
    assert!(observe_missing_resolution_round_progress(
        &mut state,
        policy,
        sample_decisions().as_slice(),
        true,
        2
    )
    .is_none());
    let reason = observe_missing_resolution_round_progress(
        &mut state,
        policy,
        sample_decisions().as_slice(),
        true,
        2,
    );
    assert_eq!(reason, Some("max_total_attempts".to_string()));
}

#[test]
fn observe_round_progress_resets_same_hash_counter_after_progress() {
    let policy = MissingResolutionTerminationPolicy {
        max_no_progress_rounds: 9,
        max_same_decision_hash_rounds: 2,
        max_total_attempts: 100,
    };
    let mut state = MissingResolutionTerminationState::default();
    assert!(observe_missing_resolution_round_progress(
        &mut state,
        policy,
        sample_decisions().as_slice(),
        false,
        1,
    )
    .is_none());
    assert!(observe_missing_resolution_round_progress(
        &mut state,
        policy,
        sample_decisions().as_slice(),
        true,
        1,
    )
    .is_none());
    assert!(observe_missing_resolution_round_progress(
        &mut state,
        policy,
        sample_decisions().as_slice(),
        false,
        1,
    )
    .is_none());
}
