use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "decree",
    about = "Task orchestration for AI-assisted development",
    disable_help_subcommand = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Initialize project
    Init,

    /// Process all migrations + drain inbox
    Process,

    /// Build starter prompt, copy or launch AI
    Starter {
        /// Starter template name
        name: Option<String>,
    },

    /// List routines or show routine detail
    Routine {
        /// Routine name to show detail for
        name: Option<String>,
    },

    /// Run all routine pre-checks
    Verify,

    /// Monitor inbox + cron
    Daemon {
        /// Poll interval in seconds
        #[arg(long, default_value = "2")]
        interval: u64,
    },

    /// Show progress
    Status,

    /// Show execution log
    Log {
        /// Message or chain ID
        id: Option<String>,
    },

    /// Show verbose help
    Help,
}
