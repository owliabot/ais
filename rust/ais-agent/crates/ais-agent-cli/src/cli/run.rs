use clap::Parser;

use crate::{
    bootstrap::build_host_service,
    cli::args::{Args, CliCommand, DaemonCommand, InspectCommand, LocalCommand},
    commands::{daemon_http, inspect_jsonl, local_jsonl},
    config::load_service_config,
};

pub async fn run<I>(args: I) -> Result<(), Box<dyn std::error::Error>>
where
    I: IntoIterator<Item = String>,
{
    let args = Args::parse_from(std::iter::once("ais-agent".to_owned()).chain(args.into_iter()));
    let config = load_service_config(&args)?;

    match args.command {
        CliCommand::Local {
            command: LocalCommand::Jsonl,
        } => {
            let mut service = build_host_service(&config);
            local_jsonl(&mut service).await?
        }
        CliCommand::Daemon {
            command: DaemonCommand::Http { .. },
        } => {
            let service = build_host_service(&config);
            daemon_http(&config, service).await?
        }
        CliCommand::Inspect {
            command: InspectCommand::Jsonl { direction, line },
        } => inspect_jsonl(direction, &line)?,
    }

    Ok(())
}
