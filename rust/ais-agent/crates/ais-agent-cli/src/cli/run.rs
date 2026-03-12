use clap::Parser;

use crate::{
    cli::args::{Args, CliCommand, DaemonCommand, InspectCommand, LocalCommand},
    commands::{daemon_http, inspect_jsonl, local_jsonl},
};

pub async fn run<I>(args: I) -> Result<(), Box<dyn std::error::Error>>
where
    I: IntoIterator<Item = String>,
{
    let args = Args::parse_from(std::iter::once("ais-agent".to_owned()).chain(args.into_iter()));

    match args.command {
        CliCommand::Local {
            command: LocalCommand::Jsonl,
        } => local_jsonl().await?,
        CliCommand::Daemon {
            command: DaemonCommand::Http { bind },
        } => daemon_http(&bind).await?,
        CliCommand::Inspect {
            command: InspectCommand::Jsonl { direction, line },
        } => inspect_jsonl(direction, &line)?,
    }

    Ok(())
}
