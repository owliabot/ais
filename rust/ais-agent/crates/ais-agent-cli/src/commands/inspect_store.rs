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
        InspectStoreCommand::Overview {
            limit,
            status,
            phase,
            active_boundary_kind,
            run_id_prefix,
        } => SqliteStoreInspectCommand::Overview {
            limit,
            status,
            phase,
            active_boundary_kind,
            run_id_prefix,
        },
        InspectStoreCommand::Run { run_id } => SqliteStoreInspectCommand::Run { run_id },
        InspectStoreCommand::Events {
            run_id,
            after_event_seq,
            checkpoint_seq,
            event_kind,
            limit,
        } => SqliteStoreInspectCommand::Events {
            run_id,
            after_event_seq,
            checkpoint_seq,
            event_kind,
            limit,
        },
        InspectStoreCommand::Audits {
            run_id,
            after_audit_seq,
            checkpoint_seq,
            audit_type,
            recovery_disposition,
            limit,
        } => SqliteStoreInspectCommand::Audits {
            run_id,
            after_audit_seq,
            checkpoint_seq,
            audit_type,
            recovery_disposition,
            limit,
        },
        InspectStoreCommand::Checkpoints {
            run_id,
            latest,
            archive_kind,
            limit,
        } => SqliteStoreInspectCommand::Checkpoints {
            run_id,
            latest,
            archive_kind,
            limit,
        },
        InspectStoreCommand::Waits {
            run_id,
            wait_kind,
            limit,
        } => SqliteStoreInspectCommand::Waits {
            run_id,
            wait_kind,
            limit,
        },
        InspectStoreCommand::Claims {
            run_id,
            status,
            owner_kind,
            host_session_id,
            limit,
        } => SqliteStoreInspectCommand::Claims {
            run_id,
            status,
            owner_kind,
            host_session_id,
            limit,
        },
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
            super::map_command(InspectStoreCommand::Overview {
                limit: 5,
                status: Some("awaiting_signer".to_owned()),
                phase: None,
                active_boundary_kind: Some("signer".to_owned()),
                run_id_prefix: Some("run-".to_owned()),
            }),
            SqliteStoreInspectCommand::Overview {
                limit: 5,
                status: Some("awaiting_signer".to_owned()),
                phase: None,
                active_boundary_kind: Some("signer".to_owned()),
                run_id_prefix: Some("run-".to_owned()),
            }
        );
        assert_eq!(
            super::map_command(InspectStoreCommand::Waits {
                run_id: Some("run-1".to_owned()),
                wait_kind: Some("signer".to_owned()),
                limit: 25,
            }),
            SqliteStoreInspectCommand::Waits {
                run_id: Some("run-1".to_owned()),
                wait_kind: Some("signer".to_owned()),
                limit: 25,
            }
        );
        assert_eq!(
            super::map_command(InspectStoreCommand::Claims {
                run_id: None,
                status: Some("active".to_owned()),
                owner_kind: Some("interactive_host".to_owned()),
                host_session_id: Some("session-1".to_owned()),
                limit: 10,
            }),
            SqliteStoreInspectCommand::Claims {
                run_id: None,
                status: Some("active".to_owned()),
                owner_kind: Some("interactive_host".to_owned()),
                host_session_id: Some("session-1".to_owned()),
                limit: 10,
            }
        );
        assert_eq!(
            super::map_command(InspectStoreCommand::Events {
                run_id: "run-1".to_owned(),
                after_event_seq: Some(2),
                checkpoint_seq: Some(5),
                event_kind: Some("awaiting_signer".to_owned()),
                limit: Some(10),
            }),
            SqliteStoreInspectCommand::Events {
                run_id: "run-1".to_owned(),
                after_event_seq: Some(2),
                checkpoint_seq: Some(5),
                event_kind: Some("awaiting_signer".to_owned()),
                limit: Some(10),
            }
        );
        assert_eq!(
            super::map_command(InspectStoreCommand::Audits {
                run_id: "run-1".to_owned(),
                after_audit_seq: Some(1),
                checkpoint_seq: Some(5),
                audit_type: Some("recovery".to_owned()),
                recovery_disposition: Some("await_signer".to_owned()),
                limit: Some(10),
            }),
            SqliteStoreInspectCommand::Audits {
                run_id: "run-1".to_owned(),
                after_audit_seq: Some(1),
                checkpoint_seq: Some(5),
                audit_type: Some("recovery".to_owned()),
                recovery_disposition: Some("await_signer".to_owned()),
                limit: Some(10),
            }
        );
        assert_eq!(
            super::map_command(InspectStoreCommand::Retention),
            SqliteStoreInspectCommand::Retention
        );
    }
}
