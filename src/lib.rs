pub(crate) mod cli;
pub(crate) mod commands;
pub(crate) mod cond;
pub(crate) mod console;
pub(crate) mod cron;
pub(crate) mod dotenv;
pub(crate) mod duration;
pub(crate) mod error;
pub(crate) mod events;
pub(crate) mod graph;
pub(crate) mod interpreter;
pub(crate) mod layout;
pub(crate) mod machine;
pub(crate) mod message;
pub(crate) mod reply;
pub(crate) mod runtime;
pub(crate) mod trace;

use clap::Parser;
use cli::{Cli, Command, Format};
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
    let root = error::require_project_root;
    // Bare `decree` runs `decree process`; every command but `init` and `help` needs a project.
    match command.unwrap_or(Command::Process {
        dry_run: false,
        retry: None,
        state: None,
        format: Format::Text,
        quiet: false,
    }) {
        Command::Init { ai, permissions } => commands::init::run(ai, permissions),
        Command::Help => commands::help(),
        Command::Process {
            dry_run,
            retry,
            state,
            format,
            quiet,
        } => {
            // `--retry` alone names no run: the blocking migration.
            let retry = retry.map(|id| commands::process::Retry {
                id: Some(id).filter(|id| !id.is_empty()),
                state,
            });
            commands::process::run(&root()?, dry_run, retry, format, quiet)
        }
        Command::Check { format } => commands::check::run(&root()?, format),
        Command::Graph { format } => commands::graph::run(&root()?, format),
        Command::Schema { format } => commands::schema::run(&root()?, format),
        Command::Skill { ai, format } => commands::skill::run(&root()?, ai, format),
        Command::Emit {
            machine,
            params,
            format,
        } => commands::emit::run(&root()?, &machine, &params, format),
        Command::Event {
            target,
            event,
            note,
            format,
        } => commands::event::run(&root()?, &target, &event, note.as_deref(), format),
        Command::Daemon { interval, quiet } => commands::daemon::run(&root()?, interval, quiet),
        Command::Tail { id } => commands::tail::run(&root()?, id.as_deref()),
        Command::Prune {
            older_than,
            dry_run,
            format,
        } => commands::prune::run(&root()?, older_than, dry_run, format),
        Command::Status { id, cron, format } => {
            if cron && format == Format::Json {
                use clap::CommandFactory;
                Cli::command()
                    .error(
                        clap::error::ErrorKind::ArgumentConflict,
                        "--cron prints text only; --format json covers `status` and `status <id>`",
                    )
                    .exit();
            }
            let cwd = std::env::current_dir()?;
            let root = error::find_project_root();
            commands::status::run_from(&cwd, root, id.as_deref(), cron, format)
        }
    }
}
