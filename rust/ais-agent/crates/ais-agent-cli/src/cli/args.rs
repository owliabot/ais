use clap::{Parser, Subcommand, ValueEnum};

#[derive(Debug, Clone, PartialEq, Eq, Parser)]
#[command(
    name = "ais-agent",
    version,
    about = "Thin CLI shell for ais-agent transports",
    long_about = None
)]
pub struct Args {
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

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::{Args, CliCommand, DaemonCommand, InspectCommand, JsonlDirection, LocalCommand};

    #[test]
    fn parses_local_jsonl_mode() {
        let parsed = Args::try_parse_from(["ais-agent", "local", "jsonl"]).expect("local");
        assert_eq!(
            parsed.command,
            CliCommand::Local {
                command: LocalCommand::Jsonl,
            }
        );
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
