pub(crate) mod attention;
pub(crate) mod cli;
pub(crate) mod commands;
pub(crate) mod cond;
pub(crate) mod console;
pub(crate) mod cron;
pub(crate) mod dotenv;
pub(crate) mod duration;
pub(crate) mod error;
pub(crate) mod events;
pub(crate) mod filter;
pub(crate) mod graph;
pub(crate) mod interpreter;
pub(crate) mod layout;
pub(crate) mod machine;
pub(crate) mod message;
pub(crate) mod migration;
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
    // Bare `decree` prints the synopsis and touches nothing; every command but `init`, `upgrade`
    // and `help` needs a project.
    let Some(command) = command else {
        return commands::synopsis();
    };
    match command {
        Command::Init { ai, permissions } => commands::init::run(ai, permissions),
        Command::Help => commands::help(),
        Command::Upgrade { format } => commands::upgrade::run(format),
        Command::Process {
            dry_run,
            retry,
            state,
            format,
            quiet,
            machines,
            exclude_machines,
        } => {
            let retry = commands::process::Retry::from_args(retry, state);
            let select = filter::Select::new(machines, exclude_machines);
            commands::process::run(&root()?, dry_run, retry, format, quiet, &select)
        }
        Command::Check { format } => commands::check::run(&root()?, format),
        Command::Graph { format } => commands::graph::run(&root()?, format),
        Command::Schema { format } => commands::schema::run(&root()?, format),
        Command::Skill { ai, format } => commands::skill::run(&root()?, ai, format),
        Command::Emit {
            machine,
            params,
            format,
        } => commands::emit::run(&root()?, machine.as_deref(), &params, format),
        Command::Event {
            target,
            event,
            note,
            format,
        } => commands::event::run(&root()?, &target, &event, note.as_deref(), format),
        Command::Daemon {
            interval,
            retry,
            state,
            quiet,
            machines,
            exclude_machines,
        } => {
            let retry = commands::process::Retry::from_args(retry, state);
            let select = filter::Select::new(machines, exclude_machines);
            commands::daemon::run(&root()?, interval, retry, quiet, &select)
        }
        Command::Stop { note, format } => commands::stop::run(&root()?, note.as_deref(), format),
        Command::Tail { id } => commands::tail::run(&root()?, id.as_deref()),
        Command::Prune {
            older_than,
            dry_run,
            format,
        } => commands::prune::run(&root()?, older_than, dry_run, format),
        Command::Status {
            id,
            cron,
            all,
            format,
        } => {
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
            commands::status::run_from(&cwd, root, id.as_deref(), cron, all, format)
        }
    }
}
