use std::collections::BTreeMap;

use ais_agent_control::{
    commands::{
        BeginRunCommand, CancelRunCommand, ClaimRunCommand, EnvelopeKind, EnvelopeSubmission,
        EvidenceKind, EvidenceSubmission, ExpectedRuntimeVersion, MissionBudgetSubmission,
        MissionSubmission, ReleaseRunClaimCommand, RenewRunClaimCommand, RequestCancelRunCommand,
        RunCommand, SignerDecisionKind, SignerDecisionSubmission, StepBudget, StepRunCommand,
        StepUntil, SubmitEnvelopeCommand, SubmitEvidenceCommand, SubmitSignerDecisionCommand,
    },
    events::RunEvent,
    ids::{CommandId, IdempotencyKey, RunId, SignerRequestId},
    ownership::{RunClaimMode, RunClaimOwnerKind},
    recovery::{
        CancelState, InterruptionClass, RunFailureCode, RunFailureContext, RunFailureStage,
    },
};
use ais_agent_core::{
    action::{
        kinds::{
            actuate::{ActuateAction, ActuateLiveBinding, ActuateMode, SolanaActuateLiveBinding},
            derive::{DeriveAction, DeriveKind},
            verify::{SolanaVerifyLiveBinding, VerifyAction, VerifyKind, VerifyLiveBinding},
        },
        ActionGraph, ActionNode, ActionNodeKind, ActionNodeStatus, ActionOrigin, ActionPayload,
    },
    binding::solana::{SolanaActuateBinding, SolanaConnectionSpec, SolanaVerifyBinding},
    checkpoint::{CheckpointSnapshot, PendingRequestsSnapshot},
    evidence::{EvidenceGraph, EvidenceRequirement},
    mission::{Mission, MissionBudget, MissionPolicy},
    runtime::{RunLifecycleState, RunPhase, SignerRequestState},
};
use ais_agent_evm::read::live::EvmAlloyReadPort;
use ais_agent_host::{
    control::{HostCommandResponse, HostCommandService},
    events::{HostRunEventQuery, HostRunEventService},
    inspect::RunStatus,
    session::{
        HostCommandEnvelope, HostRunLink, HostSessionId, HostSessionStore, InMemoryHostSessionStore,
    },
};
use alloy::{primitives::U256, providers::ProviderBuilder, transports::mock::Asserter};
use serde_json::json;

use crate::{
    persistence::{
        CheckpointArchiveEntry, CheckpointArchiveKind, CheckpointRepository, EventArchive,
        EventArchiveError, EventArchiveQuery, InMemoryCheckpointRepository, InMemoryEventArchive,
        InMemoryMissionRepository, InMemoryRunCatalogRepository, InMemoryRunClaimRepository,
        InMemorySignerStateArchive, MissionRepository, MissionRepositoryError, RunCatalogEntry,
        RunCatalogRepository, RunCatalogRepositoryError, RunClaimRepository, SignerStateArchive,
        SignerStateArchiveError,
    },
    runtime::{ActiveRun, InMemoryRunRepository, RunRepository},
    service::{RuntimeExecutionWiring, RuntimeHostService},
    tests::tracing_capture::capture_tracing_output,
};

#[tokio::test]
async fn runtime_host_service_handles_begin_inspect_and_cancel() {
    let mut service = RuntimeHostService::new(
        InMemoryRunRepository::default(),
        InMemoryCheckpointRepository::default(),
        InMemoryMissionRepository::default(),
        InMemoryRunCatalogRepository::default(),
        InMemoryEventArchive::default(),
        InMemoryHostSessionStore::default(),
    );
    let host_session_id: HostSessionId = "session-1".into();

    let begin = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: None,
            command: RunCommand::BeginRun(BeginRunCommand {
                command_id: CommandId("cmd-begin".to_owned()),
                idempotency_key: IdempotencyKey("idem-1".to_owned()),
                mission: MissionSubmission {
                    goal: "swap".to_owned(),
                    allowed_chains: vec!["eip155:1".to_owned()],
                    constraints: BTreeMap::new(),
                    budget: Some(MissionBudgetSubmission {
                        max_steps: Some(8),
                        max_signer_requests: Some(1),
                        max_wall_clock_ms: Some(30_000),
                    }),
                    metadata: BTreeMap::new(),
                },
            }),
        })
        .await;

    let run_id = match begin.response {
        HostCommandResponse::Accepted(response) => response.run_id.expect("run id"),
        other => panic!("unexpected response: {other:?}"),
    };
    assert_eq!(begin.events.len(), 1);
    assert_eq!(begin.events[0].event_seq, 1);
    assert_eq!(begin.events[0].checkpoint_seq, 0);
    assert_eq!(begin.events[0].plan_epoch, 0);
    assert!(matches!(begin.events[0].event, RunEvent::Started(_)));

    let inspect = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: None,
            command: RunCommand::InspectRun(ais_agent_control::commands::InspectRunCommand {
                command_id: CommandId("cmd-inspect".to_owned()),
                run_id: run_id.clone(),
            }),
        })
        .await;
    match inspect.response {
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(snapshot.run_id, run_id);
            assert_eq!(snapshot.status, RunStatus::Running);
        }
        other => panic!("unexpected response: {other:?}"),
    }

    let cancel = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: None,
            command: RunCommand::CancelRun(CancelRunCommand {
                command_id: CommandId("cmd-cancel".to_owned()),
                run_id,
                reason: Some("user cancelled".to_owned()),
                expected_version: None,
            }),
        })
        .await;
    match cancel.response {
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(snapshot.status, RunStatus::Cancelled);
            assert_eq!(snapshot.cancel_state, Some(CancelState::Cancelled));
        }
        other => panic!("unexpected response: {other:?}"),
    }
}

#[tokio::test]
async fn runtime_host_service_begin_run_seeds_native_transfer_action_family_checkpoint() {
    let host_session_id: HostSessionId = "session-native-transfer-begin".into();
    let mut service = RuntimeHostService::new(
        InMemoryRunRepository::default(),
        InMemoryCheckpointRepository::default(),
        InMemoryMissionRepository::default(),
        InMemoryRunCatalogRepository::default(),
        InMemoryEventArchive::default(),
        InMemoryHostSessionStore::default(),
    )
    .with_execution_wiring(RuntimeExecutionWiring {
        evm_rpc_url: Some("http://127.0.0.1:8545".to_owned()),
        solana_rpc_url: None,
        native_transfer_enabled: true,
        erc20_transfer_enabled: false,
        uniswap_v3_swap_enabled: false,
        uniswap_v3_lp_enabled: false,
    });

    let begin = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: Some("request-native-transfer-begin".into()),
            command: RunCommand::BeginRun(BeginRunCommand {
                command_id: CommandId("cmd-native-transfer-begin".to_owned()),
                idempotency_key: IdempotencyKey("idem-native-transfer-begin".to_owned()),
                mission: MissionSubmission {
                    goal: "owliabot:native_transfer".to_owned(),
                    allowed_chains: vec!["eip155:11155111".to_owned()],
                    constraints: BTreeMap::from([
                        (
                            "owliabot_action_family".to_owned(),
                            json!("native_transfer"),
                        ),
                        (
                            "owliabot_submission".to_owned(),
                            json!({
                                "payload": {
                                    "chain": "11155111",
                                    "recipient": "0x1111111111111111111111111111111111111111",
                                    "requested_amount": "0.00000000000000003",
                                    "asset_symbol": "ETH",
                                    "sender_address_hint": "0x2222222222222222222222222222222222222222"
                                },
                                "evidence": {
                                    "recipient": {
                                        "user_input": "0x1111111111111111111111111111111111111111",
                                        "normalized_address": "0x1111111111111111111111111111111111111111",
                                        "source": "wallet_transfer",
                                        "user_confirmed": true
                                    },
                                    "amount": {
                                        "user_input": "0.00000000000000003",
                                        "normalized_amount": "0.00000000000000003",
                                        "atomic_amount": "30",
                                        "decimals": 18,
                                        "source": "wallet_transfer",
                                        "user_confirmed": true
                                    }
                                }
                            }),
                        ),
                    ]),
                    budget: Some(MissionBudgetSubmission {
                        max_steps: Some(8),
                        max_signer_requests: Some(1),
                        max_wall_clock_ms: Some(30_000),
                    }),
                    metadata: BTreeMap::new(),
                },
            }),
        })
        .await;

    let run_id = match begin.response {
        HostCommandResponse::Accepted(response) => response.run_id.expect("run id"),
        other => panic!("unexpected response: {other:?}"),
    };

    let (
        run_repo,
        checkpoint_repo,
        _mission_repo,
        _run_catalog_repo,
        _event_archive,
        _session_store,
        _signer_state_archive,
    ) = service.into_parts();
    let runtime = run_repo.load(&run_id).expect("runtime");
    assert!(runtime
        .checkpoint
        .action_graph
        .nodes
        .contains_key("observe.native_transfer.recipient_balance"));
    assert!(runtime
        .checkpoint
        .action_graph
        .nodes
        .contains_key("verify.native_transfer.effect"));
    assert!(runtime
        .checkpoint
        .effect_contracts
        .contains_key("effect.native_transfer"));
    assert!(runtime
        .checkpoint
        .evidence_graph
        .records
        .contains_key("evidence.transfer.amount"));

    match &runtime
        .checkpoint
        .action_graph
        .nodes
        .get("simulate.native_transfer.call")
        .expect("simulate node")
        .payload
    {
        ActionPayload::Simulate(action) => {
            let live = action.live.as_ref().expect("live simulate binding");
            let ais_agent_core::action::kinds::simulate::SimulateLiveBinding::Evm(live) = live
            else {
                panic!("expected evm simulate binding");
            };
            assert_eq!(live.request.value, Some(U256::from(30u64)));
        }
        other => panic!("unexpected simulate payload: {other:?}"),
    }

    let latest = checkpoint_repo
        .latest(run_id.0.as_str())
        .expect("latest checkpoint");
    assert!(latest
        .action_graph
        .nodes
        .contains_key("actuate.native_transfer.send"));
}

#[tokio::test]
async fn runtime_host_service_begin_run_seeds_erc20_transfer_action_family_checkpoint() {
    let host_session_id: HostSessionId = "session-erc20-transfer-begin".into();
    let mut service = RuntimeHostService::new(
        InMemoryRunRepository::default(),
        InMemoryCheckpointRepository::default(),
        InMemoryMissionRepository::default(),
        InMemoryRunCatalogRepository::default(),
        InMemoryEventArchive::default(),
        InMemoryHostSessionStore::default(),
    )
    .with_execution_wiring(RuntimeExecutionWiring {
        evm_rpc_url: Some("http://127.0.0.1:8545".to_owned()),
        solana_rpc_url: None,
        native_transfer_enabled: false,
        erc20_transfer_enabled: true,
        uniswap_v3_swap_enabled: false,
        uniswap_v3_lp_enabled: false,
    });

    let begin = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: Some("request-erc20-transfer-begin".into()),
            command: RunCommand::BeginRun(BeginRunCommand {
                command_id: CommandId("cmd-erc20-transfer-begin".to_owned()),
                idempotency_key: IdempotencyKey("idem-erc20-transfer-begin".to_owned()),
                mission: MissionSubmission {
                    goal: "owliabot:erc20_transfer".to_owned(),
                    allowed_chains: vec!["eip155:11155111".to_owned()],
                    constraints: BTreeMap::from([
                        ("owliabot_action_family".to_owned(), json!("erc20_transfer")),
                        (
                            "owliabot_submission".to_owned(),
                            json!({
                                "payload": {
                                    "chain": "11155111",
                                    "token_address": "0x3333333333333333333333333333333333333333",
                                    "token_symbol": "USDC",
                                    "recipient": "0x1111111111111111111111111111111111111111",
                                    "requested_amount": "10",
                                    "sender_address_hint": "0x2222222222222222222222222222222222222222"
                                },
                                "evidence": {
                                    "recipient": {
                                        "user_input": "0x1111111111111111111111111111111111111111",
                                        "normalized_address": "0x1111111111111111111111111111111111111111",
                                        "source": "wallet_transfer",
                                        "user_confirmed": true
                                    },
                                    "amount": {
                                        "user_input": "10",
                                        "normalized_amount": "10",
                                        "atomic_amount": "10000000",
                                        "decimals": 6,
                                        "source": "wallet_transfer",
                                        "user_confirmed": true
                                    },
                                    "token": {
                                        "token_address": "0x3333333333333333333333333333333333333333",
                                        "token_symbol": "USDC",
                                        "decimals": 6,
                                        "resolution_source": "token_registry",
                                        "user_confirmed": true
                                    }
                                }
                            }),
                        ),
                    ]),
                    budget: Some(MissionBudgetSubmission {
                        max_steps: Some(8),
                        max_signer_requests: Some(1),
                        max_wall_clock_ms: Some(30_000),
                    }),
                    metadata: BTreeMap::new(),
                },
            }),
        })
        .await;

    let run_id = match begin.response {
        HostCommandResponse::Accepted(response) => response.run_id.expect("run id"),
        other => panic!("unexpected response: {other:?}"),
    };

    let (
        run_repo,
        checkpoint_repo,
        _mission_repo,
        _run_catalog_repo,
        _event_archive,
        _session_store,
        _signer_state_archive,
    ) = service.into_parts();
    let runtime = run_repo.load(&run_id).expect("runtime");
    assert!(runtime
        .checkpoint
        .action_graph
        .nodes
        .contains_key("observe.erc20_transfer.recipient_token_balance"));
    assert!(runtime
        .checkpoint
        .action_graph
        .nodes
        .contains_key("verify.erc20_transfer.effect"));
    assert!(runtime
        .checkpoint
        .effect_contracts
        .contains_key("effect.erc20_transfer"));
    assert!(runtime
        .checkpoint
        .evidence_graph
        .records
        .contains_key("evidence.transfer.token"));

    match &runtime
        .checkpoint
        .action_graph
        .nodes
        .get("simulate.erc20_transfer.call")
        .expect("simulate node")
        .payload
    {
        ActionPayload::Simulate(action) => {
            let live = action.live.as_ref().expect("live simulate binding");
            let ais_agent_core::action::kinds::simulate::SimulateLiveBinding::Evm(live) = live
            else {
                panic!("expected evm simulate binding");
            };
            assert_eq!(
                live.request.to,
                alloy::primitives::address!("3333333333333333333333333333333333333333")
            );
            assert_eq!(live.request.data[0..4], [0xa9, 0x05, 0x9c, 0xbb]);
        }
        other => panic!("unexpected simulate payload: {other:?}"),
    }

    let latest = checkpoint_repo
        .latest(run_id.0.as_str())
        .expect("latest checkpoint");
    assert!(latest
        .action_graph
        .nodes
        .contains_key("actuate.erc20_transfer.send"));
}

#[tokio::test]
async fn runtime_host_service_begin_run_seeds_uniswap_v3_swap_action_family_checkpoint() {
    let host_session_id: HostSessionId = "session-uniswap-v3-swap-begin".into();
    let mut service = RuntimeHostService::new(
        InMemoryRunRepository::default(),
        InMemoryCheckpointRepository::default(),
        InMemoryMissionRepository::default(),
        InMemoryRunCatalogRepository::default(),
        InMemoryEventArchive::default(),
        InMemoryHostSessionStore::default(),
    )
    .with_execution_wiring(RuntimeExecutionWiring {
        evm_rpc_url: Some("http://127.0.0.1:8545".to_owned()),
        solana_rpc_url: None,
        native_transfer_enabled: false,
        erc20_transfer_enabled: false,
        uniswap_v3_swap_enabled: true,
        uniswap_v3_lp_enabled: false,
    });

    let begin = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: Some("request-uniswap-v3-swap-begin".into()),
            command: RunCommand::BeginRun(BeginRunCommand {
                command_id: CommandId("cmd-uniswap-v3-swap-begin".to_owned()),
                idempotency_key: IdempotencyKey("idem-uniswap-v3-swap-begin".to_owned()),
                mission: MissionSubmission {
                    goal: "owliabot:uniswap_v3_swap".to_owned(),
                    allowed_chains: vec!["eip155:11155111".to_owned()],
                    constraints: BTreeMap::from([
                        ("owliabot_action_family".to_owned(), json!("uniswap_v3_swap")),
                        (
                            "owliabot_submission".to_owned(),
                            json!({
                                "payload": {
                                    "chain": "11155111",
                                    "token_in_address": "0x3333333333333333333333333333333333333333",
                                    "token_in_symbol": "USDC",
                                    "token_out_address": "0x4444444444444444444444444444444444444444",
                                    "token_out_symbol": "WETH",
                                    "fee_tier": 3000,
                                    "requested_amount": "10",
                                    "amount_mode": "exact_in",
                                    "slippage_bps": 50,
                                    "deadline_seconds": 4102444800u64,
                                    "router_address": "0x5555555555555555555555555555555555555555",
                                    "recipient_address": "0x1111111111111111111111111111111111111111",
                                    "sender_address_hint": "0x2222222222222222222222222222222222222222",
                                    "unwrap_native_out": false
                                },
                                "evidence": {
                                    "token_in": {
                                        "token_address": "0x3333333333333333333333333333333333333333",
                                        "token_symbol": "USDC",
                                        "decimals": 6,
                                        "resolution_source": "token_registry",
                                        "user_confirmed": true
                                    },
                                    "token_out": {
                                        "token_address": "0x4444444444444444444444444444444444444444",
                                        "token_symbol": "WETH",
                                        "decimals": 18,
                                        "resolution_source": "token_registry",
                                        "user_confirmed": true
                                    },
                                    "quote": {
                                        "source": "quoter",
                                        "quoted_at_ms": 4102444800000u64,
                                        "expires_at_ms": 4102444900000u64,
                                        "route_summary": "USDC/WETH 0.3%",
                                        "amount_in_atomic": "10000000",
                                        "amount_out_atomic": "3000000000000000",
                                        "min_amount_out_atomic": "2900000000000000",
                                        "user_confirmed": true
                                    },
                                    "router": {
                                        "router_address": "0x5555555555555555555555555555555555555555",
                                        "approval_target_address": "0x5555555555555555555555555555555555555555",
                                        "quoter_address": "0x6666666666666666666666666666666666666666",
                                        "resolution_source": "sepolia_registry",
                                        "user_confirmed": true
                                    },
                                    "deadline": {
                                        "deadline_unix_seconds": 4102444800u64,
                                        "source": "policy",
                                        "user_confirmed": true
                                    }
                                }
                            }),
                        ),
                    ]),
                    budget: Some(MissionBudgetSubmission {
                        max_steps: Some(8),
                        max_signer_requests: Some(1),
                        max_wall_clock_ms: Some(30_000),
                    }),
                    metadata: BTreeMap::new(),
                },
            }),
        })
        .await;

    let run_id = match begin.response {
        HostCommandResponse::Accepted(response) => response.run_id.expect("run id"),
        other => panic!("unexpected response: {other:?}"),
    };

    let (
        run_repo,
        checkpoint_repo,
        _mission_repo,
        _run_catalog_repo,
        _event_archive,
        _session_store,
        _signer_state_archive,
    ) = service.into_parts();
    let runtime = run_repo.load(&run_id).expect("runtime");
    assert!(runtime
        .checkpoint
        .action_graph
        .nodes
        .contains_key("actuate.uniswap_v3_swap.send"));
    assert!(runtime
        .checkpoint
        .effect_contracts
        .contains_key("effect.uniswap_v3_swap"));
    assert!(runtime
        .checkpoint
        .evidence_graph
        .records
        .contains_key("evidence.uniswap.swap.quote"));

    let latest = checkpoint_repo
        .latest(run_id.0.as_str())
        .expect("latest checkpoint");

    assert!(latest
        .action_graph
        .nodes
        .contains_key("actuate.uniswap_v3_swap.send"));
    assert!(latest
        .effect_contracts
        .contains_key("effect.uniswap_v3_swap"));
    assert!(latest
        .evidence_graph
        .records
        .contains_key("evidence.uniswap.swap.quote"));
}

#[tokio::test]
async fn runtime_host_service_begin_run_seeds_uniswap_v3_lp_action_family_checkpoint() {
    let host_session_id: HostSessionId = "session-uniswap-v3-lp-begin".into();
    let mut service = RuntimeHostService::new(
        InMemoryRunRepository::default(),
        InMemoryCheckpointRepository::default(),
        InMemoryMissionRepository::default(),
        InMemoryRunCatalogRepository::default(),
        InMemoryEventArchive::default(),
        InMemoryHostSessionStore::default(),
    )
    .with_execution_wiring(RuntimeExecutionWiring {
        evm_rpc_url: Some("http://127.0.0.1:8545".to_owned()),
        solana_rpc_url: None,
        native_transfer_enabled: false,
        erc20_transfer_enabled: false,
        uniswap_v3_swap_enabled: false,
        uniswap_v3_lp_enabled: true,
    });

    let begin = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: Some("request-uniswap-v3-lp-begin".into()),
            command: RunCommand::BeginRun(BeginRunCommand {
                command_id: CommandId("cmd-uniswap-v3-lp-begin".to_owned()),
                idempotency_key: IdempotencyKey("idem-uniswap-v3-lp-begin".to_owned()),
                mission: MissionSubmission {
                    goal: "owliabot:uniswap_v3_lp".to_owned(),
                    allowed_chains: vec!["eip155:11155111".to_owned()],
                    constraints: BTreeMap::from([
                        ("owliabot_action_family".to_owned(), json!("uniswap_v3_lp")),
                        (
                            "owliabot_submission".to_owned(),
                            json!({
                                "payload": {
                                    "chain": "11155111",
                                    "operation": "mint",
                                    "token0_address": "0x3333333333333333333333333333333333333333",
                                    "token0_symbol": "USDC",
                                    "token1_address": "0x4444444444444444444444444444444444444444",
                                    "token1_symbol": "WETH",
                                    "fee_tier": 3000,
                                    "desired_amount0": "10",
                                    "desired_amount1": "0.003",
                                    "tick_lower": -600,
                                    "tick_upper": 600,
                                    "position_manager_address": "0x1238536071E1c677A632429e3655c799b22cDA52",
                                    "deadline_seconds": 4102444800u64,
                                    "sender_address_hint": "0x2222222222222222222222222222222222222222"
                                },
                                "evidence": {
                                    "token0": {
                                        "token_address": "0x3333333333333333333333333333333333333333",
                                        "token_symbol": "USDC",
                                        "decimals": 6,
                                        "resolution_source": "token_registry",
                                        "user_confirmed": true
                                    },
                                    "token1": {
                                        "token_address": "0x4444444444444444444444444444444444444444",
                                        "token_symbol": "WETH",
                                        "decimals": 18,
                                        "resolution_source": "token_registry",
                                        "user_confirmed": true
                                    },
                                    "pool": {
                                        "pool_address": "0x5555555555555555555555555555555555555555",
                                        "token0_address": "0x3333333333333333333333333333333333333333",
                                        "token1_address": "0x4444444444444444444444444444444444444444",
                                        "fee_tier": 3000,
                                        "tick_spacing": 60,
                                        "slot0_tick": 0,
                                        "observed_at_ms": 4102444800000u64,
                                        "resolution_source": "sepolia_registry",
                                        "user_confirmed": true
                                    },
                                    "deadline": {
                                        "deadline_unix_seconds": 4102444800u64,
                                        "source": "policy",
                                        "user_confirmed": true
                                    }
                                }
                            }),
                        ),
                    ]),
                    budget: Some(MissionBudgetSubmission {
                        max_steps: Some(8),
                        max_signer_requests: Some(1),
                        max_wall_clock_ms: Some(30_000),
                    }),
                    metadata: BTreeMap::new(),
                },
            }),
        })
        .await;

    let run_id = match begin.response {
        HostCommandResponse::Accepted(response) => response.run_id.expect("run id"),
        other => panic!("unexpected response: {other:?}"),
    };

    let (
        run_repo,
        checkpoint_repo,
        _mission_repo,
        _run_catalog_repo,
        _event_archive,
        _session_store,
        _signer_state_archive,
    ) = service.into_parts();
    let runtime = run_repo.load(&run_id).expect("runtime");
    assert!(runtime
        .checkpoint
        .action_graph
        .nodes
        .contains_key("actuate.uniswap_v3_lp.mint"));
    assert!(runtime
        .checkpoint
        .effect_contracts
        .contains_key("effect.uniswap_v3_lp"));
    assert!(runtime
        .checkpoint
        .evidence_graph
        .records
        .contains_key("evidence.uniswap.lp.pool"));

    let latest = checkpoint_repo
        .latest(run_id.0.as_str())
        .expect("latest checkpoint");
    assert!(latest
        .action_graph
        .nodes
        .contains_key("actuate.uniswap_v3_lp.mint"));
    assert!(latest.effect_contracts.contains_key("effect.uniswap_v3_lp"));
    assert!(latest
        .evidence_graph
        .records
        .contains_key("evidence.uniswap.lp.pool"));
}

#[tokio::test]
async fn runtime_host_service_request_cancel_marks_confirmation_wait_as_pending() {
    let (
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
        run_id,
        host_session_id,
    ) = preloaded_signer_wait_runtime();
    let mut service = RuntimeHostService::new(
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
    );

    let _submitted = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: None,
            command: RunCommand::SubmitSignerDecision(SubmitSignerDecisionCommand {
                command_id: CommandId("cmd-signer-for-cancel".to_owned()),
                run_id: run_id.clone(),
                decision: SignerDecisionSubmission {
                    request_id: SignerRequestId("signer-1".to_owned()),
                    decision: SignerDecisionKind::Submitted,
                    tx_hash: Some("0xabc".to_owned()),
                    details: BTreeMap::new(),
                },
                expected_version: None,
            }),
        })
        .await;

    let _step = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: None,
            command: RunCommand::StepRun(StepRunCommand {
                command_id: CommandId("cmd-step-for-cancel".to_owned()),
                run_id: run_id.clone(),
                until: StepUntil::CompleteOrBoundary,
                budget: Some(StepBudget {
                    max_nodes: Some(8),
                    max_wall_clock_ms: None,
                }),
                expected_version: None,
            }),
        })
        .await;

    let cancel = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: None,
            command: RunCommand::RequestCancelRun(RequestCancelRunCommand {
                command_id: CommandId("cmd-request-cancel".to_owned()),
                run_id,
                reason: Some("stop after submission".to_owned()),
                expected_version: None,
            }),
        })
        .await;

    match cancel.response {
        HostCommandResponse::Pause(pause) => {
            assert_eq!(
                pause.kind,
                ais_agent_host::inspect::PauseKind::NeedConfirmation
            );
            assert_eq!(pause.cancel_state, Some(CancelState::Pending));
            assert_eq!(
                pause.interruption_class,
                Some(InterruptionClass::HostCancelRequested)
            );
            assert!(!pause
                .allowed_recovery_actions
                .contains(&ais_agent_control::recovery::RecoveryActionKind::CancelRun));
            assert_eq!(pause.pending_confirmations.len(), 1);
        }
        other => panic!("unexpected response: {other:?}"),
    }
}

#[tokio::test]
async fn runtime_host_service_rejects_cancel_request_for_terminal_run() {
    let mut service = RuntimeHostService::new(
        InMemoryRunRepository::default(),
        InMemoryCheckpointRepository::default(),
        InMemoryMissionRepository::default(),
        InMemoryRunCatalogRepository::default(),
        InMemoryEventArchive::default(),
        InMemoryHostSessionStore::default(),
    );
    let host_session_id: HostSessionId = "session-cancel-reject".into();

    let begin = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: None,
            command: RunCommand::BeginRun(BeginRunCommand {
                command_id: CommandId("cmd-begin-cancel-reject".to_owned()),
                idempotency_key: IdempotencyKey("idem-cancel-reject".to_owned()),
                mission: MissionSubmission {
                    goal: "swap".to_owned(),
                    allowed_chains: vec!["eip155:1".to_owned()],
                    constraints: BTreeMap::new(),
                    budget: None,
                    metadata: BTreeMap::new(),
                },
            }),
        })
        .await;
    let run_id = match begin.response {
        HostCommandResponse::Accepted(response) => response.run_id.expect("run id"),
        other => panic!("unexpected response: {other:?}"),
    };

    let _cancelled = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: None,
            command: RunCommand::RequestCancelRun(RequestCancelRunCommand {
                command_id: CommandId("cmd-request-cancel-terminal".to_owned()),
                run_id: run_id.clone(),
                reason: Some("first cancel".to_owned()),
                expected_version: None,
            }),
        })
        .await;

    let rejected = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: None,
            command: RunCommand::RequestCancelRun(RequestCancelRunCommand {
                command_id: CommandId("cmd-request-cancel-terminal-again".to_owned()),
                run_id,
                reason: Some("second cancel".to_owned()),
                expected_version: None,
            }),
        })
        .await;

    match rejected.response {
        HostCommandResponse::Error(error) => assert_eq!(error.code, "cancel_rejected"),
        other => panic!("unexpected response: {other:?}"),
    }
}

#[tokio::test]
async fn runtime_host_service_claim_run_acquires_unclaimed_preloaded_run() {
    let (
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        _session_store,
        run_id,
        _original_session_id,
    ) = preloaded_evidence_wait_runtime();
    let host_session_id: HostSessionId = "session-claim-acquire".into();
    let mut service = RuntimeHostService::new(
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        InMemoryHostSessionStore::default(),
    );

    let claimed = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-claim-acquire".into()),
            command: RunCommand::ClaimRun(ClaimRunCommand {
                command_id: CommandId("cmd-claim-acquire".to_owned()),
                run_id: run_id.clone(),
                owner_kind: RunClaimOwnerKind::InteractiveHost,
                owner_instance_id: host_session_id.0.clone(),
                mode: RunClaimMode::ExclusiveMutation,
                requested_lease_ms: Some(30_000),
                allow_supersede: false,
                expected_current_claim_id: None,
                expected_current_claim_epoch: None,
            }),
        })
        .await;

    let ownership = response_ownership(&claimed.response);
    let claim = ownership
        .current_claim
        .as_ref()
        .expect("claim should be present after acquire");
    assert_eq!(claim.host_session_id, host_session_id.0);
    assert_eq!(claim.mode, RunClaimMode::ExclusiveMutation);
}

#[tokio::test]
async fn runtime_host_service_renews_and_releases_pre_side_effect_claim() {
    let (
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        _session_store,
        run_id,
        _original_session_id,
    ) = preloaded_evidence_wait_runtime();
    let host_session_id: HostSessionId = "session-claim-renew-release".into();
    let mut service = RuntimeHostService::new(
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        InMemoryHostSessionStore::default(),
    );

    let claimed = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-claim-seed".into()),
            command: RunCommand::ClaimRun(ClaimRunCommand {
                command_id: CommandId("cmd-claim-seed".to_owned()),
                run_id: run_id.clone(),
                owner_kind: RunClaimOwnerKind::InteractiveHost,
                owner_instance_id: host_session_id.0.clone(),
                mode: RunClaimMode::ExclusiveMutation,
                requested_lease_ms: Some(30_000),
                allow_supersede: false,
                expected_current_claim_id: None,
                expected_current_claim_epoch: None,
            }),
        })
        .await;
    let claimed_ownership = response_ownership(&claimed.response);
    let claimed_claim = claimed_ownership
        .current_claim
        .as_ref()
        .expect("claim should be present");
    let claim_id = claimed_claim.claim_id.clone();
    let claim_epoch = claimed_claim.claim_epoch;

    let renewed = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-claim-renew".into()),
            command: RunCommand::RenewRunClaim(RenewRunClaimCommand {
                command_id: CommandId("cmd-claim-renew".to_owned()),
                run_id: run_id.clone(),
                claim_id: claim_id.clone(),
                claim_epoch,
                requested_lease_ms: Some(60_000),
            }),
        })
        .await;
    let renewed_ownership = response_ownership(&renewed.response);
    let renewed_claim = renewed_ownership
        .current_claim
        .as_ref()
        .expect("claim should remain present");
    assert_eq!(renewed_claim.claim_id, claim_id);
    assert!(renewed_claim.claim_epoch > claim_epoch);
    let renewed_epoch = renewed_claim.claim_epoch;

    let released = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-claim-release".into()),
            command: RunCommand::ReleaseRunClaim(ReleaseRunClaimCommand {
                command_id: CommandId("cmd-claim-release".to_owned()),
                run_id,
                claim_id,
                claim_epoch: renewed_epoch,
                reason: Some("handoff".to_owned()),
            }),
        })
        .await;
    assert!(response_ownership(&released.response)
        .current_claim
        .is_none());
}

#[tokio::test]
async fn runtime_host_service_claim_run_can_reacquire_after_expiry() {
    let (
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
        run_id,
        original_session_id,
    ) = preloaded_evidence_wait_runtime();
    let mut claim_repo = InMemoryRunClaimRepository::default();
    claim_repo
        .acquire(sample_runtime_claim(
            &run_id,
            &original_session_id,
            "expired-claim",
            1,
            Some(1),
        ))
        .expect("seed expiring claim");
    let new_session_id: HostSessionId = "session-claim-reacquire".into();
    let mut service = RuntimeHostService::new_with_archives_and_claim_repo(
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
        InMemorySignerStateArchive::default(),
        crate::persistence::InMemoryRuntimeAuditArchive::default(),
        claim_repo,
    );

    let claimed = service
        .handle(HostCommandEnvelope {
            host_session_id: new_session_id.clone(),
            host_request_id: Some("request-claim-reacquire".into()),
            command: RunCommand::ClaimRun(ClaimRunCommand {
                command_id: CommandId("cmd-claim-reacquire".to_owned()),
                run_id: run_id.clone(),
                owner_kind: RunClaimOwnerKind::InteractiveHost,
                owner_instance_id: new_session_id.0.clone(),
                mode: RunClaimMode::ExclusiveMutation,
                requested_lease_ms: Some(30_000),
                allow_supersede: false,
                expected_current_claim_id: None,
                expected_current_claim_epoch: None,
            }),
        })
        .await;

    let claim = response_ownership(&claimed.response)
        .current_claim
        .as_ref()
        .expect("claim should be present");
    assert_eq!(claim.host_session_id, new_session_id.0);
}

#[tokio::test]
async fn runtime_host_service_claim_run_allows_pre_side_effect_supersede_but_not_confirmation_takeover(
) {
    let (
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
        run_id,
        original_session_id,
    ) = preloaded_evidence_wait_runtime();
    let mut claim_repo = InMemoryRunClaimRepository::default();
    let original_claim = claim_repo
        .acquire(sample_runtime_claim(
            &run_id,
            &original_session_id,
            "claim-pre-side-effect",
            1,
            Some(u64::MAX / 4),
        ))
        .expect("seed active claim");
    let takeover_session: HostSessionId = "session-claim-takeover".into();
    let mut service = RuntimeHostService::new_with_archives_and_claim_repo(
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
        InMemorySignerStateArchive::default(),
        crate::persistence::InMemoryRuntimeAuditArchive::default(),
        claim_repo,
    );

    let superseded = service
        .handle(HostCommandEnvelope {
            host_session_id: takeover_session.clone(),
            host_request_id: Some("request-claim-supersede".into()),
            command: RunCommand::ClaimRun(ClaimRunCommand {
                command_id: CommandId("cmd-claim-supersede".to_owned()),
                run_id: run_id.clone(),
                owner_kind: RunClaimOwnerKind::InteractiveHost,
                owner_instance_id: takeover_session.0.clone(),
                mode: RunClaimMode::ExclusiveMutation,
                requested_lease_ms: Some(30_000),
                allow_supersede: true,
                expected_current_claim_id: Some(original_claim.claim_id.clone()),
                expected_current_claim_epoch: Some(original_claim.claim_epoch),
            }),
        })
        .await;
    assert_eq!(
        response_ownership(&superseded.response)
            .current_claim
            .as_ref()
            .map(|claim| claim.host_session_id.as_str()),
        Some(takeover_session.0.as_str())
    );

    let mut confirmation_claim_repo = InMemoryRunClaimRepository::default();
    let confirmation_run_id = RunId("run-1".to_owned());
    let confirmation_session_id: HostSessionId = "session-confirm-owner".into();
    let confirmation_mission = sample_mission();
    let confirmation_checkpoint = confirmation_wait_checkpoint();
    let mut confirmation_run_repo = InMemoryRunRepository::default();
    confirmation_run_repo
        .insert(ActiveRun::new(
            confirmation_mission.clone(),
            confirmation_checkpoint.clone(),
        ))
        .expect("insert runtime");
    let mut confirmation_checkpoint_repo = InMemoryCheckpointRepository::default();
    confirmation_checkpoint_repo
        .append(CheckpointArchiveEntry {
            snapshot: confirmation_checkpoint,
            kind: CheckpointArchiveKind::SideEffect,
        })
        .expect("append checkpoint");
    let mut confirmation_session_store = InMemoryHostSessionStore::default();
    confirmation_session_store.link_run(HostRunLink::new(
        confirmation_session_id.clone(),
        confirmation_run_id.clone(),
        confirmation_mission.goal.clone(),
        confirmation_mission.allowed_chains.clone(),
    ));
    let confirmation_mission_repo =
        preloaded_mission_repo(confirmation_run_id.clone(), confirmation_mission);
    let confirmation_claim = confirmation_claim_repo
        .acquire(sample_runtime_claim(
            &confirmation_run_id,
            &confirmation_session_id,
            "claim-confirmation",
            1,
            Some(u64::MAX / 4),
        ))
        .expect("seed confirmation claim");
    let takeover_confirmation_session: HostSessionId = "session-confirm-takeover".into();
    let mut confirmation_service = RuntimeHostService::new_with_archives_and_claim_repo(
        confirmation_run_repo,
        confirmation_checkpoint_repo,
        confirmation_mission_repo,
        InMemoryRunCatalogRepository::default(),
        InMemoryEventArchive::default(),
        confirmation_session_store,
        InMemorySignerStateArchive::default(),
        crate::persistence::InMemoryRuntimeAuditArchive::default(),
        confirmation_claim_repo,
    );

    let rejected = confirmation_service
        .handle(HostCommandEnvelope {
            host_session_id: takeover_confirmation_session,
            host_request_id: Some("request-claim-confirm-takeover".into()),
            command: RunCommand::ClaimRun(ClaimRunCommand {
                command_id: CommandId("cmd-claim-confirm-takeover".to_owned()),
                run_id: confirmation_run_id,
                owner_kind: RunClaimOwnerKind::InteractiveHost,
                owner_instance_id: "session-confirm-takeover".to_owned(),
                mode: RunClaimMode::ExclusiveMutation,
                requested_lease_ms: Some(30_000),
                allow_supersede: true,
                expected_current_claim_id: Some(confirmation_claim.claim_id),
                expected_current_claim_epoch: Some(confirmation_claim.claim_epoch),
            }),
        })
        .await;
    match rejected.response {
        HostCommandResponse::Error(error) => {
            assert_eq!(error.code, "claim_transfer_required");
        }
        other => panic!("unexpected response: {other:?}"),
    }
}

#[tokio::test]
async fn runtime_host_service_begin_run_skips_existing_durable_run_ids_after_restart() {
    let mut mission_repo = InMemoryMissionRepository::default();
    mission_repo
        .insert(RunId("run-1".to_owned()), sample_mission())
        .expect("insert first mission");
    let mut second_mission = sample_mission();
    second_mission.mission_id = "mission-2".to_owned();
    mission_repo
        .insert(RunId("run-2".to_owned()), second_mission)
        .expect("insert second mission");

    let mut service = RuntimeHostService::new(
        InMemoryRunRepository::default(),
        InMemoryCheckpointRepository::default(),
        mission_repo,
        InMemoryRunCatalogRepository::default(),
        InMemoryEventArchive::default(),
        InMemoryHostSessionStore::default(),
    );
    let host_session_id: HostSessionId = "session-restart-begin".into();

    let begin = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: None,
            command: RunCommand::BeginRun(BeginRunCommand {
                command_id: CommandId("cmd-begin-after-restart".to_owned()),
                idempotency_key: IdempotencyKey("idem-begin-after-restart".to_owned()),
                mission: MissionSubmission {
                    goal: "swap after restart".to_owned(),
                    allowed_chains: vec!["eip155:1".to_owned()],
                    constraints: BTreeMap::new(),
                    budget: Some(MissionBudgetSubmission {
                        max_steps: Some(8),
                        max_signer_requests: Some(1),
                        max_wall_clock_ms: Some(30_000),
                    }),
                    metadata: BTreeMap::new(),
                },
            }),
        })
        .await;

    let run_id = match begin.response {
        HostCommandResponse::Accepted(response) => response.run_id.expect("run id"),
        other => panic!("unexpected response: {other:?}"),
    };
    assert_eq!(run_id.0, "run-3");

    let (
        _run_repo,
        _checkpoint_repo,
        mission_repo,
        _run_catalog_repo,
        _event_archive,
        _session_store,
        _signer_state_archive,
    ) = service.into_parts();
    let mission = mission_repo.load(&run_id).expect("load persisted mission");
    assert_eq!(mission.mission_id, "mission-3");
}

#[tokio::test]
async fn runtime_host_service_fails_closed_when_grouped_begin_run_mission_write_fails() {
    let host_session_id: HostSessionId = "session-begin-mission-fail".into();
    let run_id = RunId("run-1".to_owned());
    let mut service = RuntimeHostService::new(
        InMemoryRunRepository::default(),
        InMemoryCheckpointRepository::default(),
        FailingMissionRepository::fail_on_nth_insert(1),
        InMemoryRunCatalogRepository::default(),
        InMemoryEventArchive::default(),
        InMemoryHostSessionStore::default(),
    );

    let begin = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: None,
            command: RunCommand::BeginRun(BeginRunCommand {
                command_id: CommandId("cmd-begin-mission-fail".to_owned()),
                idempotency_key: IdempotencyKey("idem-begin-mission-fail".to_owned()),
                mission: MissionSubmission {
                    goal: "swap".to_owned(),
                    allowed_chains: vec!["eip155:1".to_owned()],
                    constraints: BTreeMap::new(),
                    budget: Some(MissionBudgetSubmission {
                        max_steps: Some(8),
                        max_signer_requests: Some(1),
                        max_wall_clock_ms: Some(30_000),
                    }),
                    metadata: BTreeMap::new(),
                },
            }),
        })
        .await;

    match begin.response {
        HostCommandResponse::Error(error) => assert_eq!(error.code, "mission_error"),
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
    assert!(run_repo.load(&run_id).is_err());
    assert!(checkpoint_repo.latest(&run_id.0).is_err());
    assert!(mission_repo.load(&run_id).is_err());
    assert!(run_catalog_repo.load(&run_id).is_err());
    assert!(event_archive
        .read(EventArchiveQuery {
            run_id,
            after_event_seq: None,
            limit: Some(8),
        })
        .is_err());
}

#[tokio::test]
async fn runtime_host_service_inspect_rejects_invalid_recovery_contract() {
    let run_id = RunId("run-invalid-recovery".to_owned());
    let host_session_id: HostSessionId = "session-invalid-recovery".into();
    let mission = sample_mission();
    let mut checkpoint = checkpoint_with_nodes(
        vec![derive_terminal_node("derive-quote")],
        vec!["derive-quote".to_owned()],
    );
    checkpoint.run_id = run_id.0.clone();
    checkpoint.lifecycle.run_id = run_id.clone();
    checkpoint.pending_requests.pending_evidence_refs = vec!["evidence.quote".to_owned()];
    checkpoint.lifecycle.status = ais_agent_core::runtime::RunStatus::AwaitingEvidence;
    checkpoint.lifecycle.failure = Some(RunFailureContext::new(
        RunFailureCode::MissingEvidence,
        RunFailureStage::Observe,
        checkpoint.checkpoint_seq,
        checkpoint.plan_epoch,
        Some(ais_agent_control::recovery::StableBoundaryKind::Evidence),
        "quote missing",
    ));

    let mut mission_repo = InMemoryMissionRepository::default();
    mission_repo
        .insert(run_id.clone(), mission.clone())
        .expect("insert mission");
    let mut checkpoint_repo = InMemoryCheckpointRepository::default();
    checkpoint_repo
        .append(CheckpointArchiveEntry {
            snapshot: checkpoint,
            kind: CheckpointArchiveKind::Boundary,
        })
        .expect("append checkpoint");
    let mut session_store = InMemoryHostSessionStore::default();
    session_store.link_run(HostRunLink::new(
        host_session_id.clone(),
        run_id.clone(),
        mission.goal.clone(),
        mission.allowed_chains.clone(),
    ));

    let mut service = RuntimeHostService::new(
        InMemoryRunRepository::default(),
        checkpoint_repo,
        mission_repo,
        InMemoryRunCatalogRepository::default(),
        InMemoryEventArchive::default(),
        session_store,
    );

    let response = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: None,
            command: RunCommand::InspectRun(ais_agent_control::commands::InspectRunCommand {
                command_id: CommandId("cmd-inspect-invalid-recovery".to_owned()),
                run_id,
            }),
        })
        .await;

    match response.response {
        HostCommandResponse::Error(error) => {
            assert_eq!(error.code, "recovery_contract_invalid");
        }
        other => panic!("unexpected response: {other:?}"),
    }
}

#[tokio::test]
async fn runtime_host_service_accepts_evidence_then_steps_to_completion() {
    let (
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
        run_id,
        host_session_id,
    ) = preloaded_evidence_wait_runtime();
    let mut service = RuntimeHostService::new(
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
    );

    let submit = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: None,
            command: RunCommand::SubmitEvidence(SubmitEvidenceCommand {
                command_id: CommandId("cmd-evidence".to_owned()),
                run_id: run_id.clone(),
                evidence: EvidenceSubmission {
                    evidence_id: "quote".to_owned(),
                    kind: EvidenceKind::RouteOrQuote,
                    source: "quote-api".to_owned(),
                    observed_at_ms: Some(10),
                    chain_scope: Some("eip155:1".to_owned()),
                    payload: json!({"amount_out":"1000"}),
                    confidence: Some(0.95),
                },
                expected_version: None,
            }),
        })
        .await;
    match submit.response {
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(snapshot.run_id, run_id);
        }
        other => panic!("unexpected response: {other:?}"),
    }

    let stepped = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: None,
            command: RunCommand::StepRun(StepRunCommand {
                command_id: CommandId("cmd-step".to_owned()),
                run_id,
                until: StepUntil::CompleteOrBoundary,
                budget: Some(StepBudget {
                    max_nodes: Some(8),
                    max_wall_clock_ms: None,
                }),
                expected_version: None,
            }),
        })
        .await;
    match stepped.response {
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(snapshot.status, RunStatus::Completed);
        }
        other => panic!("unexpected response: {other:?}"),
    }
}

#[tokio::test]
async fn runtime_host_service_fails_closed_when_grouped_checkpoint_write_fails() {
    let (
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
        run_id,
        host_session_id,
    ) = preloaded_evidence_wait_runtime();

    let mut service = RuntimeHostService::new(
        run_repo,
        FailingCheckpointRepository::fail_on_nth_append(checkpoint_repo, 1),
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
    );

    let failed = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: Some("request-fail-checkpoint-archive".into()),
            command: RunCommand::SubmitEvidence(SubmitEvidenceCommand {
                command_id: CommandId("cmd-fail-checkpoint-archive".to_owned()),
                run_id: run_id.clone(),
                evidence: EvidenceSubmission {
                    evidence_id: "quote".to_owned(),
                    kind: EvidenceKind::RouteOrQuote,
                    source: "quote-api".to_owned(),
                    observed_at_ms: Some(42),
                    chain_scope: Some("eip155:1".to_owned()),
                    payload: json!({"amount_out":"1000"}),
                    confidence: Some(0.95),
                },
                expected_version: None,
            }),
        })
        .await;

    match failed.response {
        HostCommandResponse::Error(error) => assert_eq!(error.code, "checkpoint_error"),
        other => panic!("unexpected failure response: {other:?}"),
    }

    let (
        run_repo,
        checkpoint_repo,
        _mission_repo,
        run_catalog_repo,
        event_archive,
        _session_store,
        _signer_state_archive,
    ) = service.into_parts();
    let hot_runtime = run_repo
        .load(&run_id)
        .expect("hot runtime remains pre-commit");
    assert_eq!(
        hot_runtime.checkpoint.lifecycle.status,
        ais_agent_core::runtime::RunStatus::AwaitingEvidence
    );
    let checkpoint = checkpoint_repo
        .latest(run_id.0.as_str())
        .expect("latest durable checkpoint");
    assert_eq!(
        checkpoint.lifecycle.status,
        ais_agent_core::runtime::RunStatus::AwaitingEvidence
    );
    assert!(run_catalog_repo.load(&run_id).is_err());
    assert!(event_archive
        .read(EventArchiveQuery {
            run_id,
            after_event_seq: None,
            limit: Some(8),
        })
        .is_err());
}

#[tokio::test]
async fn runtime_host_service_can_complete_from_alloy_backed_live_observation() {
    let (
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
        run_id,
        host_session_id,
    ) = preloaded_evidence_wait_runtime();
    let mut service = RuntimeHostService::new(
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
    );

    let asserter = Asserter::new();
    let provider = ProviderBuilder::new().connect_mocked_client(asserter.clone());
    asserter.push_success(&777u64);
    let live_observation = EvmAlloyReadPort::get_block_number_with_provider(&provider)
        .await
        .expect("live observation");

    let submit = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-live-evidence".into()),
            command: RunCommand::SubmitEvidence(SubmitEvidenceCommand {
                command_id: CommandId("cmd-live-evidence".to_owned()),
                run_id: run_id.clone(),
                evidence: EvidenceSubmission {
                    evidence_id: "quote".to_owned(),
                    kind: EvidenceKind::RouteOrQuote,
                    source: "evm.alloy.live_read".to_owned(),
                    observed_at_ms: Some(10),
                    chain_scope: Some("eip155:1".to_owned()),
                    payload: EvmAlloyReadPort::block_observation_payload(&live_observation),
                    confidence: Some(0.95),
                },
                expected_version: None,
            }),
        })
        .await;

    match submit.response {
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(snapshot.run_id, run_id);
        }
        other => panic!("unexpected response: {other:?}"),
    }

    let stepped = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: Some("request-live-step".into()),
            command: RunCommand::StepRun(StepRunCommand {
                command_id: CommandId("cmd-live-step".to_owned()),
                run_id,
                until: StepUntil::CompleteOrBoundary,
                budget: Some(StepBudget {
                    max_nodes: Some(8),
                    max_wall_clock_ms: None,
                }),
                expected_version: None,
            }),
        })
        .await;

    match stepped.response {
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(snapshot.status, RunStatus::Completed);
            assert_eq!(
                snapshot.effect_status,
                Some(ais_agent_host::inspect::EffectStatusView::Satisfied)
            );
        }
        other => panic!("unexpected response: {other:?}"),
    }
}

#[tokio::test]
async fn runtime_host_service_accepts_signer_decision_then_steps_to_completion() {
    let (
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
        run_id,
        host_session_id,
    ) = preloaded_signer_wait_runtime();
    let mut service = RuntimeHostService::new(
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
    );

    let submit = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: None,
            command: RunCommand::SubmitSignerDecision(SubmitSignerDecisionCommand {
                command_id: CommandId("cmd-signer".to_owned()),
                run_id: run_id.clone(),
                decision: SignerDecisionSubmission {
                    request_id: SignerRequestId("signer-1".to_owned()),
                    decision: SignerDecisionKind::Submitted,
                    tx_hash: Some("0xabc".to_owned()),
                    details: BTreeMap::new(),
                },
                expected_version: None,
            }),
        })
        .await;
    match submit.response {
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(snapshot.run_id, run_id);
        }
        other => panic!("unexpected response: {other:?}"),
    }

    let stepped = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: None,
            command: RunCommand::StepRun(StepRunCommand {
                command_id: CommandId("cmd-step".to_owned()),
                run_id,
                until: StepUntil::CompleteOrBoundary,
                budget: Some(StepBudget {
                    max_nodes: Some(8),
                    max_wall_clock_ms: None,
                }),
                expected_version: None,
            }),
        })
        .await;
    match stepped.response {
        HostCommandResponse::Pause(pause) => {
            assert_eq!(
                pause.kind,
                ais_agent_host::inspect::PauseKind::NeedConfirmation
            );
            assert_eq!(pause.pending_confirmations.len(), 1);
        }
        other => panic!("unexpected response: {other:?}"),
    }
}

#[tokio::test]
async fn runtime_host_service_accepts_replacement_envelope_and_resumes_recovering() {
    let (
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
        run_id,
        host_session_id,
    ) = preloaded_envelope_wait_runtime();
    let mut service = RuntimeHostService::new(
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
    );

    let submit = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-envelope".into()),
            command: RunCommand::SubmitEnvelope(SubmitEnvelopeCommand {
                command_id: CommandId("cmd-envelope".to_owned()),
                run_id: run_id.clone(),
                envelope: sample_envelope_submission("env.swap"),
                expected_version: None,
            }),
        })
        .await;

    match submit.response {
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(snapshot.run_id, run_id);
            assert_eq!(snapshot.status, RunStatus::Running);
            assert_eq!(
                snapshot.phase,
                ais_agent_host::inspect::RunPhase::Recovering
            );
            assert!(snapshot.failure_context.is_none());
        }
        other => panic!("unexpected response: {other:?}"),
    }

    let (
        run_repo,
        checkpoint_repo,
        _mission_repo,
        _run_catalog_repo,
        _event_archive,
        _session_store,
        _signer_state_archive,
    ) = service.into_parts();
    let runtime = run_repo.load(&run_id).expect("runtime");
    assert!(runtime
        .checkpoint
        .pending_requests
        .pending_envelope_refs
        .is_empty());
    assert_eq!(
        runtime
            .checkpoint
            .action_graph
            .nodes
            .get("swap")
            .map(|node| node.status.clone()),
        Some(ActionNodeStatus::Ready)
    );
    let latest = checkpoint_repo
        .latest(run_id.0.as_str())
        .expect("latest checkpoint");
    assert!(latest.pending_requests.pending_envelope_refs.is_empty());
}

#[tokio::test]
async fn runtime_host_service_rejects_unexpected_replacement_envelope_ref() {
    let (
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
        run_id,
        host_session_id,
    ) = preloaded_envelope_wait_runtime();
    let mut service = RuntimeHostService::new(
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
    );

    let submit = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: Some("request-envelope-wrong".into()),
            command: RunCommand::SubmitEnvelope(SubmitEnvelopeCommand {
                command_id: CommandId("cmd-envelope-wrong".to_owned()),
                run_id,
                envelope: sample_envelope_submission("env.other"),
                expected_version: None,
            }),
        })
        .await;

    match submit.response {
        HostCommandResponse::Error(error) => {
            assert_eq!(error.code, "envelope_invalid");
        }
        other => panic!("unexpected response: {other:?}"),
    }
}

#[tokio::test]
async fn runtime_host_service_persists_side_effect_cut_for_signer_submitted_confirmation_wait() {
    let (
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
        run_id,
        host_session_id,
    ) = preloaded_signer_wait_runtime();
    let mut service = RuntimeHostService::new(
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
    );

    let submit = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-side-effect-signer".into()),
            command: RunCommand::SubmitSignerDecision(SubmitSignerDecisionCommand {
                command_id: CommandId("cmd-side-effect-signer".to_owned()),
                run_id: run_id.clone(),
                decision: SignerDecisionSubmission {
                    request_id: SignerRequestId("signer-1".to_owned()),
                    decision: SignerDecisionKind::Submitted,
                    tx_hash: Some("0xabc".to_owned()),
                    details: BTreeMap::new(),
                },
                expected_version: None,
            }),
        })
        .await;
    assert!(matches!(submit.response, HostCommandResponse::Inspect(_)));

    let stepped = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: Some("request-side-effect-step".into()),
            command: RunCommand::StepRun(StepRunCommand {
                command_id: CommandId("cmd-side-effect-step".to_owned()),
                run_id: run_id.clone(),
                until: StepUntil::CompleteOrBoundary,
                budget: Some(StepBudget {
                    max_nodes: Some(8),
                    max_wall_clock_ms: None,
                }),
                expected_version: None,
            }),
        })
        .await;
    assert!(matches!(stepped.response, HostCommandResponse::Pause(_)));

    let (
        _run_repo,
        checkpoint_repo,
        _mission_repo,
        _run_catalog_repo,
        _event_archive,
        _session_store,
        _signer_state_archive,
    ) = service.into_parts();
    let history = checkpoint_repo
        .history(run_id.0.as_str())
        .expect("checkpoint history");
    let latest = history.last().expect("latest checkpoint");
    assert_eq!(latest.kind, CheckpointArchiveKind::SideEffect);
    assert_eq!(
        latest
            .snapshot
            .pending_requests
            .pending_confirmation_id
            .as_deref(),
        Some("0xabc")
    );
}

#[tokio::test]
async fn runtime_host_service_rejects_stale_mutating_command_versions() {
    let (
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
        run_id,
        host_session_id,
    ) = preloaded_evidence_wait_runtime();
    let mut service = RuntimeHostService::new(
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
    );

    let stale = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: None,
            command: RunCommand::SubmitEvidence(SubmitEvidenceCommand {
                command_id: CommandId("cmd-stale".to_owned()),
                run_id,
                evidence: EvidenceSubmission {
                    evidence_id: "quote".to_owned(),
                    kind: EvidenceKind::RouteOrQuote,
                    source: "quote-api".to_owned(),
                    observed_at_ms: Some(10),
                    chain_scope: Some("eip155:1".to_owned()),
                    payload: json!({"amount_out":"1000"}),
                    confidence: Some(0.95),
                },
                expected_version: Some(ExpectedRuntimeVersion {
                    checkpoint_seq: Some(99),
                    plan_epoch: Some(99),
                }),
            }),
        })
        .await;

    match stale.response {
        HostCommandResponse::Error(error) => {
            assert_eq!(error.code, "stale_command_conflict");
            assert!(error.message.contains("submit_evidence"));
        }
        other => panic!("unexpected response: {other:?}"),
    }
}

#[tokio::test]
async fn runtime_host_service_rejects_repeated_mutation_with_stale_expected_version() {
    let (
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
        run_id,
        host_session_id,
    ) = preloaded_evidence_wait_runtime();
    let mut service = RuntimeHostService::new(
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
    );

    let expected_version = Some(ExpectedRuntimeVersion {
        checkpoint_seq: Some(0),
        plan_epoch: Some(0),
    });

    let first = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-repeat-evidence-1".into()),
            command: RunCommand::SubmitEvidence(SubmitEvidenceCommand {
                command_id: CommandId("cmd-repeat-evidence-1".to_owned()),
                run_id: run_id.clone(),
                evidence: EvidenceSubmission {
                    evidence_id: "quote".to_owned(),
                    kind: EvidenceKind::RouteOrQuote,
                    source: "quote-api".to_owned(),
                    observed_at_ms: Some(10),
                    chain_scope: Some("eip155:1".to_owned()),
                    payload: json!({"amount_out":"1000"}),
                    confidence: Some(0.95),
                },
                expected_version: expected_version.clone(),
            }),
        })
        .await;
    assert!(matches!(first.response, HostCommandResponse::Inspect(_)));

    let repeated = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: Some("request-repeat-evidence-2".into()),
            command: RunCommand::SubmitEvidence(SubmitEvidenceCommand {
                command_id: CommandId("cmd-repeat-evidence-2".to_owned()),
                run_id: run_id.clone(),
                evidence: EvidenceSubmission {
                    evidence_id: "quote-2".to_owned(),
                    kind: EvidenceKind::RouteOrQuote,
                    source: "quote-api".to_owned(),
                    observed_at_ms: Some(11),
                    chain_scope: Some("eip155:1".to_owned()),
                    payload: json!({"amount_out":"1001"}),
                    confidence: Some(0.95),
                },
                expected_version,
            }),
        })
        .await;

    match repeated.response {
        HostCommandResponse::Error(error) => {
            assert_eq!(error.code, "stale_command_conflict");
            assert!(error.message.contains("submit_evidence"));
        }
        other => panic!("unexpected repeated mutation response: {other:?}"),
    }
}

#[tokio::test]
async fn runtime_host_service_replays_mutating_command_by_host_request_id() {
    let (
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
        run_id,
        host_session_id,
    ) = preloaded_evidence_wait_runtime();
    let mut service = RuntimeHostService::new(
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
    );

    let command = HostCommandEnvelope {
        host_session_id: host_session_id.clone(),
        host_request_id: Some("request-step-replay".into()),
        command: RunCommand::StepRun(StepRunCommand {
            command_id: CommandId("cmd-step-replay".to_owned()),
            run_id: run_id.clone(),
            until: StepUntil::CompleteOrBoundary,
            budget: Some(StepBudget {
                max_nodes: Some(8),
                max_wall_clock_ms: None,
            }),
            expected_version: None,
        }),
    };

    let first = service.handle(command.clone()).await;
    let replay = service.handle(command).await;

    match (&first.response, &replay.response) {
        (HostCommandResponse::Pause(first_pause), HostCommandResponse::Pause(replay_pause)) => {
            assert_eq!(first_pause.run_id, replay_pause.run_id);
            assert_eq!(first_pause.summary, replay_pause.summary);
        }
        other => panic!("unexpected replay responses: {other:?}"),
    }
    assert_eq!(first.events.len(), replay.events.len());
    assert_eq!(
        first
            .events
            .iter()
            .filter(|event| matches!(
                event.event,
                ais_agent_control::events::RunEvent::Completed(_)
            ))
            .count(),
        replay
            .events
            .iter()
            .filter(|event| matches!(
                event.event,
                ais_agent_control::events::RunEvent::Completed(_)
            ))
            .count()
    );
}

#[tokio::test]
async fn runtime_host_service_does_not_replay_mutation_after_claim_handoff() {
    let (
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
        run_id,
        host_session_id,
    ) = preloaded_evidence_wait_runtime();
    let mut service = RuntimeHostService::new(
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
    );

    let first_step_command = HostCommandEnvelope {
        host_session_id: host_session_id.clone(),
        host_request_id: Some("request-step-claim-scope".into()),
        command: RunCommand::StepRun(StepRunCommand {
            command_id: CommandId("cmd-step-claim-scope".to_owned()),
            run_id: run_id.clone(),
            until: StepUntil::BudgetExhausted,
            budget: Some(StepBudget {
                max_nodes: Some(0),
                max_wall_clock_ms: Some(0),
            }),
            expected_version: None,
        }),
    };

    let first = service.handle(first_step_command.clone()).await;
    let first_claim = response_ownership(&first.response)
        .current_claim
        .clone()
        .expect("bootstrapped claim");

    let released = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-release-claim-scope".into()),
            command: RunCommand::ReleaseRunClaim(ReleaseRunClaimCommand {
                command_id: CommandId("cmd-release-claim-scope".to_owned()),
                run_id: run_id.clone(),
                claim_id: first_claim.claim_id.clone(),
                claim_epoch: first_claim.claim_epoch,
                reason: Some("handoff".to_owned()),
            }),
        })
        .await;
    assert!(response_ownership(&released.response)
        .current_claim
        .is_none());

    let claimed = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-claim-claim-scope-2".into()),
            command: RunCommand::ClaimRun(ClaimRunCommand {
                command_id: CommandId("cmd-claim-claim-scope-2".to_owned()),
                run_id: run_id.clone(),
                owner_kind: RunClaimOwnerKind::InteractiveHost,
                owner_instance_id: host_session_id.0.clone(),
                mode: RunClaimMode::ExclusiveMutation,
                requested_lease_ms: None,
                allow_supersede: false,
                expected_current_claim_id: None,
                expected_current_claim_epoch: None,
            }),
        })
        .await;
    let reacquired_claim = response_ownership(&claimed.response)
        .current_claim
        .clone()
        .expect("reacquired claim");
    assert_ne!(reacquired_claim.claim_id, first_claim.claim_id);

    let repeated = service.handle(first_step_command).await;
    match repeated.response {
        HostCommandResponse::Error(error) => {
            assert_eq!(error.code, "idempotency_conflict");
        }
        other => panic!("unexpected response: {other:?}"),
    }
}

#[tokio::test]
async fn runtime_host_service_lists_streamable_event_batches() {
    let mut service = RuntimeHostService::new(
        InMemoryRunRepository::default(),
        InMemoryCheckpointRepository::default(),
        InMemoryMissionRepository::default(),
        InMemoryRunCatalogRepository::default(),
        InMemoryEventArchive::default(),
        InMemoryHostSessionStore::default(),
    );
    let host_session_id: HostSessionId = "session-events".into();

    let begin = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: Some("request-begin-events".into()),
            command: RunCommand::BeginRun(BeginRunCommand {
                command_id: CommandId("cmd-begin-events".to_owned()),
                idempotency_key: IdempotencyKey("idem-events".to_owned()),
                mission: MissionSubmission {
                    goal: "swap".to_owned(),
                    allowed_chains: vec!["eip155:1".to_owned()],
                    constraints: BTreeMap::new(),
                    budget: None,
                    metadata: BTreeMap::new(),
                },
            }),
        })
        .await;

    let run_id = match begin.response {
        HostCommandResponse::Accepted(response) => response.run_id.expect("run id"),
        other => panic!("unexpected response: {other:?}"),
    };

    let batch = service
        .list_events(HostRunEventQuery {
            run_id,
            after_event_seq: Some(0),
            limit: Some(10),
        })
        .await
        .expect("event batch");

    assert_eq!(batch.latest_event_seq, Some(1));
    assert_eq!(batch.next_after_event_seq, Some(1));
    assert_eq!(batch.events.len(), 1);
}

#[tokio::test]
async fn runtime_host_service_supports_host_collaboration_loop_for_evidence_wait() {
    let (
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
        run_id,
        host_session_id,
    ) = preloaded_evidence_wait_runtime();
    let mut service = RuntimeHostService::new(
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
    );

    let inspect_before = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-collab-inspect-1".into()),
            command: RunCommand::InspectRun(ais_agent_control::commands::InspectRunCommand {
                command_id: CommandId("cmd-collab-inspect-1".to_owned()),
                run_id: run_id.clone(),
            }),
        })
        .await;
    match inspect_before.response {
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(snapshot.status, RunStatus::AwaitingEvidence);
            assert!(snapshot.ownership.current_claim.is_none());
        }
        other => panic!("unexpected inspect response: {other:?}"),
    }

    let initial_batch = service
        .list_events(HostRunEventQuery {
            run_id: run_id.clone(),
            after_event_seq: Some(0),
            limit: Some(10),
        })
        .await
        .expect("initial event batch");
    assert!(initial_batch.events.is_empty());

    let submit = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-collab-evidence".into()),
            command: RunCommand::SubmitEvidence(SubmitEvidenceCommand {
                command_id: CommandId("cmd-collab-evidence".to_owned()),
                run_id: run_id.clone(),
                evidence: EvidenceSubmission {
                    evidence_id: "quote".to_owned(),
                    kind: EvidenceKind::RouteOrQuote,
                    source: "quote-api".to_owned(),
                    observed_at_ms: Some(10),
                    chain_scope: Some("eip155:1".to_owned()),
                    payload: json!({"amount_out":"1000"}),
                    confidence: Some(0.95),
                },
                expected_version: None,
            }),
        })
        .await;
    match submit.response {
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(snapshot.status, RunStatus::AwaitingEvidence);
            assert_eq!(
                snapshot
                    .ownership
                    .current_claim
                    .as_ref()
                    .map(|claim| claim.host_session_id.as_str()),
                Some(host_session_id.0.as_str())
            );
        }
        other => panic!("unexpected evidence response: {other:?}"),
    }

    let stepped = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: Some("request-collab-step".into()),
            command: RunCommand::StepRun(StepRunCommand {
                command_id: CommandId("cmd-collab-step".to_owned()),
                run_id: run_id.clone(),
                until: StepUntil::CompleteOrBoundary,
                budget: Some(StepBudget {
                    max_nodes: Some(8),
                    max_wall_clock_ms: None,
                }),
                expected_version: None,
            }),
        })
        .await;
    match stepped.response {
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(snapshot.status, RunStatus::Completed);
        }
        other => panic!("unexpected step response: {other:?}"),
    }

    let completed_batch = service
        .list_events(HostRunEventQuery {
            run_id,
            after_event_seq: Some(0),
            limit: Some(10),
        })
        .await
        .expect("completed event batch");
    assert!(completed_batch
        .events
        .iter()
        .any(|event| matches!(event.event, RunEvent::Completed(_))));
}

#[tokio::test]
async fn runtime_host_service_supports_host_collaboration_loop_for_solana_signer_wait() {
    let (
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
        run_id,
        host_session_id,
    ) = preloaded_solana_signer_wait_runtime();
    let mut service = RuntimeHostService::new(
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
    );

    let inspect_before = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-sol-inspect-1".into()),
            command: RunCommand::InspectRun(ais_agent_control::commands::InspectRunCommand {
                command_id: CommandId("cmd-sol-inspect-1".to_owned()),
                run_id: run_id.clone(),
            }),
        })
        .await;
    match inspect_before.response {
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(snapshot.status, RunStatus::AwaitingSigner);
        }
        other => panic!("unexpected inspect response: {other:?}"),
    }

    let initial_batch = service
        .list_events(HostRunEventQuery {
            run_id: run_id.clone(),
            after_event_seq: Some(0),
            limit: Some(10),
        })
        .await
        .expect("initial solana event batch");
    assert!(initial_batch.events.is_empty());

    let submit = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-sol-signer".into()),
            command: RunCommand::SubmitSignerDecision(SubmitSignerDecisionCommand {
                command_id: CommandId("cmd-sol-signer".to_owned()),
                run_id: run_id.clone(),
                decision: SignerDecisionSubmission {
                    request_id: SignerRequestId("solana-signer-1".to_owned()),
                    decision: SignerDecisionKind::Submitted,
                    tx_hash: Some("solana-signature-1".to_owned()),
                    details: BTreeMap::new(),
                },
                expected_version: None,
            }),
        })
        .await;
    match submit.response {
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(snapshot.status, RunStatus::AwaitingSigner);
        }
        other => panic!("unexpected signer response: {other:?}"),
    }

    let stepped = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: Some("request-sol-step".into()),
            command: RunCommand::StepRun(StepRunCommand {
                command_id: CommandId("cmd-sol-step".to_owned()),
                run_id: run_id.clone(),
                until: StepUntil::CompleteOrBoundary,
                budget: Some(StepBudget {
                    max_nodes: Some(8),
                    max_wall_clock_ms: None,
                }),
                expected_version: None,
            }),
        })
        .await;
    match stepped.response {
        HostCommandResponse::Pause(pause) => {
            assert_eq!(
                pause.kind,
                ais_agent_host::inspect::PauseKind::NeedConfirmation
            );
            assert_eq!(pause.pending_confirmations.len(), 1);
        }
        other => panic!("unexpected step response: {other:?}"),
    }

    let confirmation_batch = service
        .list_events(HostRunEventQuery {
            run_id,
            after_event_seq: Some(0),
            limit: Some(20),
        })
        .await
        .expect("confirmation event batch");
    assert!(confirmation_batch
        .events
        .iter()
        .any(|event| matches!(event.event, RunEvent::AwaitingConfirm(_))));
    assert!(confirmation_batch.events.iter().any(|event| matches!(
        event.event,
        RunEvent::RecoveryAudit(ref audit)
            if audit.recovery_disposition
                == Some(ais_agent_control::recovery::RecoveryDisposition::ContinueWait)
    )));
}

#[tokio::test]
async fn runtime_host_service_persists_run_catalog_latest_pointers() {
    let mut service = RuntimeHostService::new(
        InMemoryRunRepository::default(),
        InMemoryCheckpointRepository::default(),
        InMemoryMissionRepository::default(),
        InMemoryRunCatalogRepository::default(),
        InMemoryEventArchive::default(),
        InMemoryHostSessionStore::default(),
    );
    let host_session_id: HostSessionId = "session-catalog".into();

    let begin = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-begin-catalog".into()),
            command: RunCommand::BeginRun(BeginRunCommand {
                command_id: CommandId("cmd-begin-catalog".to_owned()),
                idempotency_key: IdempotencyKey("idem-catalog".to_owned()),
                mission: MissionSubmission {
                    goal: "swap".to_owned(),
                    allowed_chains: vec!["eip155:1".to_owned()],
                    constraints: BTreeMap::new(),
                    budget: None,
                    metadata: BTreeMap::new(),
                },
            }),
        })
        .await;
    let run_id = match begin.response {
        HostCommandResponse::Accepted(response) => response.run_id.expect("run id"),
        other => panic!("unexpected response: {other:?}"),
    };

    let cancel = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: Some("request-cancel-catalog".into()),
            command: RunCommand::CancelRun(CancelRunCommand {
                command_id: CommandId("cmd-cancel-catalog".to_owned()),
                run_id: run_id.clone(),
                reason: Some("cancel for catalog coverage".to_owned()),
                expected_version: None,
            }),
        })
        .await;
    assert!(matches!(cancel.response, HostCommandResponse::Inspect(_)));

    let (
        _run_repo,
        _checkpoint_repo,
        _mission_repo,
        run_catalog_repo,
        event_archive,
        _session_store,
        _signer_state_archive,
    ) = service.into_parts();
    let catalog = run_catalog_repo.load(&run_id).expect("load catalog");
    assert_eq!(
        catalog.status,
        ais_agent_core::runtime::RunStatus::Cancelled
    );
    assert_eq!(catalog.latest_checkpoint_seq, 1);
    assert_eq!(catalog.latest_event_seq, Some(1));

    let events = event_archive
        .read(crate::persistence::EventArchiveQuery {
            run_id,
            after_event_seq: Some(0),
            limit: Some(10),
        })
        .expect("read event archive");
    assert_eq!(events.latest_event_seq, Some(1));
}

#[tokio::test]
async fn runtime_host_service_keeps_catalog_pointers_consistent_with_durable_history() {
    let (
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
        run_id,
        host_session_id,
    ) = preloaded_evidence_wait_runtime();
    let mut service = RuntimeHostService::new(
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
    );

    let submit = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-catalog-evidence".into()),
            command: RunCommand::SubmitEvidence(SubmitEvidenceCommand {
                command_id: CommandId("cmd-catalog-evidence".to_owned()),
                run_id: run_id.clone(),
                evidence: EvidenceSubmission {
                    evidence_id: "quote".to_owned(),
                    kind: EvidenceKind::RouteOrQuote,
                    source: "quote-api".to_owned(),
                    observed_at_ms: Some(10),
                    chain_scope: Some("eip155:1".to_owned()),
                    payload: json!({"amount_out":"1000"}),
                    confidence: Some(0.95),
                },
                expected_version: None,
            }),
        })
        .await;
    assert!(matches!(submit.response, HostCommandResponse::Inspect(_)));

    let stepped = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: Some("request-catalog-step".into()),
            command: RunCommand::StepRun(StepRunCommand {
                command_id: CommandId("cmd-catalog-step".to_owned()),
                run_id: run_id.clone(),
                until: StepUntil::CompleteOrBoundary,
                budget: Some(StepBudget {
                    max_nodes: Some(8),
                    max_wall_clock_ms: None,
                }),
                expected_version: None,
            }),
        })
        .await;
    assert!(matches!(stepped.response, HostCommandResponse::Inspect(_)));

    let (
        _run_repo,
        checkpoint_repo,
        _mission_repo,
        run_catalog_repo,
        event_archive,
        _session_store,
        _signer_state_archive,
    ) = service.into_parts();
    let checkpoint = checkpoint_repo
        .latest(run_id.0.as_str())
        .expect("latest checkpoint");
    let checkpoint_history = checkpoint_repo
        .history(run_id.0.as_str())
        .expect("checkpoint history");
    let catalog = run_catalog_repo.load(&run_id).expect("run catalog");
    let events = event_archive
        .read(crate::persistence::EventArchiveQuery {
            run_id: run_id.clone(),
            after_event_seq: Some(0),
            limit: Some(32),
        })
        .expect("event archive");

    assert!(checkpoint_history.len() >= 3);
    assert_eq!(catalog.latest_checkpoint_seq, checkpoint.checkpoint_seq);
    assert_eq!(catalog.latest_revision, checkpoint.checkpoint_seq);
    assert_eq!(catalog.latest_event_seq, events.latest_event_seq);
}

#[tokio::test]
async fn runtime_host_service_prefers_durable_checkpoint_after_catalog_write_failure() {
    let (
        run_repo,
        checkpoint_repo,
        mission_repo,
        _run_catalog_repo,
        event_archive,
        session_store,
        run_id,
        host_session_id,
    ) = preloaded_evidence_wait_runtime();
    let mut service = RuntimeHostService::new(
        run_repo,
        checkpoint_repo,
        mission_repo,
        FailingRunCatalogRepository::fail_on_nth_upsert(2),
        event_archive,
        session_store,
    );

    let submit = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-fail-evidence".into()),
            command: RunCommand::SubmitEvidence(SubmitEvidenceCommand {
                command_id: CommandId("cmd-fail-evidence".to_owned()),
                run_id: run_id.clone(),
                evidence: EvidenceSubmission {
                    evidence_id: "quote".to_owned(),
                    kind: EvidenceKind::RouteOrQuote,
                    source: "quote-api".to_owned(),
                    observed_at_ms: Some(10),
                    chain_scope: Some("eip155:1".to_owned()),
                    payload: json!({"amount_out":"1000"}),
                    confidence: Some(0.95),
                },
                expected_version: None,
            }),
        })
        .await;
    assert!(matches!(submit.response, HostCommandResponse::Inspect(_)));

    let failed = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-fail-step".into()),
            command: RunCommand::StepRun(StepRunCommand {
                command_id: CommandId("cmd-fail-step".to_owned()),
                run_id: run_id.clone(),
                until: StepUntil::CompleteOrBoundary,
                budget: Some(StepBudget {
                    max_nodes: Some(8),
                    max_wall_clock_ms: None,
                }),
                expected_version: None,
            }),
        })
        .await;

    match failed.response {
        HostCommandResponse::Error(error) => {
            assert_eq!(error.code, "run_catalog_error");
        }
        other => panic!("unexpected failure response: {other:?}"),
    }

    let replay = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-fail-step-replay".into()),
            command: RunCommand::StepRun(StepRunCommand {
                command_id: CommandId("cmd-fail-step-replay".to_owned()),
                run_id: run_id.clone(),
                until: StepUntil::CompleteOrBoundary,
                budget: Some(StepBudget {
                    max_nodes: Some(8),
                    max_wall_clock_ms: None,
                }),
                expected_version: None,
            }),
        })
        .await;
    match replay.response {
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(
                snapshot.status,
                ais_agent_host::inspect::RunStatus::Completed
            );
        }
        other => panic!("unexpected replay response: {other:?}"),
    }

    let (
        run_repo,
        checkpoint_repo,
        _mission_repo,
        run_catalog_repo,
        event_archive,
        _session_store,
        _signer_state_archive,
    ) = service.into_parts();
    let hot_runtime = run_repo.load(&run_id).expect("rehydrated hot runtime");
    assert_eq!(
        hot_runtime.checkpoint.lifecycle.status,
        ais_agent_core::runtime::RunStatus::Completed
    );
    let checkpoint = checkpoint_repo
        .latest(run_id.0.as_str())
        .expect("latest checkpoint");
    assert_eq!(
        checkpoint.lifecycle.status,
        ais_agent_core::runtime::RunStatus::Completed
    );
    let catalog = run_catalog_repo.load(&run_id).expect("existing catalog");
    assert_eq!(
        catalog.status,
        ais_agent_core::runtime::RunStatus::Completed
    );
    let events = event_archive
        .read(crate::persistence::EventArchiveQuery {
            run_id,
            after_event_seq: Some(0),
            limit: Some(32),
        })
        .expect("events");
    assert!(events.latest_event_seq.is_some());
}

#[test]
fn runtime_host_service_emits_tracing_for_restore_and_commit_paths() {
    let (output, (response, catalog)) = capture_tracing_output(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
            .block_on(async {
                let (
                    _run_repo,
                    checkpoint_repo,
                    mission_repo,
                    run_catalog_repo,
                    event_archive,
                    session_store,
                    run_id,
                    host_session_id,
                ) = preloaded_evidence_wait_runtime();
                let traced_run_id = run_id.clone();
                let mut service = RuntimeHostService::new(
                    InMemoryRunRepository::default(),
                    checkpoint_repo,
                    mission_repo,
                    run_catalog_repo,
                    event_archive,
                    session_store,
                );

                let response = service
                    .handle(HostCommandEnvelope {
                        host_session_id,
                        host_request_id: Some("request-tracing-step".into()),
                        command: RunCommand::StepRun(StepRunCommand {
                            command_id: CommandId("cmd-tracing-step".to_owned()),
                            run_id,
                            until: StepUntil::NextBoundary,
                            budget: Some(StepBudget {
                                max_nodes: Some(4),
                                max_wall_clock_ms: None,
                            }),
                            expected_version: None,
                        }),
                    })
                    .await;

                let (
                    _run_repo,
                    _checkpoint_repo,
                    _mission_repo,
                    run_catalog_repo,
                    _event_archive,
                    _session_store,
                    _signer_state_archive,
                ) = service.into_parts();
                let catalog = run_catalog_repo
                    .load(&traced_run_id)
                    .expect("load catalog after traced step");

                (response, catalog)
            })
    });

    assert!(matches!(response.response, HostCommandResponse::Pause(_)));
    assert!(!output.trim().is_empty());
    assert_eq!(catalog.run_id.0, "run-1");
    assert!(catalog.latest_checkpoint_seq > 0);
    assert!(catalog.latest_revision > 0);
}

#[tokio::test]
async fn runtime_host_service_fails_closed_when_durable_signer_state_lags_hot_runtime() {
    let (
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
        run_id,
        host_session_id,
    ) = preloaded_signer_wait_runtime();
    let mut service = RuntimeHostService::new_with_signer_archive(
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
        FailingSignerStateArchive::fail_on_nth_write(1),
    );

    let failed = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-fail-signer-archive".into()),
            command: RunCommand::SubmitSignerDecision(SubmitSignerDecisionCommand {
                command_id: CommandId("cmd-fail-signer-archive".to_owned()),
                run_id: run_id.clone(),
                decision: SignerDecisionSubmission {
                    request_id: SignerRequestId("signer-1".to_owned()),
                    decision: SignerDecisionKind::Submitted,
                    tx_hash: Some("0xabc".to_owned()),
                    details: BTreeMap::new(),
                },
                expected_version: None,
            }),
        })
        .await;

    match failed.response {
        HostCommandResponse::Error(error) => assert_eq!(error.code, "signer_archive_error"),
        other => panic!("unexpected failure response: {other:?}"),
    }

    let replay = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: Some("request-fail-signer-archive-replay".into()),
            command: RunCommand::StepRun(StepRunCommand {
                command_id: CommandId("cmd-fail-signer-archive-replay".to_owned()),
                run_id: run_id.clone(),
                until: StepUntil::CompleteOrBoundary,
                budget: Some(StepBudget {
                    max_nodes: Some(4),
                    max_wall_clock_ms: None,
                }),
                expected_version: None,
            }),
        })
        .await;
    match replay.response {
        HostCommandResponse::Error(error) => assert_eq!(error.code, "restore_error"),
        other => panic!("unexpected replay response: {other:?}"),
    }

    let (
        run_repo,
        checkpoint_repo,
        _mission_repo,
        run_catalog_repo,
        _event_archive,
        _session_store,
        _signer_state_archive,
    ) = service.into_parts_with_signer_archive();
    let missing = run_repo
        .load(&run_id)
        .expect_err("hot runtime should be invalidated");
    assert_eq!(
        missing,
        crate::runtime::RunRepositoryError::NotFound {
            run_id: run_id.0.clone(),
        }
    );
    let checkpoint = checkpoint_repo
        .latest(run_id.0.as_str())
        .expect("latest checkpoint");
    assert_eq!(
        checkpoint
            .pending_requests
            .pending_signer_request_id
            .as_deref(),
        Some("signer-1")
    );
    let catalog = run_catalog_repo
        .load(&run_id)
        .expect("catalog entry may persist before signer member failure");
    assert_eq!(catalog.latest_checkpoint_seq, checkpoint.checkpoint_seq);
    assert_eq!(
        catalog.active_boundary_kind,
        checkpoint
            .lifecycle
            .active_boundary
            .as_ref()
            .map(|boundary| boundary.kind.clone())
    );
}

#[tokio::test]
async fn runtime_host_service_prefers_durable_checkpoint_after_event_archive_write_failure() {
    let (
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        _event_archive,
        session_store,
        run_id,
        host_session_id,
    ) = preloaded_evidence_wait_runtime();
    let mut service = RuntimeHostService::new(
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        FailingEventArchive::fail_on_nth_append(1),
        session_store,
    );

    let submit = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-event-archive-evidence".into()),
            command: RunCommand::SubmitEvidence(SubmitEvidenceCommand {
                command_id: CommandId("cmd-event-archive-evidence".to_owned()),
                run_id: run_id.clone(),
                evidence: EvidenceSubmission {
                    evidence_id: "quote".to_owned(),
                    kind: EvidenceKind::RouteOrQuote,
                    source: "quote-api".to_owned(),
                    observed_at_ms: Some(10),
                    chain_scope: Some("eip155:1".to_owned()),
                    payload: json!({"amount_out":"1000"}),
                    confidence: Some(0.95),
                },
                expected_version: None,
            }),
        })
        .await;
    assert!(matches!(submit.response, HostCommandResponse::Inspect(_)));

    let failed = service
        .handle(HostCommandEnvelope {
            host_session_id: host_session_id.clone(),
            host_request_id: Some("request-event-archive-step".into()),
            command: RunCommand::StepRun(StepRunCommand {
                command_id: CommandId("cmd-event-archive-step".to_owned()),
                run_id: run_id.clone(),
                until: StepUntil::CompleteOrBoundary,
                budget: Some(StepBudget {
                    max_nodes: Some(8),
                    max_wall_clock_ms: None,
                }),
                expected_version: None,
            }),
        })
        .await;

    match failed.response {
        HostCommandResponse::Error(error) => assert_eq!(error.code, "event_archive_error"),
        other => panic!("unexpected failure response: {other:?}"),
    }

    let replay = service
        .handle(HostCommandEnvelope {
            host_session_id,
            host_request_id: Some("request-event-archive-step-replay".into()),
            command: RunCommand::StepRun(StepRunCommand {
                command_id: CommandId("cmd-event-archive-step-replay".to_owned()),
                run_id: run_id.clone(),
                until: StepUntil::CompleteOrBoundary,
                budget: Some(StepBudget {
                    max_nodes: Some(8),
                    max_wall_clock_ms: None,
                }),
                expected_version: None,
            }),
        })
        .await;
    match replay.response {
        HostCommandResponse::Inspect(snapshot) => {
            assert_eq!(
                snapshot.status,
                ais_agent_host::inspect::RunStatus::Completed
            );
        }
        other => panic!("unexpected replay response: {other:?}"),
    }

    let (
        run_repo,
        checkpoint_repo,
        _mission_repo,
        run_catalog_repo,
        _event_archive,
        _session_store,
        _signer_state_archive,
    ) = service.into_parts();
    let hot_runtime = run_repo.load(&run_id).expect("rehydrated hot runtime");
    assert_eq!(
        hot_runtime.checkpoint.lifecycle.status,
        ais_agent_core::runtime::RunStatus::Completed
    );
    let checkpoint = checkpoint_repo
        .latest(run_id.0.as_str())
        .expect("latest checkpoint");
    assert_eq!(
        checkpoint.lifecycle.status,
        ais_agent_core::runtime::RunStatus::Completed
    );
    let catalog = run_catalog_repo.load(&run_id).expect("run catalog");
    assert_eq!(
        catalog.status,
        ais_agent_core::runtime::RunStatus::Completed
    );
}

fn preloaded_evidence_wait_runtime() -> (
    InMemoryRunRepository,
    InMemoryCheckpointRepository,
    InMemoryMissionRepository,
    InMemoryRunCatalogRepository,
    InMemoryEventArchive,
    InMemoryHostSessionStore,
    RunId,
    HostSessionId,
) {
    let run_id = RunId("run-1".to_owned());
    let host_session_id: HostSessionId = "session-evidence".into();
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
    checkpoint_repo
        .append(CheckpointArchiveEntry {
            snapshot: checkpoint,
            kind: CheckpointArchiveKind::Boundary,
        })
        .expect("save checkpoint");
    let mut session_store = InMemoryHostSessionStore::default();
    session_store.link_run(HostRunLink::new(
        host_session_id.clone(),
        run_id.clone(),
        mission.goal.clone(),
        mission.allowed_chains.clone(),
    ));
    let mission_repo = preloaded_mission_repo(run_id.clone(), mission);
    let run_catalog_repo = InMemoryRunCatalogRepository::default();
    let event_archive = InMemoryEventArchive::default();

    (
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
        run_id,
        host_session_id,
    )
}

fn preloaded_signer_wait_runtime() -> (
    InMemoryRunRepository,
    InMemoryCheckpointRepository,
    InMemoryMissionRepository,
    InMemoryRunCatalogRepository,
    InMemoryEventArchive,
    InMemoryHostSessionStore,
    RunId,
    HostSessionId,
) {
    let run_id = RunId("run-1".to_owned());
    let host_session_id: HostSessionId = "session-signer".into();
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
    checkpoint_repo
        .append(CheckpointArchiveEntry {
            snapshot: checkpoint,
            kind: CheckpointArchiveKind::Boundary,
        })
        .expect("save checkpoint");
    let mut session_store = InMemoryHostSessionStore::default();
    session_store.link_run(HostRunLink::new(
        host_session_id.clone(),
        run_id.clone(),
        mission.goal.clone(),
        mission.allowed_chains.clone(),
    ));
    let mission_repo = preloaded_mission_repo(run_id.clone(), mission);
    let run_catalog_repo = InMemoryRunCatalogRepository::default();
    let event_archive = InMemoryEventArchive::default();

    (
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
        run_id,
        host_session_id,
    )
}

fn preloaded_solana_signer_wait_runtime() -> (
    InMemoryRunRepository,
    InMemoryCheckpointRepository,
    InMemoryMissionRepository,
    InMemoryRunCatalogRepository,
    InMemoryEventArchive,
    InMemoryHostSessionStore,
    RunId,
    HostSessionId,
) {
    let run_id = RunId("run-1".to_owned());
    let host_session_id: HostSessionId = "session-signer-solana".into();
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
    checkpoint_repo
        .append(CheckpointArchiveEntry {
            snapshot: checkpoint,
            kind: CheckpointArchiveKind::Boundary,
        })
        .expect("save checkpoint");
    let mut session_store = InMemoryHostSessionStore::default();
    session_store.link_run(HostRunLink::new(
        host_session_id.clone(),
        run_id.clone(),
        mission.goal.clone(),
        mission.allowed_chains.clone(),
    ));
    let mission_repo = preloaded_mission_repo(run_id.clone(), mission);
    let run_catalog_repo = InMemoryRunCatalogRepository::default();
    let event_archive = InMemoryEventArchive::default();

    (
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
        run_id,
        host_session_id,
    )
}

fn preloaded_mission_repo(run_id: RunId, mission: Mission) -> InMemoryMissionRepository {
    let mut mission_repo = InMemoryMissionRepository::default();
    mission_repo
        .insert(run_id, mission)
        .expect("insert mission");
    mission_repo
}

#[derive(Debug)]
struct FailingRunCatalogRepository {
    inner: InMemoryRunCatalogRepository,
    upsert_calls: usize,
    fail_on_nth_upsert: usize,
}

#[derive(Debug)]
struct FailingMissionRepository {
    inner: InMemoryMissionRepository,
    writes: usize,
    fail_on_nth_insert: Option<usize>,
    fail_on_nth_upsert: Option<usize>,
}

#[derive(Debug)]
struct FailingCheckpointRepository {
    inner: InMemoryCheckpointRepository,
    appends: usize,
    fail_on_nth_append: usize,
}

#[derive(Debug, Default)]
struct FailingSignerStateArchive {
    inner: InMemorySignerStateArchive,
    writes: usize,
    fail_on_nth_write: usize,
}

impl FailingSignerStateArchive {
    fn fail_on_nth_write(fail_on_nth_write: usize) -> Self {
        Self {
            inner: InMemorySignerStateArchive::default(),
            writes: 0,
            fail_on_nth_write,
        }
    }
}

impl FailingMissionRepository {
    fn fail_on_nth_insert(fail_on_nth_insert: usize) -> Self {
        Self {
            inner: InMemoryMissionRepository::default(),
            writes: 0,
            fail_on_nth_insert: Some(fail_on_nth_insert),
            fail_on_nth_upsert: None,
        }
    }
}

impl MissionRepository for FailingMissionRepository {
    fn insert(&mut self, run_id: RunId, mission: Mission) -> Result<(), MissionRepositoryError> {
        self.writes = self.writes.saturating_add(1);
        if self.fail_on_nth_insert == Some(self.writes) {
            return Err(MissionRepositoryError::Storage {
                message: "injected mission repository failure".to_owned(),
            });
        }
        self.inner.insert(run_id, mission)
    }

    fn upsert(&mut self, run_id: RunId, mission: Mission) -> Result<(), MissionRepositoryError> {
        self.writes = self.writes.saturating_add(1);
        if self.fail_on_nth_upsert == Some(self.writes) {
            return Err(MissionRepositoryError::Storage {
                message: "injected mission repository failure".to_owned(),
            });
        }
        self.inner.upsert(run_id, mission)
    }

    fn load(&self, run_id: &RunId) -> Result<Mission, MissionRepositoryError> {
        self.inner.load(run_id)
    }
}

impl FailingCheckpointRepository {
    fn fail_on_nth_append(inner: InMemoryCheckpointRepository, fail_on_nth_append: usize) -> Self {
        Self {
            inner,
            appends: 0,
            fail_on_nth_append,
        }
    }
}

impl CheckpointRepository for FailingCheckpointRepository {
    fn latest(
        &self,
        run_id: &str,
    ) -> Result<CheckpointSnapshot, crate::persistence::CheckpointRepositoryError> {
        self.inner.latest(run_id)
    }

    fn append(
        &mut self,
        entry: CheckpointArchiveEntry,
    ) -> Result<(), crate::persistence::CheckpointRepositoryError> {
        self.appends = self.appends.saturating_add(1);
        if self.appends == self.fail_on_nth_append {
            return Err(crate::persistence::CheckpointRepositoryError::Storage {
                message: "injected checkpoint repository failure".to_owned(),
            });
        }
        self.inner.append(entry)
    }

    fn history(
        &self,
        run_id: &str,
    ) -> Result<Vec<CheckpointArchiveEntry>, crate::persistence::CheckpointRepositoryError> {
        self.inner.history(run_id)
    }
}

impl SignerStateArchive for FailingSignerStateArchive {
    fn upsert(&mut self, signer_state: SignerRequestState) -> Result<(), SignerStateArchiveError> {
        self.writes = self.writes.saturating_add(1);
        if self.writes == self.fail_on_nth_write {
            return Err(SignerStateArchiveError::Storage {
                message: "injected signer archive failure".to_owned(),
            });
        }
        self.inner.upsert(signer_state)
    }

    fn load(&self, run_id: &RunId) -> Result<SignerRequestState, SignerStateArchiveError> {
        self.inner.load(run_id)
    }

    fn clear(&mut self, run_id: &RunId) -> Result<(), SignerStateArchiveError> {
        self.writes = self.writes.saturating_add(1);
        if self.writes == self.fail_on_nth_write {
            return Err(SignerStateArchiveError::Storage {
                message: "injected signer archive failure".to_owned(),
            });
        }
        self.inner.clear(run_id)
    }
}

#[derive(Debug, Default)]
struct FailingEventArchive {
    inner: InMemoryEventArchive,
    appends: usize,
    fail_on_nth_append: usize,
}

impl FailingEventArchive {
    fn fail_on_nth_append(fail_on_nth_append: usize) -> Self {
        Self {
            inner: InMemoryEventArchive::default(),
            appends: 0,
            fail_on_nth_append,
        }
    }
}

impl EventArchive for FailingEventArchive {
    fn append(
        &mut self,
        event: ais_agent_control::events::RunEventEnvelope,
    ) -> Result<(), EventArchiveError> {
        self.appends = self.appends.saturating_add(1);
        if self.appends == self.fail_on_nth_append {
            return Err(EventArchiveError::Storage {
                message: "injected event archive failure".to_owned(),
            });
        }
        self.inner.append(event)
    }

    fn read(
        &self,
        query: EventArchiveQuery,
    ) -> Result<crate::persistence::EventArchiveSlice, EventArchiveError> {
        self.inner.read(query)
    }
}

impl FailingRunCatalogRepository {
    fn fail_on_nth_upsert(fail_on_nth_upsert: usize) -> Self {
        Self {
            inner: InMemoryRunCatalogRepository::default(),
            upsert_calls: 0,
            fail_on_nth_upsert,
        }
    }
}

impl RunCatalogRepository for FailingRunCatalogRepository {
    fn upsert(&mut self, entry: RunCatalogEntry) -> Result<(), RunCatalogRepositoryError> {
        self.upsert_calls = self.upsert_calls.saturating_add(1);
        if self.upsert_calls == self.fail_on_nth_upsert {
            return Err(RunCatalogRepositoryError::Storage {
                message: "injected run catalog failure".to_owned(),
            });
        }
        self.inner.upsert(entry)
    }

    fn load(&self, run_id: &RunId) -> Result<RunCatalogEntry, RunCatalogRepositoryError> {
        self.inner.load(run_id)
    }
}

fn preloaded_envelope_wait_runtime() -> (
    InMemoryRunRepository,
    InMemoryCheckpointRepository,
    InMemoryMissionRepository,
    InMemoryRunCatalogRepository,
    InMemoryEventArchive,
    InMemoryHostSessionStore,
    RunId,
    HostSessionId,
) {
    let run_id = RunId("run-1".to_owned());
    let host_session_id: HostSessionId = "session-envelope".into();
    let mission = sample_mission();
    let mut checkpoint = checkpoint_with_nodes(vec![actuate_blocked_node("swap", vec![])], vec![]);
    checkpoint.pending_requests.pending_envelope_refs = vec!["env.swap".to_owned()];
    checkpoint.lifecycle.pause_with_failure(
        RunFailureStage::Broadcast,
        RunFailureCode::EnvelopeInvalid,
        "broadcast requires replacement envelope",
    );
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
    checkpoint_repo
        .append(CheckpointArchiveEntry {
            snapshot: checkpoint,
            kind: CheckpointArchiveKind::Boundary,
        })
        .expect("save checkpoint");
    let mut session_store = InMemoryHostSessionStore::default();
    session_store.link_run(HostRunLink::new(
        host_session_id.clone(),
        run_id.clone(),
        mission.goal.clone(),
        mission.allowed_chains.clone(),
    ));
    let mission_repo = preloaded_mission_repo(run_id.clone(), mission);
    let run_catalog_repo = InMemoryRunCatalogRepository::default();
    let event_archive = InMemoryEventArchive::default();

    (
        run_repo,
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        session_store,
        run_id,
        host_session_id,
    )
}

fn sample_envelope_submission(envelope_id: &str) -> EnvelopeSubmission {
    EnvelopeSubmission {
        envelope_id: envelope_id.to_owned(),
        kind: EnvelopeKind::EvmEnvelope,
        chain: "eip155:1".to_owned(),
        payload: json!({"raw_tx":"0x0102"}),
        expected_effect: Some(json!({
            "effect_id":"effect.swap",
            "kind":"state_transition",
            "assertions":[],
            "tolerance_hint":null
        })),
        provenance: Some("host.recovery".to_owned()),
    }
}

fn response_ownership(
    response: &HostCommandResponse,
) -> &ais_agent_control::ownership::RunOwnershipSnapshot {
    match response {
        HostCommandResponse::Inspect(snapshot) => &snapshot.ownership,
        HostCommandResponse::Pause(bundle) => &bundle.ownership,
        other => panic!("unexpected response shape: {other:?}"),
    }
}

fn sample_runtime_claim(
    run_id: &RunId,
    host_session_id: &HostSessionId,
    claim_id: &str,
    claim_epoch: u64,
    lease_expires_at_ms: Option<u64>,
) -> ais_agent_control::ownership::RunClaim {
    ais_agent_control::ownership::RunClaim {
        claim_id: ais_agent_control::ids::ClaimId(claim_id.to_owned()),
        run_id: run_id.clone(),
        host_session_id: host_session_id.0.clone(),
        owner_kind: RunClaimOwnerKind::InteractiveHost,
        owner_instance_id: host_session_id.0.clone(),
        lease_started_at_ms: 1,
        lease_expires_at_ms,
        last_renewed_at_ms: Some(1),
        claim_epoch,
        mode: RunClaimMode::ExclusiveMutation,
        status: ais_agent_control::ownership::RunClaimStatus::Active,
    }
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
        mission_id: "mission-sol-1".to_owned(),
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
    }
}

fn confirmation_wait_checkpoint() -> CheckpointSnapshot {
    let mut checkpoint = checkpoint_with_nodes(
        vec![
            actuate_blocked_node("swap", vec![]),
            verify_terminal_node("verify-swap", vec!["swap"]),
        ],
        vec!["verify-swap".to_owned()],
    );
    checkpoint
        .lifecycle
        .await_confirmation("waiting for receipt 0xabc");
    checkpoint.lifecycle.phase = RunPhase::AwaitingHost;
    checkpoint.pending_requests.pending_confirmation_id = Some("0xabc".to_owned());
    checkpoint.last_completed_node_id = Some("swap".to_owned());
    checkpoint
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
            live: Some(ActuateLiveBinding::Solana(SolanaActuateLiveBinding {
                connection: Some(SolanaConnectionSpec {
                    rpc_url: "http://localhost:8899".to_owned(),
                    ws_url: None,
                }),
                binding: SolanaActuateBinding::BroadcastSignedTransaction,
            })),
        }),
        implementation_hint: Some("solana.broadcast".to_owned()),
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
            live: Some(VerifyLiveBinding::Solana(SolanaVerifyLiveBinding {
                connection: Some(SolanaConnectionSpec {
                    rpc_url: "http://localhost:8899".to_owned(),
                    ws_url: None,
                }),
                binding: SolanaVerifyBinding::EffectContractFromSignatureStatus,
                post_request: None,
            })),
        }),
        implementation_hint: Some("solana.verify".to_owned()),
        expected_effect_ref: Some("effect.sol".to_owned()),
    }
}
