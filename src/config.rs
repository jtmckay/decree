use crate::error::DecreeError;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Directory and file constants.
pub const DECREE_DIR: &str = ".decree";
pub const ROUTINES_DIR: &str = "routines";
pub const PROMPTS_DIR: &str = "prompts";
pub const CRON_DIR: &str = "cron";
pub const INBOX_DIR: &str = "inbox";
pub const OUTBOX_DIR: &str = "outbox";
pub const RUNS_DIR: &str = "runs";
pub const MIGRATIONS_DIR: &str = "migrations";
pub const DEAD_DIR: &str = "dead";
pub const PROCESSED_FILE: &str = "processed.md";
pub const ROUTER_FILE: &str = "router.md";
pub const CONFIG_FILE: &str = "config.yml";
pub const GITIGNORE_FILE: &str = ".gitignore";

/// Commands configuration — AI tool settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandsConfig {
    pub ai_router: String,
    pub ai_interactive: String,
}

impl Default for CommandsConfig {
    fn default() -> Self {
        Self {
            ai_router: "opencode run {prompt}".to_string(),
            ai_interactive: "opencode".to_string(),
        }
    }
}

/// Lifecycle hooks configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HooksConfig {
    #[serde(default, rename = "beforeAll")]
    pub before_all: String,
    #[serde(default, rename = "afterAll")]
    pub after_all: String,
    #[serde(default, rename = "beforeEach")]
    pub before_each: String,
    #[serde(default, rename = "afterEach")]
    pub after_each: String,
}

/// Top-level application config (deserialized from config.yml).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub commands: CommandsConfig,
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    #[serde(default = "default_max_depth")]
    pub max_depth: u32,
    #[serde(default = "default_max_log_size")]
    pub max_log_size: u64,
    #[serde(default = "default_routine")]
    pub default_routine: String,
    #[serde(default)]
    pub hooks: HooksConfig,
}

fn default_max_retries() -> u32 {
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
            commands: CommandsConfig::default(),
            max_retries: default_max_retries(),
            max_depth: default_max_depth(),
            max_log_size: default_max_log_size(),
            default_routine: default_routine(),
            hooks: HooksConfig::default(),
        }
    }
}

impl AppConfig {
    /// Load config from a file path.
    pub fn load(path: &Path) -> Result<Self, DecreeError> {
        let contents = std::fs::read_to_string(path)?;
        let config: AppConfig = serde_yaml::from_str(&contents)?;
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = AppConfig::default();
        assert_eq!(config.commands.ai_router, "opencode run {prompt}");
        assert_eq!(config.commands.ai_interactive, "opencode");
        assert_eq!(config.max_retries, 3);
        assert_eq!(config.max_depth, 10);
        assert_eq!(config.max_log_size, 2_097_152);
        assert_eq!(config.default_routine, "develop");
    }

    #[test]
    fn test_deserialize_config() {
        let yaml = r#"
commands:
  ai_router: "claude -p {prompt}"
  ai_interactive: "claude"
max_retries: 5
max_depth: 20
max_log_size: 0
default_routine: rust-develop
hooks:
  beforeAll: ""
  afterAll: ""
  beforeEach: "git-baseline"
  afterEach: "git-stash-changes"
"#;
        let config: AppConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.commands.ai_router, "claude -p {prompt}");
        assert_eq!(config.commands.ai_interactive, "claude");
        assert_eq!(config.max_retries, 5);
        assert_eq!(config.max_depth, 20);
        assert_eq!(config.max_log_size, 0);
        assert_eq!(config.default_routine, "rust-develop");
        assert_eq!(config.hooks.before_each, "git-baseline");
        assert_eq!(config.hooks.after_each, "git-stash-changes");
    }

    #[test]
    fn test_deserialize_minimal_config() {
        let yaml = r#"
commands:
  ai_router: "opencode run {prompt}"
  ai_interactive: "opencode"
"#;
        let config: AppConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.max_retries, 3);
        assert_eq!(config.max_depth, 10);
        assert_eq!(config.default_routine, "develop");
    }
}
