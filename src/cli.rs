use clap::{Parser, Subcommand, ValueEnum};

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

    /// Manage cron schedules
    Cron {
        #[command(subcommand)]
        subcommand: CronSubcommand,
    },

    /// Install AI assistant skill/instructions
    Skill {
        /// Installation scope: project (current repo) or user (home directory)
        #[arg(long, value_enum)]
        scope: Option<SkillScope>,

        /// Target AI assistant: claude or copilot
        #[arg(long, value_enum)]
        target: Option<SkillTarget>,

        /// Overwrite existing file even if it differs from the bundled template
        #[arg(long)]
        force: bool,

        /// Skill name(s) to install (repeatable; for non-TTY / scripting)
        #[arg(long = "skill", value_name = "NAME")]
        skills: Vec<String>,

        /// Install all available skills for the selected target
        #[arg(long)]
        all: bool,
    },

    /// Verbose help
    Help,
}

#[derive(Debug, Clone, ValueEnum)]
pub enum SkillScope {
    Project,
    User,
}

#[derive(Debug, Clone, ValueEnum)]
pub enum SkillTarget {
    Claude,
    Copilot,
}

#[derive(Subcommand, Debug)]
pub enum CronSubcommand {
    /// List all cron schedules with last/next run times
    List,
}
