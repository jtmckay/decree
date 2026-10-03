use std::path::PathBuf;

/// Exit codes following the spec convention.
pub const EXIT_SUCCESS: i32 = 0;
pub const EXIT_FAILURE: i32 = 1;
pub const EXIT_USAGE: i32 = 2;
/// SIGINT or SIGTERM stopped the run (section 8).
pub const EXIT_INTERRUPTED: i32 = 130;

/// All error variants for the decree application.
#[derive(Debug, thiserror::Error)]
pub enum DecreeError {
    #[error("message not found: {0}")]
    MessageNotFound(String),

    #[error("config error: {0}")]
    Config(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("yaml error: {0}")]
    Yaml(#[from] serde_norway::Error),

    #[error(".decree/ already exists; decree init does not touch an existing project")]
    AlreadyInitialized,

    /// SIGINT or SIGTERM stopped `process`; the current run is `interrupted`.
    #[error("interrupted")]
    Interrupted,

    #[error("{0}")]
    Other(String),
}

impl DecreeError {
    /// Map error to the appropriate exit code.
    pub fn exit_code(&self) -> i32 {
        match self {
            DecreeError::AlreadyInitialized => EXIT_USAGE,
            DecreeError::Interrupted => EXIT_INTERRUPTED,
            _ => EXIT_FAILURE,
        }
    }
}

/// Find the project root by searching upward for `.decree/`.
pub fn find_project_root() -> Option<PathBuf> {
    let mut dir = std::env::current_dir().ok()?;
    loop {
        if dir.join(".decree").is_dir() {
            return Some(dir);
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// Require that we're inside a decree project, returning the root path.
pub fn require_project_root() -> Result<PathBuf, DecreeError> {
    find_project_root().ok_or_else(|| {
        DecreeError::Config("not inside a decree project (run `decree init` first)".to_string())
    })
}
