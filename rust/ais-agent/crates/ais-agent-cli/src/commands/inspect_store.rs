use std::path::Path;

use ais_agent_store_sqlite::{
    inspect_store as inspect_store_impl, StoreInspectCommand as SqliteStoreInspectCommand,
};

use crate::cli::args::InspectStoreCommand;

pub fn inspect_store(
    sqlite_path: &Path,
    command: InspectStoreCommand,
) -> Result<(), Box<dyn std::error::Error>> {
    let output = inspect_store_impl(sqlite_path, map_command(command))?;
    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

fn map_command(command: InspectStoreCommand) -> SqliteStoreInspectCommand {
    match command {
        InspectStoreCommand::Overview { limit } => SqliteStoreInspectCommand::Overview { limit },
        InspectStoreCommand::Run { run_id } => SqliteStoreInspectCommand::Run { run_id },
        InspectStoreCommand::Events {
            run_id,
            after_event_seq,
            limit,
        } => SqliteStoreInspectCommand::Events {
            run_id,
            after_event_seq,
            limit,
        },
        InspectStoreCommand::Audits {
            run_id,
            after_audit_seq,
            limit,
        } => SqliteStoreInspectCommand::Audits {
            run_id,
            after_audit_seq,
            limit,
        },
        InspectStoreCommand::Checkpoints {
            run_id,
            latest,
            limit,
        } => SqliteStoreInspectCommand::Checkpoints {
            run_id,
            latest,
            limit,
        },
        InspectStoreCommand::Waits { run_id } => SqliteStoreInspectCommand::Waits { run_id },
        InspectStoreCommand::Claims { run_id } => SqliteStoreInspectCommand::Claims { run_id },
        InspectStoreCommand::Retention => SqliteStoreInspectCommand::Retention,
        InspectStoreCommand::Storage => SqliteStoreInspectCommand::Storage,
        InspectStoreCommand::Sql { query, limit } => {
            SqliteStoreInspectCommand::Sql { query, limit }
        }
    }
}

#[cfg(test)]
mod tests {
    use ais_agent_store_sqlite::StoreInspectCommand as SqliteStoreInspectCommand;

    use crate::cli::args::InspectStoreCommand;

    #[test]
    fn maps_cli_commands_to_sqlite_forensics_commands() {
        assert_eq!(
            super::map_command(InspectStoreCommand::Overview { limit: 5 }),
            SqliteStoreInspectCommand::Overview { limit: 5 }
        );
        assert_eq!(
            super::map_command(InspectStoreCommand::Waits {
                run_id: "run-1".to_owned()
            }),
            SqliteStoreInspectCommand::Waits {
                run_id: "run-1".to_owned()
            }
        );
        assert_eq!(
            super::map_command(InspectStoreCommand::Retention),
            SqliteStoreInspectCommand::Retention
        );
    }
}
