use super::{Cli, Commands, PlanTopLevelCommand, RunCommand};
use clap::{CommandFactory, Parser};

#[test]
fn cli_help_includes_required_top_level_commands() {
    let mut command = Cli::command();
    let help = command.render_long_help().to_string();
    assert!(help.contains("run"));
    assert!(help.contains("plan"));
    assert!(help.contains("replay"));
}

#[test]
fn cli_parses_run_workflow() {
    let cli = Cli::try_parse_from([
        "ais-runner",
        "run",
        "workflow",
        "--workflow",
        "workflow.yaml",
        "--dry-run",
        "--commands-stdin-jsonl",
    ])
    .expect("run workflow must parse");
    match cli.command {
        Commands::Run(RunCommand::Workflow(command)) => {
            assert!(command.dry_run);
            assert!(command.commands_stdin_jsonl);
        }
        _ => panic!("expected run workflow"),
    }
}

#[test]
fn cli_parses_run_workflow_outputs_path() {
    let cli = Cli::try_parse_from([
        "ais-runner",
        "run",
        "workflow",
        "--workflow",
        "workflow.yaml",
        "--outputs",
        "workflow.outputs.json",
    ])
    .expect("run workflow with outputs path must parse");
    match cli.command {
        Commands::Run(RunCommand::Workflow(command)) => {
            assert_eq!(
                command.outputs.as_deref(),
                Some(std::path::Path::new("workflow.outputs.json"))
            );
        }
        _ => panic!("expected run workflow"),
    }
}

#[test]
fn cli_parses_plan_diff() {
    let cli = Cli::try_parse_from([
        "ais-runner",
        "plan",
        "diff",
        "--before",
        "before.plan.json",
        "--after",
        "after.plan.json",
    ])
    .expect("plan diff must parse");
    match cli.command {
        Commands::Plan(PlanTopLevelCommand::Diff(_)) => {}
        _ => panic!("expected plan diff"),
    }
}

#[test]
fn cli_parses_run_plan_commands_stdin_flag() {
    let cli = Cli::try_parse_from([
        "ais-runner",
        "run",
        "plan",
        "--plan",
        "test.plan.json",
        "--commands-stdin-jsonl",
        "--verbose",
    ])
    .expect("run plan with commands-stdin-jsonl must parse");
    match cli.command {
        Commands::Run(RunCommand::Plan(command)) => {
            assert!(command.commands_stdin_jsonl);
            assert!(command.verbose);
        }
        _ => panic!("expected run plan"),
    }
}

#[test]
fn cli_parses_replay_with_checkpoint_plan_and_config() {
    let cli = Cli::try_parse_from([
        "ais-runner",
        "replay",
        "--checkpoint",
        "state.checkpoint.json",
        "--plan",
        "run.plan.json",
        "--config",
        "runner.config.yaml",
        "--until-node",
        "swap-1",
    ])
    .expect("replay checkpoint command must parse");
    match cli.command {
        Commands::Replay(command) => {
            assert_eq!(
                command.plan.as_deref(),
                Some(std::path::Path::new("run.plan.json"))
            );
            assert_eq!(
                command.config.as_deref(),
                Some(std::path::Path::new("runner.config.yaml"))
            );
        }
        _ => panic!("expected replay"),
    }
}
