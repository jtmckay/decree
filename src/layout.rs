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
