use std::collections::BTreeMap;

use ais_agent_control::{
    events::RunEvent,
    ids::{CommandId, RunId},
    recovery::InterruptionClass,
};
use ais_agent_core::{
    action::{
        kinds::{
            actuate::{ActuateAction, ActuateMode},
            derive::{DeriveAction, DeriveKind},
            simulate::{SimulateAction, SimulateKind},
            verify::{VerifyAction, VerifyKind},
        },
        ActionGraph, ActionNode, ActionNodeKind, ActionNodeStatus, ActionOrigin, ActionPayload,
    },
    checkpoint::{CheckpointSnapshot, PendingRequestsSnapshot},
    evidence::EvidenceGraph,
    mission::{Mission, MissionBudget, MissionPolicy},
    runtime::{RunLifecycleState, RunPhase, RunStatus, SignerRequestState, SignerRequestStatus},
};

use crate::{
    persistence::{CheckpointArchiveKind, CheckpointRepository, InMemoryCheckpointRepository},
    runtime::ActiveRun,
    stepper::{StepBudget, StepScheduler, StepStopReason, StepUntilBoundary},
    tests::tracing_capture::{capture_tracing_output, capture_tracing_output_at_level},
};

#[tokio::test]
async fn scheduler_stops_at_stable_boundary_and_persists_checkpoint() {
    let mission = sample_mission();
    let checkpoint = checkpoint_with_nodes(vec![
        simulate_succeeded_node("simulate-swap"),
        actuate_node("swap", vec!["simulate-swap"]),
    ]);
    let mut runtime = ActiveRun::new(mission, checkpoint);
    let mut repo = InMemoryCheckpointRepository::default();

    let result = StepScheduler::step_until_boundary(
        &mut runtime,
        &mut repo,
        StepUntilBoundary::NextBoundary,
        StepBudget {
            max_transitions: Some(8),
            max_wall_clock_ms: None,
        },
    )
    .await
    .expect("scheduler should succeed");

    assert_eq!(result.stop_reason, StepStopReason::StableBoundary);
    assert_eq!(
        runtime.checkpoint.lifecycle.status,
        RunStatus::AwaitingSigner
    );
    let persisted = repo
        .latest(runtime.run_id.0.as_str())
        .expect("persisted checkpoint");
    assert_eq!(persisted.lifecycle.status, RunStatus::AwaitingSigner);
    assert_eq!(
        persisted
            .pending_requests
            .pending_signer_request_id
            .is_some(),
        true
    );
    assert_eq!(result.events.len(), 4);
    assert_eq!(result.events[0].event_seq, 1);
    assert_eq!(result.events[3].event_seq, 4);
    assert!(result.events.iter().all(|event| {
        event.checkpoint_seq == result.events[0].checkpoint_seq
            && event.plan_epoch == result.events[0].plan_epoch
    }));
    assert!(result.events.iter().any(|event| matches!(
        event.event,
        RunEvent::GovernorDecision(ref audit)
            if audit.decision == ais_agent_control::events::GovernorDecisionAuditKind::AllowWithSigner
    )));
    assert!(result.events.iter().any(|event| matches!(
        event.event,
        RunEvent::RecoveryAudit(ref audit)
            if audit.recovery_disposition
                == Some(ais_agent_control::recovery::RecoveryDisposition::AwaitSigner)
    )));
}

#[tokio::test]
async fn scheduler_stops_on_budget_exhaustion_and_persists_progress() {
    let mission = sample_mission();
    let checkpoint = checkpoint_with_nodes(vec![
        derive_node("derive-a", vec![]),
        derive_node("derive-b", vec![]),
    ]);
    let mut runtime = ActiveRun::new(mission, checkpoint);
    let mut repo = InMemoryCheckpointRepository::default();

    let result = StepScheduler::step_until_boundary(
        &mut runtime,
        &mut repo,
        StepUntilBoundary::BudgetExhausted,
        StepBudget {
            max_transitions: Some(1),
            max_wall_clock_ms: None,
        },
    )
    .await
    .expect("scheduler should succeed");

    assert_eq!(result.stop_reason, StepStopReason::BudgetExhausted);
    assert_eq!(result.transitions.len(), 1);
    assert_eq!(
        runtime
            .checkpoint
            .action_graph
            .nodes
            .get("derive-a")
            .map(|node| node.status.clone()),
        Some(ActionNodeStatus::Succeeded)
    );
    let persisted = repo
        .latest(runtime.run_id.0.as_str())
        .expect("persisted checkpoint");
    assert_eq!(persisted.checkpoint_seq, runtime.checkpoint_seq());
    assert_eq!(
        persisted
            .lifecycle
            .interruption
            .as_ref()
            .map(|interruption| interruption.class.clone()),
        Some(InterruptionClass::StepBudgetExhausted)
    );
}

#[tokio::test]
async fn scheduler_stops_on_wall_clock_budget_without_advancing_runtime() {
    let mission = sample_mission();
    let checkpoint = checkpoint_with_nodes(vec![derive_node("derive-a", vec![])]);
    let mut runtime = ActiveRun::new(mission, checkpoint);
    let mut repo = InMemoryCheckpointRepository::default();

    let result = StepScheduler::step_until_boundary(
        &mut runtime,
        &mut repo,
        StepUntilBoundary::BudgetExhausted,
        StepBudget {
            max_transitions: Some(8),
            max_wall_clock_ms: Some(0),
        },
    )
    .await
    .expect("scheduler should stop on wall clock budget");

    assert_eq!(result.stop_reason, StepStopReason::BudgetExhausted);
    assert!(result.transitions.is_empty());
    assert_eq!(runtime.checkpoint_seq(), 1);
    assert_eq!(
        runtime
            .checkpoint
            .lifecycle
            .interruption
            .as_ref()
            .map(|interruption| interruption.class.clone()),
        Some(InterruptionClass::WallClockBudgetExhausted)
    );
    assert_eq!(
        runtime
            .checkpoint
            .action_graph
            .nodes
            .get("derive-a")
            .map(|node| node.status.clone()),
        Some(ActionNodeStatus::Pending)
    );
    let persisted = repo
        .latest(runtime.run_id.0.as_str())
        .expect("persisted checkpoint");
    assert_eq!(persisted.checkpoint_seq, 1);
    assert_eq!(
        persisted
            .action_graph
            .nodes
            .get("derive-a")
            .map(|node| node.status.clone()),
        Some(ActionNodeStatus::Pending)
    );
    assert_eq!(
        persisted
            .lifecycle
            .interruption
            .as_ref()
            .map(|interruption| interruption.class.clone()),
        Some(InterruptionClass::WallClockBudgetExhausted)
    );
}

#[tokio::test]
async fn scheduler_completes_and_persists_final_checkpoint() {
    let mission = sample_mission();
    let checkpoint = checkpoint_with_terminal_nodes(
        vec![verify_node("verify-swap", vec![])],
        vec!["verify-swap".to_owned()],
    );
    let mut runtime = ActiveRun::new(mission, checkpoint);
    let mut repo = InMemoryCheckpointRepository::default();

    let result = StepScheduler::step_until_boundary(
        &mut runtime,
        &mut repo,
        StepUntilBoundary::CompleteOrBoundary,
        StepBudget {
            max_transitions: Some(8),
            max_wall_clock_ms: None,
        },
    )
    .await
    .expect("scheduler should succeed");

    assert_eq!(result.stop_reason, StepStopReason::Completed);
    assert_eq!(runtime.checkpoint.lifecycle.status, RunStatus::Completed);
    let persisted = repo
        .latest(runtime.run_id.0.as_str())
        .expect("persisted checkpoint");
    assert_eq!(persisted.lifecycle.status, RunStatus::Completed);
}

#[tokio::test]
async fn scheduler_marks_failed_when_no_progress_is_possible() {
    let mission = sample_mission();
    let checkpoint = checkpoint_with_nodes(vec![actuate_node("swap", vec!["missing-simulate"])]);
    let mut runtime = ActiveRun::new(mission, checkpoint);
    let mut repo = InMemoryCheckpointRepository::default();

    let result = StepScheduler::step_until_boundary(
        &mut runtime,
        &mut repo,
        StepUntilBoundary::NextBoundary,
        StepBudget {
            max_transitions: Some(4),
            max_wall_clock_ms: None,
        },
    )
    .await
    .expect("scheduler should return failed result");

    assert_eq!(result.stop_reason, StepStopReason::Failed);
    assert_eq!(runtime.checkpoint.lifecycle.status, RunStatus::Failed);
    let failure = runtime
        .checkpoint
        .lifecycle
        .failure
        .as_ref()
        .expect("typed failure context");
    assert_eq!(
        failure.code,
        ais_agent_control::recovery::RunFailureCode::RuntimeInvariantViolation
    );
    assert_eq!(
        failure.stage,
        ais_agent_control::recovery::RunFailureStage::Recover
    );
    assert!(result.transitions.iter().any(|transition| {
        transition.kind == crate::stepper::StepTransitionKind::Recover
            && transition.summary == "scheduler declared stalled runtime as failed"
    }));
    let persisted = repo
        .latest(runtime.run_id.0.as_str())
        .expect("persisted checkpoint");
    assert_eq!(persisted.lifecycle.status, RunStatus::Failed);
    assert_eq!(
        persisted
            .lifecycle
            .interruption
            .as_ref()
            .map(|interruption| interruption.class.clone()),
        Some(InterruptionClass::RuntimeStallDetected)
    );
}

#[tokio::test]
async fn scheduler_persists_side_effect_cut_when_signer_submission_enters_confirmation_wait() {
    let mission = sample_mission();
    let mut checkpoint = checkpoint_with_terminal_nodes(
        vec![
            actuate_node("swap", vec![]),
            verify_node("verify-swap", vec!["swap"]),
        ],
        vec!["verify-swap".to_owned()],
    );
    checkpoint.pending_requests.pending_signer_request_id = Some("signer-1".to_owned());
    checkpoint
        .lifecycle
        .await_signer("await signer", "signer-1".into());

    let mut runtime = ActiveRun::new(mission, checkpoint);
    runtime.pending_signer_state = Some(
        SignerRequestState::new_pending(
            "signer-1".into(),
            RunId("run-1".to_owned()),
            "eip155:1",
            "sign swap",
        )
        .with_node_id("swap"),
    );
    runtime
        .pending_signer_state
        .as_mut()
        .expect("signer state")
        .status = SignerRequestStatus::Submitted;
    runtime
        .pending_signer_state
        .as_mut()
        .expect("signer state")
        .submitted_submission_id = Some("0xabc".to_owned());

    let mut repo = InMemoryCheckpointRepository::default();
    let result = StepScheduler::step_until_boundary(
        &mut runtime,
        &mut repo,
        StepUntilBoundary::NextBoundary,
        StepBudget {
            max_transitions: Some(4),
            max_wall_clock_ms: None,
        },
    )
    .await
    .expect("scheduler should succeed");

    assert_eq!(result.stop_reason, StepStopReason::StableBoundary);
    assert_eq!(
        runtime.checkpoint.lifecycle.status,
        RunStatus::AwaitingConfirmation
    );
    let history = repo.history("run-1").expect("checkpoint history");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].kind, CheckpointArchiveKind::SideEffect);
    assert_eq!(
        history[0]
            .snapshot
            .pending_requests
            .pending_submission_id
            .as_deref(),
        Some("0xabc")
    );
}

#[test]
fn scheduler_emits_tracing_for_transition_and_stop() {
    let (output, result) = capture_tracing_output(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
            .block_on(async {
                let mission = sample_mission();
                let checkpoint = checkpoint_with_nodes(vec![
                    simulate_succeeded_node("simulate-swap"),
                    actuate_node("swap", vec!["simulate-swap"]),
                ]);
                let mut runtime = ActiveRun::new(mission, checkpoint);
                runtime.record_command(CommandId("cmd-scheduler-trace".to_owned()), None);
                let mut repo = InMemoryCheckpointRepository::default();

                StepScheduler::step_until_boundary(
                    &mut runtime,
                    &mut repo,
                    StepUntilBoundary::NextBoundary,
                    StepBudget {
                        max_transitions: Some(8),
                        max_wall_clock_ms: None,
                    },
                )
                .await
                .expect("scheduler should succeed")
            })
    });

    assert_eq!(result.stop_reason, StepStopReason::StableBoundary);
    assert!(!output.trim().is_empty());
    assert!(output.contains("run.awaiting_signer"));
}

#[test]
fn scheduler_info_logs_prefer_operator_events_over_scheduler_plumbing() {
    let (output, result) = capture_tracing_output_at_level(tracing::Level::INFO, || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
            .block_on(async {
                let mission = sample_mission();
                let checkpoint = checkpoint_with_nodes(vec![
                    simulate_succeeded_node("simulate-swap"),
                    actuate_node("swap", vec!["simulate-swap"]),
                ]);
                let mut runtime = ActiveRun::new(mission, checkpoint);
                runtime.record_command(CommandId("cmd-scheduler-info".to_owned()), None);
                let mut repo = InMemoryCheckpointRepository::default();

                StepScheduler::step_until_boundary(
                    &mut runtime,
                    &mut repo,
                    StepUntilBoundary::NextBoundary,
                    StepBudget {
                        max_transitions: Some(8),
                        max_wall_clock_ms: None,
                    },
                )
                .await
                .expect("scheduler should succeed")
            })
    });

    assert_eq!(result.stop_reason, StepStopReason::StableBoundary);
    assert!(!output.contains("runtime.scheduler.stop"));
    assert!(!output.contains("runtime.scheduler.side_effect_cut_persisted"));
    assert!(output.contains("run.awaiting_signer"));
}

fn sample_mission() -> Mission {
    Mission {
        mission_id: "mission-1".to_owned(),
        goal: "swap usdc to eth".to_owned(),
        allowed_chains: vec!["eip155:1".to_owned()],
        budget: MissionBudget {
            max_steps: Some(16),
            max_signer_requests: Some(2),
            max_wall_clock_ms: Some(60_000),
        },
        policy: MissionPolicy {
            policy_mode: Some("guarded".to_owned()),
            allow_raw_envelopes: true,
            require_effect_contract_for_writes: true,
        },
        constraints: BTreeMap::new(),
        metadata: BTreeMap::new(),
    }
}

fn checkpoint_with_nodes(nodes: Vec<ActionNode>) -> CheckpointSnapshot {
    checkpoint_with_terminal_nodes(nodes, Vec::new())
}

fn checkpoint_with_terminal_nodes(
    nodes: Vec<ActionNode>,
    terminals: Vec<String>,
) -> CheckpointSnapshot {
    let mut lifecycle = RunLifecycleState::new(RunId("run-1".to_owned()), "mission-1");
    lifecycle.mark_running(RunPhase::Planning);

    CheckpointSnapshot {
        run_id: "run-1".to_owned(),
        mission_id: "mission-1".to_owned(),
        checkpoint_seq: 0,
        plan_epoch: 0,
        lifecycle,
        action_graph: ActionGraph {
            graph_id: Some("graph-1".to_owned()),
            roots: Vec::new(),
            terminals,
            nodes: nodes
                .into_iter()
                .map(|node| (node.node_id.clone(), node))
                .collect(),
        },
        evidence_graph: EvidenceGraph::default(),
        effect_contracts: Default::default(),
        pending_requests: PendingRequestsSnapshot::default(),
        last_completed_node_id: None,
        actuation_records: Vec::new(),
        execution_artifact: None,
    }
}

fn derive_node(node_id: &str, depends_on: Vec<&str>) -> ActionNode {
    ActionNode {
        node_id: node_id.to_owned(),
        kind: ActionNodeKind::Derive,
        origin: ActionOrigin::DriverFragment,
        status: ActionNodeStatus::Pending,
        depends_on: depends_on.into_iter().map(str::to_owned).collect(),
        inputs: Vec::new(),
        evidence_refs: Vec::new(),
        payload: ActionPayload::Derive(DeriveAction {
            derive_kind: DeriveKind::Parameter,
            derivation_hint: "derive amount".to_owned(),
            output_key: Some("amount".to_owned()),
        }),
        implementation_hint: None,
        expected_effect_ref: None,
    }
}

fn simulate_succeeded_node(node_id: &str) -> ActionNode {
    ActionNode {
        node_id: node_id.to_owned(),
        kind: ActionNodeKind::Simulate,
        origin: ActionOrigin::DriverFragment,
        status: ActionNodeStatus::Succeeded,
        depends_on: Vec::new(),
        inputs: Vec::new(),
        evidence_refs: Vec::new(),
        payload: ActionPayload::Simulate(SimulateAction {
            simulate_kind: SimulateKind::Call,
            simulator_hint: "rpc simulation".to_owned(),
            live: None,
        }),
        implementation_hint: None,
        expected_effect_ref: None,
    }
}

fn actuate_node(node_id: &str, depends_on: Vec<&str>) -> ActionNode {
    ActionNode {
        node_id: node_id.to_owned(),
        kind: ActionNodeKind::Actuate,
        origin: ActionOrigin::DriverFragment,
        status: ActionNodeStatus::Pending,
        depends_on: depends_on.into_iter().map(str::to_owned).collect(),
        inputs: Vec::new(),
        evidence_refs: Vec::new(),
        payload: ActionPayload::Actuate(ActuateAction {
            mode: ActuateMode::DriverCall,
            actuator_hint: "swap".to_owned(),
            chain: Some("eip155:1".to_owned()),
            envelope_ref: Some("env.swap".to_owned()),
            requires_effect_contract: true,
            live: None,
        }),
        implementation_hint: None,
        expected_effect_ref: Some("effect.swap".to_owned()),
    }
}

fn verify_node(node_id: &str, depends_on: Vec<&str>) -> ActionNode {
    ActionNode {
        node_id: node_id.to_owned(),
        kind: ActionNodeKind::Verify,
        origin: ActionOrigin::DriverFragment,
        status: ActionNodeStatus::Pending,
        depends_on: depends_on.into_iter().map(str::to_owned).collect(),
        inputs: Vec::new(),
        evidence_refs: Vec::new(),
        payload: ActionPayload::Verify(VerifyAction {
            verify_kind: VerifyKind::EffectContract,
            verifier_hint: "effect verifier".to_owned(),
            pre_observation_ref: None,
            post_observation_ref: None,
            live: None,
        }),
        implementation_hint: None,
        expected_effect_ref: Some("effect.swap".to_owned()),
    }
}
