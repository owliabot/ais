use clap::{Parser, Subcommand, ValueEnum};

#[derive(Debug, Clone, PartialEq, Eq, Parser)]
#[command(
    name = "ais-agent",
    version,
    about = "Thin CLI shell for ais-agent transports",
    long_about = None
)]
pub struct Args {
    #[arg(long)]
    pub config: Option<String>,
    #[arg(long)]
    pub sqlite_path: Option<String>,
    #[arg(long)]
    pub evm_rpc_url: Option<String>,
    #[arg(long)]
    pub solana_rpc_url: Option<String>,
    #[arg(long)]
    pub claim_lease_seconds: Option<u64>,
    #[arg(long, value_enum)]
    pub log_level: Option<LogLevelArg>,
    #[command(subcommand)]
    pub command: CliCommand,
}

#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub enum CliCommand {
    Local {
        #[command(subcommand)]
        command: LocalCommand,
    },
    Daemon {
        #[command(subcommand)]
        command: DaemonCommand,
    },
    Inspect {
        #[command(subcommand)]
        command: InspectCommand,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub enum LocalCommand {
    Jsonl,
}

#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub enum DaemonCommand {
    Http {
        #[arg(long, default_value = "127.0.0.1:3000")]
        bind: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub enum InspectCommand {
    Jsonl {
        #[arg(long, value_enum)]
        direction: JsonlDirection,
        #[arg(long)]
        line: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
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
        Args, CliCommand, DaemonCommand, InspectCommand, JsonlDirection, LocalCommand, LogLevelArg,
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
                    bind: "0.0.0.0:8080".to_owned(),
                },
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
            "--evm-rpc-url",
            "https://rpc.example/evm",
            "--solana-rpc-url",
            "https://rpc.example/solana",
            "--claim-lease-seconds",
            "90",
            "--log-level",
            "debug",
            "local",
            "jsonl",
        ])
        .expect("global overrides");

        assert_eq!(parsed.config.as_deref(), Some("./ais-agent.yaml"));
        assert_eq!(parsed.sqlite_path.as_deref(), Some("./var/ais-agent.db"));
        assert_eq!(
            parsed.evm_rpc_url.as_deref(),
            Some("https://rpc.example/evm")
        );
        assert_eq!(
            parsed.solana_rpc_url.as_deref(),
            Some("https://rpc.example/solana")
        );
        assert_eq!(parsed.claim_lease_seconds, Some(90));
        assert_eq!(parsed.log_level, Some(LogLevelArg::Debug));
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
}
