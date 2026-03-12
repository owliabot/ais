use std::collections::BTreeMap;

use ais_agent_control::ids::RunId;
use ais_agent_core::{
    action::ActionGraph,
    checkpoint::{CheckpointSnapshot, PendingRequestsSnapshot},
    evidence::EvidenceGraph,
    runtime::{RunLifecycleState, RunPhase},
};

use crate::persistence::{
    CheckpointArchiveEntry, CheckpointArchiveKind, CheckpointRepository, CheckpointRepositoryError,
    InMemoryCheckpointRepository,
};

#[test]
fn checkpoint_repository_appends_history_and_reads_latest_snapshot() {
    let mut repo = InMemoryCheckpointRepository::default();

    let mut first = sample_checkpoint();
    first.checkpoint_seq = 1;
    first.lifecycle.checkpoint_seq = 1;
    repo.append(CheckpointArchiveEntry {
        snapshot: first,
        kind: CheckpointArchiveKind::Boundary,
    })
    .expect("append first");

    let mut second = sample_checkpoint();
    second.checkpoint_seq = 2;
    second.lifecycle.checkpoint_seq = 2;
    second.last_completed_node_id = Some("node-2".to_owned());
    repo.append(CheckpointArchiveEntry {
        snapshot: second.clone(),
        kind: CheckpointArchiveKind::Progress,
    })
    .expect("append second");

    let latest = repo.latest("run-1").expect("latest checkpoint");
    assert_eq!(latest.checkpoint_seq, 2);
    assert_eq!(latest.last_completed_node_id.as_deref(), Some("node-2"));
    assert_eq!(repo.history_len("run-1"), 2);
}

#[test]
fn checkpoint_repository_returns_structured_not_found() {
    let repo = InMemoryCheckpointRepository::default();

    let error = repo.latest("missing-run").expect_err("missing checkpoint");
    assert_eq!(
        error,
        CheckpointRepositoryError::NotFound {
            run_id: "missing-run".to_owned(),
        }
    );
}

#[test]
fn checkpoint_repository_latest_prefers_highest_checkpoint_truth_over_append_order() {
    let mut repo = InMemoryCheckpointRepository::default();

    let mut newest = sample_checkpoint();
    newest.checkpoint_seq = 2;
    newest.lifecycle.checkpoint_seq = 2;
    newest.last_completed_node_id = Some("node-2".to_owned());
    repo.append(CheckpointArchiveEntry {
        snapshot: newest.clone(),
        kind: CheckpointArchiveKind::Progress,
    })
    .expect("append newest");

    let mut stale = sample_checkpoint();
    stale.checkpoint_seq = 1;
    stale.lifecycle.checkpoint_seq = 1;
    stale.last_completed_node_id = Some("node-1".to_owned());
    repo.append(CheckpointArchiveEntry {
        snapshot: stale,
        kind: CheckpointArchiveKind::Boundary,
    })
    .expect("append stale later");

    let latest = repo.latest("run-1").expect("latest checkpoint");
    assert_eq!(latest.checkpoint_seq, 2);
    assert_eq!(latest.last_completed_node_id.as_deref(), Some("node-2"));
}

pub(crate) fn sample_checkpoint() -> CheckpointSnapshot {
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
            terminals: Vec::new(),
            nodes: BTreeMap::new(),
        },
        evidence_graph: EvidenceGraph::default(),
        effect_contracts: Default::default(),
        pending_requests: PendingRequestsSnapshot::default(),
        last_completed_node_id: None,
        actuation_records: Vec::new(),
    }
}
