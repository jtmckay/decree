//! The `.decree/` layout (docs/reference/README.md): a project is machines, scripts and messages.

pub const DECREE_DIR: &str = ".decree";
pub const CRON_DIR: &str = "cron";
pub const INBOX_DIR: &str = "inbox";
pub const RUNS_DIR: &str = "runs";
/// What each machine keeps between runs, `store/<machine>/` (docs/reference/scripts.md, Store).
pub const STORE_DIR: &str = "store";
pub const MIGRATIONS_DIR: &str = "migrations";
pub const PROCESSED_FILE: &str = "processed.md";
pub const GITIGNORE_FILE: &str = ".gitignore";

/// `.decree/attention/`: one file per run that needs a person; a file with `holds: true`, or
/// one decree cannot read, stops `process` and `daemon` (docs/reference/cli.md, Attention).
pub const ATTENTION_DIR: &str = "attention";

/// `runs/<id>/hold`: written by a script (the built-in `keep`), it makes the run's
/// attention file hold the queue (docs/reference/scripts.md, Run directory).
pub const HOLD_FILE: &str = "hold";

/// `runs/<id>/leftovers`: written by a script (the built-in `clean_tree`), it names the
/// runs that may have left the changes this run found; the run's attention file quotes it
/// (docs/reference/scripts.md, Run directory).
pub const LEFTOVERS_FILE: &str = "leftovers";

/// `runs/<id>/STOP`: an agent's question, which the run's attention file quotes
/// (docs/reference/scripts.md).
pub const STOP_FILE: &str = "STOP";

/// `.decree/process.log`: what `process` and `daemon` print as runs go, kept even with
/// `--quiet`, its last 2 MiB (docs/reference/cli.md, Output).
pub const PROCESS_LOG_FILE: &str = "process.log";

/// Code that scripts source, config and data; decree never runs anything in it
/// (docs/reference/scripts.md, Shared code).
pub const LIB_DIR: &str = "lib";

/// The dotenv file every script gets, `.decree/.env` (docs/reference/scripts.md,
/// Environment). Never committed, as no `.env*` file but `ENV_EXAMPLE_FILE` is.
pub const ENV_FILE: &str = ".env";

/// The committed template of the `.env*` files, which decree never reads.
pub const ENV_EXAMPLE_FILE: &str = ".env.example";

/// The claimed message, in the run directory (docs/reference/README.md).
pub const MESSAGE_FILE: &str = "message.md";

/// Folder in the run directory that delivered replies are moved into (docs/reference/messages.md).
pub const RECEIVED_DIR: &str = "received";

/// `.decree/store/<machine>/`, the folder `DECREE_STORE` names for `machine`'s scripts.
pub fn store_dir(project_root: &std::path::Path, machine: &str) -> std::path::PathBuf {
    project_root.join(DECREE_DIR).join(STORE_DIR).join(machine)
}
