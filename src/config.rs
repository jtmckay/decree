use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::error::DecreeError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub ai: AiConfig,
    pub commands: CommandsConfig,
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    #[serde(default = "default_max_depth")]
    pub max_depth: u32,
    #[serde(default = "default_routine")]
    pub default_routine: String,
    #[serde(default)]
    pub notebook_support: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiConfig {
    #[serde(default = "default_model_path")]
    pub model_path: String,
    #[serde(default)]
    pub n_gpu_layers: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandsConfig {
    pub planning: String,
    pub planning_continue: String,
    pub router: String,
}

fn default_max_retries() -> u32 {
    3
}
fn default_max_depth() -> u32 {
    10
}
fn default_routine() -> String {
    "develop".into()
}
fn default_model_path() -> String {
    "~/.decree/models/qwen2.5-1.5b-instruct-q5_k_m.gguf".into()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            ai: AiConfig {
                model_path: default_model_path(),
                n_gpu_layers: 0,
            },
            commands: CommandsConfig {
                planning: "claude -p {prompt}".into(),
                planning_continue: "claude --continue".into(),
                router: "decree ai".into(),
            },
            max_retries: default_max_retries(),
            max_depth: default_max_depth(),
            default_routine: default_routine(),
            notebook_support: false,
        }
    }
}

/// AI provider choices for interactive selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiProvider {
    ClaudeCli,
    CopilotCli,
    Embedded,
}

impl std::fmt::Display for AiProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AiProvider::ClaudeCli => write!(f, "Claude CLI"),
            AiProvider::CopilotCli => write!(f, "GitHub Copilot CLI"),
            AiProvider::Embedded => write!(f, "Embedded (decree ai)"),
        }
    }
}

impl AiProvider {
    pub fn planning_command(&self) -> &str {
        match self {
            AiProvider::ClaudeCli => "claude -p {prompt}",
            AiProvider::CopilotCli => "copilot -p {prompt}",
            AiProvider::Embedded => "decree ai",
        }
    }

    pub fn planning_continue_command(&self) -> &str {
        match self {
            AiProvider::ClaudeCli => "claude --continue",
            AiProvider::CopilotCli => "copilot --continue",
            AiProvider::Embedded => "",
        }
    }

    pub fn router_command(&self) -> &str {
        match self {
            AiProvider::Embedded => "decree ai",
            AiProvider::ClaudeCli => "claude -p {prompt}",
            AiProvider::CopilotCli => "copilot -p {prompt}",
        }
    }
}

impl Config {
    /// Load config from `.decree/config.yml` relative to the project root.
    pub fn load(project_root: &Path) -> Result<Self, DecreeError> {
        let path = project_root.join(".decree/config.yml");
        let content = std::fs::read_to_string(&path).map_err(|e| {
            DecreeError::Config(format!("failed to read {}: {}", path.display(), e))
        })?;
        let config: Config = serde_yaml::from_str(&content)?;
        Ok(config)
    }

    /// Write config to `.decree/config.yml`.
    pub fn save(&self, project_root: &Path) -> Result<(), DecreeError> {
        let path = project_root.join(".decree/config.yml");
        let content = serde_yaml::to_string(self)?;
        std::fs::write(&path, content)?;
        Ok(())
    }

    /// Expand `~` in `model_path` to the user's home directory.
    pub fn resolved_model_path(&self) -> PathBuf {
        let p = &self.ai.model_path;
        if let Some(rest) = p.strip_prefix("~/") {
            if let Some(home) = dirs::home_dir() {
                return home.join(rest);
            }
        }
        PathBuf::from(p)
    }
}
