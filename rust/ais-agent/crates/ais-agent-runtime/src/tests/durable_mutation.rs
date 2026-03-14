use std::collections::BTreeMap;

use ais_agent_control::{
    audit::{
        GovernorDecisionAuditKind, GovernorDecisionAuditRecord, RuntimeAudit, RuntimeAuditRecord,
    },
    events::{RunEvent, RunEventEnvelope, RunProgress},
    ids::{AuditId, EventId, RunId},
};
use ais_agent_core::{
    checkpoint::{CheckpointSnapshot, PendingRequestsSnapshot},
    mission::{Mission, MissionBudget, MissionPolicy},
    runtime::{RunLifecycleState, RunPhase, RunStatus},
};

use crate::persistence::{
    validate_durable_mutation_unit, AuditWriteBatch, CatalogWrite, CheckpointArchiveEntry,
    CheckpointArchiveKind, CheckpointWrite, DurableMutationContractError, DurableMutationKind,
    DurableMutationUnit, EventWriteBatch, InMemoryRuntimeAuditArchive, MissionWrite,
    MissionWriteMode, RunCatalogEntry, RuntimeAuditArchive, RuntimeAuditQuery,
};

#[test]
fn durable_mutation_unit_accepts_consistent_progress_write_set() {
    let unit = sample_unit();

    validate_durable_mutation_unit(&unit).expect("valid durable mutation unit");
}

#[test]
fn durable_mutation_unit_rejects_catalog_checkpoint_mismatch() {
    let mut unit = sample_unit();
    unit.catalog_write.entry.latest_checkpoint_seq += 1;

    let error = validate_durable_mutation_unit(&unit).expect_err("mismatch must fail");
    assert!(matches!(
        error,
        DurableMutationContractError::CatalogCheckpointMismatch { .. }
    ));
}

#[test]
fn durable_mutation_unit_rejects_non_monotonic_audit_batch() {
    let mut unit = sample_unit();
    unit.audit_write.records.push(RuntimeAuditRecord {
        audit_id: AuditId("audit-0".to_owned()),
        run_id: RunId("run-1".to_owned()),
        audit_seq: 1,
        checkpoint_seq: 1,
        plan_epoch: 0,
        audit: RuntimeAudit::GovernorDecision(GovernorDecisionAuditRecord {
            node_id: Some("node-2".to_owned()),
            decision: GovernorDecisionAuditKind::Reject,
            reason: "duplicate seq".to_owned(),
            evidence_refs: Vec::new(),
            signer_request_id: None,
            rejection_code: Some("duplicate".to_owned()),
        }),
    });

    let error = validate_durable_mutation_unit(&unit).expect_err("non monotonic audit seq");
    assert!(matches!(
        error,
        DurableMutationContractError::AuditBatchNotMonotonic { .. }
    ));
}

#[test]
fn in_memory_runtime_audit_archive_round_trips_cursor_reads() {
    let mut archive = InMemoryRuntimeAuditArchive::default();
    let run_id = RunId("run-1".to_owned());
    let first = sample_audit(1);
    let second = sample_audit(2);
    archive.append(first.clone()).expect("append first audit");
    archive.append(second.clone()).expect("append second audit");

    let slice = archive
        .read(RuntimeAuditQuery {
            run_id: run_id.clone(),
            after_audit_seq: Some(1),
            limit: Some(10),
        })
        .expect("read audit slice");

    assert_eq!(slice.run_id, run_id);
    assert_eq!(slice.latest_audit_seq, Some(2));
    assert_eq!(slice.next_after_audit_seq, Some(2));
    assert!(!slice.truncated);
    assert_eq!(slice.records.len(), 1);
    assert_eq!(slice.records[0].audit_seq, second.audit_seq);
}

pub(super) fn sample_unit() -> DurableMutationUnit {
    let run_id = RunId("run-1".to_owned());
    let mission = sample_mission();
    let checkpoint = sample_checkpoint(&run_id, &mission.mission_id, 1);
    let event = sample_event(1);
    DurableMutationUnit {
        run_id: run_id.clone(),
        kind: DurableMutationKind::Progress,
        mission_write: Some(MissionWrite {
            run_id: run_id.clone(),
            mode: MissionWriteMode::Upsert,
            mission: mission.clone(),
        }),
        checkpoint_write: CheckpointWrite {
            entry: CheckpointArchiveEntry {
                snapshot: checkpoint,
                kind: CheckpointArchiveKind::Progress,
            },
        },
        event_write: EventWriteBatch {
            events: vec![event.clone()],
        },
        catalog_write: CatalogWrite {
            entry: RunCatalogEntry {
                run_id: run_id.clone(),
                mission_id: mission.mission_id.clone(),
                status: RunStatus::Running,
                phase: RunPhase::Planning,
                active_boundary_kind: None,
                latest_checkpoint_seq: 1,
                latest_event_seq: Some(event.event_seq),
                latest_revision: 1,
                created_at_ms: None,
                updated_at_ms: None,
                terminal_at_ms: None,
            },
        },
        signer_write: None,
        audit_write: AuditWriteBatch {
            records: vec![sample_audit(1)],
        },
    }
}

fn sample_mission() -> Mission {
    Mission {
        mission_id: "mission-1".to_owned(),
        goal: "test mission".to_owned(),
        allowed_chains: vec!["evm:1".to_owned()],
        budget: MissionBudget {
            max_steps: Some(10),
            max_signer_requests: Some(3),
            max_wall_clock_ms: Some(30_000),
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

fn sample_checkpoint(run_id: &RunId, mission_id: &str, checkpoint_seq: u64) -> CheckpointSnapshot {
    let mut lifecycle = RunLifecycleState::new(run_id.clone(), mission_id.to_owned());
    lifecycle.mark_running(RunPhase::Planning);
    lifecycle.checkpoint_seq = checkpoint_seq;

    CheckpointSnapshot {
        run_id: run_id.0.clone(),
        mission_id: mission_id.to_owned(),
        checkpoint_seq,
        plan_epoch: 0,
        lifecycle,
        action_graph: Default::default(),
        evidence_graph: Default::default(),
        effect_contracts: Default::default(),
        pending_requests: PendingRequestsSnapshot::default(),
        last_completed_node_id: None,
        actuation_records: Vec::new(),
        execution_artifact: None,
    }
}

fn sample_event(event_seq: u64) -> RunEventEnvelope {
    RunEventEnvelope {
        run_id: RunId("run-1".to_owned()),
        event_seq,
        checkpoint_seq: 1,
        plan_epoch: 0,
        event: RunEvent::Progress(RunProgress {
            event_id: EventId(format!("event-{event_seq}")),
            run_id: RunId("run-1".to_owned()),
            phase: "observe".to_owned(),
            summary: "progress".to_owned(),
        }),
    }
}

fn sample_audit(audit_seq: u64) -> RuntimeAuditRecord {
    RuntimeAuditRecord {
        audit_id: AuditId(format!("audit-{audit_seq}")),
        run_id: RunId("run-1".to_owned()),
        audit_seq,
        checkpoint_seq: 1,
        plan_epoch: 0,
        audit: RuntimeAudit::GovernorDecision(GovernorDecisionAuditRecord {
            node_id: Some("node-1".to_owned()),
            decision: GovernorDecisionAuditKind::Allow,
            reason: "ok".to_owned(),
            evidence_refs: vec!["evidence-1".to_owned()],
            signer_request_id: None,
            rejection_code: None,
        }),
    }
}
