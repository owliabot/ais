//! Budget-aware stepping toward the next stable boundary.

use std::time::Instant;

use ais_agent_control::{
    events::RunEventEnvelope,
    recovery::{InterruptionClass, RunFailureCode, RunFailureStage},
};
use ais_agent_core::{
    actuation::ActuationKind,
    recovery::classify_side_effect_phase,
    runtime::{RunPhase, RunStatus},
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{debug, info, warn};

use crate::{
    events::RuntimeEventEmitter,
    persistence::{
        persist_boundary_checkpoint, persist_side_effect_checkpoint, CheckpointRepository,
        CheckpointRepositoryError,
    },
    runtime::ActiveRun,
    stepper::{StepOnce, StepTransition, StepTransitionKind},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepUntilBoundary {
    NextBoundary,
    CompleteOrBoundary,
    BudgetExhausted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct StepBudget {
    pub max_transitions: Option<u32>,
    pub max_wall_clock_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStopReason {
    StableBoundary,
    Completed,
    Failed,
    Cancelled,
    BudgetExhausted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepResult {
    pub stop_reason: StepStopReason,
    #[serde(default)]
    pub transitions: Vec<StepTransition>,
    #[serde(default)]
    pub events: Vec<RunEventEnvelope>,
    pub checkpoint_seq: u64,
    pub plan_epoch: u64,
    pub revision: u64,
}

#[derive(Debug, Error)]
pub enum StepSchedulerError {
    #[error(transparent)]
    Checkpoint(#[from] CheckpointRepositoryError),
}

#[derive(Debug, Default)]
pub struct StepScheduler;

impl StepScheduler {
    pub async fn step_until_boundary(
        runtime: &mut ActiveRun,
        checkpoint_repo: &mut impl CheckpointRepository,
        until: StepUntilBoundary,
        budget: StepBudget,
    ) -> Result<StepResult, StepSchedulerError> {
        let started_at = Instant::now();
        let mut transitions = Vec::new();
        let mut events = Vec::new();
        let max_transitions = budget.max_transitions.unwrap_or(64);
        let mut persisted_side_effect_cut = false;
        debug!(
            run_id = %runtime.run_id.0,
            until = ?until,
            max_transitions,
            max_wall_clock_ms = budget.max_wall_clock_ms,
            checkpoint_seq = runtime.checkpoint_seq(),
            plan_epoch = runtime.plan_epoch(),
            revision = runtime.revision,
            "runtime.scheduler.start"
        );

        loop {
            if transitions.len() as u32 >= max_transitions {
                record_budget_interruption(
                    runtime,
                    InterruptionClass::StepBudgetExhausted,
                    format!(
                        "step budget exhausted after {} transitions",
                        transitions.len()
                    ),
                );
                return finish_step_result(
                    runtime,
                    checkpoint_repo,
                    persisted_side_effect_cut,
                    StepStopReason::BudgetExhausted,
                    transitions,
                    events,
                );
            }

            if let Some(max_wall_clock_ms) = budget.max_wall_clock_ms {
                if started_at.elapsed().as_millis() as u64 >= max_wall_clock_ms {
                    record_budget_interruption(
                        runtime,
                        InterruptionClass::WallClockBudgetExhausted,
                        format!("wall clock budget exhausted after {max_wall_clock_ms} ms"),
                    );
                    return finish_step_result(
                        runtime,
                        checkpoint_repo,
                        persisted_side_effect_cut,
                        StepStopReason::BudgetExhausted,
                        transitions,
                        events,
                    );
                }
            }

            let before_revision = runtime.revision;
            let step = StepOnce::apply(runtime).await;

            match step.applied_transition {
                Some(transition) => {
                    persisted_side_effect_cut =
                        persist_side_effect_durability_cut(runtime, checkpoint_repo, &transition)?;
                    debug!(
                        run_id = %runtime.run_id.0,
                        transition_kind = ?transition.kind,
                        node_id = ?transition.node_id,
                        summary = %transition.summary,
                        checkpoint_seq = runtime.checkpoint_seq(),
                        plan_epoch = runtime.plan_epoch(),
                        revision = runtime.revision,
                        persisted_side_effect_cut,
                        "runtime.scheduler.transition_applied"
                    );
                    events.extend(RuntimeEventEmitter::emit_after_step(runtime, &transition));
                    transitions.push(transition);
                }
                None => {
                    if let Some(stop_reason) = stop_reason_for(runtime, until) {
                        return finish_step_result(
                            runtime,
                            checkpoint_repo,
                            persisted_side_effect_cut,
                            stop_reason,
                            transitions,
                            events,
                        );
                    }

                    warn!(
                        run_id = %runtime.run_id.0,
                        checkpoint_seq = runtime.checkpoint_seq(),
                        plan_epoch = runtime.plan_epoch(),
                        revision = runtime.revision,
                        "runtime.scheduler.stall_detected"
                    );
                    runtime.checkpoint.lifecycle.fail(
                        RunFailureStage::Recover,
                        RunFailureCode::RuntimeInvariantViolation,
                        "stepper could not make progress and no stable boundary was reached",
                    );
                    runtime.checkpoint.lifecycle.record_interruption(
                        InterruptionClass::RuntimeStallDetected,
                        Some(RunFailureStage::Recover),
                        classify_side_effect_phase(&runtime.checkpoint),
                        "stepper could not make progress and no stable boundary was reached",
                    );
                    runtime.touch_transition();
                    let synthetic = StepTransition {
                        kind: crate::stepper::StepTransitionKind::Recover,
                        node_id: None,
                        summary: "scheduler declared stalled runtime as failed".to_owned(),
                    };
                    events.extend(RuntimeEventEmitter::emit_after_step(runtime, &synthetic));
                    transitions.push(synthetic);
                    return finish_step_result(
                        runtime,
                        checkpoint_repo,
                        persisted_side_effect_cut,
                        StepStopReason::Failed,
                        transitions,
                        events,
                    );
                }
            }

            if let Some(stop_reason) = stop_reason_for(runtime, until) {
                return finish_step_result(
                    runtime,
                    checkpoint_repo,
                    persisted_side_effect_cut,
                    stop_reason,
                    transitions,
                    events,
                );
            }

            if runtime.revision == before_revision {
                warn!(
                    run_id = %runtime.run_id.0,
                    checkpoint_seq = runtime.checkpoint_seq(),
                    plan_epoch = runtime.plan_epoch(),
                    revision = runtime.revision,
                    "runtime.scheduler.revision_invariant_failed"
                );
                runtime.checkpoint.lifecycle.fail(
                    RunFailureStage::Recover,
                    RunFailureCode::RuntimeInvariantViolation,
                    "stepper transition completed without mutating runtime revision",
                );
                runtime.touch_transition();
                return finish_step_result(
                    runtime,
                    checkpoint_repo,
                    persisted_side_effect_cut,
                    StepStopReason::Failed,
                    transitions,
                    events,
                );
            }
        }
    }
}

fn record_budget_interruption(runtime: &mut ActiveRun, class: InterruptionClass, summary: String) {
    runtime.checkpoint.lifecycle.record_interruption(
        class,
        run_phase_to_failure_stage(runtime.checkpoint.lifecycle.phase.clone()),
        classify_side_effect_phase(&runtime.checkpoint),
        summary,
    );
    runtime.touch_transition();
}

fn run_phase_to_failure_stage(phase: RunPhase) -> Option<RunFailureStage> {
    match phase {
        RunPhase::MissionAccepted | RunPhase::Planning => Some(RunFailureStage::Derive),
        RunPhase::Simulating => Some(RunFailureStage::Simulate),
        RunPhase::Governing => Some(RunFailureStage::Govern),
        RunPhase::AwaitingHost => None,
        RunPhase::Broadcasting => Some(RunFailureStage::Broadcast),
        RunPhase::Verifying => Some(RunFailureStage::Verify),
        RunPhase::Recovering => Some(RunFailureStage::Recover),
        RunPhase::Finalized => None,
    }
}

fn finish_step_result(
    runtime: &ActiveRun,
    checkpoint_repo: &mut impl CheckpointRepository,
    persisted_side_effect_cut: bool,
    stop_reason: StepStopReason,
    transitions: Vec<StepTransition>,
    events: Vec<RunEventEnvelope>,
) -> Result<StepResult, StepSchedulerError> {
    if let Err(error) =
        persist_checkpoint_on_return(runtime, checkpoint_repo, persisted_side_effect_cut)
    {
        warn!(
            run_id = %runtime.run_id.0,
            stop_reason = ?stop_reason,
            checkpoint_seq = runtime.checkpoint_seq(),
            plan_epoch = runtime.plan_epoch(),
            revision = runtime.revision,
            error = %error,
            "runtime.scheduler.persist_on_return_failed"
        );
        return Err(error.into());
    }

    info!(
        run_id = %runtime.run_id.0,
        stop_reason = ?stop_reason,
        status = ?runtime.checkpoint.lifecycle.status,
        checkpoint_seq = runtime.checkpoint_seq(),
        plan_epoch = runtime.plan_epoch(),
        revision = runtime.revision,
        transition_count = transitions.len(),
        event_count = events.len(),
        persisted_side_effect_cut,
        "runtime.scheduler.stop"
    );

    Ok(step_result(runtime, stop_reason, transitions, events))
}

fn stop_reason_for(runtime: &ActiveRun, until: StepUntilBoundary) -> Option<StepStopReason> {
    match runtime.checkpoint.lifecycle.status {
        RunStatus::Completed => Some(StepStopReason::Completed),
        RunStatus::Failed => Some(StepStopReason::Failed),
        RunStatus::Cancelled => Some(StepStopReason::Cancelled),
        _ if runtime.checkpoint.lifecycle.is_stably_paused() => {
            Some(StepStopReason::StableBoundary)
        }
        _ => match until {
            StepUntilBoundary::NextBoundary
            | StepUntilBoundary::CompleteOrBoundary
            | StepUntilBoundary::BudgetExhausted => None,
        },
    }
}

fn persist_checkpoint_on_return(
    runtime: &ActiveRun,
    checkpoint_repo: &mut impl CheckpointRepository,
    persisted_side_effect_cut: bool,
) -> Result<(), CheckpointRepositoryError> {
    if persisted_side_effect_cut
        && runtime.checkpoint.lifecycle.status == RunStatus::AwaitingConfirmation
    {
        return Ok(());
    }

    persist_boundary_checkpoint(checkpoint_repo, runtime).map(|_| ())
}

fn persist_side_effect_durability_cut(
    runtime: &ActiveRun,
    checkpoint_repo: &mut impl CheckpointRepository,
    transition: &StepTransition,
) -> Result<bool, CheckpointRepositoryError> {
    if !requires_side_effect_durability_cut(runtime, transition) {
        return Ok(false);
    }

    match persist_side_effect_checkpoint(checkpoint_repo, runtime) {
        Ok(_) => {
            info!(
                run_id = %runtime.run_id.0,
                transition_kind = ?transition.kind,
                confirmation_id = runtime.checkpoint.pending_requests.pending_confirmation_id.as_deref(),
                checkpoint_seq = runtime.checkpoint_seq(),
                plan_epoch = runtime.plan_epoch(),
                "runtime.scheduler.side_effect_cut_persisted"
            );
            Ok(true)
        }
        Err(error) => {
            warn!(
                run_id = %runtime.run_id.0,
                transition_kind = ?transition.kind,
                error = %error,
                "runtime.scheduler.side_effect_cut_persist_failed"
            );
            Err(error)
        }
    }
}

fn requires_side_effect_durability_cut(runtime: &ActiveRun, transition: &StepTransition) -> bool {
    if !matches!(
        transition.kind,
        StepTransitionKind::Broadcast | StepTransitionKind::Signer
    ) {
        return false;
    }

    if runtime.checkpoint.lifecycle.status != RunStatus::AwaitingConfirmation {
        return false;
    }

    let Some(pending_confirmation_id) = runtime
        .checkpoint
        .pending_requests
        .pending_confirmation_id
        .as_deref()
    else {
        return false;
    };

    runtime
        .checkpoint
        .actuation_records
        .last()
        .is_some_and(|record| {
            matches!(record.kind, ActuationKind::BroadcastSubmitted)
                && record.tx_hash.as_deref() == Some(pending_confirmation_id)
        })
}

fn step_result(
    runtime: &ActiveRun,
    stop_reason: StepStopReason,
    transitions: Vec<StepTransition>,
    events: Vec<RunEventEnvelope>,
) -> StepResult {
    StepResult {
        stop_reason,
        transitions,
        events,
        checkpoint_seq: runtime.checkpoint_seq(),
        plan_epoch: runtime.plan_epoch(),
        revision: runtime.revision,
    }
}
