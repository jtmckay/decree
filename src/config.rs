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

/// Top-level application config (deserialized from config.yml, section 3).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppConfig {
    /// Router machine for `choose: model` invokes that name none (section 3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_router: Option<String>,
    #[serde(default = "default_max_attempts")]
    pub max_attempts: u32,
    #[serde(default = "default_max_depth")]
    pub max_depth: u32,
    #[serde(default = "default_max_log_size")]
    pub max_log_size: u64,
    /// Machine for messages with no `machine:` key; unset means they fail validation (M1–M3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_machine: Option<String>,
    /// Directory with shared `machines/` and `scripts/` (section 3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shared_source: Option<String>,
}

/// Top-level keys of a 0.4 `config.yml` that 0.5 removed or renamed (section 3).
const KEYS_0_4: [&str; 7] = [
    "routines",
    "shared_routines",
    "hooks",
    "commands",
    "default_routine",
    "routine_source",
    "max_retries",
];

fn default_max_attempts() -> u32 {
    3
}
fn default_max_depth() -> u32 {
    10
}
fn default_max_log_size() -> u64 {
    2_097_152
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            default_router: None,
            max_attempts: default_max_attempts(),
            max_depth: default_max_depth(),
            max_log_size: default_max_log_size(),
            default_machine: None,
            shared_source: None,
        }
    }
}

impl AppConfig {
    /// Load config from a file path.
    pub fn load(path: &Path) -> Result<Self, DecreeError> {
        let contents = std::fs::read_to_string(path)?;
        Self::parse(&contents)
    }

    /// Parse `config.yml` text. Errors read `config.yml: <serde's message>`, plus a pointer
    /// to the M5.4 script when the text has a 0.4 key.
    pub fn parse(text: &str) -> Result<Self, DecreeError> {
        serde_norway::from_str(text).map_err(|e| {
            let mut msg = format!("{CONFIG_FILE}: {e}");
            if has_0_4_key(text) {
                msg.push_str("; this is a 0.4 config; run scripts/migrate-0.4-to-0.5.sh");
            }
            DecreeError::Other(msg)
        })
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

    /// Resolve `shared_source` with tilde expansion.
    pub fn resolved_shared_source(&self) -> Option<PathBuf> {
        self.shared_source.as_ref().map(|s| expand_tilde(s))
    }
}

/// Whether `text` is a YAML mapping with a top-level key from a 0.4 config.
fn has_0_4_key(text: &str) -> bool {
    match serde_norway::from_str::<serde_norway::Value>(text) {
        Ok(serde_norway::Value::Mapping(map)) => map
            .keys()
            .any(|k| k.as_str().is_some_and(|k| KEYS_0_4.contains(&k))),
        _ => false,
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
        assert!(config.default_machine.is_none());
        assert!(config.shared_source.is_none());
    }

    #[test]
    fn test_deserialize_config() {
        let yaml = r#"
default_router: claude_router
max_attempts: 5
max_depth: 20
max_log_size: 0
default_machine: rust-develop
shared_source: ~/.decree/shared
"#;
        let config: AppConfig = serde_norway::from_str(yaml).unwrap();
        assert_eq!(config.default_router.as_deref(), Some("claude_router"));
        assert_eq!(config.max_attempts, 5);
        assert_eq!(config.max_depth, 20);
        assert_eq!(config.max_log_size, 0);
        assert_eq!(config.default_machine.as_deref(), Some("rust-develop"));
        assert_eq!(config.shared_source.as_deref(), Some("~/.decree/shared"));
    }

    #[test]
    fn test_deserialize_minimal_config() {
        let yaml = "default_router: claude_router\n";
        let config: AppConfig = serde_norway::from_str(yaml).unwrap();
        assert_eq!(config.max_attempts, 3);
        assert_eq!(config.max_depth, 10);
        assert!(config.default_machine.is_none());
    }

    #[test]
    fn test_parse_rejects_unknown_key() {
        let err = AppConfig::parse("max_attempts: 2\nbogus: 1\n")
            .unwrap_err()
            .to_string();
        assert!(err.starts_with("config.yml: "), "{err}");
        assert!(err.contains("bogus"), "{err}");
        assert!(!err.contains("migrate-0.4-to-0.5.sh"), "{err}");
    }

    #[test]
    fn test_parse_rejects_0_4_keys_with_migration_hint() {
        for key in KEYS_0_4 {
            let err = AppConfig::parse(&format!("{key}: 1\n"))
                .unwrap_err()
                .to_string();
            assert!(err.starts_with("config.yml: "), "{err}");
            assert!(err.contains(key), "{err}");
            assert!(
                err.ends_with("this is a 0.4 config; run scripts/migrate-0.4-to-0.5.sh"),
                "{err}"
            );
        }
    }

    #[test]
    fn test_parse_reports_type_errors() {
        let err = AppConfig::parse("max_attempts: many\n")
            .unwrap_err()
            .to_string();
        assert!(err.starts_with("config.yml: max_attempts"), "{err}");
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
    fn test_resolved_shared_source() {
        let config = AppConfig {
            shared_source: Some("~/.decree/shared".to_string()),
            ..AppConfig::default()
        };
        let home = std::env::var("HOME").unwrap();
        assert_eq!(
            config.resolved_shared_source().unwrap(),
            PathBuf::from(&home).join(".decree/shared")
        );
    }
}
