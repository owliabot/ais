use ais_runner::{
    execute_plan_diff, execute_replay, execute_run_plan, execute_run_workflow, Cli, Commands,
    PlanTopLevelCommand, RunCommand,
};
use clap::Parser;

fn main() {
    let cli = Cli::parse();
    let result = match cli.command {
        Commands::Run(run_command) => match run_command {
            RunCommand::Plan(command) => execute_run_plan(&command),
            RunCommand::Workflow(command) => execute_run_workflow(&command),
        },
        Commands::Plan(plan_command) => match plan_command {
            PlanTopLevelCommand::Diff(command) => execute_plan_diff(&command),
        },
        Commands::Replay(command) => execute_replay(&command),
    };

    match result {
        Ok(output) => {
            println!("{output}");
        }
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
#[path = "main_test.rs"]
mod tests;
