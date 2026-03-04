use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{DecreeError, Result};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub commands: Commands,
    pub max_retries: u32,
    pub max_depth: u32,
    pub max_log_size: u64,
    pub default_routine: String,
    pub hooks: Hooks,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Commands {
    pub ai: String,
    pub interactive_ai: String,
}

impl Default for Commands {
    fn default() -> Self {
        Self {
            ai: "opencode run {prompt}".to_string(),
            interactive_ai: "opencode".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Hooks {
    #[serde(rename = "beforeAll")]
    pub before_all: String,
    #[serde(rename = "afterAll")]
    pub after_all: String,
    #[serde(rename = "beforeEach")]
    pub before_each: String,
    #[serde(rename = "afterEach")]
    pub after_each: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            commands: Commands::default(),
            max_retries: 3,
            max_depth: 10,
            max_log_size: 2_097_152,
            default_routine: "develop".to_string(),
            hooks: Hooks::default(),
        }
    }
}

impl Default for Hooks {
    fn default() -> Self {
        Self {
            before_all: String::new(),
            after_all: String::new(),
            before_each: String::new(),
            after_each: String::new(),
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let contents = std::fs::read_to_string(path).map_err(|e| {
            DecreeError::Config(format!("failed to read {}: {}", path.display(), e))
        })?;
        let config: Config = serde_yaml::from_str(&contents)
            .map_err(|e| DecreeError::Config(format!("failed to parse config: {e}")))?;
        Ok(config)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let contents = serde_yaml::to_string(self)
            .map_err(|e| DecreeError::Config(format!("failed to serialize config: {e}")))?;
        std::fs::write(path, contents)?;
        Ok(())
    }

    pub fn with_ai_command(mut self, tool: &AiTool) -> Self {
        let (ai, interactive) = match tool {
            AiTool::Opencode => ("opencode run {prompt}".to_string(), "opencode".to_string()),
            AiTool::Claude => (
                "claude -p {prompt}".to_string(),
                "claude".to_string(),
            ),
            AiTool::Copilot => ("copilot run {prompt}".to_string(), "copilot".to_string()),
        };
        self.commands.ai = ai;
        self.commands.interactive_ai = interactive;
        self
    }

    pub fn with_git_hooks(mut self) -> Self {
        self.hooks.before_each = "git-baseline".to_string();
        self.hooks.after_each = "git-stash-changes".to_string();
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiTool {
    Opencode,
    Claude,
    Copilot,
}

impl std::fmt::Display for AiTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AiTool::Opencode => write!(f, "opencode"),
            AiTool::Claude => write!(f, "claude"),
            AiTool::Copilot => write!(f, "copilot"),
        }
    }
}

/// The ordered list of AI tools to detect.
pub const AI_TOOLS: &[(AiTool, &str)] = &[
    (AiTool::Opencode, "opencode"),
    (AiTool::Claude, "claude"),
    (AiTool::Copilot, "copilot"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config_values() {
        let config = Config::default();
        assert_eq!(config.max_retries, 3);
        assert_eq!(config.max_depth, 10);
        assert_eq!(config.max_log_size, 2_097_152);
        assert_eq!(config.default_routine, "develop");
        assert!(config.hooks.before_all.is_empty());
        assert!(config.hooks.after_all.is_empty());
        assert!(config.hooks.before_each.is_empty());
        assert!(config.hooks.after_each.is_empty());
    }

    #[test]
    fn test_with_ai_command() {
        let config = Config::default().with_ai_command(&AiTool::Claude);
        assert_eq!(config.commands.ai, "claude -p {prompt}");
        assert_eq!(config.commands.interactive_ai, "claude");
    }

    #[test]
    fn test_with_git_hooks() {
        let config = Config::default().with_git_hooks();
        assert_eq!(config.hooks.before_each, "git-baseline");
        assert_eq!(config.hooks.after_each, "git-stash-changes");
        assert!(config.hooks.before_all.is_empty());
        assert!(config.hooks.after_all.is_empty());
    }

    #[test]
    fn test_config_roundtrip() {
        let config = Config::default().with_ai_command(&AiTool::Claude).with_git_hooks();
        let yaml = serde_yaml::to_string(&config).unwrap();
        let parsed: Config = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed.commands.ai, config.commands.ai);
        assert_eq!(parsed.hooks.before_each, config.hooks.before_each);
    }
}
