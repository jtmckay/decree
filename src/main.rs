use clap::Parser;
use decree::cli::{Cli, Command, CronSubcommand};
use decree::commands;
use decree::error::{self, color, DecreeError, EXIT_SUCCESS};
use std::process;

fn main() {
    let cli = Cli::parse();

    // Initialize color settings
    color::init(cli.no_color);

    let result = dispatch(cli.command);

    match result {
        Ok(()) => process::exit(EXIT_SUCCESS),
        Err(e) => {
            eprintln!("{}: {e}", color::error("error"));
            process::exit(e.exit_code());
        }
    }
}

fn dispatch(command: Option<Command>) -> Result<(), DecreeError> {
    match command {
        // `decree init`, `decree help`, and `decree skill` don't require an existing project
        Some(Command::Init) => commands::init::run(),
        Some(Command::Help) => commands::help(),
        Some(Command::Skill { scope, target, force, skills, all }) => {
            commands::skill::run(scope, target, force, skills, all)
        }

        // Bare `decree` defaults to `decree process`
        None => {
            let root = error::require_project_root()?;
            commands::process::run(&root, false)
        }

        // All other commands require an existing project
        Some(cmd) => {
            let root = error::require_project_root()?;
            match cmd {
                Command::Process { dry_run } => commands::process::run(&root, dry_run),
                Command::Routine { name } => commands::routine::run(&root, name.as_deref()),
                Command::Verify => commands::routine::verify(&root),
                Command::Daemon { interval } => commands::daemon::run(&root, interval),
                Command::Status => commands::status::run(&root),
                Command::Log { id } => commands::log::run(&root, id.as_deref()),
                Command::RoutineSync { source } => {
                    commands::routine_sync::run(&root, source.as_deref())
                }
                Command::Cron { subcommand } => match subcommand {
                    CronSubcommand::List => commands::cron_list::run(&root),
                },
                Command::Init | Command::Help | Command::Skill { .. } => unreachable!(),
            }
        }
    }
}
