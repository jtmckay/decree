use crate::config::{self, AppConfig, HooksConfig};
use crate::routine;
use std::fmt;
use std::path::Path;
use std::process::Command;

/// The four lifecycle hook types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookType {
    BeforeAll,
    AfterAll,
    BeforeEach,
    AfterEach,
}

impl HookType {
    /// The string representation used in the `DECREE_HOOK` env var.
    pub fn as_str(&self) -> &'static str {
        match self {
            HookType::BeforeAll => "beforeAll",
            HookType::AfterAll => "afterAll",
            HookType::BeforeEach => "beforeEach",
            HookType::AfterEach => "afterEach",
        }
    }
}

impl fmt::Display for HookType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Context for hook execution, carrying message-scoped and retry-scoped env vars.
#[derive(Debug, Clone, Default)]
pub struct HookContext {
    /// Path to the message file (empty for beforeAll/afterAll).
    pub message_file: String,
    /// Full message ID (empty for beforeAll/afterAll).
    pub message_id: String,
    /// Run directory for the message (empty for beforeAll/afterAll).
    pub message_dir: String,
    /// Chain ID (empty for beforeAll/afterAll).
    pub chain: String,
    /// Sequence number as string (empty for beforeAll/afterAll).
    pub seq: String,
    /// Current attempt number, 1-indexed (beforeEach/afterEach only).
    pub attempt: Option<u32>,
    /// Configured max retries (beforeEach/afterEach only).
    pub max_retries: Option<u32>,
    /// Exit code of the routine (afterEach only).
    pub routine_exit_code: Option<i32>,
}

/// Captured output from a successful hook execution.
#[derive(Debug, Clone, Default)]
pub struct HookOutput {
    /// Combined stdout and stderr from the hook.
    pub output: String,
}

impl HookOutput {
    /// Returns true if the hook produced no output.
    pub fn is_empty(&self) -> bool {
        self.output.is_empty()
    }
}

/// Result of a failed hook execution.
#[derive(Debug)]
pub struct HookError {
    pub hook_type: HookType,
    pub routine_name: String,
    pub exit_code: i32,
    pub message: String,
    /// Captured output from the hook before it failed.
    pub output: String,
}

impl fmt::Display for HookError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} hook '{}' failed (exit {}): {}",
            self.hook_type, self.routine_name, self.exit_code, self.message
        )
    }
}

/// Resolve the routine name for a given hook type from config.
/// Returns `None` if the hook value is empty or absent.
pub fn hook_routine_name<'a>(hooks: &'a HooksConfig, hook_type: HookType) -> Option<&'a str> {
    let name = match hook_type {
        HookType::BeforeAll => &hooks.before_all,
        HookType::AfterAll => &hooks.after_all,
        HookType::BeforeEach => &hooks.before_each,
        HookType::AfterEach => &hooks.after_each,
    };
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

/// Collect all configured (non-empty) hook routine names.
pub fn configured_hook_names(hooks: &HooksConfig) -> Vec<(&str, HookType)> {
    let types = [
        HookType::BeforeAll,
        HookType::AfterAll,
        HookType::BeforeEach,
        HookType::AfterEach,
    ];
    types
        .into_iter()
        .filter_map(|ht| hook_routine_name(hooks, ht).map(|name| (name, ht)))
        .collect()
}

/// Run a lifecycle hook.
///
/// Returns `Ok(HookOutput)` if the hook ran successfully or was not configured.
/// Returns `Err(HookError)` if the hook script failed.
///
/// Hooks bypass the routine registry — they only need the script to exist on disk.
pub fn run_hook(
    project_root: &Path,
    hooks: &HooksConfig,
    hook_type: HookType,
    ctx: &HookContext,
) -> Result<HookOutput, HookError> {
    run_hook_with_config(project_root, hooks, hook_type, ctx, None)
}

/// Run a lifecycle hook with optional config for layered directory lookup.
pub fn run_hook_with_config(
    project_root: &Path,
    hooks: &HooksConfig,
    hook_type: HookType,
    ctx: &HookContext,
    config: Option<&AppConfig>,
) -> Result<HookOutput, HookError> {
    let routine_name = match hook_routine_name(hooks, hook_type) {
        Some(name) => name,
        None => return Ok(HookOutput::default()),
    };

    // If we have config, use layered lookup (project + shared).
    // Otherwise, fall back to project-local only.
    let script_path = if let Some(cfg) = config {
        routine::find_routine_script_layered(project_root, cfg, routine_name)
    } else {
        let routines_dir = project_root
            .join(config::DECREE_DIR)
            .join(config::ROUTINES_DIR);
        routine::find_routine_script(&routines_dir, routine_name)
    }
    .map_err(|e| HookError {
        hook_type,
        routine_name: routine_name.to_string(),
        exit_code: 1,
        message: e.to_string(),
        output: String::new(),
    })?;

    let mut cmd = Command::new("bash");
    cmd.arg(&script_path)
        .current_dir(project_root)
        .env("DECREE_HOOK", hook_type.as_str())
        .env("message_file", &ctx.message_file)
        .env("message_id", &ctx.message_id)
        .env("message_dir", &ctx.message_dir)
        .env("chain", &ctx.chain)
        .env("seq", &ctx.seq);

    if let Some(attempt) = ctx.attempt {
        cmd.env("DECREE_ATTEMPT", attempt.to_string());
    }
    if let Some(max_retries) = ctx.max_retries {
        cmd.env("DECREE_MAX_RETRIES", max_retries.to_string());
    }
    if let Some(exit_code) = ctx.routine_exit_code {
        cmd.env("DECREE_ROUTINE_EXIT_CODE", exit_code.to_string());
    }

    let cmd_output = cmd.output().map_err(|e| HookError {
        hook_type,
        routine_name: routine_name.to_string(),
        exit_code: 1,
        message: format!("failed to execute hook: {e}"),
        output: String::new(),
    })?;

    // Combine stdout and stderr into a single output string
    let stdout = String::from_utf8_lossy(&cmd_output.stdout);
    let stderr = String::from_utf8_lossy(&cmd_output.stderr);
    let combined = format!("{}{}", stdout, stderr);
    let combined = combined.trim().to_string();

    if cmd_output.status.success() {
        Ok(HookOutput { output: combined })
    } else {
        let exit_code = cmd_output.status.code().unwrap_or(1);
        Err(HookError {
            hook_type,
            routine_name: routine_name.to_string(),
            exit_code,
            message: if stderr.trim().is_empty() {
                format!("hook exited with code {exit_code}")
            } else {
                stderr.trim().to_string()
            },
            output: combined,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::HooksConfig;

    #[test]
    fn test_hook_type_as_str() {
        assert_eq!(HookType::BeforeAll.as_str(), "beforeAll");
        assert_eq!(HookType::AfterAll.as_str(), "afterAll");
        assert_eq!(HookType::BeforeEach.as_str(), "beforeEach");
        assert_eq!(HookType::AfterEach.as_str(), "afterEach");
    }

    #[test]
    fn test_hook_type_display() {
        assert_eq!(format!("{}", HookType::BeforeAll), "beforeAll");
        assert_eq!(format!("{}", HookType::AfterEach), "afterEach");
    }

    #[test]
    fn test_hook_routine_name_empty() {
        let hooks = HooksConfig::default();
        assert_eq!(hook_routine_name(&hooks, HookType::BeforeAll), None);
        assert_eq!(hook_routine_name(&hooks, HookType::AfterAll), None);
        assert_eq!(hook_routine_name(&hooks, HookType::BeforeEach), None);
        assert_eq!(hook_routine_name(&hooks, HookType::AfterEach), None);
    }

    #[test]
    fn test_hook_routine_name_configured() {
        let hooks = HooksConfig {
            before_all: "setup".to_string(),
            after_all: "".to_string(),
            before_each: "git-baseline".to_string(),
            after_each: "git-stash-changes".to_string(),
        };
        assert_eq!(
            hook_routine_name(&hooks, HookType::BeforeAll),
            Some("setup")
        );
        assert_eq!(hook_routine_name(&hooks, HookType::AfterAll), None);
        assert_eq!(
            hook_routine_name(&hooks, HookType::BeforeEach),
            Some("git-baseline")
        );
        assert_eq!(
            hook_routine_name(&hooks, HookType::AfterEach),
            Some("git-stash-changes")
        );
    }

    #[test]
    fn test_configured_hook_names_empty() {
        let hooks = HooksConfig::default();
        assert!(configured_hook_names(&hooks).is_empty());
    }

    #[test]
    fn test_configured_hook_names_partial() {
        let hooks = HooksConfig {
            before_all: "".to_string(),
            after_all: "".to_string(),
            before_each: "pre-flight".to_string(),
            after_each: "post-flight".to_string(),
        };
        let names = configured_hook_names(&hooks);
        assert_eq!(names.len(), 2);
        assert_eq!(names[0], ("pre-flight", HookType::BeforeEach));
        assert_eq!(names[1], ("post-flight", HookType::AfterEach));
    }

    #[test]
    fn test_run_hook_no_hook_configured() {
        let hooks = HooksConfig::default();
        let ctx = HookContext::default();
        // Should return Ok since no hook is configured
        assert!(run_hook(Path::new("/nonexistent"), &hooks, HookType::BeforeAll, &ctx).is_ok());
    }

    #[test]
    fn test_run_hook_missing_script() {
        let dir = tempfile::TempDir::new().unwrap();
        let routines_dir = dir
            .path()
            .join(config::DECREE_DIR)
            .join(config::ROUTINES_DIR);
        std::fs::create_dir_all(&routines_dir).unwrap();

        let hooks = HooksConfig {
            before_all: "nonexistent".to_string(),
            ..HooksConfig::default()
        };
        let ctx = HookContext::default();
        let err = run_hook(dir.path(), &hooks, HookType::BeforeAll, &ctx).unwrap_err();
        assert_eq!(err.hook_type, HookType::BeforeAll);
        assert_eq!(err.routine_name, "nonexistent");
        assert!(err.message.contains("routine not found"));
    }

    #[test]
    fn test_run_hook_success() {
        let dir = tempfile::TempDir::new().unwrap();
        let routines_dir = dir
            .path()
            .join(config::DECREE_DIR)
            .join(config::ROUTINES_DIR);
        std::fs::create_dir_all(&routines_dir).unwrap();

        std::fs::write(
            routines_dir.join("setup.sh"),
            "#!/usr/bin/env bash\nexit 0\n",
        )
        .unwrap();

        let hooks = HooksConfig {
            before_all: "setup".to_string(),
            ..HooksConfig::default()
        };
        let ctx = HookContext::default();
        assert!(run_hook(dir.path(), &hooks, HookType::BeforeAll, &ctx).is_ok());
    }

    #[test]
    fn test_run_hook_failure() {
        let dir = tempfile::TempDir::new().unwrap();
        let routines_dir = dir
            .path()
            .join(config::DECREE_DIR)
            .join(config::ROUTINES_DIR);
        std::fs::create_dir_all(&routines_dir).unwrap();

        std::fs::write(
            routines_dir.join("bad.sh"),
            "#!/usr/bin/env bash\necho 'something broke' >&2\nexit 42\n",
        )
        .unwrap();

        let hooks = HooksConfig {
            before_all: "bad".to_string(),
            ..HooksConfig::default()
        };
        let ctx = HookContext::default();
        let err = run_hook(dir.path(), &hooks, HookType::BeforeAll, &ctx).unwrap_err();
        assert_eq!(err.exit_code, 42);
        assert!(err.message.contains("something broke"));
    }

    #[test]
    fn test_run_hook_sets_env_vars() {
        let dir = tempfile::TempDir::new().unwrap();
        let routines_dir = dir
            .path()
            .join(config::DECREE_DIR)
            .join(config::ROUTINES_DIR);
        std::fs::create_dir_all(&routines_dir).unwrap();

        // Script that checks env vars are set and fails if any are missing
        std::fs::write(
            routines_dir.join("check-env.sh"),
            r#"#!/usr/bin/env bash
[ "$DECREE_HOOK" = "beforeEach" ] || { echo "DECREE_HOOK wrong: $DECREE_HOOK" >&2; exit 1; }
[ "$DECREE_ATTEMPT" = "2" ] || { echo "DECREE_ATTEMPT wrong: $DECREE_ATTEMPT" >&2; exit 1; }
[ "$DECREE_MAX_RETRIES" = "3" ] || { echo "DECREE_MAX_RETRIES wrong: $DECREE_MAX_RETRIES" >&2; exit 1; }
[ "$message_id" = "test-msg-0" ] || { echo "message_id wrong: $message_id" >&2; exit 1; }
[ "$chain" = "test-msg" ] || { echo "chain wrong: $chain" >&2; exit 1; }
[ "$seq" = "0" ] || { echo "seq wrong: $seq" >&2; exit 1; }
exit 0
"#,
        )
        .unwrap();

        let hooks = HooksConfig {
            before_each: "check-env".to_string(),
            ..HooksConfig::default()
        };
        let ctx = HookContext {
            message_id: "test-msg-0".to_string(),
            chain: "test-msg".to_string(),
            seq: "0".to_string(),
            attempt: Some(2),
            max_retries: Some(3),
            ..HookContext::default()
        };
        let result = run_hook(dir.path(), &hooks, HookType::BeforeEach, &ctx);
        if let Err(ref e) = result {
            panic!("hook failed: {}", e.message);
        }
        assert!(result.is_ok());
    }

    #[test]
    fn test_run_hook_after_each_exit_code_env() {
        let dir = tempfile::TempDir::new().unwrap();
        let routines_dir = dir
            .path()
            .join(config::DECREE_DIR)
            .join(config::ROUTINES_DIR);
        std::fs::create_dir_all(&routines_dir).unwrap();

        std::fs::write(
            routines_dir.join("check-exit.sh"),
            r#"#!/usr/bin/env bash
[ "$DECREE_ROUTINE_EXIT_CODE" = "1" ] || { echo "exit code wrong: $DECREE_ROUTINE_EXIT_CODE" >&2; exit 1; }
exit 0
"#,
        )
        .unwrap();

        let hooks = HooksConfig {
            after_each: "check-exit".to_string(),
            ..HooksConfig::default()
        };
        let ctx = HookContext {
            attempt: Some(1),
            max_retries: Some(3),
            routine_exit_code: Some(1),
            ..HookContext::default()
        };
        assert!(run_hook(dir.path(), &hooks, HookType::AfterEach, &ctx).is_ok());
    }

    #[test]
    fn test_hook_context_default() {
        let ctx = HookContext::default();
        assert_eq!(ctx.message_file, "");
        assert_eq!(ctx.message_id, "");
        assert_eq!(ctx.message_dir, "");
        assert_eq!(ctx.chain, "");
        assert_eq!(ctx.seq, "");
        assert_eq!(ctx.attempt, None);
        assert_eq!(ctx.max_retries, None);
        assert_eq!(ctx.routine_exit_code, None);
    }

    #[test]
    fn test_run_hook_captures_stdout() {
        let dir = tempfile::TempDir::new().unwrap();
        let routines_dir = dir
            .path()
            .join(config::DECREE_DIR)
            .join(config::ROUTINES_DIR);
        std::fs::create_dir_all(&routines_dir).unwrap();

        std::fs::write(
            routines_dir.join("echo-hook.sh"),
            "#!/usr/bin/env bash\necho 'BASELINE SAVED'\n",
        )
        .unwrap();

        let hooks = HooksConfig {
            before_each: "echo-hook".to_string(),
            ..HooksConfig::default()
        };
        let ctx = HookContext::default();
        let result = run_hook(dir.path(), &hooks, HookType::BeforeEach, &ctx).unwrap();
        assert!(result.output.contains("BASELINE SAVED"));
    }

    #[test]
    fn test_run_hook_captures_stderr() {
        let dir = tempfile::TempDir::new().unwrap();
        let routines_dir = dir
            .path()
            .join(config::DECREE_DIR)
            .join(config::ROUTINES_DIR);
        std::fs::create_dir_all(&routines_dir).unwrap();

        std::fs::write(
            routines_dir.join("stderr-hook.sh"),
            "#!/usr/bin/env bash\necho 'stderr output' >&2\n",
        )
        .unwrap();

        let hooks = HooksConfig {
            after_each: "stderr-hook".to_string(),
            ..HooksConfig::default()
        };
        let ctx = HookContext::default();
        let result = run_hook(dir.path(), &hooks, HookType::AfterEach, &ctx).unwrap();
        assert!(result.output.contains("stderr output"));
    }

    #[test]
    fn test_run_hook_no_output_returns_empty() {
        let dir = tempfile::TempDir::new().unwrap();
        let routines_dir = dir
            .path()
            .join(config::DECREE_DIR)
            .join(config::ROUTINES_DIR);
        std::fs::create_dir_all(&routines_dir).unwrap();

        std::fs::write(
            routines_dir.join("silent.sh"),
            "#!/usr/bin/env bash\nexit 0\n",
        )
        .unwrap();

        let hooks = HooksConfig {
            before_each: "silent".to_string(),
            ..HooksConfig::default()
        };
        let ctx = HookContext::default();
        let result = run_hook(dir.path(), &hooks, HookType::BeforeEach, &ctx).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_run_hook_failure_includes_output() {
        let dir = tempfile::TempDir::new().unwrap();
        let routines_dir = dir
            .path()
            .join(config::DECREE_DIR)
            .join(config::ROUTINES_DIR);
        std::fs::create_dir_all(&routines_dir).unwrap();

        std::fs::write(
            routines_dir.join("fail-verbose.sh"),
            "#!/usr/bin/env bash\necho 'partial work done'\necho 'error details' >&2\nexit 1\n",
        )
        .unwrap();

        let hooks = HooksConfig {
            before_each: "fail-verbose".to_string(),
            ..HooksConfig::default()
        };
        let ctx = HookContext::default();
        let err = run_hook(dir.path(), &hooks, HookType::BeforeEach, &ctx).unwrap_err();
        assert!(err.output.contains("partial work done"));
        assert!(err.output.contains("error details"));
    }

    #[test]
    fn test_hook_error_display() {
        let err = HookError {
            hook_type: HookType::BeforeAll,
            routine_name: "setup".to_string(),
            exit_code: 1,
            message: "setup failed".to_string(),
            output: String::new(),
        };
        let s = format!("{err}");
        assert!(s.contains("beforeAll"));
        assert!(s.contains("setup"));
        assert!(s.contains("exit 1"));
        assert!(s.contains("setup failed"));
    }
}
