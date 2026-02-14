use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Debug, Clone, Parser)]
#[command(name = "ais-runner")]
#[command(about = "AIS runner CLI")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Clone, Subcommand)]
pub enum Commands {
    #[command(subcommand)]
    Run(RunCommand),
    #[command(subcommand)]
    Plan(PlanTopLevelCommand),
    Replay(ReplayCommand),
}

#[derive(Debug, Clone, Subcommand)]
pub enum RunCommand {
    Plan(PlanCommand),
    Workflow(WorkflowCommand),
}

#[derive(Debug, Clone, Subcommand)]
pub enum PlanTopLevelCommand {
    Diff(PlanDiffCommand),
}

#[derive(Debug, Clone, ValueEnum, PartialEq, Eq)]
pub enum OutputFormat {
    Text,
    Json,
}

#[derive(Debug, Clone, clap::Args)]
pub struct PlanCommand {
    #[arg(long)]
    pub plan: PathBuf,
    #[arg(long)]
    pub config: Option<PathBuf>,
    #[arg(long)]
    pub runtime: Option<PathBuf>,
    #[arg(long, default_value_t = false)]
    pub dry_run: bool,
    #[arg(long)]
    pub events_jsonl: Option<String>,
    #[arg(long)]
    pub trace: Option<PathBuf>,
    #[arg(long)]
    pub checkpoint: Option<PathBuf>,
    #[arg(long, default_value_t = false)]
    pub commands_stdin_jsonl: bool,
    #[arg(long, default_value_t = false)]
    pub verbose: bool,
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub format: OutputFormat,
}

#[derive(Debug, Clone, clap::Args)]
pub struct WorkflowCommand {
    #[arg(long)]
    pub workflow: PathBuf,
    #[arg(long)]
    pub workspace: Option<PathBuf>,
    #[arg(long)]
    pub config: Option<PathBuf>,
    #[arg(long)]
    pub runtime: Option<PathBuf>,
    #[arg(long, default_value_t = false)]
    pub dry_run: bool,
    #[arg(long)]
    pub events_jsonl: Option<String>,
    #[arg(long)]
    pub trace: Option<PathBuf>,
    #[arg(long)]
    pub checkpoint: Option<PathBuf>,
    #[arg(long)]
    pub outputs: Option<PathBuf>,
    #[arg(long, default_value_t = false)]
    pub commands_stdin_jsonl: bool,
    #[arg(long, default_value_t = false)]
    pub verbose: bool,
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub format: OutputFormat,
}

#[derive(Debug, Clone, clap::Args)]
pub struct PlanDiffCommand {
    #[arg(long)]
    pub before: PathBuf,
    #[arg(long)]
    pub after: PathBuf,
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub format: OutputFormat,
}

#[derive(Debug, Clone, clap::Args)]
pub struct ReplayCommand {
    #[arg(long)]
    pub trace_jsonl: Option<PathBuf>,
    #[arg(long)]
    pub checkpoint: Option<PathBuf>,
    #[arg(long)]
    pub plan: Option<PathBuf>,
    #[arg(long)]
    pub config: Option<PathBuf>,
    #[arg(long)]
    pub until_node: Option<String>,
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub format: OutputFormat,
}

#[cfg(test)]
#[path = "cli_test.rs"]
mod tests;
