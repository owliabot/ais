use clap::{ArgGroup, Parser, Subcommand, ValueEnum};
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
    Agent(AgentCommand),
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

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
pub enum ApprovalsMode {
    Safe,
    Assist,
    Yolo,
}

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
pub enum AgentProfile {
    Standard,
    DemoScripted,
}

#[derive(Debug, Clone, clap::Args)]
#[command(group(
    ArgGroup::new("agent_input")
        .required(true)
        .args(["plan", "intent", "intent_file"])
        .multiple(false)
))]
pub struct AgentCommand {
    #[arg(long)]
    pub plan: Option<PathBuf>,
    #[arg(long)]
    pub intent: Option<String>,
    #[arg(long)]
    pub intent_file: Option<PathBuf>,
    #[arg(long)]
    pub workspace: Option<PathBuf>,
    #[arg(long)]
    pub config: PathBuf,
    #[arg(long)]
    pub pack: Option<PathBuf>,
    #[arg(long)]
    pub runtime: Option<PathBuf>,
    #[arg(long)]
    pub events_jsonl: Option<String>,
    #[arg(long)]
    pub trace: Option<PathBuf>,
    #[arg(long)]
    pub checkpoint: Option<PathBuf>,
    #[arg(long, value_enum, default_value_t = AgentProfile::Standard)]
    pub profile: AgentProfile,
    #[arg(
        long,
        help_heading = "Demo Options",
        help = "Scripted LLM response JSONL path (demo-only)",
        required_if_eq("profile", "demo-scripted")
    )]
    pub llm_script_jsonl: Option<PathBuf>,
    #[arg(long, default_value_t = false)]
    pub verbose: bool,
    #[arg(long, default_value_t = false)]
    pub verbose_llm: bool,
    #[arg(long, value_enum)]
    pub approvals_mode: Option<ApprovalsMode>,
    #[arg(long)]
    pub max_iterations: Option<usize>,
    #[arg(long)]
    pub max_planner_rounds: Option<u8>,
    #[arg(long)]
    pub max_tool_rounds: Option<u8>,
    #[arg(long)]
    pub max_index_candidates: Option<usize>,
    #[arg(long)]
    pub planner_context_token_budget: Option<usize>,
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub format: OutputFormat,
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
