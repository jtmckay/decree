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
        /// AI backend whose skill init writes, and AI_CLI in .env.example [default: first found on PATH, else opencode]
        #[arg(long, value_enum)]
        ai: Option<AiBackend>,

        /// Write the backend's default permissions file
        #[arg(long)]
        permissions: bool,
    },

    /// Validate machines and pending messages; prints one line per error
    Check {
        /// Output format: text, a JSON document, or a SARIF 2.1.0 log
        #[arg(long, value_enum, default_value_t)]
        format: CheckFormat,
    },

    /// Write a Mermaid diagram in Markdown per machine (.decree/machines/<name>/graph.md) and .decree/system.md
    #[command(after_help = GRAPH_VIEWING)]
    Graph {
        /// Output format
        #[arg(long, value_enum, default_value_t)]
        format: Format,
    },

    /// Queue a message in inbox/ from stdin: a body, or a whole message with frontmatter. Prints its id
    Emit {
        /// Machine the message names; may be left out when stdin's frontmatter names one
        #[arg(long)]
        machine: Option<String>,

        /// Sets the machine's data for this run (repeatable)
        #[arg(long = "param", value_name = "NAME=VALUE")]
        params: Vec<String>,

        /// Output format
        #[arg(long, value_enum, default_value_t)]
        format: Format,
    },

    /// Reply to a run waiting in a `reply` state
    Reply {
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

    /// Work the queue and fire cron until stopped, waiting for replies; with --once, work until idle and exit
    Daemon {
        /// Start, fire the @reboot cron files, work until nothing can go on without a person
        /// or a program, print what waits or failed, and exit (for CI and cron)
        #[arg(long, conflicts_with = "interval")]
        once: bool,

        /// Polling interval: a whole number and s, m, h or d (2s, 1m)
        #[arg(long, value_name = "DURATION", default_value = "2s", value_parser = crate::duration::parse)]
        interval: std::time::Duration,

        /// Print no run output, status line or run summaries: only what needs attention
        #[arg(long, short = 'q')]
        quiet: bool,

        /// Run only these machines: repeatable, comma-separated [env: DECREE_MACHINES]
        #[arg(
            long = "machine",
            value_name = "MACHINE",
            value_delimiter = ',',
            conflicts_with = "exclude_machines"
        )]
        machines: Vec<String>,

        /// Run every machine but these: repeatable, comma-separated [env:
        /// DECREE_EXCLUDE_MACHINES]
        #[arg(
            long = "exclude-machine",
            value_name = "MACHINE",
            value_delimiter = ','
        )]
        exclude_machines: Vec<String>,
    },

    /// Continue an interrupted or finished run: make it pending, for the daemon to continue
    Retry {
        /// Run id
        id: String,

        /// Atomic state to continue in [default: the interrupted run's state, or the state a
        /// finished run last left]
        #[arg(long = "state", value_name = "STATE")]
        state: Option<String>,

        /// Output format
        #[arg(long, value_enum, default_value_t)]
        format: Format,
    },

    /// End a waiting, pending or interrupted run, and the child runs below it, in `failed`
    Cancel {
        /// Run id
        id: String,

        /// Note, recorded in the run's `cancel` event
        #[arg(short = 'm', value_name = "NOTE")]
        note: Option<String>,

        /// Output format
        #[arg(long, value_enum, default_value_t)]
        format: Format,
    },

    /// Pause the queue: runs going now finish their step, and nothing more runs until `decree resume`
    Pause {
        /// Note, written into .decree/attention/pause.md
        #[arg(short = 'm', value_name = "NOTE")]
        note: Option<String>,

        /// Output format
        #[arg(long, value_enum, default_value_t)]
        format: Format,
    },

    /// Resume the queue `decree pause` held: delete .decree/attention/pause.md
    Resume {
        /// Output format
        #[arg(long, value_enum, default_value_t)]
        format: Format,
    },

    /// Runs by status and queued messages; with an id, one run's events
    Status {
        /// Run id
        id: Option<String>,

        /// Cron files and when each fires next
        #[arg(long)]
        cron: bool,

        /// List every run and queued message, not only the latest of each kind
        #[arg(long, conflicts_with_all = ["id", "cron"])]
        all: bool,

        /// Output format
        #[arg(long, value_enum, default_value_t)]
        format: Format,
    },

    /// Follow the live output of runs: the active one and each after it, or with an id that run until it stops
    Tail {
        /// Run id
        id: Option<String>,
    },

    /// Print how to upgrade decree (the install line, pinned and `cargo install` forms, and what to run in each project after), then offer to run the install line, asking y/N
    Upgrade {
        /// Run the install line without asking
        #[arg(long, short = 'y')]
        yes: bool,

        /// Output format
        #[arg(long, value_enum, default_value_t)]
        format: Format,
    },

    /// Install plugins: every plugin in .decree/plugins/ as its file says (after a checkout), the named ones (an official one is seeded into .decree/plugins/ first), or a plugin file or - for stdin (`curl -fsSL <url> | decree install -`); an edited installed copy, or a replaced plugin file, is first archived as a plugin file in .decree/archive/
    Install {
        /// Plugin names, or one plugin file, or - for stdin [default: every plugin in .decree/plugins/; stdin is never read]
        sources: Vec<String>,

        /// Install decree's built-in copy of each named official plugin, or with no name of every official plugin in .decree/plugins/, replacing its file (archived) and printing what changes: how a project takes a new decree version's plugins
        #[arg(long)]
        official: bool,

        /// Also install the cron files of plugins already installed, archiving edited ones; otherwise only a plugin installed for the first time gets its cron files
        #[arg(long)]
        cron: bool,

        /// Output format
        #[arg(long, value_enum, default_value_t)]
        format: Format,
    },

    /// List the plugins in .decree/plugins/ (official or own, differs, install state), the machines no plugin installed, and the official plugins not yet installed
    List {
        /// Output format
        #[arg(long, value_enum, default_value_t)]
        format: Format,
    },

    /// Write .decree/plugins/<name>.md from the installed copy in .decree/machines/<name>/
    Export {
        /// Machine to export [default: every plugin whose installed copy differs from its file]
        name: Option<String>,

        /// With no name: print the plugins whose installed copy differs from their file, and write nothing
        #[arg(long, conflicts_with = "name")]
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
  1. Run `decree graph`, then open `.decree/machines/<machine>/graph.md` (or `.decree/system.md`).
  2. In VS Code, press Ctrl+Shift+V (Cmd+Shift+V on macOS) for the preview;
     VS Code 1.121 and later render Mermaid in Markdown without an extension.
     GitHub, GitLab and Obsidian render the committed files as they are.
  3. Without any of those, copy the lines inside the `mermaid` fence into https://mermaid.live.";

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
