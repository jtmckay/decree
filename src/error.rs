use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DecreeError {
    #[error("routine not found: {0}")]
    RoutineNotFound(String),

    #[error("max depth exceeded ({0}) — inbox recursion limit hit")]
    MaxDepthExceeded(u32),

    #[error("no spec files found in specs/")]
    NoSpecs,

    #[error("message not found: {0}")]
    MessageNotFound(String),

    #[error("ambiguous ID prefix '{prefix}' matches: {candidates:?}")]
    AmbiguousId {
        prefix: String,
        candidates: Vec<String>,
    },

    #[error("not a decree project (no .decree/ directory found)")]
    NotInitialized,

    #[error("config error: {0}")]
    Config(String),

    #[error("model error: {0}")]
    Model(String),

    #[error("session error: {0}")]
    Session(String),

    #[error("checkpoint integrity failure: hash mismatch for {0:?}")]
    CheckpointIntegrity(Vec<String>),

    #[error("revert failed: {0}")]
    RevertFailed(String),

    #[error("diff parse error: {0}")]
    DiffParse(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Yaml(#[from] serde_yaml::Error),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// Find the project root by searching for `.decree/` starting from the current
/// directory and walking up. Returns the directory containing `.decree/`.
pub fn find_project_root() -> Result<PathBuf, DecreeError> {
    let mut dir = std::env::current_dir()?;
    loop {
        if dir.join(".decree").is_dir() {
            return Ok(dir);
        }
        if !dir.pop() {
            return Err(DecreeError::NotInitialized);
        }
    }
}
