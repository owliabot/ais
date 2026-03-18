use std::io::{Error, ErrorKind};

use ais_agent_store_sqlite::{
    SqliteStore, StorePurgeRequest, StorePurgeTable, StorePurgeTarget,
    STORE_RETENTION_SCHEMA_VERSION,
};
use serde_json::json;

use crate::{
    cli::args::{MaintenanceCommand, MaintenancePurgeCommand, MaintenanceTableArg},
    config::{AisAgentServiceConfig, AisAgentStorageConfig},
    storage_maintenance::{
        build_prune_request, build_vacuum_request, current_time_ms, prepare_sqlite_path,
    },
};

pub fn maintenance_store(
    config: &AisAgentServiceConfig,
    command: MaintenanceCommand,
) -> Result<(), Box<dyn std::error::Error>> {
    let sqlite = sqlite_config(config)?;
    prepare_sqlite_path(sqlite)?;

    match command {
        MaintenanceCommand::Prune { now_ms } => {
            let started_at_ms = now_ms.unwrap_or_else(current_time_ms);
            let request = build_prune_request(&sqlite.retention, started_at_ms);
            let mut store = SqliteStore::open_path(&sqlite.path)?;
            let result = store.prune_retention(&request)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "request": request,
                    "result": result,
                }))?
            );
        }
        MaintenanceCommand::Vacuum { now_ms } => {
            let started_at_ms = now_ms.unwrap_or_else(current_time_ms);
            let request = build_vacuum_request(&sqlite.retention, started_at_ms);
            let mut store = SqliteStore::open_path(&sqlite.path)?;
            let result = store.maybe_vacuum_retention(&request)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "request": request,
                    "result": result,
                }))?
            );
        }
        MaintenanceCommand::Purge { yes, command } => {
            if !sqlite.retention.destructive_purge_enabled {
                return Err(invalid_input(
                    "maintenance purge is disabled; set storage.retention.destructive_purge_enabled=true first",
                )
                .into());
            }
            if sqlite.retention.require_purge_confirmation && !yes {
                return Err(invalid_input(
                    "maintenance purge requires --yes when confirmation is enabled",
                )
                .into());
            }
            let started_at_ms = current_time_ms();
            let request = StorePurgeRequest {
                started_at_ms,
                finished_at_ms: started_at_ms,
                target: match command {
                    MaintenancePurgeCommand::RunId { run_id } => StorePurgeTarget::RunId { run_id },
                    MaintenancePurgeCommand::TerminalBefore { terminal_before_ms } => {
                        StorePurgeTarget::TerminalBefore { terminal_before_ms }
                    }
                    MaintenancePurgeCommand::Table { table } => StorePurgeTarget::Table {
                        table: map_purge_table(table),
                    },
                },
                vacuum_freelist_threshold_pages: sqlite.retention.vacuum_freelist_threshold_pages,
                schema_retention_version: STORE_RETENTION_SCHEMA_VERSION,
            };
            let mut store = SqliteStore::open_path(&sqlite.path)?;
            let result = store.purge_retention(&request)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "request": request,
                    "result": result,
                }))?
            );
        }
    }

    Ok(())
}

fn sqlite_config(
    config: &AisAgentServiceConfig,
) -> Result<&crate::config::types::AisAgentSqliteStorageConfig, Error> {
    match &config.storage {
        AisAgentStorageConfig::Sqlite(sqlite) => Ok(sqlite),
        AisAgentStorageConfig::InMemory => Err(invalid_input(
            "maintenance commands require SQLite-backed storage; pass --sqlite-path or configure storage.backend=sqlite",
        )),
    }
}

fn map_purge_table(table: MaintenanceTableArg) -> StorePurgeTable {
    match table {
        MaintenanceTableArg::Runs => StorePurgeTable::Runs,
        MaintenanceTableArg::RunInputs => StorePurgeTable::RunInputs,
        MaintenanceTableArg::RunEvents => StorePurgeTable::RunEvents,
        MaintenanceTableArg::RunAudits => StorePurgeTable::RunAudits,
        MaintenanceTableArg::RunCheckpoints => StorePurgeTable::RunCheckpoints,
        MaintenanceTableArg::RunWaitStates => StorePurgeTable::RunWaitStates,
        MaintenanceTableArg::RunClaimHistory => StorePurgeTable::RunClaimHistory,
        MaintenanceTableArg::MaintenanceJournal => StorePurgeTable::MaintenanceJournal,
        MaintenanceTableArg::StoreMaintenanceState => StorePurgeTable::StoreMaintenanceState,
    }
}

fn invalid_input(message: &str) -> Error {
    Error::new(ErrorKind::InvalidInput, message.to_owned())
}

#[cfg(test)]
mod tests {
    use ais_agent_store_sqlite::{StorePurgeTable, StorePurgeTarget};

    use crate::{
        cli::args::{MaintenanceCommand, MaintenancePurgeCommand, MaintenanceTableArg},
        config::{AisAgentServiceConfig, AisAgentStorageConfig},
    };

    #[test]
    fn maps_cli_table_variants_to_store_tables() {
        assert_eq!(
            super::map_purge_table(MaintenanceTableArg::RunEvents),
            StorePurgeTable::RunEvents
        );
        assert_eq!(
            super::map_purge_table(MaintenanceTableArg::StoreMaintenanceState),
            StorePurgeTable::StoreMaintenanceState
        );
    }

    #[test]
    fn rejects_purge_when_confirmation_or_destructive_gate_missing() {
        let sqlite_path = std::env::temp_dir().join("ais-agent-cli-maintenance-gate.sqlite");
        let mut config = AisAgentServiceConfig {
            storage: AisAgentStorageConfig::Sqlite(Default::default()),
            ..AisAgentServiceConfig::default()
        };
        let sqlite = match &mut config.storage {
            AisAgentStorageConfig::Sqlite(sqlite) => sqlite,
            AisAgentStorageConfig::InMemory => unreachable!(),
        };
        sqlite.path = sqlite_path.clone();
        let error = super::maintenance_store(
            &config,
            MaintenanceCommand::Purge {
                yes: false,
                command: MaintenancePurgeCommand::RunId {
                    run_id: "run-1".to_owned(),
                },
            },
        )
        .expect_err("purge should be gated");
        assert!(error.to_string().contains("disabled"));

        let sqlite = match &mut config.storage {
            AisAgentStorageConfig::Sqlite(sqlite) => sqlite,
            AisAgentStorageConfig::InMemory => unreachable!(),
        };
        sqlite.retention.destructive_purge_enabled = true;
        let error = super::maintenance_store(
            &config,
            MaintenanceCommand::Purge {
                yes: false,
                command: MaintenancePurgeCommand::Table {
                    table: MaintenanceTableArg::RunEvents,
                },
            },
        )
        .expect_err("purge should require confirmation");
        assert!(error.to_string().contains("--yes"));

        let _ = std::fs::remove_file(sqlite_path);
    }

    #[test]
    fn builds_purge_target_shape() {
        let target = StorePurgeTarget::Table {
            table: super::map_purge_table(MaintenanceTableArg::RunEvents),
        };
        assert_eq!(
            target,
            StorePurgeTarget::Table {
                table: StorePurgeTable::RunEvents,
            }
        );
    }
}
