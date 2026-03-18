use clap::Parser;
use tracing::info;

use crate::{
    bootstrap::build_host_service,
    cli::args::{Args, CliCommand, DaemonCommand, InspectCommand, LocalCommand},
    commands::{
        daemon_http, inspect_jsonl, inspect_jsonl_file, inspect_log_file, inspect_store,
        local_jsonl, maintenance_store,
    },
    config::{load_service_config, AisAgentServiceConfig, AisAgentStorageConfig},
    observability::install_tracing,
};

pub async fn run<I>(args: I) -> Result<(), Box<dyn std::error::Error>>
where
    I: IntoIterator<Item = String>,
{
    let args = Args::parse_from(std::iter::once("ais-agent".to_owned()).chain(args.into_iter()));
    match args.command.clone() {
        CliCommand::Local {
            command: LocalCommand::Jsonl,
        } => {
            let config = load_service_config(&args)?;
            install_tracing(&config)?;
            info!(
                mode = "local_jsonl",
                instance_id = %config.service.instance_id,
                storage_backend = storage_backend_label(&config),
                "ais_agent.cli_mode_start"
            );
            let mut service = build_host_service(&config);
            local_jsonl(&mut service, Some(&config.observability.jsonl_capture)).await?
        }
        CliCommand::Daemon {
            command: DaemonCommand::Http { .. },
        } => {
            let config = load_service_config(&args)?;
            install_tracing(&config)?;
            info!(
                mode = "daemon_http",
                instance_id = %config.service.instance_id,
                storage_backend = storage_backend_label(&config),
                "ais_agent.cli_mode_start"
            );
            let service = build_host_service(&config);
            daemon_http(&config, service).await?
        }
        CliCommand::Inspect {
            command: InspectCommand::Config,
        } => {
            let config = load_service_config(&args)?;
            println!("{}", serde_json::to_string_pretty(&config)?);
        }
        CliCommand::Inspect {
            command: InspectCommand::Jsonl { direction, line },
        } => inspect_jsonl(direction, &line)?,
        CliCommand::Inspect {
            command:
                InspectCommand::JsonlFile {
                    direction,
                    path,
                    tail,
                },
        } => inspect_jsonl_file(direction, std::path::Path::new(&path), tail)?,
        CliCommand::Inspect {
            command: InspectCommand::LogFile { path, tail },
        } => inspect_log_file(std::path::Path::new(&path), tail)?,
        CliCommand::Inspect {
            command: InspectCommand::Store { command },
        } => {
            let config = load_service_config(&args)?;
            inspect_store(resolve_sqlite_path(&config)?, command)?
        }
        CliCommand::Maintenance { command } => {
            let config = load_service_config(&args)?;
            install_tracing(&config)?;
            info!(
                mode = "maintenance",
                instance_id = %config.service.instance_id,
                storage_backend = storage_backend_label(&config),
                "ais_agent.cli_mode_start"
            );
            maintenance_store(&config, command)?
        }
    }

    Ok(())
}

fn resolve_sqlite_path(
    config: &AisAgentServiceConfig,
) -> Result<&std::path::Path, Box<dyn std::error::Error>> {
    match &config.storage {
        AisAgentStorageConfig::Sqlite(sqlite) => Ok(sqlite.path.as_path()),
        AisAgentStorageConfig::InMemory => Err(
            "inspect store requires SQLite-backed storage; pass --sqlite-path or configure storage.backend=sqlite"
                .into(),
        ),
    }
}

fn storage_backend_label(config: &AisAgentServiceConfig) -> &'static str {
    match config.storage {
        AisAgentStorageConfig::InMemory => "in_memory",
        AisAgentStorageConfig::Sqlite(_) => "sqlite",
    }
}

#[cfg(test)]
mod tests {
    use super::storage_backend_label;
    use crate::config::types::AisAgentSqliteStorageConfig;
    use crate::config::{AisAgentServiceConfig, AisAgentStorageConfig};

    #[test]
    fn reports_storage_backend_label() {
        let config = AisAgentServiceConfig::default();
        assert_eq!(storage_backend_label(&config), "in_memory");

        let sqlite = AisAgentServiceConfig {
            storage: AisAgentStorageConfig::Sqlite(AisAgentSqliteStorageConfig::default()),
            ..AisAgentServiceConfig::default()
        };
        assert_eq!(storage_backend_label(&sqlite), "sqlite");
    }

    #[test]
    fn resolves_sqlite_path_from_sqlite_config() {
        let config = AisAgentServiceConfig {
            storage: AisAgentStorageConfig::Sqlite(AisAgentSqliteStorageConfig::default()),
            ..AisAgentServiceConfig::default()
        };
        let path = super::resolve_sqlite_path(&config).expect("sqlite path");
        assert!(path.ends_with("ais-agent.db"));
    }

    #[test]
    fn rejects_store_inspect_for_in_memory_config() {
        let error = super::resolve_sqlite_path(&AisAgentServiceConfig::default())
            .expect_err("in-memory should fail");
        assert!(error
            .to_string()
            .contains("inspect store requires SQLite-backed storage"));
    }
}
