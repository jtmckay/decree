use clap::Parser;
use decree::cli::{Cli, Command};
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
        // `decree init` and `decree help` don't require an existing project
        Some(Command::Init) => commands::init::run(),
        Some(Command::Help) => commands::help(),

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
                Command::Prompt { name } => commands::prompt::run(&root, name.as_deref()),
                Command::Routine { name } => commands::routine::run(&root, name.as_deref()),
                Command::Verify => commands::routine::verify(&root),
                Command::Daemon { interval } => commands::daemon::run(&root, interval),
                Command::Status => commands::status::run(&root),
                Command::Log { id } => commands::log::run(&root, id.as_deref()),
                // Already handled above
                Command::Init | Command::Help => unreachable!(),
            }
        }
    }
}
