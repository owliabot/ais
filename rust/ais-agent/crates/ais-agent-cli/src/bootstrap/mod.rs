use std::{fs, path::Path};

use ais_agent_core::binding::{evm::EvmConnectionSpec, solana::SolanaConnectionSpec};
use ais_agent_runtime::{
    persistence::{
        InMemoryCheckpointRepository, InMemoryEventArchive, InMemoryMissionRepository,
        InMemoryRunCatalogRepository,
    },
    runtime::InMemoryRunRepository,
    service::{
        RuntimeChainConnection, RuntimeChainProviderEntry, RuntimeExecutionWiring,
        RuntimeHostService, RuntimeProviderRegistry,
    },
};
use ais_agent_store_sqlite::SqliteStore;
use tracing::{info, warn};

use crate::{
    config::{
        AisAgentChainProviderEntry as ConfigChainProviderEntry, AisAgentProviderConfig,
        AisAgentServiceConfig, AisAgentStorageConfig,
    },
    service::{CliHostService, SqliteCliRuntimeHostService},
    storage_maintenance::{current_time_ms, maybe_auto_prune_sqlite},
};

pub fn build_host_service(config: &AisAgentServiceConfig) -> CliHostService {
    build_host_service_with_now(config, current_time_ms())
}

fn build_host_service_with_now(config: &AisAgentServiceConfig, now_ms: i64) -> CliHostService {
    let execution_wiring = build_execution_wiring(&config.providers);

    match &config.storage {
        AisAgentStorageConfig::InMemory => CliHostService::RuntimeInMemory(
            RuntimeHostService::new(
                InMemoryRunRepository::default(),
                InMemoryCheckpointRepository::default(),
                InMemoryMissionRepository::default(),
                InMemoryRunCatalogRepository::default(),
                InMemoryEventArchive::default(),
                ais_agent_host::session::InMemoryHostSessionStore::default(),
            )
            .with_execution_wiring(execution_wiring),
        ),
        AisAgentStorageConfig::Sqlite(sqlite) => {
            match maybe_auto_prune_sqlite(sqlite, now_ms) {
                Ok(Some(result)) => info!(
                    deleted_checkpoints = result.deleted_checkpoints,
                    deleted_wait_states = result.deleted_wait_states,
                    terminal_before_ms = result.terminal_before_ms,
                    wait_state_orphan_before_ms = result.wait_state_orphan_before_ms,
                    "ais_agent.sqlite_auto_prune_executed"
                ),
                Ok(None) => {}
                Err(error) => warn!(
                    sqlite_path = %sqlite.path.display(),
                    message = %error,
                    "ais_agent.sqlite_auto_prune_failed"
                ),
            }
            match build_sqlite_host_service(
                &sqlite.path,
                sqlite.create_if_missing,
                execution_wiring,
            ) {
                Ok(service) => CliHostService::RuntimeSqlite(service),
                Err(message) => CliHostService::unavailable("runtime_bootstrap_failed", message),
            }
        }
    }
}

fn build_execution_wiring(config: &AisAgentProviderConfig) -> RuntimeExecutionWiring {
    let chains = config
        .chains
        .iter()
        .map(map_chain_provider_entry)
        .map(|entry| (entry.chain.to_ascii_lowercase(), entry))
        .collect();

    RuntimeExecutionWiring {
        providers: RuntimeProviderRegistry { chains },
    }
}

fn map_chain_provider_entry(entry: &ConfigChainProviderEntry) -> RuntimeChainProviderEntry {
    let connection = match chain_scope_family(&entry.chain) {
        Some("eip155") => RuntimeChainConnection::Evm(EvmConnectionSpec {
            http_url: entry.connection.http_url.clone(),
            ws_url: entry.connection.ws_url.clone(),
        }),
        Some("solana") => RuntimeChainConnection::Solana(SolanaConnectionSpec {
            http_url: entry.connection.http_url.clone(),
            ws_url: entry.connection.ws_url.clone(),
        }),
        other => panic!(
            "unsupported provider chain scope `{}` in bootstrap mapping: {other:?}",
            entry.chain
        ),
    };
    RuntimeChainProviderEntry {
        chain: entry.chain.clone(),
        labels: entry.labels.clone(),
        connection,
    }
}

fn chain_scope_family(value: &str) -> Option<&str> {
    let (prefix, suffix) = value.split_once(':')?;
    if prefix.is_empty() || suffix.trim().is_empty() {
        return None;
    }
    match prefix {
        "eip155" | "solana" => Some(prefix),
        _ => None,
    }
}

fn build_sqlite_host_service(
    path: &Path,
    create_if_missing: bool,
    execution_wiring: RuntimeExecutionWiring,
) -> Result<SqliteCliRuntimeHostService, String> {
    if !path.exists() && !create_if_missing {
        return Err(format!(
            "ais-agent SQLite store does not exist and create_if_missing=false: {}",
            path.display()
        ));
    }

    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() && create_if_missing {
            fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "failed to create SQLite parent directory {}: {error}",
                    parent.display()
                )
            })?;
        }
    }

    let checkpoint_repo = SqliteStore::open_path(path).map_err(|error| {
        format!(
            "failed to open checkpoint archive store {}: {error}",
            path.display()
        )
    })?;
    let mission_repo = SqliteStore::open_path(path)
        .map_err(|error| format!("failed to open mission store {}: {error}", path.display()))?;
    let run_catalog_repo = SqliteStore::open_path(path).map_err(|error| {
        format!(
            "failed to open run catalog store {}: {error}",
            path.display()
        )
    })?;
    let event_archive = SqliteStore::open_path(path).map_err(|error| {
        format!(
            "failed to open event archive store {}: {error}",
            path.display()
        )
    })?;
    let signer_state_store = SqliteStore::open_path(path).map_err(|error| {
        format!(
            "failed to open signer state store {}: {error}",
            path.display()
        )
    })?;
    let audit_archive = SqliteStore::open_path(path).map_err(|error| {
        format!(
            "failed to open runtime audit store {}: {error}",
            path.display()
        )
    })?;
    let claim_repo = SqliteStore::open_path(path)
        .map_err(|error| format!("failed to open claim store {}: {error}", path.display()))?;

    Ok(RuntimeHostService::new_with_archives_and_claim_repo(
        InMemoryRunRepository::default(),
        checkpoint_repo,
        mission_repo,
        run_catalog_repo,
        event_archive,
        ais_agent_host::session::InMemoryHostSessionStore::default(),
        signer_state_store,
        audit_archive,
        claim_repo,
    )
    .with_execution_wiring(execution_wiring))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use ais_agent_control::{
        commands::{BeginRunCommand, MissionSubmission, RunCommand},
        execution_artifact::{
            EvmTransactionCandidate, ExecutionArtifactLaunchSpec, ExecutionChainFamily,
            ExecutionStage, ExecutionTransactionCandidate, TransactionStage,
        },
        ids::{CommandId, IdempotencyKey, RunId},
        launch_spec::LaunchSpecSubmission,
    };
    use ais_agent_host::{
        control::{HostCommandResponse, HostCommandService},
        session::{HostCommandEnvelope, HostSessionId},
    };
    use ais_agent_store_sqlite::{
        MaintenanceOperationKind, MaintenanceOperationStatus, SqliteStore, StoreMaintenanceState,
        StoredRunCheckpoint, StoredRunHead, STORE_METADATA_SCHEMA_VERSION,
        STORE_RETENTION_SCHEMA_VERSION,
    };
    use serde_json::json;

    use crate::{
        config::types::AisAgentSqliteStorageConfig,
        config::{
            AisAgentChainConnectionConfig, AisAgentChainProviderEntry, AisAgentProviderConfig,
            AisAgentServiceConfig, AisAgentStorageConfig,
        },
    };

    use super::{build_execution_wiring, build_host_service};

    fn service_config_with_exact_evm_provider(
        storage: AisAgentStorageConfig,
    ) -> AisAgentServiceConfig {
        AisAgentServiceConfig {
            storage,
            providers: AisAgentProviderConfig {
                chains: vec![AisAgentChainProviderEntry {
                    chain: "eip155:1".to_owned(),
                    labels: vec!["ethereum".to_owned(), "mainnet".to_owned()],
                    connection: AisAgentChainConnectionConfig {
                        http_url: "http://127.0.0.1:8545".to_owned(),
                        ws_url: None,
                    },
                }],
                ..AisAgentProviderConfig::default()
            },
            ..AisAgentServiceConfig::default()
        }
    }

    fn begin_run_command() -> ais_agent_host::session::HostedRunCommand {
        HostCommandEnvelope {
            host_session_id: HostSessionId("session-1".to_owned()),
            host_request_id: None,
            command: RunCommand::BeginRun(BeginRunCommand {
                command_id: CommandId("cmd-1".to_owned()),
                idempotency_key: IdempotencyKey("idem-1".to_owned()),
                mission: MissionSubmission {
                    goal: "transfer".to_owned(),
                    allowed_chains: vec!["eip155:1".to_owned()],
                    constraints: Default::default(),
                    budget: None,
                    metadata: Default::default(),
                },
                launch_spec: Some(LaunchSpecSubmission::ExecutionArtifact(
                    ExecutionArtifactLaunchSpec {
                        protocol_package_id: "owliabot.transfer".to_owned(),
                        action_key: "native_transfer".to_owned(),
                        chain_family: ExecutionChainFamily::Evm,
                        allowed_chains: vec!["eip155:1".to_owned()],
                        entry_stage_id: "stage.transfer".into(),
                        actor: None,
                        transactions: vec![ExecutionTransactionCandidate::EvmTransaction(
                            EvmTransactionCandidate {
                                candidate_id: "transfer.direct".into(),
                                to: "0x1111111111111111111111111111111111111111".to_owned(),
                                value: Some("1".to_owned()),
                                calldata: None,
                            },
                        )],
                        stages: vec![ExecutionStage::Transaction(TransactionStage {
                            stage_id: "stage.transfer".into(),
                            candidate_ref: "transfer.direct".into(),
                            exports: Vec::new(),
                            next_stage_id: None,
                        })],
                        observations: Vec::new(),
                        preconditions: Vec::new(),
                        postconditions: Vec::new(),
                        expected_effects: Vec::new(),
                        execution_policy: None,
                        risk_class: None,
                        risk_tags: Vec::new(),
                        decoded_intent: None,
                        candidate_envelopes: Vec::new(),
                        decode_spec: None,
                        validation_plan: None,
                        evidence: serde_json::json!({}),
                        metadata: Default::default(),
                    },
                )),
            }),
        }
    }

    #[tokio::test]
    async fn in_memory_bootstrap_builds_runtime_service() {
        let mut service = build_host_service(&service_config_with_exact_evm_provider(
            AisAgentStorageConfig::InMemory,
        ));
        let outcome = service.handle(begin_run_command()).await;

        match outcome.response {
            HostCommandResponse::Accepted(response) => {
                assert_eq!(response.run_id, Some(RunId("run-1".to_owned())));
            }
            other => panic!("unexpected response: {other:?}"),
        }
    }

    #[tokio::test]
    async fn sqlite_bootstrap_builds_runtime_service() {
        let sqlite_path = std::env::temp_dir().join("ais-agent-cli-bootstrap.sqlite");
        let _ = fs::remove_file(&sqlite_path);
        let mut service = build_host_service(&service_config_with_exact_evm_provider(
            AisAgentStorageConfig::Sqlite(AisAgentSqliteStorageConfig {
                path: sqlite_path.clone(),
                create_if_missing: true,
                ..AisAgentSqliteStorageConfig::default()
            }),
        ));
        let outcome = service.handle(begin_run_command()).await;

        match outcome.response {
            HostCommandResponse::Accepted(response) => {
                assert_eq!(response.run_id, Some(RunId("run-1".to_owned())));
            }
            other => panic!("unexpected response: {other:?}"),
        }

        let _ = fs::remove_file(sqlite_path);
    }

    #[test]
    fn build_execution_wiring_maps_chain_registry_entries() {
        let wiring = build_execution_wiring(&AisAgentProviderConfig {
            chains: vec![AisAgentChainProviderEntry {
                chain: "eip155:8453".to_owned(),
                labels: vec!["base".to_owned(), "base-mainnet".to_owned()],
                connection: AisAgentChainConnectionConfig {
                    http_url: "https://base.example".to_owned(),
                    ws_url: Some("wss://base.example/ws".to_owned()),
                },
            }],
            ..AisAgentProviderConfig::default()
        });

        let resolved = wiring
            .resolve_chain_connection("eip155:8453")
            .expect("resolve")
            .expect("provider");
        let ais_agent_runtime::service::RuntimeChainConnectionRef::Evm(connection) = resolved
        else {
            panic!("expected evm connection");
        };
        assert_eq!(connection.http_url, "https://base.example");
        assert_eq!(connection.ws_url.as_deref(), Some("wss://base.example/ws"));
    }

    #[test]
    fn build_execution_wiring_requires_exact_chain_match() {
        let wiring = build_execution_wiring(&AisAgentProviderConfig {
            chains: vec![AisAgentChainProviderEntry {
                chain: "eip155:8453".to_owned(),
                labels: vec!["base".to_owned()],
                connection: AisAgentChainConnectionConfig {
                    http_url: "https://base.example".to_owned(),
                    ws_url: None,
                },
            }],
        });

        assert_eq!(
            wiring
                .resolve_chain_connection("eip155:1")
                .expect("resolve evm"),
            None
        );
    }

    #[test]
    fn sqlite_bootstrap_runs_auto_prune_when_retention_window_is_due() {
        let sqlite_path = std::env::temp_dir().join("ais-agent-cli-auto-prune-due.sqlite");
        let _ = fs::remove_file(&sqlite_path);
        let mut store = SqliteStore::open_path(&sqlite_path).expect("open sqlite");
        store
            .upsert_run_head(&StoredRunHead {
                run_id: "run-prune".to_owned(),
                mission_id: "mission-prune".to_owned(),
                status: "completed".to_owned(),
                phase: Some("completed".to_owned()),
                active_boundary_kind: None,
                active_wait_kind: None,
                latest_checkpoint_seq: Some(2),
                latest_event_seq: None,
                latest_audit_seq: None,
                latest_claim_epoch: None,
                retention_mode: Some("terminal_tiered".to_owned()),
                created_at_ms: Some(1_000),
                updated_at_ms: Some(1_000),
                terminal_at_ms: Some(1_000),
            })
            .expect("upsert head");
        store
            .append_run_checkpoint_record(&StoredRunCheckpoint {
                checkpoint_id: None,
                run_id: "run-prune".to_owned(),
                checkpoint_seq: 1,
                plan_epoch: 0,
                checkpoint_kind: "boundary".to_owned(),
                retention_tier: "terminal_intermediate".to_owned(),
                created_at_ms: 900,
                is_terminal: false,
                is_side_effect_boundary: false,
                is_recovery_boundary: false,
                is_first_wait_checkpoint: false,
                snapshot: json!({"checkpoint_seq":1}),
            })
            .expect("append intermediate checkpoint");
        store
            .append_run_checkpoint_record(&StoredRunCheckpoint {
                checkpoint_id: None,
                run_id: "run-prune".to_owned(),
                checkpoint_seq: 2,
                plan_epoch: 0,
                checkpoint_kind: "boundary".to_owned(),
                retention_tier: "terminal_final".to_owned(),
                created_at_ms: 1_000,
                is_terminal: true,
                is_side_effect_boundary: false,
                is_recovery_boundary: false,
                is_first_wait_checkpoint: false,
                snapshot: json!({"checkpoint_seq":2}),
            })
            .expect("append final checkpoint");
        store
            .upsert_store_maintenance_state(&StoreMaintenanceState {
                last_operation_kind: Some(MaintenanceOperationKind::Prune),
                last_operation_status: Some(MaintenanceOperationStatus::Succeeded),
                last_store_opened_at_ms: Some(1_000),
                last_prune_started_at_ms: Some(1_000),
                last_prune_finished_at_ms: Some(1_000),
                last_pruned_terminal_before_ms: Some(900),
                last_prune_deleted_rows: Some(0),
                last_purge_deleted_rows: None,
                last_vacuum_started_at_ms: None,
                last_vacuum_finished_at_ms: None,
                last_vacuum_at_ms: None,
                last_wal_checkpoint_at_ms: None,
                last_known_page_count: Some(1),
                last_known_freelist_count: Some(0),
                last_known_db_bytes: Some(4_096),
                last_growth_sampled_at_ms: Some(1_000),
                schema_retention_version: STORE_RETENTION_SCHEMA_VERSION,
                metadata_schema_version: STORE_METADATA_SCHEMA_VERSION,
            })
            .expect("seed maintenance state");
        drop(store);

        let mut config = AisAgentServiceConfig {
            storage: AisAgentStorageConfig::Sqlite(AisAgentSqliteStorageConfig {
                path: sqlite_path.clone(),
                create_if_missing: true,
                ..AisAgentSqliteStorageConfig::default()
            }),
            ..AisAgentServiceConfig::default()
        };
        let sqlite = match &mut config.storage {
            AisAgentStorageConfig::Sqlite(sqlite) => sqlite,
            AisAgentStorageConfig::InMemory => unreachable!(),
        };
        sqlite.retention.auto_prune_cadence_minutes = 60;
        sqlite.retention.checkpoint_full_window_days = 1;
        sqlite.retention.wait_state_orphan_ttl_days = 1;

        let _service =
            super::build_host_service_with_now(&config, 2 * 24 * 60 * 60 * 1_000 + 1_000);

        let store = SqliteStore::open_path(&sqlite_path).expect("reopen sqlite");
        let remaining_intermediate: i64 = store
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM run_checkpoints WHERE run_id = 'run-prune' AND retention_tier = 'terminal_intermediate'",
                [],
                |row| row.get(0),
            )
            .expect("count remaining intermediate");
        assert_eq!(remaining_intermediate, 0);
        let journal = store.list_maintenance_journal(5).expect("list journal");
        assert!(journal
            .iter()
            .any(|entry| entry.operation_kind == MaintenanceOperationKind::Prune));

        let _ = fs::remove_file(sqlite_path);
    }

    #[test]
    fn sqlite_bootstrap_skips_auto_prune_when_cadence_not_due() {
        let sqlite_path = std::env::temp_dir().join("ais-agent-cli-auto-prune-skip.sqlite");
        let _ = fs::remove_file(&sqlite_path);
        let mut store = SqliteStore::open_path(&sqlite_path).expect("open sqlite");
        store
            .upsert_run_head(&StoredRunHead {
                run_id: "run-prune".to_owned(),
                mission_id: "mission-prune".to_owned(),
                status: "completed".to_owned(),
                phase: Some("completed".to_owned()),
                active_boundary_kind: None,
                active_wait_kind: None,
                latest_checkpoint_seq: Some(2),
                latest_event_seq: None,
                latest_audit_seq: None,
                latest_claim_epoch: None,
                retention_mode: Some("terminal_tiered".to_owned()),
                created_at_ms: Some(1_000),
                updated_at_ms: Some(1_000),
                terminal_at_ms: Some(1_000),
            })
            .expect("upsert head");
        store
            .append_run_checkpoint_record(&StoredRunCheckpoint {
                checkpoint_id: None,
                run_id: "run-prune".to_owned(),
                checkpoint_seq: 1,
                plan_epoch: 0,
                checkpoint_kind: "boundary".to_owned(),
                retention_tier: "terminal_intermediate".to_owned(),
                created_at_ms: 900,
                is_terminal: false,
                is_side_effect_boundary: false,
                is_recovery_boundary: false,
                is_first_wait_checkpoint: false,
                snapshot: json!({"checkpoint_seq":1}),
            })
            .expect("append intermediate checkpoint");
        store
            .upsert_store_maintenance_state(&StoreMaintenanceState {
                last_operation_kind: Some(MaintenanceOperationKind::Prune),
                last_operation_status: Some(MaintenanceOperationStatus::Succeeded),
                last_store_opened_at_ms: Some(1_000),
                last_prune_started_at_ms: Some(1_000),
                last_prune_finished_at_ms: Some(1_000),
                last_pruned_terminal_before_ms: Some(900),
                last_prune_deleted_rows: Some(0),
                last_purge_deleted_rows: None,
                last_vacuum_started_at_ms: None,
                last_vacuum_finished_at_ms: None,
                last_vacuum_at_ms: None,
                last_wal_checkpoint_at_ms: None,
                last_known_page_count: Some(1),
                last_known_freelist_count: Some(0),
                last_known_db_bytes: Some(4_096),
                last_growth_sampled_at_ms: Some(1_000),
                schema_retention_version: STORE_RETENTION_SCHEMA_VERSION,
                metadata_schema_version: STORE_METADATA_SCHEMA_VERSION,
            })
            .expect("seed maintenance state");
        drop(store);

        let mut config = AisAgentServiceConfig {
            storage: AisAgentStorageConfig::Sqlite(AisAgentSqliteStorageConfig {
                path: sqlite_path.clone(),
                create_if_missing: true,
                ..AisAgentSqliteStorageConfig::default()
            }),
            ..AisAgentServiceConfig::default()
        };
        let sqlite = match &mut config.storage {
            AisAgentStorageConfig::Sqlite(sqlite) => sqlite,
            AisAgentStorageConfig::InMemory => unreachable!(),
        };
        sqlite.retention.auto_prune_cadence_minutes = 60;

        let _service = super::build_host_service_with_now(&config, 30 * 60 * 1_000 + 1_000);

        let store = SqliteStore::open_path(&sqlite_path).expect("reopen sqlite");
        let remaining_intermediate: i64 = store
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM run_checkpoints WHERE run_id = 'run-prune' AND retention_tier = 'terminal_intermediate'",
                [],
                |row| row.get(0),
            )
            .expect("count remaining intermediate");
        assert_eq!(remaining_intermediate, 1);
        let journal = store.list_maintenance_journal(5).expect("list journal");
        assert!(
            !journal
                .iter()
                .any(|entry| entry.operation_kind == MaintenanceOperationKind::Prune),
            "no auto-prune journal should be appended when cadence is not due"
        );

        let _ = fs::remove_file(sqlite_path);
    }
}
