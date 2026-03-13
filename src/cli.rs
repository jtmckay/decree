use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "decree",
    version,
    about = "AI orchestrator for structured, reproducible workflows",
    disable_help_subcommand = true,
    disable_version_flag = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Print version
    #[arg(short = 'v', long = "version", action = clap::ArgAction::Version)]
    pub version: (),

    /// Disable color output
    #[arg(long = "no-color", global = true)]
    pub no_color: bool,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Initialize project
    Init,

    /// Process all migrations + drain inbox
    Process {
        /// Show what would be processed without executing
        #[arg(long)]
        dry_run: bool,
    },

    /// Build prompt, copy or launch AI
    Prompt {
        /// Prompt template name
        name: Option<String>,
    },

    /// List routines or show routine detail
    Routine {
        /// Routine name to show detail for
        name: Option<String>,
    },

    /// Run all routine pre-checks
    Verify,

    /// Daemon: monitor inbox + cron
    Daemon {
        /// Polling interval in seconds
        #[arg(long, default_value = "2")]
        interval: u64,
    },

    /// Show progress
    Status,

    /// Show execution log
    Log {
        /// Message ID (full, chain, or prefix)
        id: Option<String>,
    },

    /// Sync routine registry with filesystem
    #[command(name = "routine-sync")]
    RoutineSync {
        /// Override shared routines directory
        #[arg(long)]
        source: Option<String>,
    },

    /// Verbose help
    Help,
}
