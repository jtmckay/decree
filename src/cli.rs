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
    /// Initialize project (refuses to touch an existing .decree/)
    Init {
        /// AI backend of the router machine init writes [default: first found on PATH, else opencode]
        #[arg(long, value_enum)]
        ai: Option<AiBackend>,

        /// Write the backend's default permissions file
        #[arg(long)]
        permissions: bool,
    },

    /// Process all migrations + drain inbox
    Process {
        /// Show what would be processed without executing
        #[arg(long)]
        dry_run: bool,
    },

    /// Validate machines and pending messages; prints one line per error
    Check,

    /// Write .decree/graph/: a Mermaid diagram in Markdown per machine, and system.md
    #[command(after_help = GRAPH_VIEWING)]
    Graph,

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

/// How to view `decree graph` output (spec section 9, Viewing).
const GRAPH_VIEWING: &str = "\
Viewing:
  1. Run `decree graph`, then open `.decree/graph/<machine>.md` (or `system.md`).
  2. In VS Code, press Ctrl+Shift+V (Cmd+Shift+V on macOS) for the preview;
     VS Code 1.121 and later render Mermaid in Markdown without an extension.
     GitHub, GitLab and Obsidian render the committed files as they are.
  3. Without any of those, copy the lines inside the `mermaid` fence into https://mermaid.live.";

/// AI backends `init` can configure, in 0.4.2's detection order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum AiBackend {
    Opencode,
    Claude,
    Copilot,
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
