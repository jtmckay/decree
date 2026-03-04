use std::path::Path;
use std::process;

use clap::Parser;

use decree::cli::{Cli, Command};
use decree::error::DecreeError;

fn main() {
    let cli = Cli::parse();

    if let Err(e) = dispatch(cli) {
        let code = match &e {
            DecreeError::HookFailed { code, .. } => *code,
            _ => 1,
        };
        eprintln!("error: {e}");
        process::exit(code);
    }
}

fn dispatch(cli: Cli) -> decree::error::Result<()> {
    let command = cli.command.unwrap_or(Command::Process);

    // Commands that don't require .decree/ to exist
    match &command {
        Command::Init => return decree::commands::init::run(),
        Command::Help => {
            print_help();
            return Ok(());
        }
        _ => {}
    }

    // All other commands require .decree/ to exist
    let decree_dir = Path::new(".decree");
    if !decree_dir.exists() {
        return Err(DecreeError::NotInitialized);
    }

    match command {
        Command::Process => decree::commands::process::run(),
        Command::Starter { name } => decree::commands::starter::run(name.as_deref()),
        Command::Routine { name } => decree::commands::routine::run(name.as_deref()),
        Command::Verify => {
            let all_pass = decree::commands::verify::run()?;
            if !all_pass {
                process::exit(1);
            }
            Ok(())
        }
        Command::Daemon { interval } => decree::commands::daemon::run(interval),
        Command::Status => decree::commands::status::run(),
        Command::Log { id } => decree::commands::log::run(id.as_deref()),
        Command::Init | Command::Help => unreachable!(),
    }
}

fn print_help() {
    print!("{}", include_str!("templates/help.txt"));
}
