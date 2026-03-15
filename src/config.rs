use crate::error::DecreeError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
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

/// A routine entry in the registry (routines/shared_routines sections).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutineEntry {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub deprecated: bool,
}

fn default_true() -> bool {
    true
}

fn is_false(v: &bool) -> bool {
    !v
}

impl RoutineEntry {
    /// Create a new entry with the given enabled state.
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            deprecated: false,
        }
    }

    /// A routine is active only if enabled AND not deprecated.
    pub fn is_active(&self) -> bool {
        self.enabled && !self.deprecated
    }
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub routine_source: Option<String>,
    #[serde(default)]
    pub hooks: HooksConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub routines: Option<BTreeMap<String, RoutineEntry>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shared_routines: Option<BTreeMap<String, RoutineEntry>>,
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
            routine_source: None,
            hooks: HooksConfig::default(),
            routines: None,
            shared_routines: None,
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

    /// Resolve `routine_source` with tilde expansion.
    pub fn resolved_routine_source(&self) -> Option<PathBuf> {
        self.routine_source.as_ref().map(|s| expand_tilde(s))
    }

    /// Derive the shared prompts directory from `routine_source`.
    ///
    /// If `routine_source` is `~/.decree/routines`, this returns `~/.decree/prompts`.
    pub fn resolved_shared_prompts_dir(&self) -> Option<PathBuf> {
        self.resolved_routine_source()
            .and_then(|p| p.parent().map(|parent| parent.join(PROMPTS_DIR)))
    }

    /// Save config to the project's `.decree/config.yml`.
    pub fn save(&self, project_root: &Path) -> Result<(), DecreeError> {
        let path = project_root.join(DECREE_DIR).join(CONFIG_FILE);
        let yaml = serde_yaml::to_string(self)?;
        std::fs::write(&path, yaml)?;
        Ok(())
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
        assert_eq!(config.commands.ai_router, "opencode run {prompt}");
        assert_eq!(config.commands.ai_interactive, "opencode");
        assert_eq!(config.max_retries, 3);
        assert_eq!(config.max_depth, 10);
        assert_eq!(config.max_log_size, 2_097_152);
        assert_eq!(config.default_routine, "develop");
        assert!(config.routine_source.is_none());
        assert!(config.routines.is_none());
        assert!(config.shared_routines.is_none());
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
        assert!(config.routines.is_none());
    }

    #[test]
    fn test_deserialize_config_with_routines() {
        let yaml = r#"
commands:
  ai_router: "claude -p {prompt}"
  ai_interactive: "claude"
routine_source: "~/.decree/routines"
routines:
  develop:
    enabled: true
  rust-develop:
    enabled: true
  old-routine:
    enabled: true
    deprecated: true
shared_routines:
  deploy:
    enabled: true
  notify:
    enabled: false
"#;
        let config: AppConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(
            config.routine_source.as_deref(),
            Some("~/.decree/routines")
        );

        let routines = config.routines.as_ref().unwrap();
        assert_eq!(routines.len(), 3);
        assert!(routines["develop"].is_active());
        assert!(routines["rust-develop"].is_active());
        assert!(!routines["old-routine"].is_active()); // deprecated

        let shared = config.shared_routines.as_ref().unwrap();
        assert_eq!(shared.len(), 2);
        assert!(shared["deploy"].is_active());
        assert!(!shared["notify"].is_active()); // disabled
    }

    #[test]
    fn test_routine_entry_defaults() {
        // enabled defaults to true, deprecated to false
        let yaml = "{}";
        let entry: RoutineEntry = serde_yaml::from_str(yaml).unwrap();
        assert!(entry.enabled);
        assert!(!entry.deprecated);
        assert!(entry.is_active());
    }

    #[test]
    fn test_routine_entry_deprecated_overrides_enabled() {
        let entry = RoutineEntry {
            enabled: true,
            deprecated: true,
        };
        assert!(!entry.is_active());
    }

    #[test]
    fn test_expand_tilde() {
        // Can't test with actual HOME since it varies, but test the non-tilde case
        assert_eq!(expand_tilde("/absolute/path"), PathBuf::from("/absolute/path"));
        assert_eq!(expand_tilde("relative/path"), PathBuf::from("relative/path"));
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

    #[test]
    fn test_resolved_shared_prompts_dir() {
        let config = AppConfig {
            routine_source: Some("~/.decree/routines".to_string()),
            ..AppConfig::default()
        };
        let home = std::env::var("HOME").unwrap();
        assert_eq!(
            config.resolved_shared_prompts_dir().unwrap(),
            PathBuf::from(&home).join(".decree/prompts")
        );
    }

    #[test]
    fn test_routine_entry_serialization_skips_deprecated_false() {
        let entry = RoutineEntry::new(true);
        let yaml = serde_yaml::to_string(&entry).unwrap();
        assert!(yaml.contains("enabled: true"));
        assert!(!yaml.contains("deprecated"));
    }
}
