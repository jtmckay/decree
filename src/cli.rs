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

    /// Write .decree/schema/: the JSON Schemas for machines and message frontmatter
    #[command(after_help = SCHEMA_EDITORS)]
    Schema,

    /// Queue a message for a machine in inbox/; the body is read from stdin. Prints its id
    Emit {
        /// Machine the message names
        #[arg(long)]
        machine: String,

        /// Sets the machine's data for this run (repeatable)
        #[arg(long = "param", value_name = "NAME=VALUE")]
        params: Vec<String>,
    },

    /// Reply to a run waiting in a `person` state
    Event {
        /// Wait id (<run id>.w<seq>), or run id meaning its current wait
        target: String,

        /// Event to deliver: one of the waiting state's options
        event: String,

        /// Note, written as the reply's body
        #[arg(short = 'm', value_name = "NOTE")]
        note: Option<String>,
    },

    /// Daemon: monitor inbox + cron
    Daemon {
        /// Polling interval in seconds
        #[arg(long, default_value = "2")]
        interval: u64,
    },

    /// Runs by status and queued messages; with an id, one run's events
    Status {
        /// Run id
        id: Option<String>,

        /// Cron files and when each fires next
        #[arg(long)]
        cron: bool,
    },

    /// Follow the live output of a run, by default the active one, until it stops
    Tail {
        /// Run id
        id: Option<String>,
    },

    /// Make an interrupted or finished run pending again; the next `process` continues it
    Retry {
        /// Run id
        id: String,

        /// Atomic state to continue in [default: the interrupted run's state, or the state
        /// a finished run last left]
        #[arg(long = "state", value_name = "STATE")]
        state: Option<String>,
    },

    /// Delete the folders of finished runs older than an age; only this command deletes runs
    Prune {
        /// Age of the run's `run_finished` event: a whole number and d, h or m (30d, 12h, 90m);
        /// 0m means every finished run
        #[arg(long, value_name = "AGE", value_parser = crate::commands::prune::parse_age)]
        older_than: chrono::TimeDelta,

        /// List the runs it would delete, and delete nothing
        #[arg(long)]
        dry_run: bool,
    },

    /// Verbose help
    Help,
}

/// How to view `decree graph` output (docs/reference/graph.md, Viewing).
const GRAPH_VIEWING: &str = "\
Viewing:
  1. Run `decree graph`, then open `.decree/graph/<machine>.md` (or `system.md`).
  2. In VS Code, press Ctrl+Shift+V (Cmd+Shift+V on macOS) for the preview;
     VS Code 1.121 and later render Mermaid in Markdown without an extension.
     GitHub, GitLab and Obsidian render the committed files as they are.
  3. Without any of those, copy the lines inside the `mermaid` fence into https://mermaid.live.";

/// How editors use `decree schema` output (docs/reference/machines.md, Schema).
const SCHEMA_EDITORS: &str = "\
Editors:
  Every machine starts with `# yaml-language-server: $schema=../schema/machine.schema.json`.
  Editors with the YAML language server (VS Code's YAML extension by Red Hat, and others)
  then complete keys and underline mistakes as you type. `decree check` remains the authority:
  the schema checks shape, `decree check` also checks names, targets and reachability.";

/// AI backends `init` can configure, in the order it looks for them on `PATH`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum AiBackend {
    Opencode,
    Claude,
    Copilot,
}
