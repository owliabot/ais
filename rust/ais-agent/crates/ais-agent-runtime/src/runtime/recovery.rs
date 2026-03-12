use ais_agent_control::recovery::{
    validate_recovery_contract, RecoveryActionKind, RecoveryDisposition, RecoverySuggestion,
};
use ais_agent_core::{
    checkpoint::CheckpointSnapshot,
    recovery::{
        classify_allowed_recovery_actions as core_classify_allowed_recovery_actions,
        classify_recovery_disposition as core_classify_recovery_disposition,
        classify_recovery_suggestions as core_classify_recovery_suggestions,
        classify_recovery_view as core_classify_recovery_view,
    },
};
use ais_agent_host::inspect::RecoveryView;

pub fn classify_recovery_view(checkpoint: &CheckpointSnapshot) -> RecoveryView {
    let recovery = core_classify_recovery_view(checkpoint);
    RecoveryView {
        recovery_disposition: recovery.recovery_disposition,
        failure_context: recovery.failure_context,
        recovery_suggestions: recovery.recovery_suggestions,
        allowed_recovery_actions: recovery.allowed_recovery_actions,
        interruption_class: recovery.interruption_class,
        cancel_state: recovery.cancel_state,
        side_effect_phase: recovery.side_effect_phase,
    }
}

pub fn classify_validated_recovery_view(
    checkpoint: &CheckpointSnapshot,
) -> Result<RecoveryView, String> {
    validate_checkpoint_recovery_contract(checkpoint)?;
    Ok(classify_recovery_view(checkpoint))
}

pub fn classify_recovery_disposition(
    checkpoint: &CheckpointSnapshot,
) -> Option<RecoveryDisposition> {
    core_classify_recovery_disposition(checkpoint)
}

pub fn classify_allowed_recovery_actions(
    checkpoint: &CheckpointSnapshot,
) -> Vec<RecoveryActionKind> {
    core_classify_allowed_recovery_actions(checkpoint)
}

pub fn classify_recovery_suggestions(checkpoint: &CheckpointSnapshot) -> Vec<RecoverySuggestion> {
    core_classify_recovery_suggestions(checkpoint)
}

pub fn validate_checkpoint_recovery_contract(
    checkpoint: &CheckpointSnapshot,
) -> Result<(), String> {
    let recovery = core_classify_recovery_view(checkpoint);
    validate_recovery_contract(
        recovery.recovery_disposition.as_ref(),
        recovery.failure_context.as_ref(),
        &recovery.recovery_suggestions,
        &recovery.allowed_recovery_actions,
        checkpoint.checkpoint_seq,
        checkpoint.plan_epoch,
    )
}
