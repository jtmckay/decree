use std::path::PathBuf;

/// Exit codes following the spec convention.
pub const EXIT_SUCCESS: i32 = 0;
pub const EXIT_FAILURE: i32 = 1;
pub const EXIT_USAGE: i32 = 2;
pub const EXIT_PRECHECK: i32 = 3;

/// All error variants for the decree application.
#[derive(Debug, thiserror::Error)]
pub enum DecreeError {
    #[error("routine not found: {0}")]
    RoutineNotFound(String),

    #[error("max retries exhausted for message {0}")]
    MaxRetriesExhausted(String),

    #[error("max depth exceeded (limit: {0})")]
    MaxDepthExceeded(u32),

    #[error("no migration files found")]
    NoMigrations,

    #[error("message not found: {0}")]
    MessageNotFound(String),

    #[error("pre-check failed: {0}")]
    PreCheckFailed(String),

    #[error("config error: {0}")]
    Config(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("yaml error: {0}")]
    Yaml(#[from] serde_yaml::Error),

    #[error("{0}")]
    Other(String),
}

impl DecreeError {
    /// Map error to the appropriate exit code.
    pub fn exit_code(&self) -> i32 {
        match self {
            DecreeError::PreCheckFailed(_) => EXIT_PRECHECK,
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
        DecreeError::Config(
            "not inside a decree project (run `decree init` first)".to_string(),
        )
    })
}

/// Color output helper module.
pub mod color {
    use colored::Colorize;
    use std::io::IsTerminal;
    use std::sync::Once;

    static INIT: Once = Once::new();

    /// Initialize color settings based on --no-color flag, NO_COLOR env, and TTY detection.
    /// Must be called once at startup.
    pub fn init(no_color_flag: bool) {
        INIT.call_once(|| {
            if no_color_flag {
                colored::control::set_override(false);
            } else if std::env::var("NO_COLOR").is_ok() {
                colored::control::set_override(false);
            } else if !std::io::stdout().is_terminal() {
                colored::control::set_override(false);
            }
            // else: color enabled by default
        });
    }

    pub fn success(s: &str) -> String {
        s.green().to_string()
    }

    pub fn error(s: &str) -> String {
        s.red().to_string()
    }

    pub fn warning(s: &str) -> String {
        s.yellow().to_string()
    }

    pub fn bold(s: &str) -> String {
        s.bold().to_string()
    }

    pub fn dim(s: &str) -> String {
        s.dimmed().to_string()
    }

    pub fn is_tty() -> bool {
        std::io::stdout().is_terminal()
    }
}
