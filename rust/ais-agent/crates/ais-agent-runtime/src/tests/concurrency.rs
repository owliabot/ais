use ais_agent_control::{
    commands::{
        BeginRunCommand, CancelRunCommand, EvidenceKind, EvidenceSubmission,
        ExpectedRuntimeVersion, MissionSubmission, RunCommand, StepBudget, StepRunCommand,
        StepUntil, SubmitEvidenceCommand,
    },
    ids::{CommandId, IdempotencyKey, RunId},
};
use ais_agent_core::{
    action::{
        kinds::recover::{RecoverAction, RecoverKind},
        ActionGraph, ActionNode, ActionNodeKind, ActionNodeStatus, ActionOrigin, ActionPayload,
    },
    checkpoint::{CheckpointSnapshot, PendingRequestsSnapshot},
    mission::{Mission, MissionBudget, MissionPolicy},
    runtime::{RunLifecycleState, RunPhase},
};
use serde_json::json;

use crate::{
    concurrency::{guard_run_command_version, CommandVersionMismatchField},
    runtime::ActiveRun,
};

#[test]
fn matching_expected_version_allows_mutating_command() {
    let runtime = sample_runtime(3, 7);
    let command = RunCommand::StepRun(StepRunCommand {
        command_id: CommandId::from("cmd-step"),
        run_id: runtime.run_id.clone(),
        until: StepUntil::CompleteOrBoundary,
        budget: Some(StepBudget {
            max_nodes: Some(1),
            max_wall_clock_ms: Some(100),
        }),
        expected_version: Some(ExpectedRuntimeVersion {
            checkpoint_seq: Some(3),
            plan_epoch: Some(7),
        }),
    });

    let version = guard_run_command_version(&command, &runtime).expect("matching version");
    assert_eq!(version.checkpoint_seq, 3);
    assert_eq!(version.plan_epoch, 7);
}

#[test]
fn stale_checkpoint_seq_fails_closed_with_machine_readable_conflict() {
    let runtime = sample_runtime(4, 2);
    let before_seq = runtime.checkpoint_seq();
    let command = RunCommand::SubmitEvidence(SubmitEvidenceCommand {
        command_id: CommandId::from("cmd-evidence"),
        run_id: runtime.run_id.clone(),
        evidence: EvidenceSubmission {
            evidence_id: "ev-1".to_string(),
            kind: EvidenceKind::Fact,
            source: "host".to_string(),
            observed_at_ms: Some(123),
            chain_scope: None,
            payload: json!({"ok": true}),
            confidence: None,
        },
        expected_version: Some(ExpectedRuntimeVersion {
            checkpoint_seq: Some(3),
            plan_epoch: Some(2),
        }),
    });

    let conflict = guard_run_command_version(&command, &runtime).expect_err("stale checkpoint");
    assert_eq!(conflict.code, "stale_command_conflict");
    assert_eq!(conflict.command_kind, "submit_evidence");
    assert_eq!(conflict.current.checkpoint_seq, 4);
    assert_eq!(conflict.mismatches.len(), 1);
    assert_eq!(
        conflict.mismatches[0].field,
        CommandVersionMismatchField::CheckpointSeq
    );
    assert_eq!(runtime.checkpoint_seq(), before_seq);
}

#[test]
fn stale_plan_epoch_reports_all_mismatches() {
    let runtime = sample_runtime(5, 9);
    let command = RunCommand::CancelRun(CancelRunCommand {
        command_id: CommandId::from("cmd-cancel"),
        run_id: runtime.run_id.clone(),
        reason: Some("no longer needed".to_string()),
        expected_version: Some(ExpectedRuntimeVersion {
            checkpoint_seq: Some(4),
            plan_epoch: Some(8),
        }),
    });

    let conflict = guard_run_command_version(&command, &runtime).expect_err("stale plan epoch");
    assert_eq!(conflict.command_kind, "cancel_run");
    assert_eq!(conflict.mismatches.len(), 2);
    assert_eq!(
        conflict.mismatches[0].field,
        CommandVersionMismatchField::CheckpointSeq
    );
    assert_eq!(
        conflict.mismatches[1].field,
        CommandVersionMismatchField::PlanEpoch
    );
}

#[test]
fn non_mutating_command_skips_concurrency_guard() {
    let runtime = sample_runtime(8, 13);
    let command = RunCommand::BeginRun(BeginRunCommand {
        command_id: CommandId::from("cmd-begin"),
        idempotency_key: IdempotencyKey::from("idem-begin"),
        mission: MissionSubmission {
            goal: "swap".to_string(),
            allowed_chains: vec!["eip155:1".to_string()],
            constraints: Default::default(),
            budget: None,
            metadata: Default::default(),
        },
    });

    let version =
        guard_run_command_version(&command, &runtime).expect("non-mutating commands pass");
    assert_eq!(version.checkpoint_seq, 8);
    assert_eq!(version.plan_epoch, 13);
}

fn sample_runtime(checkpoint_seq: u64, plan_epoch: u64) -> ActiveRun {
    let mission = Mission {
        mission_id: "mission-concurrency".to_string(),
        goal: "swap".to_string(),
        allowed_chains: vec!["eip155:1".to_string()],
        budget: MissionBudget {
            max_steps: Some(5),
            max_wall_clock_ms: Some(1_000),
            max_signer_requests: Some(1),
        },
        policy: MissionPolicy {
            policy_mode: None,
            allow_raw_envelopes: true,
            require_effect_contract_for_writes: true,
        },
        constraints: Default::default(),
        metadata: Default::default(),
    };

    let run_id = RunId::from("run-concurrency");
    let mut lifecycle = RunLifecycleState::new(run_id.clone(), "mission-concurrency");
    lifecycle.phase = RunPhase::Planning;
    lifecycle.checkpoint_seq = checkpoint_seq;
    lifecycle.plan_epoch = plan_epoch;

    let checkpoint = CheckpointSnapshot {
        run_id: run_id.0.clone(),
        mission_id: "mission-concurrency".to_string(),
        lifecycle,
        action_graph: ActionGraph {
            graph_id: Some("graph-1".to_string()),
            roots: vec!["root".to_string()],
            terminals: vec!["root".to_string()],
            nodes: [(
                "root".to_string(),
                ActionNode {
                    node_id: "root".to_string(),
                    kind: ActionNodeKind::Recover,
                    origin: ActionOrigin::RecoveryRuntime,
                    status: ActionNodeStatus::Pending,
                    depends_on: vec![],
                    inputs: vec![],
                    evidence_refs: vec![],
                    payload: ActionPayload::Recover(RecoverAction {
                        recover_kind: RecoverKind::Escalate,
                        recovery_hint: "test".to_string(),
                    }),
                    implementation_hint: None,
                    expected_effect_ref: None,
                },
            )]
            .into_iter()
            .collect(),
        },
        evidence_graph: Default::default(),
        effect_contracts: Default::default(),
        pending_requests: PendingRequestsSnapshot::default(),
        last_completed_node_id: None,
        actuation_records: vec![],
        checkpoint_seq,
        plan_epoch,
    };

    ActiveRun::new(mission, checkpoint)
}
