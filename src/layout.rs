//! The `.decree/` layout (docs/reference/README.md): a project is machines, scripts and messages.

pub const DECREE_DIR: &str = ".decree";
pub const CRON_DIR: &str = "cron";
pub const INBOX_DIR: &str = "inbox";
pub const RUNS_DIR: &str = "runs";
pub const MIGRATIONS_DIR: &str = "migrations";
pub const PROCESSED_FILE: &str = "processed.md";
pub const GITIGNORE_FILE: &str = ".gitignore";

/// Code that scripts source, config and data; decree never runs anything in it
/// (docs/reference/scripts.md, Shared code).
pub const LIB_DIR: &str = "lib";

/// The project's dotenv file, `.decree/env` (docs/reference/scripts.md, Environment).
pub const ENV_FILE: &str = "env";

/// The claimed message, in the run directory (docs/reference/README.md).
pub const MESSAGE_FILE: &str = "message.md";

/// Folder in the run directory that delivered replies are moved into (docs/reference/messages.md).
pub const RECEIVED_DIR: &str = "received";
