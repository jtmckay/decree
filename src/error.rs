use std::path::{Path, PathBuf};

use crate::layout::DECREE_DIR;

/// Exit codes (docs/reference/cli.md).
pub const EXIT_SUCCESS: i32 = 0;
pub const EXIT_FAILURE: i32 = 1;
pub const EXIT_USAGE: i32 = 2;
/// SIGINT or SIGTERM stopped the run (docs/reference/cli.md).
pub const EXIT_INTERRUPTED: i32 = 130;

/// All error variants for the decree application.
#[derive(Debug, thiserror::Error)]
pub enum DecreeError {
    #[error("message not found: {0}")]
    MessageNotFound(String),

    #[error("not inside a decree project (run `decree init` first)")]
    NoProject,

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
        if dir.join(DECREE_DIR).is_dir() {
            return Some(dir);
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// The 0.4 configuration file, which 0.5 replaced with conventions and fixed limits
/// (docs/reference/README.md, No configuration file).
const LEGACY_CONFIG: &str = "config.yml";

/// Require that we're inside a decree project, returning the root path. A project that still
/// has a 0.4 configuration file is an error naming the layout migration script.
pub fn require_project_root() -> Result<PathBuf, DecreeError> {
    let root = find_project_root().ok_or(DecreeError::NoProject)?;
    check_no_legacy_config(&root)?;
    Ok(root)
}

/// Fail if `root/.decree/` holds a 0.4 configuration file.
pub fn check_no_legacy_config(root: &Path) -> Result<(), DecreeError> {
    if root.join(DECREE_DIR).join(LEGACY_CONFIG).exists() {
        return Err(DecreeError::Other(format!(
            "{DECREE_DIR}/{LEGACY_CONFIG} is not used by decree 0.5; run scripts/migrate-0.4-to-0.5.sh"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_legacy_config_names_the_migration_script() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::create_dir(tmp.path().join(DECREE_DIR)).unwrap();
        assert!(check_no_legacy_config(tmp.path()).is_ok());
        std::fs::write(tmp.path().join(DECREE_DIR).join(LEGACY_CONFIG), "").unwrap();
        let err = check_no_legacy_config(tmp.path()).unwrap_err();
        assert_eq!(
            err.to_string(),
            ".decree/config.yml is not used by decree 0.5; run scripts/migrate-0.4-to-0.5.sh"
        );
        assert_eq!(err.exit_code(), EXIT_FAILURE);
    }
}
