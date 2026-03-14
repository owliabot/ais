use std::collections::BTreeMap;

use ais_agent_control::ids::{CommandId, RunId, SignerRequestId};
use ais_agent_core::{
    action::ActionGraph,
    checkpoint::{CheckpointSnapshot, PendingRequestsSnapshot},
    evidence::EvidenceGraph,
    mission::{Mission, MissionBudget, MissionPolicy},
    runtime::{RunLifecycleState, RunPhase, SignerRequestState},
};

use crate::runtime::{ActiveRun, InMemoryRunRepository, RunRepository, RunRepositoryError};

#[test]
fn active_run_carries_mission_checkpoint_and_pending_signer_state() {
    let mission = sample_mission();
    let checkpoint = sample_checkpoint();
    let mut runtime = ActiveRun::new(mission.clone(), checkpoint.clone());
    let signer_state = SignerRequestState::new_pending(
        SignerRequestId("signer-1".to_owned()),
        RunId("run-1".to_owned()),
        "eip155:1",
        "sign swap",
    )
    .with_node_id("broadcast-swap");

    runtime.set_pending_signer_state(Some(signer_state.clone()));
    runtime.record_command(CommandId("cmd-1".to_owned()), Some(1_000));
    let event_seq = runtime.next_event_seq();
    runtime.bump_revision();

    assert_eq!(runtime.run_id.0, "run-1");
    assert_eq!(runtime.mission.goal, mission.goal);
    assert_eq!(runtime.checkpoint.mission_id, checkpoint.mission_id);
    assert!(runtime.envelopes.is_empty());
    assert_eq!(
        runtime
            .pending_signer_state
            .as_ref()
            .map(|state| state.request_id.0.as_str()),
        Some("signer-1")
    );
    assert_eq!(
        runtime.last_command_id.as_ref().map(|id| id.0.as_str()),
        Some("cmd-1")
    );
    assert_eq!(runtime.last_updated_at_ms, Some(1_000));
    assert_eq!(event_seq, 1);
    assert_eq!(runtime.revision, 1);
    assert_eq!(runtime.checkpoint_seq(), checkpoint.checkpoint_seq);
    assert_eq!(runtime.plan_epoch(), checkpoint.plan_epoch);
}

#[test]
fn in_memory_run_repository_insert_load_and_version_guard_work() {
    let mut repo = InMemoryRunRepository::default();
    let mut runtime = ActiveRun::new(sample_mission(), sample_checkpoint());

    repo.insert(runtime.clone()).expect("insert");
    assert_eq!(repo.load(&runtime.run_id).expect("load").run_id.0, "run-1");

    runtime.bump_revision();
    runtime.record_command(CommandId("cmd-2".to_owned()), Some(2_000));
    repo.save(runtime.clone(), Some(0))
        .expect("save with matching revision");

    let loaded = repo.load(&runtime.run_id).expect("load updated");
    assert_eq!(loaded.revision, 1);
    assert_eq!(
        loaded.last_command_id.as_ref().map(|id| id.0.as_str()),
        Some("cmd-2")
    );

    let conflict = repo
        .save(runtime.clone(), Some(0))
        .expect_err("stale revision should fail");
    assert_eq!(
        conflict,
        RunRepositoryError::VersionConflict {
            run_id: "run-1".to_owned(),
            expected: 0,
            actual: 1,
        }
    );
}

#[test]
fn in_memory_run_repository_delete_and_duplicate_insert_are_structured() {
    let mut repo = InMemoryRunRepository::default();
    let runtime = ActiveRun::new(sample_mission(), sample_checkpoint());
    let run_id = runtime.run_id.clone();

    repo.insert(runtime.clone()).expect("insert");
    let duplicate = repo.insert(runtime).expect_err("duplicate should fail");
    assert_eq!(
        duplicate,
        RunRepositoryError::AlreadyExists {
            run_id: "run-1".to_owned(),
        }
    );

    repo.delete(&run_id).expect("delete");
    let not_found = repo
        .load(&run_id)
        .expect_err("deleted runtime should be gone");
    assert_eq!(
        not_found,
        RunRepositoryError::NotFound {
            run_id: "run-1".to_owned(),
        }
    );
}

fn sample_mission() -> Mission {
    Mission {
        mission_id: "mission-1".to_owned(),
        goal: "swap usdc to eth".to_owned(),
        allowed_chains: vec!["eip155:1".to_owned()],
        budget: MissionBudget {
            max_steps: Some(10),
            max_signer_requests: Some(1),
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

fn sample_checkpoint() -> CheckpointSnapshot {
    let mut lifecycle = RunLifecycleState::new(RunId("run-1".to_owned()), "mission-1");
    lifecycle.mark_running(RunPhase::Planning);
    lifecycle.bump_checkpoint();
    lifecycle.bump_plan_epoch();

    CheckpointSnapshot {
        run_id: "run-1".to_owned(),
        mission_id: "mission-1".to_owned(),
        checkpoint_seq: lifecycle.checkpoint_seq,
        plan_epoch: lifecycle.plan_epoch,
        lifecycle,
        action_graph: ActionGraph {
            graph_id: Some("graph-1".to_owned()),
            roots: Vec::new(),
            terminals: Vec::new(),
            nodes: BTreeMap::new(),
        },
        evidence_graph: EvidenceGraph::default(),
        effect_contracts: Default::default(),
        pending_requests: PendingRequestsSnapshot {
            pending_evidence_refs: vec!["evidence.quote".to_owned()],
            pending_envelope_refs: Vec::new(),
            pending_signer_request_id: None,
            pending_confirmation_id: None,
        },
        last_completed_node_id: None,
        actuation_records: Vec::new(),
        execution_artifact: None,
    }
}
