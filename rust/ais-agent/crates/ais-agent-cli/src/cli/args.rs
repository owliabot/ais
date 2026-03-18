use clap::{Parser, Subcommand, ValueEnum};
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Parser)]
#[command(
    name = "ais-agent",
    version,
    about = "CLI entrypoint for ais-agent runtime, inspection, and store maintenance",
    long_about = r#"Operate `ais-agent` in three broad modes:

- `local` runs the JSONL transport loop over stdin/stdout.
- `daemon` starts long-running service endpoints such as HTTP.
- `inspect` reads config, JSONL frames, logs, or SQLite durable state without mutating it.
- `maintenance` executes explicit SQLite prune, purge, and vacuum operations.

Global flags let you override the service config file for storage, lease timing,
and file-based observability paths."#,
    after_long_help = r#"Examples:
  ais-agent local jsonl
  ais-agent daemon http --bind 127.0.0.1:8080
  ais-agent --sqlite-path ./var/ais-agent.db inspect store overview --limit 10
  ais-agent --sqlite-path ./var/ais-agent.db maintenance prune
  ais-agent --sqlite-path ./var/ais-agent.db maintenance purge --yes run-id --run-id run-42"#
)]
pub struct Args {
    #[arg(
        long,
        help = "Path to the YAML config file to load before applying CLI overrides",
        long_help = "Path to the YAML config file to load before applying environment variables and CLI overrides. If omitted, the built-in defaults are used unless another config source provides values."
    )]
    pub config: Option<String>,
    #[arg(
        long,
        help = "Override the SQLite database path",
        long_help = "Override `storage.sqlite.path` from config. Use this when you want inspect or maintenance commands to point at a specific ais-agent durable store file."
    )]
    pub sqlite_path: Option<String>,
    #[arg(
        long,
        help = "Override host-session claim lease duration in seconds",
        long_help = "Override the host-session claim lease duration in seconds. This controls how long a claimed run stays owned before renewal is required."
    )]
    pub claim_lease_seconds: Option<u64>,
    #[arg(
        long,
        value_enum,
        help = "Set stderr and file log verbosity",
        long_help = "Set the tracing level for stderr and file logs. `info` is the default operator view; `debug` and `trace` add lower-level runtime details."
    )]
    pub log_level: Option<LogLevelArg>,
    #[arg(
        long,
        help = "Override the directory used for rotated log files",
        long_help = "Override the directory used for daily rotated log files produced by ais-agent observability sinks."
    )]
    pub log_dir: Option<String>,
    #[arg(
        long,
        help = "Override how many days of log files are retained",
        long_help = "Override the retention window, in days, for rotated log files under `--log-dir`."
    )]
    pub log_retention_days: Option<u16>,
    #[arg(
        long,
        help = "Override the directory used for JSONL capture files",
        long_help = "Override the directory used to store inbound/outbound JSONL transport captures when running `local jsonl`."
    )]
    pub jsonl_capture_dir: Option<String>,
    #[arg(
        long,
        help = "Override how many days of JSONL captures are retained",
        long_help = "Override the retention window, in days, for JSONL transport capture files under `--jsonl-capture-dir`."
    )]
    pub jsonl_capture_retention_days: Option<u16>,
    #[command(subcommand)]
    pub command: CliCommand,
}

#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub enum CliCommand {
    #[command(
        about = "Run local transport loops over stdin/stdout",
        long_about = "Run local transport loops over stdin/stdout. This is the simplest way to embed ais-agent behind a parent process that speaks the JSONL transport protocol."
    )]
    Local {
        #[command(subcommand)]
        command: LocalCommand,
    },
    #[command(
        about = "Start long-running service endpoints",
        long_about = "Start long-running service endpoints backed by the ais-agent runtime. Use this for daemon-style operation instead of one-shot local transport loops."
    )]
    Daemon {
        #[command(subcommand)]
        command: DaemonCommand,
    },
    #[command(
        about = "Inspect config, capture files, logs, or SQLite durable state",
        long_about = "Read ais-agent config, decode JSONL capture lines/files, tail text logs, or inspect the SQLite durable store. These commands are read-only and are intended for debugging and operator forensics."
    )]
    Inspect {
        #[command(subcommand)]
        command: InspectCommand,
    },
    #[command(
        about = "Execute explicit SQLite retention and deletion operations",
        long_about = "Execute explicit SQLite maintenance flows such as prune, purge, and vacuum. These commands mutate the durable store and should be used deliberately."
    )]
    Maintenance {
        #[command(subcommand)]
        command: MaintenanceCommand,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub enum LocalCommand {
    #[command(
        about = "Serve the JSONL transport protocol on stdin/stdout",
        long_about = "Read inbound JSONL frames from stdin, execute them against the runtime-backed host service, and write outbound JSONL frames to stdout. When JSONL capture is enabled, inbound and outbound frames are also persisted to rotated capture files."
    )]
    Jsonl,
}

#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub enum DaemonCommand {
    #[command(
        about = "Start the HTTP daemon",
        long_about = "Start the HTTP daemon endpoint for ais-agent. The daemon exposes runtime-backed command handling over HTTP and keeps the process alive as a long-running service."
    )]
    Http {
        #[arg(
            long,
            help = "Override the bind address for the HTTP daemon",
            long_help = "Override the bind address for the HTTP daemon, for example `127.0.0.1:8080`. If omitted, the configured daemon bind is used."
        )]
        bind: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub enum InspectCommand {
    #[command(
        about = "Print the fully resolved service configuration",
        long_about = "Print the fully resolved service configuration after merging defaults, config file values, environment variables, and CLI overrides."
    )]
    Config,
    #[command(
        about = "Decode a single inbound or outbound JSONL frame",
        long_about = "Decode one raw JSONL line as either an inbound or outbound transport frame. This is useful when you have copied a single capture line from a file or logs and want a structured rendering."
    )]
    Jsonl {
        #[arg(
            long,
            value_enum,
            help = "Whether the line should be decoded as an inbound or outbound frame"
        )]
        direction: JsonlDirection,
        #[arg(
            long,
            help = "The raw JSONL line to decode",
            long_help = "The raw JSONL line to decode. Pass the complete one-line JSON object exactly as it appeared in the capture stream."
        )]
        line: String,
    },
    #[command(
        about = "Tail and decode a JSONL capture file",
        long_about = "Read the tail of an inbound or outbound JSONL capture file and decode each line into structured transport frames."
    )]
    JsonlFile {
        #[arg(
            long,
            value_enum,
            help = "Whether the file contains inbound or outbound frames"
        )]
        direction: JsonlDirection,
        #[arg(long, help = "Path to the JSONL capture file to inspect")]
        path: String,
        #[arg(
            long,
            default_value_t = 50,
            help = "How many lines from the end of the file to read"
        )]
        tail: usize,
    },
    #[command(
        about = "Tail a plain text log file",
        long_about = "Read the tail of a plain text log file, such as the rotated ais-agent tracing logs under the configured log directory."
    )]
    LogFile {
        #[arg(long, help = "Path to the log file to inspect")]
        path: String,
        #[arg(
            long,
            default_value_t = 50,
            help = "How many lines from the end of the file to read"
        )]
        tail: usize,
    },
    #[command(
        about = "Inspect the SQLite durable store",
        long_about = "Run read-only SQLite forensics queries against the configured ais-agent durable store. These subcommands summarize run state, timelines, maintenance metadata, and raw SQL snapshots."
    )]
    Store {
        #[command(subcommand)]
        command: InspectStoreCommand,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub enum InspectStoreCommand {
    #[command(
        about = "Summarize top-level store state and recent runs",
        long_about = "Show table counts, schema version, and a recent run summary list. Start here when you need a quick high-level view of what is stored."
    )]
    Overview {
        #[arg(
            long,
            default_value_t = 20,
            help = "How many recent runs to include in the overview"
        )]
        limit: usize,
    },
    #[command(
        about = "Inspect one run across catalog, input, checkpoint, wait, and claim views",
        long_about = "Aggregate the main durable views for a single run: run head, mission/launch input, latest checkpoint, wait-state, and claim state."
    )]
    Run {
        #[arg(long, help = "Run identifier to inspect")]
        run_id: String,
    },
    #[command(
        about = "Read the run event timeline",
        long_about = "Read structured rows from `run_events` for one run. Use `--after-event-seq` and `--limit` to page through long timelines."
    )]
    Events {
        #[arg(long, help = "Run identifier to inspect")]
        run_id: String,
        #[arg(
            long,
            help = "Only return events strictly after this event sequence number"
        )]
        after_event_seq: Option<u64>,
        #[arg(long, help = "Maximum number of events to return")]
        limit: Option<usize>,
    },
    #[command(
        about = "Read the run audit timeline",
        long_about = "Read structured rows from `run_audits` for one run. Use `--after-audit-seq` and `--limit` to page through long timelines."
    )]
    Audits {
        #[arg(long, help = "Run identifier to inspect")]
        run_id: String,
        #[arg(
            long,
            help = "Only return audits strictly after this audit sequence number"
        )]
        after_audit_seq: Option<u64>,
        #[arg(long, help = "Maximum number of audits to return")]
        limit: Option<usize>,
    },
    #[command(
        about = "Read stored checkpoints for one run",
        long_about = "Read checkpoint rows for a run. Use `--latest` when you only need the latest checkpoint instead of the full history slice."
    )]
    Checkpoints {
        #[arg(long, help = "Run identifier to inspect")]
        run_id: String,
        #[arg(long, help = "Only return the latest checkpoint summary")]
        latest: bool,
        #[arg(
            long,
            help = "Maximum number of checkpoints to return when not using --latest"
        )]
        limit: Option<usize>,
    },
    #[command(
        about = "Inspect the current wait-state row for one run",
        long_about = "Read the current `run_wait_states` row for a run, such as an awaiting signer, evidence, or confirmation pause."
    )]
    Waits {
        #[arg(long, help = "Run identifier to inspect")]
        run_id: String,
    },
    #[command(
        about = "Read claim history for one run",
        long_about = "Read `run_claim_history` rows for a run, including active, released, expired, or superseded ownership records."
    )]
    Claims {
        #[arg(long, help = "Run identifier to inspect")]
        run_id: String,
    },
    #[command(
        about = "Summarize retention posture and maintenance history",
        long_about = "Show run retention modes, checkpoint tiers, wait-state counts, maintenance metadata, and recent growth/reclaim deltas from maintenance operations."
    )]
    Retention,
    #[command(
        about = "Summarize current SQLite footprint and recent storage deltas",
        long_about = "Show current SQLite page counts, freelist, file sizes, per-table row counts, and recent storage growth/reclaim information derived from maintenance journal entries."
    )]
    Storage,
    #[command(
        about = "Run a read-only SQL query against the SQLite store",
        long_about = "Execute a read-only `SELECT`, `WITH`, or `PRAGMA` query against the SQLite store. This is an escape hatch for deeper forensics when the built-in inspect subcommands are not enough."
    )]
    Sql {
        #[arg(long, help = "Read-only SQL query text")]
        query: String,
        #[arg(long, help = "Maximum number of rows to return")]
        limit: Option<usize>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub enum MaintenanceCommand {
    #[command(
        about = "Prune old terminal checkpoints and orphaned wait-state rows",
        long_about = "Apply the configured non-destructive retention policy to the SQLite store. This removes old `terminal_intermediate` checkpoints and stale/orphaned wait-state rows while preserving core run, event, and audit truth."
    )]
    Prune {
        #[arg(
            long,
            help = "Override the wall-clock time used for retention calculations, in Unix milliseconds"
        )]
        now_ms: Option<i64>,
    },
    #[command(
        about = "Run SQLite VACUUM immediately",
        long_about = "Force a SQLite `VACUUM` pass immediately, regardless of the configured freelist threshold. This is a physical compaction step, not a logical retention step."
    )]
    Vacuum {
        #[arg(
            long,
            help = "Override the wall-clock time recorded for the operation, in Unix milliseconds"
        )]
        now_ms: Option<i64>,
    },
    #[command(
        about = "Delete data explicitly from the SQLite store",
        long_about = "Run destructive deletion against the SQLite store. Purge is explicitly gated by config and usually requires `--yes`; use it when you intentionally want to remove runs, old terminal data, or whole tables."
    )]
    Purge {
        #[arg(
            long,
            default_value_t = false,
            help = "Acknowledge destructive deletion when confirmation is enabled"
        )]
        yes: bool,
        #[command(subcommand)]
        command: MaintenancePurgeCommand,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub enum MaintenancePurgeCommand {
    #[command(
        about = "Delete one run and all of its dependent rows",
        long_about = "Delete a single run and its dependent input, event, audit, checkpoint, wait-state, and claim rows."
    )]
    RunId {
        #[arg(long, help = "Run identifier to delete")]
        run_id: String,
    },
    #[command(
        about = "Delete all terminal runs older than a cutoff",
        long_about = "Delete runs whose `terminal_at_ms` is older than the given Unix-millisecond cutoff, along with their dependent rows."
    )]
    TerminalBefore {
        #[arg(
            long,
            help = "Delete terminal runs strictly older than this Unix-millisecond cutoff"
        )]
        terminal_before_ms: i64,
    },
    #[command(
        about = "Delete every row from one explicit table",
        long_about = "Delete every row from one explicit table. This is the sharpest destructive option and is mainly intended for operator repair or test-store cleanup."
    )]
    Table {
        #[arg(long, value_enum, help = "Exact table to clear")]
        table: MaintenanceTableArg,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum MaintenanceTableArg {
    Runs,
    RunInputs,
    RunEvents,
    RunAudits,
    RunCheckpoints,
    RunWaitStates,
    RunClaimHistory,
    MaintenanceJournal,
    StoreMaintenanceState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Serialize)]
pub enum JsonlDirection {
    Inbound,
    Outbound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum LogLevelArg {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::{
        Args, CliCommand, DaemonCommand, InspectCommand, InspectStoreCommand, JsonlDirection,
        LocalCommand, LogLevelArg, MaintenanceCommand, MaintenancePurgeCommand,
        MaintenanceTableArg,
    };

    #[test]
    fn parses_local_jsonl_mode() {
        let parsed = Args::try_parse_from(["ais-agent", "local", "jsonl"]).expect("local");
        assert_eq!(
            parsed.command,
            CliCommand::Local {
                command: LocalCommand::Jsonl,
            }
        );
        assert!(parsed.config.is_none());
    }

    #[test]
    fn parses_http_daemon_mode() {
        let parsed =
            Args::try_parse_from(["ais-agent", "daemon", "http", "--bind", "0.0.0.0:8080"])
                .expect("daemon");

        assert_eq!(
            parsed.command,
            CliCommand::Daemon {
                command: DaemonCommand::Http {
                    bind: Some("0.0.0.0:8080".to_owned()),
                },
            }
        );
    }

    #[test]
    fn http_daemon_bind_is_optional_without_flag() {
        let parsed = Args::try_parse_from(["ais-agent", "daemon", "http"]).expect("daemon");

        assert_eq!(
            parsed.command,
            CliCommand::Daemon {
                command: DaemonCommand::Http { bind: None },
            }
        );
    }

    #[test]
    fn parses_global_service_config_overrides() {
        let parsed = Args::try_parse_from([
            "ais-agent",
            "--config",
            "./ais-agent.yaml",
            "--sqlite-path",
            "./var/ais-agent.db",
            "--claim-lease-seconds",
            "90",
            "--log-level",
            "debug",
            "--log-dir",
            "./var/log/ais-agent",
            "--log-retention-days",
            "14",
            "--jsonl-capture-dir",
            "./var/captures/jsonl",
            "--jsonl-capture-retention-days",
            "3",
            "local",
            "jsonl",
        ])
        .expect("global overrides");

        assert_eq!(parsed.config.as_deref(), Some("./ais-agent.yaml"));
        assert_eq!(parsed.sqlite_path.as_deref(), Some("./var/ais-agent.db"));
        assert_eq!(parsed.claim_lease_seconds, Some(90));
        assert_eq!(parsed.log_level, Some(LogLevelArg::Debug));
        assert_eq!(parsed.log_dir.as_deref(), Some("./var/log/ais-agent"));
        assert_eq!(parsed.log_retention_days, Some(14));
        assert_eq!(
            parsed.jsonl_capture_dir.as_deref(),
            Some("./var/captures/jsonl")
        );
        assert_eq!(parsed.jsonl_capture_retention_days, Some(3));
    }

    #[test]
    fn parses_jsonl_inspect_mode() {
        let parsed = Args::try_parse_from([
            "ais-agent",
            "inspect",
            "jsonl",
            "--direction",
            "outbound",
            "--line",
            "{\"type\":\"response\"}",
        ])
        .expect("inspect");

        assert_eq!(
            parsed.command,
            CliCommand::Inspect {
                command: InspectCommand::Jsonl {
                    direction: JsonlDirection::Outbound,
                    line: "{\"type\":\"response\"}".to_owned(),
                },
            }
        );
    }

    #[test]
    fn parses_store_overview_inspect_mode() {
        let parsed = Args::try_parse_from([
            "ais-agent",
            "--sqlite-path",
            "./var/ais-agent.db",
            "inspect",
            "store",
            "overview",
            "--limit",
            "10",
        ])
        .expect("inspect store overview");

        assert_eq!(
            parsed.command,
            CliCommand::Inspect {
                command: InspectCommand::Store {
                    command: InspectStoreCommand::Overview { limit: 10 },
                },
            }
        );
    }

    #[test]
    fn parses_store_waits_and_retention_inspect_modes() {
        let waits = Args::try_parse_from([
            "ais-agent",
            "inspect",
            "store",
            "waits",
            "--run-id",
            "run-1",
        ])
        .expect("inspect store waits");
        assert_eq!(
            waits.command,
            CliCommand::Inspect {
                command: InspectCommand::Store {
                    command: InspectStoreCommand::Waits {
                        run_id: "run-1".to_owned(),
                    },
                },
            }
        );

        let retention = Args::try_parse_from(["ais-agent", "inspect", "store", "retention"])
            .expect("inspect store retention");
        assert_eq!(
            retention.command,
            CliCommand::Inspect {
                command: InspectCommand::Store {
                    command: InspectStoreCommand::Retention,
                },
            }
        );
    }

    #[test]
    fn parses_jsonl_file_inspect_mode() {
        let parsed = Args::try_parse_from([
            "ais-agent",
            "inspect",
            "jsonl-file",
            "--direction",
            "inbound",
            "--path",
            "./captures/inbound.jsonl",
            "--tail",
            "5",
        ])
        .expect("inspect jsonl file");

        assert_eq!(
            parsed.command,
            CliCommand::Inspect {
                command: InspectCommand::JsonlFile {
                    direction: JsonlDirection::Inbound,
                    path: "./captures/inbound.jsonl".to_owned(),
                    tail: 5,
                },
            }
        );
    }

    #[test]
    fn parses_log_file_inspect_mode() {
        let parsed = Args::try_parse_from([
            "ais-agent",
            "inspect",
            "log-file",
            "--path",
            "./var/ais-agent.log",
        ])
        .expect("inspect log file");

        assert_eq!(
            parsed.command,
            CliCommand::Inspect {
                command: InspectCommand::LogFile {
                    path: "./var/ais-agent.log".to_owned(),
                    tail: 50,
                },
            }
        );
    }

    #[test]
    fn parses_maintenance_modes() {
        let prune = Args::try_parse_from(["ais-agent", "maintenance", "prune", "--now-ms", "1000"])
            .expect("maintenance prune");
        assert_eq!(
            prune.command,
            CliCommand::Maintenance {
                command: MaintenanceCommand::Prune { now_ms: Some(1000) },
            }
        );

        let purge = Args::try_parse_from([
            "ais-agent",
            "maintenance",
            "purge",
            "--yes",
            "table",
            "--table",
            "run-events",
        ])
        .expect("maintenance purge");
        assert_eq!(
            purge.command,
            CliCommand::Maintenance {
                command: MaintenanceCommand::Purge {
                    yes: true,
                    command: MaintenancePurgeCommand::Table {
                        table: MaintenanceTableArg::RunEvents,
                    },
                },
            }
        );

        let vacuum = Args::try_parse_from(["ais-agent", "maintenance", "vacuum"]).expect("vacuum");
        assert_eq!(
            vacuum.command,
            CliCommand::Maintenance {
                command: MaintenanceCommand::Vacuum { now_ms: None },
            }
        );
    }
}
