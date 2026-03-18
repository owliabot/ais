use std::collections::BTreeMap;

use serde_json::json;

use ais_agent_control::ids::{RunId, SignerRequestId};

use crate::{
    action::ActionGraph,
    actuation::{ActuationKind, ActuationRecord, ActuationStatus},
    checkpoint::{
        CheckpointSnapshot, CheckpointStore, InMemoryCheckpointStore, PendingRequestsSnapshot,
        PendingSignerRequestSnapshot, ReplayCursor,
    },
    evidence::{
        EvidenceFreshness, EvidenceGraph, EvidenceKind, EvidenceProvenance, EvidenceRecord,
        EvidenceRequirement,
    },
    runtime::{RunLifecycleState, RunPhase, SignerRequestState},
};

#[test]
fn restart_preserves_pending_evidence_wait_from_checkpoint() {
    let snapshot = sample_evidence_wait_checkpoint();
    let run_id = snapshot.run_id.clone();
    let checkpoint_seq = snapshot.checkpoint_seq;
    let last_completed_node_id = snapshot.last_completed_node_id.clone();
    let mut store = InMemoryCheckpointStore::default();

    let pointer = store.save(snapshot).expect("save checkpoint");
    let restored = store.latest(&run_id).expect("latest checkpoint");
    let replay = ReplayCursor {
        run_id: restored.run_id.clone(),
        from_checkpoint_seq: restored.checkpoint_seq,
        resume_phase: restored.lifecycle.phase.clone(),
        last_completed_node_id: restored.last_completed_node_id.clone(),
    };

    assert_eq!(pointer.run_id, run_id);
    assert_eq!(pointer.checkpoint_seq, checkpoint_seq);
    assert!(restored.lifecycle.is_stably_paused());
    assert_eq!(restored.lifecycle.phase, RunPhase::AwaitingHost);
    assert_eq!(
        restored.pending_requests.pending_evidence_refs,
        vec!["evidence.quote".to_owned()]
    );
    assert_eq!(
        restored
            .lifecycle
            .active_boundary
            .as_ref()
            .map(|boundary| boundary.blocking_refs.clone()),
        Some(vec!["evidence.quote".to_owned()])
    );
    assert_eq!(replay.resume_phase, RunPhase::AwaitingHost);
    assert_eq!(replay.last_completed_node_id, last_completed_node_id);
}

#[test]
fn restart_preserves_pending_signer_wait_from_checkpoint() {
    let snapshot = sample_signer_wait_checkpoint();
    let mut store = InMemoryCheckpointStore::default();

    store.save(snapshot).expect("save checkpoint");
    let restored = store.latest("run-signer").expect("latest checkpoint");

    assert!(restored.lifecycle.is_stably_paused());
    assert_eq!(restored.lifecycle.phase, RunPhase::AwaitingHost);
    assert_eq!(
        restored
            .pending_requests
            .pending_signer_request_id
            .as_deref(),
        Some("signer-1")
    );
    assert_eq!(
        restored
            .lifecycle
            .active_boundary
            .as_ref()
            .and_then(|boundary| boundary.signer_request_id.as_ref())
            .map(|request_id| request_id.0.as_str()),
        Some("signer-1")
    );
    assert_eq!(restored.actuation_records.len(), 1);
    assert_eq!(
        restored.actuation_records[0].kind,
        ActuationKind::SignerRequested
    );
}

#[test]
fn restart_after_tx_submission_preserves_verification_resume_state() {
    let snapshot = sample_submitted_tx_checkpoint();
    let mut store = InMemoryCheckpointStore::default();

    let pointer = store.save(snapshot.clone()).expect("save checkpoint");
    let restored = store.load(&pointer).expect("load checkpoint");
    let replay = ReplayCursor {
        run_id: restored.run_id.clone(),
        from_checkpoint_seq: restored.checkpoint_seq,
        resume_phase: restored.lifecycle.phase.clone(),
        last_completed_node_id: restored.last_completed_node_id.clone(),
    };

    assert_eq!(restored.lifecycle.phase, RunPhase::Verifying);
    assert_eq!(replay.resume_phase, RunPhase::Verifying);
    assert_eq!(
        replay.last_completed_node_id.as_deref(),
        Some("broadcast-swap")
    );
    assert_eq!(restored.actuation_records.len(), 1);
    assert_eq!(
        restored.actuation_records[0].kind,
        ActuationKind::BroadcastSubmitted
    );
    assert_eq!(
        restored.actuation_records[0].tx_hash.as_deref(),
        Some("0xdeadbeef")
    );
}

fn sample_evidence_wait_checkpoint() -> CheckpointSnapshot {
    let mut lifecycle =
        RunLifecycleState::new(RunId("run-evidence".to_owned()), "mission-evidence");
    lifecycle.mark_running(RunPhase::Planning);
    lifecycle.bump_checkpoint();
    lifecycle.bump_plan_epoch();
    lifecycle.await_evidence(
        "quote is required before planning can continue",
        vec!["evidence.quote".to_owned()],
    );

    CheckpointSnapshot {
        run_id: "run-evidence".to_owned(),
        mission_id: "mission-evidence".to_owned(),
        checkpoint_seq: lifecycle.checkpoint_seq,
        plan_epoch: lifecycle.plan_epoch,
        lifecycle,
        action_graph: ActionGraph {
            graph_id: Some("graph-evidence".to_owned()),
            roots: Vec::new(),
            terminals: Vec::new(),
            nodes: BTreeMap::new(),
        },
        evidence_graph: EvidenceGraph {
            records: BTreeMap::new(),
            requirements: vec![EvidenceRequirement {
                requirement_id: "req-quote".to_owned(),
                reference: "evidence.quote".to_owned(),
                reason: "route quote is required".to_owned(),
                required_by_node_id: Some("plan-swap".to_owned()),
                satisfied_by_evidence_id: None,
            }],
            usages: Vec::new(),
        },
        effect_contracts: Default::default(),
        pending_requests: PendingRequestsSnapshot {
            pending_evidence_refs: vec!["evidence.quote".to_owned()],
            pending_envelope_refs: Vec::new(),
            pending_signer_request: None,
            pending_signer_request_id: None,
            pending_confirmation_id: None,
        },
        last_completed_node_id: Some("observe-balance".to_owned()),
        actuation_records: Vec::new(),
        execution_artifact: None,
    }
}

fn sample_signer_wait_checkpoint() -> CheckpointSnapshot {
    let request = SignerRequestState::new_pending(
        SignerRequestId("signer-1".to_owned()),
        RunId("run-signer".to_owned()),
        "eip155:1",
        "submit swap transaction",
    )
    .with_node_id("broadcast-swap");
    let mut lifecycle = RunLifecycleState::new(RunId("run-signer".to_owned()), "mission-signer");
    lifecycle.mark_running(RunPhase::Governing);
    lifecycle.bump_checkpoint();
    lifecycle.bump_plan_epoch();
    lifecycle.await_signer_request(&request);

    CheckpointSnapshot {
        run_id: "run-signer".to_owned(),
        mission_id: "mission-signer".to_owned(),
        checkpoint_seq: lifecycle.checkpoint_seq,
        plan_epoch: lifecycle.plan_epoch,
        lifecycle,
        action_graph: ActionGraph {
            graph_id: Some("graph-signer".to_owned()),
            roots: Vec::new(),
            terminals: Vec::new(),
            nodes: BTreeMap::new(),
        },
        evidence_graph: EvidenceGraph::default(),
        effect_contracts: Default::default(),
        pending_requests: PendingRequestsSnapshot {
            pending_evidence_refs: Vec::new(),
            pending_envelope_refs: Vec::new(),
            pending_signer_request: Some(PendingSignerRequestSnapshot {
                request_id: request.request_id.0.clone(),
                node_id: request.node_id.clone(),
                chain: Some(request.chain.clone()),
                summary: request.summary.clone(),
                payload: request.payload.clone(),
                timeout_policy: None,
            }),
            pending_signer_request_id: Some("signer-1".to_owned()),
            pending_confirmation_id: None,
        },
        last_completed_node_id: Some("simulate-swap".to_owned()),
        actuation_records: vec![ActuationRecord {
            record_id: "act-signer-1".to_owned(),
            node_id: "broadcast-swap".to_owned(),
            kind: ActuationKind::SignerRequested,
            status: ActuationStatus::Pending,
            chain: Some("eip155:1".to_owned()),
            tx_hash: None,
            summary: "signer request issued".to_owned(),
        }],
        execution_artifact: None,
    }
}

fn sample_submitted_tx_checkpoint() -> CheckpointSnapshot {
    let mut lifecycle = RunLifecycleState::new(RunId("run-verify".to_owned()), "mission-verify");
    lifecycle.mark_running(RunPhase::Broadcasting);
    lifecycle.bump_checkpoint();
    lifecycle.bump_plan_epoch();
    lifecycle.mark_running(RunPhase::Verifying);

    let mut records = BTreeMap::new();
    records.insert(
        "receipt-hint".to_owned(),
        EvidenceRecord {
            evidence_id: "receipt-hint".to_owned(),
            kind: EvidenceKind::ExternalObservation,
            provenance: EvidenceProvenance {
                source: "broadcast-layer".to_owned(),
                chain_scope: Some("eip155:1".to_owned()),
                trace_hint: Some("tx-1".to_owned()),
            },
            freshness: EvidenceFreshness {
                observed_at_ms: Some(2_000),
                expires_at_ms: None,
                max_age_ms: None,
            },
            confidence_ppm: Some(1_000_000),
            payload: json!({ "tx_hash": "0xdeadbeef" }),
        },
    );

    CheckpointSnapshot {
        run_id: "run-verify".to_owned(),
        mission_id: "mission-verify".to_owned(),
        checkpoint_seq: lifecycle.checkpoint_seq,
        plan_epoch: lifecycle.plan_epoch,
        lifecycle,
        action_graph: ActionGraph {
            graph_id: Some("graph-verify".to_owned()),
            roots: Vec::new(),
            terminals: Vec::new(),
            nodes: BTreeMap::new(),
        },
        evidence_graph: EvidenceGraph {
            records,
            requirements: Vec::new(),
            usages: Vec::new(),
        },
        effect_contracts: Default::default(),
        pending_requests: PendingRequestsSnapshot::default(),
        last_completed_node_id: Some("broadcast-swap".to_owned()),
        actuation_records: vec![ActuationRecord {
            record_id: "act-broadcast-1".to_owned(),
            node_id: "broadcast-swap".to_owned(),
            kind: ActuationKind::BroadcastSubmitted,
            status: ActuationStatus::Pending,
            chain: Some("eip155:1".to_owned()),
            tx_hash: Some("0xdeadbeef".to_owned()),
            summary: "transaction broadcasted; awaiting verification".to_owned(),
        }],
        execution_artifact: None,
    }
}
