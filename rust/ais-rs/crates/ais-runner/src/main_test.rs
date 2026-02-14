use ais_runner::Cli;
use clap::CommandFactory;

#[test]
fn help_smoke_lists_core_subcommands() {
    let mut command = Cli::command();
    let help = command.render_long_help().to_string();
    assert!(help.contains("run"));
    assert!(help.contains("plan"));
    assert!(help.contains("replay"));
}
