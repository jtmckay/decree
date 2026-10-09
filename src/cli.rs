use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser, Debug)]
#[command(
    name = "decree",
    version,
    about = "Durable state-machine workflows from plain files, for scripts, AI agents and people",
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

    /// Process everything once: replies and timeouts, pending runs, the inbox, then migrations in order
    Process {
        /// Show what would be processed without executing
        #[arg(long, conflicts_with = "retry")]
        dry_run: bool,

        /// First continue an interrupted or finished run: ID, or the migration that blocks
        /// the queue
        #[arg(long, value_name = "ID", num_args = 0..=1, default_missing_value = "")]
        retry: Option<String>,

        /// Atomic state the retried run continues in [default: the interrupted run's state,
        /// or the state a finished run last left]
        #[arg(long = "state", value_name = "STATE", requires = "retry")]
        state: Option<String>,

        /// Print no run output, status line or run summaries: only what needs attention (for cron and CI)
        #[arg(long, short = 'q', conflicts_with = "dry_run")]
        quiet: bool,

        /// Output format of --dry-run
        #[arg(
            long,
            value_enum,
            default_value_t,
            requires = "dry_run",
            conflicts_with = "retry"
        )]
        format: Format,
    },

    /// Validate machines and pending messages; prints one line per error
    Check {
        /// Output format: text, a JSON document, or a SARIF 2.1.0 log
        #[arg(long, value_enum, default_value_t)]
        format: CheckFormat,
    },

    /// Write .decree/graph/: a Mermaid diagram in Markdown per machine, and system.md
    #[command(after_help = GRAPH_VIEWING)]
    Graph {
        /// Output format
        #[arg(long, value_enum, default_value_t)]
        format: Format,
    },

    /// Write .decree/schema/v1/: the JSON Schemas of machines, messages, events and router files
    #[command(after_help = SCHEMA_EDITORS)]
    Schema {
        /// Output format
        #[arg(long, value_enum, default_value_t)]
        format: Format,
    },

    /// Write the decree skill (.claude/skills/decree/ or .github/skills/decree/), overwriting decree's own files
    Skill {
        /// AI backend whose skill folder to write [default: every skill folder that exists, else the backend init would pick]
        #[arg(long, value_enum)]
        ai: Option<AiBackend>,

        /// Output format
        #[arg(long, value_enum, default_value_t)]
        format: Format,
    },

    /// Queue a message for a machine in inbox/; the body is read from stdin. Prints its id
    Emit {
        /// Machine the message names
        #[arg(long)]
        machine: String,

        /// Sets the machine's data for this run (repeatable)
        #[arg(long = "param", value_name = "NAME=VALUE")]
        params: Vec<String>,

        /// Output format
        #[arg(long, value_enum, default_value_t)]
        format: Format,
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

        /// Output format
        #[arg(long, value_enum, default_value_t)]
        format: Format,
    },

    /// Run the process loop repeatedly, plus cron, until stopped
    Daemon {
        /// Polling interval: a whole number and s, m, h or d (2s, 1m)
        #[arg(long, value_name = "DURATION", default_value = "2s", value_parser = crate::duration::parse)]
        interval: std::time::Duration,

        /// Print no run output, status line or run summaries: only what needs attention
        #[arg(long, short = 'q')]
        quiet: bool,
    },

    /// Runs by status and queued messages; with an id, one run's events
    Status {
        /// Run id
        id: Option<String>,

        /// Cron files and when each fires next
        #[arg(long)]
        cron: bool,

        /// Output format, without --cron
        #[arg(long, value_enum, default_value_t)]
        format: Format,
    },

    /// Follow the live output of runs: the active one and each after it, or with an id that run until it stops
    Tail {
        /// Run id
        id: Option<String>,
    },

    /// Delete the folders of finished runs older than an age; only this command deletes runs
    Prune {
        /// Age of the run's `run_finished` event: a whole number and s, m, h or d (30d, 12h,
        /// 90m); 0s means every finished run
        #[arg(long, value_name = "AGE", value_parser = crate::duration::parse)]
        older_than: std::time::Duration,

        /// List the runs it would delete, and delete nothing
        #[arg(long)]
        dry_run: bool,

        /// Output format
        #[arg(long, value_enum, default_value_t)]
        format: Format,
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
  Editors that read SchemaStore (VS Code's YAML extension by Red Hat, JetBrains IDEs, and
  others) apply the hosted machine schema to .decree/machines/*.yml and complete keys and
  underline mistakes as you type; this local copy is for agents and offline editors, and git
  ignores it (docs/editors.md). `decree check` remains the authority:
  the schema checks shape, `decree check` also checks names, targets and reachability.";

/// How a command prints its report (docs/reference/cli.md, Machine-readable output).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
pub enum Format {
    /// Lines for people
    #[default]
    Text,
    /// One JSON document on stdout, described by .decree/schema/v1/cli/<command>.schema.json
    Json,
}

/// `decree check` also prints SARIF 2.1.0, the OASIS format code scanning tools read.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
pub enum CheckFormat {
    /// Lines for people
    #[default]
    Text,
    /// One JSON document on stdout, described by .decree/schema/v1/cli/check.schema.json
    Json,
    /// A SARIF 2.1.0 log on stdout
    Sarif,
}

/// AI backends `init` can configure (and `skill` write for), in the order it looks for them on `PATH`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum AiBackend {
    Opencode,
    Claude,
    Copilot,
}
