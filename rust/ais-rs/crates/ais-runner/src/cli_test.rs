use super::{AgentProfile, Cli, Commands, PlanTopLevelCommand, RunCommand};
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

#[test]
fn cli_parses_agent_command() {
    let cli = Cli::try_parse_from([
        "ais-runner",
        "agent",
        "--plan",
        "run.plan.json",
        "--workspace",
        "./workspace",
        "--config",
        "runner.config.yaml",
        "--pack",
        "safe.pack.yaml",
        "--profile",
        "demo-scripted",
        "--llm-script-jsonl",
        "assist.jsonl",
        "--verbose-llm",
        "--max-tool-rounds",
        "24",
        "--max-index-candidates",
        "16",
        "--planner-context-token-budget",
        "9000",
        "--llm-transcript-path",
        "llm.full.md",
        "--llm-transcript-append",
        "--approvals-mode",
        "yolo",
    ])
    .expect("agent command must parse");

    match cli.command {
        Commands::Agent(command) => {
            assert_eq!(
                command.plan.as_deref(),
                Some(std::path::Path::new("run.plan.json"))
            );
            assert_eq!(
                command.workspace.as_deref(),
                Some(std::path::Path::new("./workspace"))
            );
            assert_eq!(
                command.config.as_path(),
                std::path::Path::new("runner.config.yaml")
            );
            assert_eq!(
                command.pack.as_deref(),
                Some(std::path::Path::new("safe.pack.yaml"))
            );
            assert_eq!(
                command.llm_script_jsonl.as_deref(),
                Some(std::path::Path::new("assist.jsonl"))
            );
            assert_eq!(command.profile, AgentProfile::DemoScripted);
            assert_eq!(command.approvals_mode, Some(super::ApprovalsMode::Yolo));
            assert_eq!(command.max_tool_rounds, Some(24));
            assert_eq!(command.max_index_candidates, Some(16));
            assert_eq!(command.planner_context_token_budget, Some(9000));
            assert_eq!(
                command.llm_transcript_path.as_deref(),
                Some(std::path::Path::new("llm.full.md"))
            );
            assert!(command.llm_transcript_append);
            assert!(command.verbose_llm);
        }
        _ => panic!("expected agent command"),
    }
}

#[test]
fn cli_rejects_demo_scripted_profile_without_script() {
    let result = Cli::try_parse_from([
        "ais-runner",
        "agent",
        "--plan",
        "run.plan.json",
        "--config",
        "runner.config.yaml",
        "--profile",
        "demo-scripted",
    ]);
    assert!(result.is_err());
}

#[test]
fn cli_parses_agent_intent_command() {
    let cli = Cli::try_parse_from([
        "ais-runner",
        "agent",
        "--intent",
        "check balances and transfer",
        "--config",
        "runner.config.yaml",
    ])
    .expect("agent intent command must parse");

    match cli.command {
        Commands::Agent(command) => {
            assert_eq!(
                command.intent.as_deref(),
                Some("check balances and transfer")
            );
            assert!(command.plan.is_none());
            assert!(command.intent_file.is_none());
        }
        _ => panic!("expected agent command"),
    }
}

#[test]
fn cli_parses_agent_intent_file_command() {
    let cli = Cli::try_parse_from([
        "ais-runner",
        "agent",
        "--intent-file",
        "intent.txt",
        "--config",
        "runner.config.yaml",
    ])
    .expect("agent intent-file command must parse");

    match cli.command {
        Commands::Agent(command) => {
            assert_eq!(
                command.intent_file.as_deref(),
                Some(std::path::Path::new("intent.txt"))
            );
            assert!(command.plan.is_none());
            assert!(command.intent.is_none());
        }
        _ => panic!("expected agent command"),
    }
}

#[test]
fn cli_rejects_agent_with_both_plan_and_intent() {
    let result = Cli::try_parse_from([
        "ais-runner",
        "agent",
        "--plan",
        "run.plan.json",
        "--intent",
        "do something",
        "--config",
        "runner.config.yaml",
    ]);
    assert!(result.is_err());
}

#[test]
fn cli_rejects_agent_with_both_intent_and_intent_file() {
    let result = Cli::try_parse_from([
        "ais-runner",
        "agent",
        "--intent",
        "do something",
        "--intent-file",
        "intent.txt",
        "--config",
        "runner.config.yaml",
    ]);
    assert!(result.is_err());
}

#[test]
fn cli_rejects_agent_without_plan_or_intent() {
    let result = Cli::try_parse_from(["ais-runner", "agent", "--config", "runner.config.yaml"]);
    assert!(result.is_err());
}
