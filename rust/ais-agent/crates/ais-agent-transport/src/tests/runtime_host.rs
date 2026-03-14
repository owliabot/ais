use std::{collections::BTreeMap, path::Path};

use ais_agent_control::{
    ids::{RunId, SignerRequestId},
    recovery::{RunFailureCode, RunFailureStage},
};
use ais_agent_core::{
    action::{
        kinds::{
            actuate::{ActuateAction, ActuateMode},
            derive::{DeriveAction, DeriveKind},
            verify::{VerifyAction, VerifyKind},
        },
        ActionGraph, ActionNode, ActionNodeKind, ActionNodeStatus, ActionOrigin, ActionPayload,
    },
    checkpoint::{CheckpointSnapshot, PendingRequestsSnapshot},
    evidence::{EvidenceGraph, EvidenceRequirement},
    mission::{Mission, MissionBudget, MissionPolicy},
    runtime::{RunLifecycleState, RunPhase, SignerRequestState},
};
use ais_agent_host::session::{
    HostRunLink, HostSessionId, HostSessionStore, InMemoryHostSessionStore,
};
use ais_agent_runtime::{
    persistence::{
        CheckpointArchiveEntry, CheckpointArchiveKind, InMemoryCheckpointRepository,
        InMemoryEventArchive, InMemoryMissionRepository, InMemoryRunCatalogRepository,
        MissionRepository,
    },
    runtime::{ActiveRun, InMemoryRunRepository, RunRepository},
    service::RuntimeHostService,
};
use ais_agent_store_sqlite::SqliteStore;

pub fn build_runtime_host_service() -> RuntimeHostService<
    InMemoryRunRepository,
    InMemoryCheckpointRepository,
    InMemoryMissionRepository,
    InMemoryRunCatalogRepository,
    InMemoryEventArchive,
    InMemoryHostSessionStore,
> {
    RuntimeHostService::new(
        InMemoryRunRepository::default(),
        InMemoryCheckpointRepository::default(),
        InMemoryMissionRepository::default(),
        InMemoryRunCatalogRepository::default(),
        InMemoryEventArchive::default(),
        InMemoryHostSessionStore::default(),
    )
}

pub fn build_preloaded_evidence_wait_service() -> RuntimeHostService<
    InMemoryRunRepository,
    InMemoryCheckpointRepository,
    InMemoryMissionRepository,
    InMemoryRunCatalogRepository,
    InMemoryEventArchive,
    InMemoryHostSessionStore,
> {
    let run_id = RunId("run-1".to_owned());
    let host_session_id: HostSessionId = "session-1".into();
    let mission = sample_mission();
    let mut checkpoint = checkpoint_with_nodes(
        vec![derive_terminal_node("derive-quote")],
        vec!["derive-quote".to_owned()],
    );
    checkpoint
        .evidence_graph
        .requirements
        .push(EvidenceRequirement {
            requirement_id: "req-1".to_owned(),
            reference: "evidence.quote".to_owned(),
            reason: "quote required".to_owned(),
            required_by_node_id: Some("derive-quote".to_owned()),
            satisfied_by_evidence_id: None,
        });
    checkpoint.pending_requests.pending_evidence_refs = vec!["evidence.quote".to_owned()];
    checkpoint
        .lifecycle
        .await_evidence("need quote", vec!["evidence.quote".to_owned()]);

    let runtime = ActiveRun::new(mission.clone(), checkpoint.clone());
    let mut run_repo = InMemoryRunRepository::default();
    run_repo.insert(runtime).expect("insert runtime");
    let mut checkpoint_repo = InMemoryCheckpointRepository::default();
    ais_agent_runtime::persistence::CheckpointRepository::append(
        &mut checkpoint_repo,
        CheckpointArchiveEntry {
            snapshot: checkpoint,
            kind: CheckpointArchiveKind::Boundary,
        },
    )
    .expect("save checkpoint");
    let mut session_store = InMemoryHostSessionStore::default();
    session_store.link_run(HostRunLink::new(
        host_session_id,
        run_id,
        mission.goal.clone(),
        mission.allowed_chains.clone(),
    ));
    let mission_repo = preloaded_mission_repo(RunId("run-1".to_owned()), mission);
    let run_catalog_repo = InMemoryRunCatalogRepository::default();
    let event_archive = InMemoryEventArchive::default();

    RuntimeHostService::new(
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
    )
}

pub fn build_sqlite_preloaded_evidence_wait_service(
    sqlite_path: &Path,
    session_store: InMemoryHostSessionStore,
) -> RuntimeHostService<
    InMemoryRunRepository,
    SqliteStore,
    SqliteStore,
    SqliteStore,
    SqliteStore,
    InMemoryHostSessionStore,
    SqliteStore,
    SqliteStore,
    SqliteStore,
> {
    let run_id = RunId("run-1".to_owned());
    let mission = sample_mission();
    let mut checkpoint = checkpoint_with_nodes(
        vec![derive_terminal_node("derive-quote")],
        vec!["derive-quote".to_owned()],
    );
    checkpoint
        .evidence_graph
        .requirements
        .push(EvidenceRequirement {
            requirement_id: "req-1".to_owned(),
            reference: "evidence.quote".to_owned(),
            reason: "quote required".to_owned(),
            required_by_node_id: Some("derive-quote".to_owned()),
            satisfied_by_evidence_id: None,
        });
    checkpoint.pending_requests.pending_evidence_refs = vec!["evidence.quote".to_owned()];
    checkpoint
        .lifecycle
        .await_evidence("need quote", vec!["evidence.quote".to_owned()]);

    let mut mission_store = SqliteStore::open_path(sqlite_path).expect("mission store");
    mission_store
        .insert(run_id.clone(), mission.clone())
        .expect("insert mission");

    let mut checkpoint_store = SqliteStore::open_path(sqlite_path).expect("checkpoint store");
    ais_agent_runtime::persistence::CheckpointRepository::append(
        &mut checkpoint_store,
        CheckpointArchiveEntry {
            snapshot: checkpoint,
            kind: CheckpointArchiveKind::Boundary,
        },
    )
    .expect("save checkpoint");

    RuntimeHostService::new_with_archives_and_claim_repo(
        InMemoryRunRepository::default(),
        SqliteStore::open_path(sqlite_path).expect("checkpoint store"),
        SqliteStore::open_path(sqlite_path).expect("mission store"),
        SqliteStore::open_path(sqlite_path).expect("catalog store"),
        SqliteStore::open_path(sqlite_path).expect("event store"),
        session_store,
        SqliteStore::open_path(sqlite_path).expect("signer store"),
        SqliteStore::open_path(sqlite_path).expect("audit store"),
        SqliteStore::open_path(sqlite_path).expect("claim store"),
    )
}

pub fn build_preloaded_signer_wait_service() -> RuntimeHostService<
    InMemoryRunRepository,
    InMemoryCheckpointRepository,
    InMemoryMissionRepository,
    InMemoryRunCatalogRepository,
    InMemoryEventArchive,
    InMemoryHostSessionStore,
> {
    let run_id = RunId("run-1".to_owned());
    let host_session_id: HostSessionId = "session-1".into();
    let mission = sample_mission();
    let mut checkpoint = checkpoint_with_nodes(
        vec![
            actuate_blocked_node("swap", vec![]),
            verify_terminal_node("verify-swap", vec!["swap"]),
        ],
        vec!["verify-swap".to_owned()],
    );
    checkpoint.pending_requests.pending_signer_request_id = Some("signer-1".to_owned());
    checkpoint
        .lifecycle
        .await_signer("await signer", SignerRequestId("signer-1".to_owned()));

    let mut runtime = ActiveRun::new(mission.clone(), checkpoint.clone());
    runtime.pending_signer_state = Some(
        SignerRequestState::new_pending(
            SignerRequestId("signer-1".to_owned()),
            run_id.clone(),
            "eip155:1",
            "sign swap",
        )
        .with_node_id("swap"),
    );

    let mut run_repo = InMemoryRunRepository::default();
    run_repo.insert(runtime).expect("insert runtime");
    let mut checkpoint_repo = InMemoryCheckpointRepository::default();
    ais_agent_runtime::persistence::CheckpointRepository::append(
        &mut checkpoint_repo,
        CheckpointArchiveEntry {
            snapshot: checkpoint,
            kind: CheckpointArchiveKind::Boundary,
        },
    )
    .expect("save checkpoint");
    let mut session_store = InMemoryHostSessionStore::default();
    session_store.link_run(HostRunLink::new(
        host_session_id,
        run_id,
        mission.goal.clone(),
        mission.allowed_chains.clone(),
    ));
    let mission_repo = preloaded_mission_repo(RunId("run-1".to_owned()), mission);
    let run_catalog_repo = InMemoryRunCatalogRepository::default();
    let event_archive = InMemoryEventArchive::default();

    RuntimeHostService::new(
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
    )
}

pub fn build_preloaded_solana_signer_wait_service() -> RuntimeHostService<
    InMemoryRunRepository,
    InMemoryCheckpointRepository,
    InMemoryMissionRepository,
    InMemoryRunCatalogRepository,
    InMemoryEventArchive,
    InMemoryHostSessionStore,
> {
    let run_id = RunId("run-1".to_owned());
    let host_session_id: HostSessionId = "session-1".into();
    let mission = sample_solana_mission();
    let mut checkpoint = checkpoint_with_nodes(
        vec![
            actuate_solana_blocked_node("swap-sol", vec![]),
            verify_solana_terminal_node("verify-swap-sol", vec!["swap-sol"]),
        ],
        vec!["verify-swap-sol".to_owned()],
    );
    checkpoint.pending_requests.pending_signer_request_id = Some("solana-signer-1".to_owned());
    checkpoint.lifecycle.await_signer(
        "await signer",
        SignerRequestId("solana-signer-1".to_owned()),
    );

    let mut runtime = ActiveRun::new(mission.clone(), checkpoint.clone());
    runtime.pending_signer_state = Some(
        SignerRequestState::new_pending(
            SignerRequestId("solana-signer-1".to_owned()),
            run_id.clone(),
            "solana:mainnet",
            "sign solana swap",
        )
        .with_node_id("swap-sol"),
    );

    let mut run_repo = InMemoryRunRepository::default();
    run_repo.insert(runtime).expect("insert runtime");
    let mut checkpoint_repo = InMemoryCheckpointRepository::default();
    ais_agent_runtime::persistence::CheckpointRepository::append(
        &mut checkpoint_repo,
        CheckpointArchiveEntry {
            snapshot: checkpoint,
            kind: CheckpointArchiveKind::Boundary,
        },
    )
    .expect("save checkpoint");
    let mut session_store = InMemoryHostSessionStore::default();
    session_store.link_run(HostRunLink::new(
        host_session_id,
        run_id,
        mission.goal.clone(),
        mission.allowed_chains.clone(),
    ));
    let mission_repo = preloaded_mission_repo(RunId("run-1".to_owned()), mission);
    let run_catalog_repo = InMemoryRunCatalogRepository::default();
    let event_archive = InMemoryEventArchive::default();

    RuntimeHostService::new(
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
    )
}

pub fn build_preloaded_patch_wait_service() -> RuntimeHostService<
    InMemoryRunRepository,
    InMemoryCheckpointRepository,
    InMemoryMissionRepository,
    InMemoryRunCatalogRepository,
    InMemoryEventArchive,
    InMemoryHostSessionStore,
> {
    let run_id = RunId("run-1".to_owned());
    let host_session_id: HostSessionId = "session-1".into();
    let mission = sample_mission();
    let mut checkpoint = checkpoint_with_nodes(
        vec![failed_derive_node("derive-failed")],
        vec!["derive-failed".to_owned()],
    );
    checkpoint.lifecycle.phase = RunPhase::Governing;
    checkpoint.lifecycle.checkpoint_seq = 4;
    checkpoint.lifecycle.plan_epoch = 2;
    checkpoint.checkpoint_seq = 4;
    checkpoint.plan_epoch = 2;
    checkpoint.lifecycle.pause_with_failure(
        RunFailureStage::Govern,
        RunFailureCode::GovernorDenied,
        "governor requested recovery patch",
    );
    if let Some(failure) = checkpoint.lifecycle.failure.as_mut() {
        failure.node_refs.push("derive-failed".to_owned());
    }

    let runtime = ActiveRun::new(mission.clone(), checkpoint.clone());
    let mut run_repo = InMemoryRunRepository::default();
    run_repo.insert(runtime).expect("insert runtime");
    let mut checkpoint_repo = InMemoryCheckpointRepository::default();
    ais_agent_runtime::persistence::CheckpointRepository::append(
        &mut checkpoint_repo,
        CheckpointArchiveEntry {
            snapshot: checkpoint,
            kind: CheckpointArchiveKind::Boundary,
        },
    )
    .expect("save checkpoint");
    let mut session_store = InMemoryHostSessionStore::default();
    session_store.link_run(HostRunLink::new(
        host_session_id,
        run_id,
        mission.goal.clone(),
        mission.allowed_chains.clone(),
    ));
    let mission_repo = preloaded_mission_repo(RunId("run-1".to_owned()), mission);
    let run_catalog_repo = InMemoryRunCatalogRepository::default();
    let event_archive = InMemoryEventArchive::default();

    RuntimeHostService::new(
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
    )
}

pub fn build_preloaded_envelope_wait_service() -> RuntimeHostService<
    InMemoryRunRepository,
    InMemoryCheckpointRepository,
    InMemoryMissionRepository,
    InMemoryRunCatalogRepository,
    InMemoryEventArchive,
    InMemoryHostSessionStore,
> {
    let run_id = RunId("run-1".to_owned());
    let host_session_id: HostSessionId = "session-1".into();
    let mission = sample_mission();
    let mut checkpoint = checkpoint_with_nodes(
        vec![
            actuate_blocked_node("swap", vec![]),
            verify_terminal_node("verify-swap", vec!["swap"]),
        ],
        vec!["verify-swap".to_owned()],
    );
    checkpoint.lifecycle.phase = RunPhase::Broadcasting;
    checkpoint.lifecycle.pause_with_failure(
        RunFailureStage::Broadcast,
        RunFailureCode::EnvelopeInvalid,
        "replacement envelope required",
    );
    checkpoint.pending_requests.pending_envelope_refs = vec!["env.swap".to_owned()];
    if let Some(boundary) = checkpoint.lifecycle.active_boundary.as_mut() {
        boundary.blocking_refs = checkpoint.pending_requests.pending_envelope_refs.clone();
    }
    if let Some(failure) = checkpoint.lifecycle.failure.as_mut() {
        failure.node_refs.push("swap".to_owned());
        failure.effect_refs.push("effect.swap".to_owned());
    }

    let runtime = ActiveRun::new(mission.clone(), checkpoint.clone());
    let mut run_repo = InMemoryRunRepository::default();
    run_repo.insert(runtime).expect("insert runtime");
    let mut checkpoint_repo = InMemoryCheckpointRepository::default();
    ais_agent_runtime::persistence::CheckpointRepository::append(
        &mut checkpoint_repo,
        CheckpointArchiveEntry {
            snapshot: checkpoint,
            kind: CheckpointArchiveKind::Boundary,
        },
    )
    .expect("save checkpoint");
    let mut session_store = InMemoryHostSessionStore::default();
    session_store.link_run(HostRunLink::new(
        host_session_id,
        run_id,
        mission.goal.clone(),
        mission.allowed_chains.clone(),
    ));
    let mission_repo = preloaded_mission_repo(RunId("run-1".to_owned()), mission);
    let run_catalog_repo = InMemoryRunCatalogRepository::default();
    let event_archive = InMemoryEventArchive::default();

    RuntimeHostService::new(
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
    )
}

fn preloaded_mission_repo(run_id: RunId, mission: Mission) -> InMemoryMissionRepository {
    let mut mission_repo = InMemoryMissionRepository::default();
    mission_repo
        .insert(run_id, mission)
        .expect("insert mission");
    mission_repo
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

fn sample_solana_mission() -> Mission {
    Mission {
        mission_id: "mission-1".to_owned(),
        goal: "swap usdc to sol".to_owned(),
        allowed_chains: vec!["solana:mainnet".to_owned()],
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

fn checkpoint_with_nodes(nodes: Vec<ActionNode>, terminals: Vec<String>) -> CheckpointSnapshot {
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

fn derive_terminal_node(node_id: &str) -> ActionNode {
    ActionNode {
        node_id: node_id.to_owned(),
        kind: ActionNodeKind::Derive,
        origin: ActionOrigin::DriverFragment,
        status: ActionNodeStatus::Pending,
        depends_on: Vec::new(),
        inputs: Vec::new(),
        evidence_refs: vec!["evidence.quote".to_owned()],
        payload: ActionPayload::Derive(DeriveAction {
            derive_kind: DeriveKind::Parameter,
            derivation_hint: "derive quote".to_owned(),
            output_key: Some("quote".to_owned()),
        }),
        implementation_hint: None,
        expected_effect_ref: None,
    }
}

fn failed_derive_node(node_id: &str) -> ActionNode {
    let mut node = derive_terminal_node(node_id);
    node.status = ActionNodeStatus::Failed;
    node
}

fn actuate_blocked_node(node_id: &str, depends_on: Vec<&str>) -> ActionNode {
    ActionNode {
        node_id: node_id.to_owned(),
        kind: ActionNodeKind::Actuate,
        origin: ActionOrigin::DriverFragment,
        status: ActionNodeStatus::Blocked,
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

fn verify_terminal_node(node_id: &str, depends_on: Vec<&str>) -> ActionNode {
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

fn actuate_solana_blocked_node(node_id: &str, depends_on: Vec<&str>) -> ActionNode {
    ActionNode {
        node_id: node_id.to_owned(),
        kind: ActionNodeKind::Actuate,
        origin: ActionOrigin::DriverFragment,
        status: ActionNodeStatus::Blocked,
        depends_on: depends_on.into_iter().map(str::to_owned).collect(),
        inputs: Vec::new(),
        evidence_refs: Vec::new(),
        payload: ActionPayload::Actuate(ActuateAction {
            mode: ActuateMode::DriverCall,
            actuator_hint: "solana swap".to_owned(),
            chain: Some("solana:mainnet".to_owned()),
            envelope_ref: Some("env.sol".to_owned()),
            requires_effect_contract: true,
            live: None,
        }),
        implementation_hint: None,
        expected_effect_ref: Some("effect.sol".to_owned()),
    }
}

fn verify_solana_terminal_node(node_id: &str, depends_on: Vec<&str>) -> ActionNode {
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
            verifier_hint: "solana effect verifier".to_owned(),
            pre_observation_ref: None,
            post_observation_ref: None,
            live: None,
        }),
        implementation_hint: None,
        expected_effect_ref: Some("effect.sol".to_owned()),
    }
}
