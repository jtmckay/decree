use crate::error::DecreeError;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Directory and file constants.
pub const DECREE_DIR: &str = ".decree";
pub const CRON_DIR: &str = "cron";
pub const INBOX_DIR: &str = "inbox";
pub const RUNS_DIR: &str = "runs";
pub const MIGRATIONS_DIR: &str = "migrations";
pub const PROCESSED_FILE: &str = "processed.md";
pub const CONFIG_FILE: &str = "config.yml";
pub const GITIGNORE_FILE: &str = ".gitignore";

/// Top-level application config (deserialized from config.yml).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    /// Router machine for `choose: model` invokes that name none (section 3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_router: Option<String>,
    #[serde(default = "default_max_attempts", alias = "max_retries")]
    pub max_attempts: u32,
    #[serde(default = "default_max_depth")]
    pub max_depth: u32,
    #[serde(default = "default_max_log_size")]
    pub max_log_size: u64,
    // `default_machine` and `shared_source` are the section 3 names, read until M4.1.
    #[serde(default = "default_routine", alias = "default_machine")]
    pub default_routine: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "shared_source"
    )]
    pub routine_source: Option<String>,
}

fn default_max_attempts() -> u32 {
    3
}
fn default_max_depth() -> u32 {
    10
}
fn default_max_log_size() -> u64 {
    2_097_152
}
fn default_routine() -> String {
    "develop".to_string()
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            default_router: None,
            max_attempts: default_max_attempts(),
            max_depth: default_max_depth(),
            max_log_size: default_max_log_size(),
            default_routine: default_routine(),
            routine_source: None,
        }
    }
}

impl AppConfig {
    /// Load config from a file path.
    pub fn load(path: &Path) -> Result<Self, DecreeError> {
        let contents = std::fs::read_to_string(path)?;
        let config: AppConfig = serde_norway::from_str(&contents)?;
        Ok(config)
    }

    /// Load config from the project root's `.decree/config.yml`.
    pub fn load_from_project(project_root: &Path) -> Result<Self, DecreeError> {
        let path = project_root.join(DECREE_DIR).join(CONFIG_FILE);
        Self::load(&path)
    }

    /// Return the `.decree/` path for a given project root.
    pub fn decree_dir(project_root: &Path) -> PathBuf {
        project_root.join(DECREE_DIR)
    }

    /// Resolve `routine_source` with tilde expansion.
    pub fn resolved_routine_source(&self) -> Option<PathBuf> {
        self.routine_source.as_ref().map(|s| expand_tilde(s))
    }
}

/// Expand a leading `~` to the user's home directory.
pub fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(rest);
        }
    } else if path == "~" {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home);
        }
    }
    PathBuf::from(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = AppConfig::default();
        assert!(config.default_router.is_none());
        assert_eq!(config.max_attempts, 3);
        assert_eq!(config.max_depth, 10);
        assert_eq!(config.max_log_size, 2_097_152);
        assert_eq!(config.default_routine, "develop");
        assert!(config.routine_source.is_none());
    }

    #[test]
    fn test_deserialize_config() {
        let yaml = r#"
default_router: claude_router
max_attempts: 5
max_depth: 20
max_log_size: 0
default_routine: rust-develop
"#;
        let config: AppConfig = serde_norway::from_str(yaml).unwrap();
        assert_eq!(config.default_router.as_deref(), Some("claude_router"));
        assert_eq!(config.max_attempts, 5);
        assert_eq!(config.max_depth, 20);
        assert_eq!(config.max_log_size, 0);
        assert_eq!(config.default_routine, "rust-develop");
    }

    #[test]
    fn test_deserialize_minimal_config() {
        let yaml = "default_router: claude_router\n";
        let config: AppConfig = serde_norway::from_str(yaml).unwrap();
        assert_eq!(config.max_attempts, 3);
        assert_eq!(config.max_depth, 10);
        assert_eq!(config.default_routine, "develop");
    }

    #[test]
    fn test_deserialize_legacy_max_retries_alias() {
        // Existing on-disk configs use the old `max_retries` key. The serde
        // alias keeps them working after the rename to `max_attempts`.
        let config: AppConfig = serde_norway::from_str("max_retries: 7\n").unwrap();
        assert_eq!(config.max_attempts, 7);
    }

    #[test]
    fn test_expand_tilde() {
        // Can't test with actual HOME since it varies, but test the non-tilde case
        assert_eq!(
            expand_tilde("/absolute/path"),
            PathBuf::from("/absolute/path")
        );
        assert_eq!(
            expand_tilde("relative/path"),
            PathBuf::from("relative/path")
        );
    }

    #[test]
    fn test_expand_tilde_with_home() {
        let home = std::env::var("HOME").unwrap();
        let expanded = expand_tilde("~/.decree/routines");
        assert_eq!(expanded, PathBuf::from(&home).join(".decree/routines"));

        let expanded = expand_tilde("~");
        assert_eq!(expanded, PathBuf::from(&home));
    }

    #[test]
    fn test_resolved_routine_source() {
        let config = AppConfig {
            routine_source: Some("~/.decree/routines".to_string()),
            ..AppConfig::default()
        };
        let home = std::env::var("HOME").unwrap();
        assert_eq!(
            config.resolved_routine_source().unwrap(),
            PathBuf::from(&home).join(".decree/routines")
        );
    }
}
