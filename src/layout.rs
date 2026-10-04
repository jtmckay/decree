//! The `.decree/` layout (docs/reference/README.md): a project is machines, scripts and messages.

pub const DECREE_DIR: &str = ".decree";
pub const CRON_DIR: &str = "cron";
pub const INBOX_DIR: &str = "inbox";
pub const RUNS_DIR: &str = "runs";
pub const MIGRATIONS_DIR: &str = "migrations";
pub const PROCESSED_FILE: &str = "processed.md";
pub const GITIGNORE_FILE: &str = ".gitignore";

/// The claimed message, in the run directory (docs/reference/README.md).
pub const MESSAGE_FILE: &str = "message.md";

/// Folder in the run directory that delivered replies are moved into (docs/reference/messages.md).
pub const RECEIVED_DIR: &str = "received";
