use std::{fs, path::Path};

use ais_agent_runtime::{
    persistence::{
        InMemoryCheckpointRepository, InMemoryEventArchive, InMemoryMissionRepository,
        InMemoryRunCatalogRepository,
    },
    runtime::InMemoryRunRepository,
    service::{RuntimeExecutionWiring, RuntimeHostService},
};
use ais_agent_store_sqlite::SqliteStore;

use crate::{
    config::{AisAgentServiceConfig, AisAgentStorageConfig},
    service::{CliHostService, SqliteCliRuntimeHostService},
};

pub fn build_host_service(config: &AisAgentServiceConfig) -> CliHostService {
    let execution_wiring = RuntimeExecutionWiring {
        evm_rpc_url: config.providers.evm_rpc_url.clone(),
        solana_rpc_url: config.providers.solana_rpc_url.clone(),
        allowed_protocol_packages: config.protocol_packages.allow.clone(),
    };

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
    let signer_state_archive = SqliteStore::open_path(path).map_err(|error| {
        format!(
            "failed to open signer archive store {}: {error}",
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
        signer_state_archive,
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
        ids::{CommandId, IdempotencyKey, RunId},
    };
    use ais_agent_host::{
        control::{HostCommandResponse, HostCommandService},
        session::{HostCommandEnvelope, HostSessionId},
    };

    use crate::config::{
        AisAgentServiceConfig, AisAgentSqliteStorageConfig, AisAgentStorageConfig,
    };

    use super::build_host_service;

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
                launch_spec: None,
            }),
        }
    }

    #[tokio::test]
    async fn in_memory_bootstrap_builds_runtime_service() {
        let mut service = build_host_service(&AisAgentServiceConfig {
            storage: AisAgentStorageConfig::InMemory,
            ..AisAgentServiceConfig::default()
        });
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
        let mut service = build_host_service(&AisAgentServiceConfig {
            storage: AisAgentStorageConfig::Sqlite(AisAgentSqliteStorageConfig {
                path: sqlite_path.clone(),
                create_if_missing: true,
            }),
            ..AisAgentServiceConfig::default()
        });
        let outcome = service.handle(begin_run_command()).await;

        match outcome.response {
            HostCommandResponse::Accepted(response) => {
                assert_eq!(response.run_id, Some(RunId("run-1".to_owned())));
            }
            other => panic!("unexpected response: {other:?}"),
        }

        let _ = fs::remove_file(sqlite_path);
    }
}
