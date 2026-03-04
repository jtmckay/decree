use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum DecreeError {
    #[error("unknown routine '{0}'")]
    RoutineNotFound(String),

    #[error("max retries exhausted for message {0}")]
    MaxRetriesExhausted(String),

    #[error("max depth exceeded ({0})")]
    MaxDepthExceeded(u32),

    #[error("no migration files found")]
    NoMigrations,

    #[error("message not found: {0}")]
    MessageNotFound(String),

    #[error("pre-check failed for routine {routine}: {reason}")]
    PreCheckFailed { routine: String, reason: String },

    #[error("unknown starter '{0}'")]
    StarterNotFound(String),

    #[error("not initialized — run `decree init` first")]
    NotInitialized,

    #[error("already initialized: {0}")]
    AlreadyInitialized(PathBuf),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error("{hook} hook failed (exit {code})")]
    HookFailed { hook: String, code: i32 },

    #[error("config error: {0}")]
    Config(String),
}

pub type Result<T> = std::result::Result<T, DecreeError>;
