// Crate-private modules expose 0.4.2 items nothing calls; docs/0.5-inventory.md lists them.
#![allow(dead_code)] // removed in M5.3

pub(crate) mod cli;
pub(crate) mod commands;
pub(crate) mod cond;
pub(crate) mod config;
pub(crate) mod cron;
pub(crate) mod error;
pub(crate) mod graph;
pub(crate) mod hooks;
pub(crate) mod interpreter;
pub(crate) mod machine;
pub(crate) mod message;
pub(crate) mod reply;
pub(crate) mod routine;
pub(crate) mod runtime;

use clap::Parser;
use cli::{Cli, Command, CronSubcommand};
use colored::Colorize;
use error::{DecreeError, EXIT_SUCCESS};

/// Parse the command line, run the command and return the process exit code.
/// This is the crate's only public item; `src/main.rs` calls it.
pub fn run() -> i32 {
    let cli = Cli::parse();

    // `colored` already honours NO_COLOR, CLICOLOR and TTY detection; the flag overrides it.
    if cli.no_color {
        colored::control::set_override(false);
    }

    match dispatch(cli.command) {
        Ok(()) => EXIT_SUCCESS,
        Err(e) => {
            eprintln!("{}: {e}", "error".red());
            e.exit_code()
        }
    }
}

fn dispatch(command: Option<Command>) -> Result<(), DecreeError> {
    match command {
        // `decree init`, `decree help`, and `decree skill` don't require an existing project
        Some(Command::Init { ai, permissions }) => commands::init::run(ai, permissions),
        Some(Command::Help) => commands::help(),
        Some(Command::Skill {
            scope,
            target,
            force,
            skills,
            all,
        }) => commands::skill::run(scope, target, force, skills, all),

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
                Command::Check => commands::check::run(&root),
                Command::Graph => commands::graph::run(&root),
                Command::Emit { machine, params } => commands::emit::run(&root, &machine, &params),
                Command::Event {
                    target,
                    event,
                    note,
                } => commands::event::run(&root, &target, &event, note.as_deref()),
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
                Command::Init { .. } | Command::Help | Command::Skill { .. } => unreachable!(),
            }
        }
    }
}
