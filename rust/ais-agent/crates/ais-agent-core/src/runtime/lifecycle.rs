use serde::{Deserialize, Serialize};

use ais_agent_control::{
    ids::{RunId, SignerRequestId},
    recovery::{
        CancelState, InterruptionClass, InterruptionState, RunFailureCode, RunFailureContext,
        RunFailureStage, SideEffectPhase, StableBoundaryKind,
    },
};

use crate::runtime::{BoundaryKind, RunPhase, SignerRequestState, StableBoundary};

/// Stable run-level lifecycle states.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Created,
    Running,
    Paused,
    AwaitingEvidence,
    AwaitingSigner,
    AwaitingConfirmation,
    Completed,
    Failed,
    Cancelled,
}

pub type RuntimeFailure = RunFailureContext;
pub type RuntimeInterruption = InterruptionState;

/// Runtime-owned lifecycle state.
///
/// This is the canonical source for coarse execution state and explicit stable
/// boundaries. Higher layers should project from this state instead of
/// inventing their own lifecycle truth.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunLifecycleState {
    pub run_id: RunId,
    pub mission_id: String,
    pub status: RunStatus,
    pub phase: RunPhase,
    pub checkpoint_seq: u64,
    pub plan_epoch: u64,
    pub active_boundary: Option<StableBoundary>,
    pub failure: Option<RuntimeFailure>,
    #[serde(default)]
    pub interruption: Option<RuntimeInterruption>,
    #[serde(default)]
    pub cancel_state: Option<CancelState>,
    pub cancelled_reason: Option<String>,
}

impl RunLifecycleState {
    pub fn new(run_id: RunId, mission_id: impl Into<String>) -> Self {
        Self {
            run_id,
            mission_id: mission_id.into(),
            status: RunStatus::Created,
            phase: RunPhase::MissionAccepted,
            checkpoint_seq: 0,
            plan_epoch: 0,
            active_boundary: None,
            failure: None,
            interruption: None,
            cancel_state: None,
            cancelled_reason: None,
        }
    }

    pub fn mark_running(&mut self, phase: RunPhase) {
        self.status = RunStatus::Running;
        self.phase = phase;
        self.active_boundary = None;
        self.failure = None;
        self.interruption = None;
    }

    pub fn pause(&mut self, summary: impl Into<String>) {
        self.status = RunStatus::Paused;
        self.phase = RunPhase::AwaitingHost;
        self.interruption = None;
        self.active_boundary = Some(StableBoundary {
            kind: BoundaryKind::Pause,
            summary: summary.into(),
            blocking_refs: Vec::new(),
            signer_request_id: None,
        });
    }

    pub fn pause_with_failure(
        &mut self,
        stage: RunFailureStage,
        code: RunFailureCode,
        message: impl Into<String>,
    ) {
        let message = message.into();
        self.status = RunStatus::Paused;
        self.phase = RunPhase::AwaitingHost;
        self.failure = Some(RuntimeFailure::new(
            code,
            stage,
            self.checkpoint_seq,
            self.plan_epoch,
            Some(StableBoundaryKind::Pause),
            message.clone(),
        ));
        self.active_boundary = Some(StableBoundary {
            kind: BoundaryKind::Pause,
            summary: message,
            blocking_refs: Vec::new(),
            signer_request_id: None,
        });
        self.interruption = None;
    }

    pub fn await_evidence(&mut self, summary: impl Into<String>, blocking_refs: Vec<String>) {
        self.status = RunStatus::AwaitingEvidence;
        self.phase = RunPhase::AwaitingHost;
        self.interruption = None;
        self.active_boundary = Some(StableBoundary {
            kind: BoundaryKind::Evidence,
            summary: summary.into(),
            blocking_refs,
            signer_request_id: None,
        });
    }

    pub fn await_evidence_with_failure(
        &mut self,
        stage: RunFailureStage,
        code: RunFailureCode,
        summary: impl Into<String>,
        blocking_refs: Vec<String>,
    ) {
        let summary = summary.into();
        self.status = RunStatus::AwaitingEvidence;
        self.phase = RunPhase::AwaitingHost;
        self.failure = Some(RuntimeFailure::new(
            code,
            stage,
            self.checkpoint_seq,
            self.plan_epoch,
            Some(StableBoundaryKind::Evidence),
            summary.clone(),
        ));
        self.active_boundary = Some(StableBoundary {
            kind: BoundaryKind::Evidence,
            summary,
            blocking_refs,
            signer_request_id: None,
        });
        self.interruption = None;
    }

    pub fn await_signer(&mut self, summary: impl Into<String>, signer_request_id: SignerRequestId) {
        self.status = RunStatus::AwaitingSigner;
        self.phase = RunPhase::AwaitingHost;
        self.interruption = None;
        self.active_boundary = Some(StableBoundary {
            kind: BoundaryKind::Signer,
            summary: summary.into(),
            blocking_refs: Vec::new(),
            signer_request_id: Some(signer_request_id),
        });
    }

    pub fn await_signer_request(&mut self, request: &SignerRequestState) {
        self.await_signer(request.summary.clone(), request.request_id.clone());
    }

    pub fn resolve_signer_wait(&mut self, phase: RunPhase) {
        self.status = RunStatus::Running;
        self.phase = phase;
        self.active_boundary = None;
        self.interruption = None;
    }

    pub fn await_confirmation(&mut self, summary: impl Into<String>) {
        self.status = RunStatus::AwaitingConfirmation;
        self.phase = RunPhase::AwaitingHost;
        self.interruption = None;
        self.active_boundary = Some(StableBoundary {
            kind: BoundaryKind::Confirmation,
            summary: summary.into(),
            blocking_refs: Vec::new(),
            signer_request_id: None,
        });
    }

    pub fn resolve_confirmation_wait(&mut self, phase: RunPhase) {
        self.status = RunStatus::Running;
        self.phase = phase;
        self.active_boundary = None;
        self.interruption = None;
    }

    pub fn complete(&mut self, summary: impl Into<String>) {
        self.status = RunStatus::Completed;
        self.phase = RunPhase::Finalized;
        self.active_boundary = Some(StableBoundary {
            kind: BoundaryKind::Completion,
            summary: summary.into(),
            blocking_refs: Vec::new(),
            signer_request_id: None,
        });
        self.failure = None;
        self.interruption = None;
        self.cancel_state = None;
        self.cancelled_reason = None;
    }

    pub fn fail(
        &mut self,
        stage: RunFailureStage,
        code: RunFailureCode,
        message: impl Into<String>,
    ) {
        self.status = RunStatus::Failed;
        self.phase = RunPhase::Finalized;
        let message = message.into();
        self.failure = Some(RuntimeFailure::new(
            code,
            stage,
            self.checkpoint_seq,
            self.plan_epoch,
            Some(StableBoundaryKind::Failure),
            message.clone(),
        ));
        self.active_boundary = Some(StableBoundary {
            kind: BoundaryKind::Failure,
            summary: message,
            blocking_refs: Vec::new(),
            signer_request_id: None,
        });
        self.interruption = None;
        self.cancel_state = None;
        self.cancelled_reason = None;
    }

    pub fn cancel(&mut self, reason: impl Into<String>) {
        let reason = reason.into();
        self.status = RunStatus::Cancelled;
        self.phase = RunPhase::Finalized;
        self.cancelled_reason = Some(reason.clone());
        self.active_boundary = Some(StableBoundary {
            kind: BoundaryKind::Cancellation,
            summary: reason,
            blocking_refs: Vec::new(),
            signer_request_id: None,
        });
        self.failure = None;
        self.interruption = None;
        self.cancel_state = Some(CancelState::Cancelled);
    }

    pub fn request_cancel_pending(
        &mut self,
        reason: impl Into<String>,
        side_effect_phase: Option<SideEffectPhase>,
    ) {
        let reason = reason.into();
        self.cancel_state = Some(CancelState::Pending);
        self.cancelled_reason = Some(reason.clone());
        self.record_interruption(
            InterruptionClass::HostCancelRequested,
            None,
            side_effect_phase,
            reason,
        );
    }

    pub fn record_interruption(
        &mut self,
        class: InterruptionClass,
        stage: Option<RunFailureStage>,
        side_effect_phase: Option<SideEffectPhase>,
        summary: impl Into<String>,
    ) {
        self.interruption = Some(RuntimeInterruption {
            class,
            stage,
            side_effect_phase,
            summary: summary.into(),
        });
    }

    pub fn bump_checkpoint(&mut self) {
        self.checkpoint_seq = self.checkpoint_seq.saturating_add(1);
    }

    pub fn bump_plan_epoch(&mut self) {
        self.plan_epoch = self.plan_epoch.saturating_add(1);
    }

    pub fn is_stably_paused(&self) -> bool {
        matches!(
            self.status,
            RunStatus::Paused
                | RunStatus::AwaitingEvidence
                | RunStatus::AwaitingSigner
                | RunStatus::AwaitingConfirmation
        )
    }
}
