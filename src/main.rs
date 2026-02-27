use clap::{Parser, Subcommand};

use decree::commands;
use decree::error;


#[derive(Parser)]
#[command(name = "decree", about = "Specification-driven project execution framework")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize a decree project
    Init {
        /// Override the GGUF model file path
        #[arg(long = "model-path")]
        model_path: Option<String>,
    },

    /// Interactive planning
    Plan {
        /// Plan template name
        plan: Option<String>,
    },

    /// Run a message
    Run {
        /// Routine name
        #[arg(short, long)]
        m: Option<String>,
        /// Prompt text
        #[arg(short, long)]
        p: Option<String>,
        /// Variables (KEY=VALUE)
        #[arg(short, long, num_args = 1..)]
        v: Vec<String>,
    },

    /// Batch-process all unprocessed specs
    Process,

    /// Daemon: monitor inbox and cron
    Daemon {
        /// Polling interval in seconds
        #[arg(long, default_value = "2")]
        interval: u64,
    },

    /// Show diff for a message
    Diff {
        /// Message or chain ID (or prefix)
        id: Option<String>,
        /// Show changes from this message onward
        #[arg(long)]
        since: Option<String>,
    },

    /// Apply message changes
    Apply {
        /// Message or chain ID (or prefix)
        id: Option<String>,
        /// Apply all messages up through this ID
        #[arg(long)]
        through: Option<String>,
        /// Apply all messages from this ID onward
        #[arg(long)]
        since: Option<String>,
        /// Apply all messages
        #[arg(long)]
        all: bool,
        /// Skip conflict check
        #[arg(long)]
        force: bool,
    },

    /// Derive a Statement of Work from specs
    Sow,

    /// Embedded AI session
    Ai {
        /// One-shot prompt
        #[arg(short, long)]
        p: Option<String>,
        /// Output JSON
        #[arg(long)]
        json: bool,
        /// Maximum tokens to generate
        #[arg(long = "max-tokens")]
        max_tokens: Option<u32>,
        /// Resume a previous session
        #[arg(long)]
        resume: Option<Option<String>>,
    },

    /// Benchmark the embedded model
    Bench {
        /// Prompt to benchmark
        prompt: Option<String>,
        /// Number of runs
        #[arg(long, default_value = "3")]
        runs: u32,
        /// Maximum tokens per run
        #[arg(long = "max-tokens")]
        max_tokens: Option<u32>,
        /// Context size
        #[arg(long)]
        ctx: Option<u32>,
        /// Verbose: show llama.cpp internal logs
        #[arg(short, long)]
        verbose: bool,
    },

    /// Show progress summary
    Status,

    /// Show execution log
    Log {
        /// Message or chain ID (or prefix)
        id: Option<String>,
    },
}

fn main() {
    let cli = Cli::parse();

    let result = match cli.command {
        Some(Commands::Init { model_path }) => {
            commands::init::run(model_path.as_deref())
        }
        Some(Commands::Sow) => commands::sow::run(),
        Some(Commands::Status) => commands::status::run(),
        Some(Commands::Log { id }) => commands::log::run(id.as_deref()),

        Some(Commands::Plan { plan }) => {
            commands::plan::run(plan.as_deref())
        }
        Some(Commands::Run { m, p, v }) => {
            commands::run::run(m.as_deref(), p.as_deref(), &v)
        }
        Some(Commands::Process) => commands::process::run(),
        Some(Commands::Daemon { interval }) => {
            commands::daemon::run(interval)
        }
        Some(Commands::Diff { id, since }) => {
            commands::diff::run(id.as_deref(), since.as_deref())
        }
        Some(Commands::Apply {
            id,
            through,
            since,
            all,
            force,
        }) => commands::apply::run(id, through, since, all, force),
        Some(Commands::Ai {
            p,
            json,
            max_tokens,
            resume,
        }) => {
            let resume_ref = resume
                .as_ref()
                .map(|opt| opt.as_deref());
            commands::ai::run(p.as_deref(), json, max_tokens, resume_ref)
        }
        Some(Commands::Bench {
            prompt,
            runs,
            max_tokens,
            ctx,
            verbose,
        }) => commands::bench::run(prompt.as_deref(), runs, max_tokens, ctx, verbose),

        None => {
            // Smart default: show status if initialized, else hint at init
            match error::find_project_root() {
                Ok(_) => commands::status::run(),
                Err(_) => {
                    println!("No decree project found. Run `decree init` to get started.");
                    Ok(())
                }
            }
        }
    };

    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
