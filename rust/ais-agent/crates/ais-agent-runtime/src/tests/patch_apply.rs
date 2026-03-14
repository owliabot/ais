use std::collections::BTreeMap;

use ais_agent_control::{
    commands::{ExpectedRuntimeVersion, RunCommand, SubmitPlanPatchCommand},
    events::RunEvent,
    ids::{CommandId, RunId},
    patch::{PlanPatchOperation, PlanPatchSubmission, PlanPatchTarget},
    recovery::RunFailureCode,
};
use ais_agent_core::{
    action::{
        kinds::{
            actuate::{ActuateAction, ActuateMode},
            verify::{VerifyAction, VerifyKind},
        },
        ActionGraph, ActionNode, ActionNodeKind, ActionNodeStatus, ActionOrigin, ActionPayload,
    },
    checkpoint::{CheckpointSnapshot, PendingRequestsSnapshot},
    effect::{EffectContract, EffectContractKind},
    mission::{Mission, MissionBudget, MissionPolicy},
    runtime::{RunLifecycleState, RunPhase, RunStatus},
};
use ais_agent_host::{
    control::{HostCommandResponse, HostCommandService},
    session::{
        HostCommandEnvelope, HostRunLink, HostSessionId, HostSessionStore, InMemoryHostSessionStore,
    },
};
use serde_json::json;

use crate::{
    persistence::{
        CheckpointArchiveEntry, CheckpointArchiveKind, CheckpointRepository, EventArchive,
        EventArchiveQuery, InMemoryCheckpointRepository, InMemoryEventArchive,
        InMemoryMissionRepository, InMemoryRunCatalogRepository, InMemoryRuntimeAuditArchive,
        InMemorySignerStateArchive, MissionRepository, RunCatalogRepository, RuntimeAuditArchive,
        RuntimeAuditArchiveError, RuntimeAuditQuery,
    },
    runtime::{apply_plan_patch, ActiveRun, InMemoryRunRepository, RunRepository},
    service::RuntimeHostService,
};

#[test]
fn apply_plan_patch_replaces_failed_fragment_and_advances_runtime_version() {
    let (mut runtime, patch) = failed_runtime_and_patch();
    let original_checkpoint_seq = runtime.checkpoint_seq();
    let original_plan_epoch = runtime.plan_epoch();

    let outcome = apply_plan_patch(&mut runtime, &patch).expect("apply patch");

    assert_eq!(
        runtime.checkpoint.action_graph.nodes["swap"].status,
        ActionNodeStatus::Skipped
    );
    assert_eq!(
        runtime.checkpoint.action_graph.nodes["verify-swap"].status,
        ActionNodeStatus::Skipped
    );
    assert!(runtime
        .checkpoint
        .action_graph
        .nodes
        .contains_key("swap-retry"));
    assert!(runtime
        .checkpoint
        .action_graph
        .nodes
        .contains_key("verify-swap-retry"));
    assert_eq!(runtime.checkpoint.lifecycle.status, RunStatus::Running);
    assert_eq!(runtime.checkpoint.lifecycle.phase, RunPhase::Recovering);
    assert_eq!(runtime.plan_epoch(), original_plan_epoch + 1);
    assert_eq!(runtime.checkpoint_seq(), original_checkpoint_seq + 1);
    assert!(outcome
        .patched_node_refs
        .iter()
        .any(|node_id| node_id == "swap-retry"));
    assert!(outcome.mission_constraints_updated);
}

#[test]
fn apply_plan_patch_rejects_executed_history_targets() {
    let (mut runtime, mut patch) = failed_runtime_and_patch();
    runtime
        .checkpoint
        .action_graph
        .nodes
        .get_mut("swap")
        .unwrap()
        .status = ActionNodeStatus::Succeeded;
    patch.target = PlanPatchTarget::NodeSet {
        node_ids: vec!["swap".to_owned()],
    };

    let error = apply_plan_patch(&mut runtime, &patch).expect_err("executed history should fail");
    assert!(error.to_string().contains("already executed history"));
}

#[tokio::test]
async fn runtime_host_service_applies_plan_patch_and_persists_durable_state() {
    let run_id = RunId("run-1".to_owned());
    let host_session_id: HostSessionId = "session-patch".into();
    let (runtime, patch) = failed_runtime_and_patch();
    let expected_version = ExpectedRuntimeVersion {
        checkpoint_seq: Some(runtime.checkpoint_seq()),
        plan_epoch: Some(runtime.plan_epoch()),
    };

    let mut run_repo = InMemoryRunRepository::default();
    run_repo.insert(runtime.clone()).expect("insert runtime");
    let mut checkpoint_repo = InMemoryCheckpointRepository::default();
    checkpoint_repo
        .append(CheckpointArchiveEntry {
            snapshot: runtime.checkpoint.clone(),
            kind: CheckpointArchiveKind::Boundary,
        })
        .expect("append checkpoint");
    let mut mission_repo = InMemoryMissionRepository::default();
    mission_repo
        .insert(run_id.clone(), runtime.mission.clone())
        .expect("insert mission");
    let run_catalog_repo = InMemoryRunCatalogRepository::default();
    let event_archive = InMemoryEventArchive::default();
    let mut session_store = InMemoryHostSessionStore::default();
    session_store.link_run(HostRunLink::new(
        host_session_id.clone(),
        run_id.clone(),
        runtime.mission.goal.clone(),
        runtime.mission.allowed_chains.clone(),
    ));

    let mut service = RuntimeHostService::new(
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
    );

    let response = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-patch-1".into()),
            command: RunCommand::SubmitPlanPatch(SubmitPlanPatchCommand {
                command_id: CommandId("cmd-patch".to_owned()),
                run_id: run_id.clone(),
                patch,
                expected_version: Some(expected_version),
            }),
        })
        .await;

    match response.response {
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(snapshot.status, ais_agent_host::inspect::RunStatus::Running);
            assert_eq!(
                snapshot.phase,
                ais_agent_host::inspect::RunPhase::Recovering
            );
        }
        other => panic!("unexpected response: {other:?}"),
    }

    let (
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        _session_store,
        _signer_state_archive,
    ) = service.into_parts();
    let hot_runtime = run_repo.load(&run_id).expect("hot runtime");
    assert!(hot_runtime
        .checkpoint
        .action_graph
        .nodes
        .contains_key("swap-retry"));

    let latest = checkpoint_repo
        .latest(&run_id.0)
        .expect("latest checkpoint");
    assert_eq!(latest.lifecycle.phase, RunPhase::Recovering);
    assert_eq!(latest.plan_epoch, runtime.plan_epoch() + 1);
    let mission = mission_repo.load(&run_id).expect("mission");
    assert_eq!(mission.constraints.get("slippage_bps"), Some(&json!(50)));
    let catalog = run_catalog_repo.load(&run_id).expect("catalog");
    assert_eq!(catalog.latest_checkpoint_seq, latest.checkpoint_seq);
    assert_eq!(catalog.phase, RunPhase::Recovering);
    let events = event_archive
        .read(EventArchiveQuery {
            run_id,
            after_event_seq: Some(0),
            limit: Some(16),
        })
        .expect("patch audit events");
    assert!(events.events.iter().any(|event| matches!(
        event.event,
        RunEvent::PlanPatchAudit(ref audit)
            if audit.status == ais_agent_control::events::PlanPatchAuditStatus::Submitted
    )));
    assert!(events.events.iter().any(|event| matches!(
        event.event,
        RunEvent::PlanPatchAudit(ref audit)
            if audit.status == ais_agent_control::events::PlanPatchAuditStatus::Applied
    )));
}

#[tokio::test]
async fn runtime_host_service_rejects_stale_plan_patch_version() {
    let run_id = RunId("run-1".to_owned());
    let host_session_id: HostSessionId = "session-patch".into();
    let (runtime, patch) = failed_runtime_and_patch();

    let mut run_repo = InMemoryRunRepository::default();
    run_repo.insert(runtime.clone()).expect("insert runtime");
    let mut checkpoint_repo = InMemoryCheckpointRepository::default();
    checkpoint_repo
        .append(CheckpointArchiveEntry {
            snapshot: runtime.checkpoint.clone(),
            kind: CheckpointArchiveKind::Boundary,
        })
        .expect("append checkpoint");
    let mut mission_repo = InMemoryMissionRepository::default();
    mission_repo
        .insert(run_id.clone(), runtime.mission.clone())
        .expect("insert mission");
    let run_catalog_repo = InMemoryRunCatalogRepository::default();
    let event_archive = InMemoryEventArchive::default();
    let mut session_store = InMemoryHostSessionStore::default();
    session_store.link_run(HostRunLink::new(
        host_session_id.clone(),
        run_id.clone(),
        runtime.mission.goal.clone(),
        runtime.mission.allowed_chains.clone(),
    ));

    let mut service = RuntimeHostService::new(
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
    );

    let response = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: Some("request-patch-stale".into()),
            command: RunCommand::SubmitPlanPatch(SubmitPlanPatchCommand {
                command_id: CommandId("cmd-patch-stale".to_owned()),
                run_id,
                patch,
                expected_version: Some(ExpectedRuntimeVersion {
                    checkpoint_seq: Some(3),
                    plan_epoch: Some(1),
                }),
            }),
        })
        .await;

    match response.response {
        HostCommandResponse::Error(error) => {
            assert_eq!(error.code, "stale_command_conflict");
        }
        other => panic!("unexpected response: {other:?}"),
    }

    let (
        _run_repo,
        _checkpoint_repo,
        _mission_repo,
        run_catalog_repo,
        event_archive,
        _session_store,
        _signer_state_archive,
    ) = service.into_parts();
    let catalog = run_catalog_repo
        .load(&RunId("run-1".to_owned()))
        .expect("catalog");
    let events = event_archive
        .read(EventArchiveQuery {
            run_id: RunId("run-1".to_owned()),
            after_event_seq: Some(0),
            limit: Some(16),
        })
        .expect("patch rejection events");
    assert_eq!(catalog.latest_event_seq, events.latest_event_seq);
    assert!(events.events.iter().any(|event| matches!(
        event.event,
        RunEvent::PlanPatchAudit(ref audit)
            if audit.status == ais_agent_control::events::PlanPatchAuditStatus::Rejected
    )));
}

#[tokio::test]
async fn runtime_host_service_rejects_illegal_plan_patch_submission() {
    let run_id = RunId("run-1".to_owned());
    let host_session_id: HostSessionId = "session-patch".into();
    let (runtime, mut patch) = failed_runtime_and_patch();
    patch.target = PlanPatchTarget::NodeSet {
        node_ids: Vec::new(),
    };

    let mut run_repo = InMemoryRunRepository::default();
    run_repo.insert(runtime.clone()).expect("insert runtime");
    let mut checkpoint_repo = InMemoryCheckpointRepository::default();
    checkpoint_repo
        .append(CheckpointArchiveEntry {
            snapshot: runtime.checkpoint.clone(),
            kind: CheckpointArchiveKind::Boundary,
        })
        .expect("append checkpoint");
    let mut mission_repo = InMemoryMissionRepository::default();
    mission_repo
        .insert(run_id.clone(), runtime.mission.clone())
        .expect("insert mission");
    let run_catalog_repo = InMemoryRunCatalogRepository::default();
    let event_archive = InMemoryEventArchive::default();
    let mut session_store = InMemoryHostSessionStore::default();
    session_store.link_run(HostRunLink::new(
        host_session_id.clone(),
        run_id.clone(),
        runtime.mission.goal.clone(),
        runtime.mission.allowed_chains.clone(),
    ));

    let mut service = RuntimeHostService::new(
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
    );

    let response = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: Some("request-patch-illegal".into()),
            command: RunCommand::SubmitPlanPatch(SubmitPlanPatchCommand {
                command_id: CommandId("cmd-patch-illegal".to_owned()),
                run_id,
                patch,
                expected_version: Some(ExpectedRuntimeVersion {
                    checkpoint_seq: Some(4),
                    plan_epoch: Some(2),
                }),
            }),
        })
        .await;

    match response.response {
        HostCommandResponse::Error(error) => {
            assert_eq!(error.code, "plan_patch_illegal");
        }
        other => panic!("unexpected response: {other:?}"),
    }

    let (
        _run_repo,
        _checkpoint_repo,
        _mission_repo,
        run_catalog_repo,
        event_archive,
        _session_store,
        _signer_state_archive,
    ) = service.into_parts();
    let catalog = run_catalog_repo
        .load(&RunId("run-1".to_owned()))
        .expect("catalog");
    let events = event_archive
        .read(EventArchiveQuery {
            run_id: RunId("run-1".to_owned()),
            after_event_seq: Some(0),
            limit: Some(16),
        })
        .expect("illegal patch rejection events");
    assert_eq!(catalog.latest_event_seq, events.latest_event_seq);
    assert!(events.events.iter().any(|event| matches!(
        event.event,
        RunEvent::PlanPatchAudit(ref audit)
            if audit.status == ais_agent_control::events::PlanPatchAuditStatus::Rejected
    )));
}

#[tokio::test]
async fn runtime_host_service_fails_closed_when_grouped_plan_patch_audit_write_fails() {
    let run_id = RunId("run-1".to_owned());
    let host_session_id: HostSessionId = "session-patch-audit-fail".into();
    let (runtime, patch) = failed_runtime_and_patch();

    let mut run_repo = InMemoryRunRepository::default();
    run_repo.insert(runtime.clone()).expect("insert runtime");
    let mut checkpoint_repo = InMemoryCheckpointRepository::default();
    checkpoint_repo
        .append(CheckpointArchiveEntry {
            snapshot: runtime.checkpoint.clone(),
            kind: CheckpointArchiveKind::Boundary,
        })
        .expect("append checkpoint");
    let mut mission_repo = InMemoryMissionRepository::default();
    mission_repo
        .insert(run_id.clone(), runtime.mission.clone())
        .expect("insert mission");
    let run_catalog_repo = InMemoryRunCatalogRepository::default();
    let event_archive = InMemoryEventArchive::default();
    let mut session_store = InMemoryHostSessionStore::default();
    session_store.link_run(HostRunLink::new(
        host_session_id.clone(),
        run_id.clone(),
        runtime.mission.goal.clone(),
        runtime.mission.allowed_chains.clone(),
    ));

    let mut service = RuntimeHostService::new_with_archives(
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
        InMemorySignerStateArchive::default(),
        FailingRuntimeAuditArchive::fail_on_nth_append(1),
    );

    let response = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: Some("request-patch-audit-fail".into()),
            command: RunCommand::SubmitPlanPatch(SubmitPlanPatchCommand {
                command_id: CommandId("cmd-patch-audit-fail".to_owned()),
                run_id: run_id.clone(),
                patch,
                expected_version: Some(ExpectedRuntimeVersion {
                    checkpoint_seq: Some(runtime.checkpoint_seq()),
                    plan_epoch: Some(runtime.plan_epoch()),
                }),
            }),
        })
        .await;

    match response.response {
        HostCommandResponse::Error(error) => {
            assert_eq!(error.code, "runtime_audit_archive_error");
        }
        other => panic!("unexpected response: {other:?}"),
    }

    let (
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        _session_store,
        _signer_state_archive,
    ) = service.into_parts_with_signer_archive();
    assert!(run_repo.load(&run_id).is_err());
    let latest = checkpoint_repo
        .latest(&run_id.0)
        .expect("latest checkpoint remains original");
    assert_eq!(latest.plan_epoch, runtime.checkpoint.plan_epoch + 1);
    let mission = mission_repo
        .load(&run_id)
        .expect("mission upsert may persist before audit failure");
    assert_eq!(mission.constraints.get("slippage_bps"), Some(&json!(50)));
    let catalog = run_catalog_repo
        .load(&run_id)
        .expect("catalog may persist before audit failure");
    assert_eq!(catalog.latest_checkpoint_seq, latest.checkpoint_seq);
    let events = event_archive
        .read(EventArchiveQuery {
            run_id: run_id.clone(),
            after_event_seq: None,
            limit: Some(16),
        })
        .expect("event archive may persist before audit failure");
    assert_eq!(catalog.latest_event_seq, events.latest_event_seq);
    assert!(events.events.iter().any(|event| matches!(
        event.event,
        RunEvent::PlanPatchAudit(ref audit)
            if audit.status == ais_agent_control::events::PlanPatchAuditStatus::Submitted
    )));
}

fn failed_runtime_and_patch() -> (ActiveRun, PlanPatchSubmission) {
    let run_id = RunId("run-1".to_owned());
    let mission = Mission {
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
    };

    let mut lifecycle = RunLifecycleState::new(run_id.clone(), mission.mission_id.clone());
    lifecycle.phase = RunPhase::Governing;
    lifecycle.checkpoint_seq = 4;
    lifecycle.plan_epoch = 2;
    lifecycle.fail(
        ais_agent_control::recovery::RunFailureStage::Govern,
        RunFailureCode::GovernorDenied,
        "governor rejected node swap",
    );

    let checkpoint = CheckpointSnapshot {
        run_id: run_id.0.clone(),
        mission_id: mission.mission_id.clone(),
        checkpoint_seq: 4,
        plan_epoch: 2,
        lifecycle,
        action_graph: ActionGraph {
            graph_id: Some("graph-1".to_owned()),
            roots: vec!["swap".to_owned()],
            terminals: vec!["verify-swap".to_owned()],
            nodes: BTreeMap::from([
                (
                    "swap".to_owned(),
                    ActionNode {
                        node_id: "swap".to_owned(),
                        kind: ActionNodeKind::Actuate,
                        origin: ActionOrigin::DriverFragment,
                        status: ActionNodeStatus::Failed,
                        depends_on: Vec::new(),
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
                    },
                ),
                (
                    "verify-swap".to_owned(),
                    ActionNode {
                        node_id: "verify-swap".to_owned(),
                        kind: ActionNodeKind::Verify,
                        origin: ActionOrigin::DriverFragment,
                        status: ActionNodeStatus::Pending,
                        depends_on: vec!["swap".to_owned()],
                        inputs: Vec::new(),
                        evidence_refs: Vec::new(),
                        payload: ActionPayload::Verify(VerifyAction {
                            verify_kind: VerifyKind::EffectContract,
                            verifier_hint: "verify swap".to_owned(),
                            pre_observation_ref: None,
                            post_observation_ref: None,
                            live: None,
                        }),
                        implementation_hint: None,
                        expected_effect_ref: Some("effect.swap".to_owned()),
                    },
                ),
            ]),
        },
        evidence_graph: Default::default(),
        effect_contracts: BTreeMap::from([(
            "effect.swap".to_owned(),
            EffectContract {
                effect_id: "effect.swap".to_owned(),
                kind: EffectContractKind::StateTransition,
                assertions: Vec::new(),
                tolerance_hint: None,
            },
        )]),
        pending_requests: PendingRequestsSnapshot::default(),
        last_completed_node_id: None,
        actuation_records: Vec::new(),
        execution_artifact: None,
    };

    let runtime = ActiveRun::new(mission, checkpoint);
    let patch = PlanPatchSubmission {
        patch_id: "patch-1".to_owned(),
        run_id: run_id.clone(),
        basis_checkpoint_seq: 4,
        basis_plan_epoch: 2,
        reason_code: RunFailureCode::GovernorDenied,
        target: PlanPatchTarget::FailedFragment {
            node_ids: vec!["swap".to_owned()],
        },
        operations: vec![
            PlanPatchOperation::ReplaceFragment {
                fragment: json!({
                    "roots": ["swap-retry"],
                    "terminals": ["verify-swap-retry"],
                    "nodes": {
                        "swap-retry": {
                            "node_id": "swap-retry",
                            "kind": "actuate",
                            "origin": "driver_fragment",
                            "status": "pending",
                            "depends_on": [],
                            "inputs": [],
                            "evidence_refs": [],
                            "payload": {
                                "type": "actuate",
                                "mode": "driver_call",
                                "actuator_hint": "swap retry",
                                "chain": "eip155:1",
                                "envelope_ref": "env.swap.retry",
                                "requires_effect_contract": true,
                                "live": null
                            },
                            "implementation_hint": null,
                            "expected_effect_ref": "effect.swap"
                        },
                        "verify-swap-retry": {
                            "node_id": "verify-swap-retry",
                            "kind": "verify",
                            "origin": "driver_fragment",
                            "status": "pending",
                            "depends_on": ["swap-retry"],
                            "inputs": [],
                            "evidence_refs": [],
                            "payload": {
                                "type": "verify",
                                "verify_kind": "effect_contract",
                                "verifier_hint": "verify swap retry",
                                "pre_observation_ref": null,
                                "post_observation_ref": null,
                                "live": null
                            },
                            "implementation_hint": null,
                            "expected_effect_ref": "effect.swap"
                        }
                    },
                    "live_binding_hints": {}
                }),
                preserved_effect_refs: vec!["effect.swap".to_owned()],
            },
            PlanPatchOperation::TightenConstraints {
                constraints: BTreeMap::from([("slippage_bps".to_owned(), json!(50))]),
            },
        ],
        expected_outcome: None,
    };

    (runtime, patch)
}

#[derive(Debug, Default)]
struct FailingRuntimeAuditArchive {
    inner: InMemoryRuntimeAuditArchive,
    appends: usize,
    fail_on_nth_append: usize,
}

impl FailingRuntimeAuditArchive {
    fn fail_on_nth_append(fail_on_nth_append: usize) -> Self {
        Self {
            inner: InMemoryRuntimeAuditArchive::default(),
            appends: 0,
            fail_on_nth_append,
        }
    }
}

impl RuntimeAuditArchive for FailingRuntimeAuditArchive {
    fn append(
        &mut self,
        record: ais_agent_control::audit::RuntimeAuditRecord,
    ) -> Result<(), RuntimeAuditArchiveError> {
        self.appends = self.appends.saturating_add(1);
        if self.appends == self.fail_on_nth_append {
            return Err(RuntimeAuditArchiveError::Storage {
                message: "injected runtime audit archive failure".to_owned(),
            });
        }
        self.inner.append(record)
    }

    fn read(
        &self,
        query: RuntimeAuditQuery,
    ) -> Result<crate::persistence::RuntimeAuditSlice, RuntimeAuditArchiveError> {
        self.inner.read(query)
    }
}
