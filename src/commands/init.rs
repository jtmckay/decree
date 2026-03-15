use crate::config;
use crate::error::color::is_tty;
use crate::error::DecreeError;
use std::io::Write;
use std::path::Path;
use std::process::Command;

/// AI backends we search for, in priority order.
const AI_BACKENDS: &[(&str, &str, &str)] = &[
    // (command, ai_router template, ai_interactive template)
    ("opencode", "opencode run {prompt}", "opencode"),
    ("claude", "claude -p {prompt}", "claude"),
    ("copilot", "copilot -p {prompt}", "copilot"),
];

/// Git stash hook routine: git-baseline.sh (beforeEach hook)
const GIT_BASELINE_SH: &str = include_str!("../templates/git-baseline.sh");

/// Git stash hook routine: git-stash-changes.sh (afterEach hook)
const GIT_STASH_CHANGES_SH: &str = include_str!("../templates/git-stash-changes.sh");

// Templates embedded from src/templates/ at compile time.
const DEVELOP_SH: &str = include_str!("../templates/develop.sh");
const RUST_DEVELOP_SH: &str = include_str!("../templates/rust-develop.sh");
const ROUTER_MD: &str = include_str!("../templates/router.md");
const SOW_PROMPT_MD: &str = include_str!("../templates/sow.md");
const MIGRATION_PROMPT_MD: &str = include_str!("../templates/migration.md");
const ROUTINE_PROMPT_MD: &str = include_str!("../templates/routine.md");
const DECREE_GITIGNORE: &str = include_str!("../templates/gitignore");

/// Check if a command exists on PATH.
fn command_exists(name: &str) -> bool {
    Command::new("which")
        .arg(name)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Check if we're inside a git repository.
fn is_git_repo() -> bool {
    Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Detect available AI backends. Returns list of (command, router_template, interactive_template).
fn detect_ai_backends() -> Vec<(&'static str, &'static str, &'static str)> {
    AI_BACKENDS
        .iter()
        .copied()
        .filter(|(cmd, _, _)| command_exists(cmd))
        .collect()
}

/// Derive the AI invocation prefix from an ai_router template by stripping {prompt}.
fn ai_invoke_prefix(ai_router: &str) -> String {
    ai_router
        .replace(" {prompt}", "")
        .replace("{prompt}", "")
        .trim()
        .to_string()
}

/// Detect shared routines in `~/.decree/routines/`.
fn detect_shared_routines() -> Vec<String> {
    let shared_dir = config::expand_tilde("~/.decree/routines");
    if !shared_dir.is_dir() {
        return Vec::new();
    }

    let mut names = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&shared_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && path.extension().is_some_and(|ext| ext == "sh") {
                if let Some(stem) = path.file_stem() {
                    names.push(stem.to_string_lossy().to_string());
                }
            }
        }
    }
    names.sort();
    names
}

/// Replace AI placeholders in a routine template.
fn replace_ai_placeholders(template: &str, ai_name: &str, ai_router: &str) -> String {
    let invoke = ai_invoke_prefix(ai_router);
    template
        .replace("{ai_name}", ai_name)
        .replace("{ai_invoke}", &invoke)
}

/// Generate config.yml content with the selected AI command.
fn generate_config(
    ai_name: &str,
    ai_router: &str,
    ai_interactive: &str,
    git_hooks: bool,
    routine_names: &[&str],
    shared_routine_names: &[String],
) -> String {
    let mut config = String::new();

    config.push_str("commands:\n");
    config.push_str(&format!("  ai_router: \"{ai_router}\"\n"));
    config.push_str(&format!("  ai_interactive: \"{ai_interactive}\"\n"));

    // Add commented alternatives
    for &(name, router, interactive) in AI_BACKENDS {
        if name != ai_name {
            config.push_str(&format!("  # ai_router: \"{router}\"\n"));
            config.push_str(&format!("  # ai_interactive: \"{interactive}\"\n"));
        }
    }

    config.push('\n');
    config.push_str("max_retries: 3\n");
    config.push_str("max_depth: 10\n");
    config.push_str("max_log_size: 2097152 # Per-log size cap in bytes (2MB), 0 to disable\n");
    config.push_str("default_routine: develop\n");
    config.push_str("routine_source: \"~/.decree/routines\" # optional, shared routines directory\n");
    config.push('\n');

    config.push_str("hooks:\n");
    config.push_str("  beforeAll: \"\"\n");
    config.push_str("  afterAll: \"\"\n");

    if git_hooks {
        config.push_str("  beforeEach: \"git-baseline\"\n");
        config.push_str("  afterEach: \"git-stash-changes\"\n");
    } else {
        config.push_str("  beforeEach: \"\"\n");
        config.push_str("  afterEach: \"\"\n");
    }

    config.push_str("  # --- Git stash workflow (uncomment to enable) ---\n");
    config.push_str("  # beforeEach: \"git-baseline\"\n");
    config.push_str("  # afterEach: \"git-stash-changes\"\n");

    // Routine registry
    if !routine_names.is_empty() {
        config.push('\n');
        config.push_str("routines:\n");
        let mut sorted: Vec<&str> = routine_names.to_vec();
        sorted.sort();
        for name in sorted {
            config.push_str(&format!("  {name}:\n    enabled: true\n"));
        }
    }

    // Shared routine registry
    if !shared_routine_names.is_empty() {
        config.push('\n');
        config.push_str("shared_routines:\n");
        let mut sorted = shared_routine_names.to_vec();
        sorted.sort();
        for name in sorted {
            config.push_str(&format!("  {name}:\n    enabled: false\n"));
        }
    }

    config
}

/// Run `decree init`.
pub fn run() -> Result<(), DecreeError> {
    let decree_dir = Path::new(config::DECREE_DIR);

    // Re-run check
    if decree_dir.exists() {
        if is_tty() {
            eprint!("Decree is already configured in this directory.\nOverwrite existing configuration? [y/N] ");
            std::io::stderr().flush()?;
            let mut input = String::new();
            std::io::stdin().read_line(&mut input)?;
            if !input.trim().eq_ignore_ascii_case("y") {
                println!("Aborted.");
                return Ok(());
            }
        } else {
            // Non-TTY: error if unresolvable (spec says auto-detect AI, accept hooks)
            // We proceed with overwrite in non-TTY mode since there's no way to ask
            eprintln!("Decree is already configured in this directory. Overwriting in non-TTY mode.");
        }
    }

    // 1. Detect AI backend
    let available = detect_ai_backends();
    let (ai_name, ai_router, ai_interactive) = if available.is_empty() {
        println!("No AI backend detected (opencode, claude, copilot).");
        println!("Visit https://opencode.ai/ to install opencode.");
        println!("Defaulting to opencode.");
        AI_BACKENDS[0] // opencode defaults
    } else if available.len() == 1 {
        println!("Detected AI backend: {}", available[0].0);
        available[0]
    } else if is_tty() {
        // Multiple backends found — present selector
        let options: Vec<String> = available.iter().map(|(cmd, _, _)| cmd.to_string()).collect();
        let selection = inquire::Select::new("Select AI backend:", options)
            .prompt()
            .map_err(|e| DecreeError::Other(format!("selection cancelled: {e}")))?;
        *available
            .iter()
            .find(|(cmd, _, _)| *cmd == selection.as_str())
            .expect("selection came from available list")
    } else {
        // Non-TTY with multiple: pick first
        println!("Multiple AI backends detected, using: {}", available[0].0);
        available[0]
    };

    // 2. Detect git and ask about lifecycle hooks
    let has_git = command_exists("git") && is_git_repo();
    let git_hooks = if has_git {
        if is_tty() {
            eprint!("Enable git stash hooks for change tracking? [Y/n] ");
            std::io::stderr().flush()?;
            let mut input = String::new();
            std::io::stdin().read_line(&mut input)?;
            let trimmed = input.trim();
            trimmed.is_empty() || trimmed.eq_ignore_ascii_case("y")
        } else {
            // Non-TTY: accept git hooks if detected
            true
        }
    } else {
        if !command_exists("git") || !is_git_repo() {
            println!("git not found — skipping lifecycle hook setup");
        }
        false
    };

    // 3. Create directory structure
    let dirs = [
        config::DECREE_DIR,
        &format!("{}/{}", config::DECREE_DIR, config::ROUTINES_DIR),
        &format!("{}/{}", config::DECREE_DIR, config::PROMPTS_DIR),
        &format!("{}/{}", config::DECREE_DIR, config::CRON_DIR),
        &format!("{}/{}", config::DECREE_DIR, config::INBOX_DIR),
        &format!("{}/{}/{}", config::DECREE_DIR, config::INBOX_DIR, config::DEAD_DIR),
        &format!("{}/{}", config::DECREE_DIR, config::OUTBOX_DIR),
        &format!("{}/{}/{}", config::DECREE_DIR, config::OUTBOX_DIR, config::DEAD_DIR),
        &format!("{}/{}", config::DECREE_DIR, config::RUNS_DIR),
        &format!("{}/{}", config::DECREE_DIR, config::MIGRATIONS_DIR),
    ];
    for dir in &dirs {
        std::fs::create_dir_all(dir)?;
    }

    // 4. Write config.yml
    let mut routine_names: Vec<&str> = vec!["develop", "rust-develop"];
    if git_hooks {
        routine_names.push("git-baseline");
        routine_names.push("git-stash-changes");
    }

    // Check for shared routines at ~/.decree/routines/
    let shared_routine_names = detect_shared_routines();

    let config_content = generate_config(
        ai_name,
        ai_router,
        ai_interactive,
        git_hooks,
        &routine_names,
        &shared_routine_names,
    );
    std::fs::write(
        format!("{}/{}", config::DECREE_DIR, config::CONFIG_FILE),
        &config_content,
    )?;

    // 5. Write .gitignore
    std::fs::write(
        format!("{}/{}", config::DECREE_DIR, config::GITIGNORE_FILE),
        DECREE_GITIGNORE,
    )?;

    // 6. Write router.md
    std::fs::write(
        format!("{}/{}", config::DECREE_DIR, config::ROUTER_FILE),
        ROUTER_MD,
    )?;

    // 7. Write prompt templates
    let prompts_base = format!("{}/{}", config::DECREE_DIR, config::PROMPTS_DIR);
    std::fs::write(format!("{prompts_base}/migration.md"), MIGRATION_PROMPT_MD)?;
    std::fs::write(format!("{prompts_base}/sow.md"), SOW_PROMPT_MD)?;
    std::fs::write(format!("{prompts_base}/routine.md"), ROUTINE_PROMPT_MD)?;

    // 8. Write routine templates (replace {ai_name}/{ai_invoke} with detected backend)
    let routines_base = format!("{}/{}", config::DECREE_DIR, config::ROUTINES_DIR);
    std::fs::write(
        format!("{routines_base}/develop.sh"),
        replace_ai_placeholders(DEVELOP_SH, ai_name, ai_router),
    )?;
    std::fs::write(
        format!("{routines_base}/rust-develop.sh"),
        replace_ai_placeholders(RUST_DEVELOP_SH, ai_name, ai_router),
    )?;

    // 9. Write git hook routines if accepted
    if git_hooks {
        std::fs::write(
            format!("{routines_base}/git-baseline.sh"),
            GIT_BASELINE_SH,
        )?;
        std::fs::write(
            format!("{routines_base}/git-stash-changes.sh"),
            GIT_STASH_CHANGES_SH,
        )?;
    }

    // 10. Write empty processed.md tracker
    std::fs::write(
        format!("{}/{}", config::DECREE_DIR, config::PROCESSED_FILE),
        "",
    )?;

    // Make routine scripts executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let routines_path = Path::new(&routines_base);
        if let Ok(entries) = std::fs::read_dir(routines_path) {
            for entry in entries.flatten() {
                if entry.path().extension().is_some_and(|ext| ext == "sh") {
                    let mut perms = std::fs::metadata(entry.path())?.permissions();
                    perms.set_mode(0o755);
                    std::fs::set_permissions(entry.path(), perms)?;
                }
            }
        }
    }

    println!("Decree initialized successfully.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_config_without_git_hooks() {
        let config = generate_config(
            "claude",
            "claude -p {prompt}",
            "claude",
            false,
            &["develop", "rust-develop"],
            &[],
        );
        assert!(config.contains("ai_router: \"claude -p {prompt}\""));
        assert!(config.contains("ai_interactive: \"claude\""));
        assert!(!config.contains("ai_command"));
        assert!(config.contains("max_retries: 3"));
        assert!(config.contains("beforeEach: \"\""));
        assert!(config.contains("# beforeEach: \"git-baseline\""));
        assert!(config.contains("routine_source: \"~/.decree/routines\""));
        assert!(config.contains("routines:\n"));
        assert!(config.contains("  develop:\n    enabled: true"));
        assert!(config.contains("  rust-develop:\n    enabled: true"));
    }

    #[test]
    fn test_generate_config_with_git_hooks() {
        let config = generate_config(
            "opencode",
            "opencode run {prompt}",
            "opencode",
            true,
            &["develop", "rust-develop", "git-baseline", "git-stash-changes"],
            &[],
        );
        assert!(config.contains("ai_router: \"opencode run {prompt}\""));
        assert!(config.contains("beforeEach: \"git-baseline\""));
        assert!(config.contains("afterEach: \"git-stash-changes\""));
        // Should still contain commented versions
        assert!(config.contains("# beforeEach: \"git-baseline\""));
        // Routines section should include hook routines
        assert!(config.contains("  git-baseline:\n    enabled: true"));
        assert!(config.contains("  git-stash-changes:\n    enabled: true"));
    }

    #[test]
    fn test_generate_config_includes_alternatives() {
        let config = generate_config(
            "claude",
            "claude -p {prompt}",
            "claude",
            false,
            &["develop"],
            &[],
        );
        // Other backends should be commented out
        assert!(config.contains("# ai_router: \"opencode run {prompt}\""));
        assert!(config.contains("# ai_router: \"copilot -p {prompt}\""));
        // Selected should not be commented
        assert!(config.contains("  ai_router: \"claude -p {prompt}\"\n"));
    }

    #[test]
    fn test_generate_config_with_shared_routines() {
        let config = generate_config(
            "claude",
            "claude -p {prompt}",
            "claude",
            false,
            &["develop"],
            &["deploy".to_string(), "notify".to_string()],
        );
        assert!(config.contains("shared_routines:\n"));
        assert!(config.contains("  deploy:\n    enabled: false"));
        assert!(config.contains("  notify:\n    enabled: false"));
    }

    #[test]
    fn test_command_exists_true() {
        // `ls` should exist on any system
        assert!(command_exists("ls"));
    }

    #[test]
    fn test_command_exists_false() {
        assert!(!command_exists("definitely_not_a_real_command_xyz"));
    }

    #[test]
    fn test_develop_template_has_precheck() {
        assert!(DEVELOP_SH.contains("DECREE_PRE_CHECK"));
        assert!(DEVELOP_SH.contains("{ai_name}"));
        assert!(DEVELOP_SH.contains("{ai_invoke}"));
    }

    #[test]
    fn test_rust_develop_template_has_precheck() {
        assert!(RUST_DEVELOP_SH.contains("DECREE_PRE_CHECK"));
        assert!(RUST_DEVELOP_SH.contains("{ai_name}"));
        assert!(RUST_DEVELOP_SH.contains("{ai_invoke}"));
        assert!(RUST_DEVELOP_SH.contains("cargo"));
    }

    #[test]
    fn test_develop_template_has_description_header() {
        // First non-shebang comment line is the title
        let lines: Vec<&str> = DEVELOP_SH.lines().collect();
        assert_eq!(lines[1], "# Develop");
        assert_eq!(lines[2], "#");
        // Description follows
        assert!(lines[3].starts_with("# "));
    }

    #[test]
    fn test_rust_develop_template_has_description_header() {
        let lines: Vec<&str> = RUST_DEVELOP_SH.lines().collect();
        assert_eq!(lines[1], "# Rust Develop");
        assert_eq!(lines[2], "#");
        assert!(lines[3].starts_with("# "));
    }

    #[test]
    fn test_develop_template_references_message_dir() {
        assert!(DEVELOP_SH.contains("${message_dir}"));
    }

    #[test]
    fn test_rust_develop_template_references_message_dir() {
        assert!(RUST_DEVELOP_SH.contains("${message_dir}"));
    }

    #[test]
    fn test_precheck_prints_to_stderr() {
        // Both routines should print errors to stderr (>&2)
        assert!(DEVELOP_SH.contains(">&2"));
        assert!(RUST_DEVELOP_SH.contains(">&2"));
    }

    #[test]
    fn test_router_has_placeholders() {
        assert!(ROUTER_MD.contains("{routines}"));
        assert!(ROUTER_MD.contains("{message}"));
    }

    #[test]
    fn test_gitignore_content() {
        assert!(DECREE_GITIGNORE.contains("inbox/"));
        assert!(DECREE_GITIGNORE.contains("outbox/"));
        assert!(DECREE_GITIGNORE.contains("runs/"));
    }

    #[test]
    fn test_ai_placeholder_replacement() {
        let replaced = replace_ai_placeholders(DEVELOP_SH, "claude", "claude -p {prompt}");
        assert!(replaced.contains("claude -p \"Read"));
        assert!(replaced.contains("command -v claude"));
        assert!(!replaced.contains("{ai_name}"));
        assert!(!replaced.contains("{ai_invoke}"));
    }

    #[test]
    fn test_ai_invoke_prefix() {
        assert_eq!(ai_invoke_prefix("opencode run {prompt}"), "opencode run");
        assert_eq!(ai_invoke_prefix("claude -p {prompt}"), "claude -p");
        assert_eq!(ai_invoke_prefix("copilot -p {prompt}"), "copilot -p");
    }

    #[test]
    fn test_git_baseline_has_precheck() {
        assert!(GIT_BASELINE_SH.contains("DECREE_PRE_CHECK"));
        assert!(GIT_BASELINE_SH.contains("git rev-parse --is-inside-work-tree"));
    }

    #[test]
    fn test_git_baseline_has_description_header() {
        let lines: Vec<&str> = GIT_BASELINE_SH.lines().collect();
        assert_eq!(lines[1], "# Git Baseline");
        assert_eq!(lines[2], "#");
        assert!(lines[3].starts_with("# "));
    }

    #[test]
    fn test_git_baseline_uses_env_vars() {
        assert!(GIT_BASELINE_SH.contains("DECREE_ATTEMPT"));
        assert!(GIT_BASELINE_SH.contains("DECREE_MAX_RETRIES"));
    }

    #[test]
    fn test_git_baseline_named_stashes() {
        assert!(GIT_BASELINE_SH.contains("decree-baseline: ${message_id}"));
        assert!(GIT_BASELINE_SH.contains("decree-failed: ${message_id}"));
    }

    #[test]
    fn test_git_baseline_has_parameters() {
        assert!(GIT_BASELINE_SH.contains("message_file="));
        assert!(GIT_BASELINE_SH.contains("message_id="));
        assert!(GIT_BASELINE_SH.contains("message_dir="));
        assert!(GIT_BASELINE_SH.contains("chain="));
        assert!(GIT_BASELINE_SH.contains("seq="));
    }

    #[test]
    fn test_git_baseline_no_destructive_commands() {
        assert!(!GIT_BASELINE_SH.contains("git reset"));
        assert!(!GIT_BASELINE_SH.contains("git clean"));
        assert!(!GIT_BASELINE_SH.contains("git checkout ."));
    }

    #[test]
    fn test_git_stash_changes_has_precheck() {
        assert!(GIT_STASH_CHANGES_SH.contains("DECREE_PRE_CHECK"));
    }

    #[test]
    fn test_git_stash_changes_has_description_header() {
        let lines: Vec<&str> = GIT_STASH_CHANGES_SH.lines().collect();
        assert_eq!(lines[1], "# Git Stash Changes");
        assert_eq!(lines[2], "#");
        assert!(lines[3].starts_with("# "));
    }

    #[test]
    fn test_git_stash_changes_uses_env_vars() {
        assert!(GIT_STASH_CHANGES_SH.contains("DECREE_ATTEMPT"));
        assert!(GIT_STASH_CHANGES_SH.contains("DECREE_MAX_RETRIES"));
        assert!(GIT_STASH_CHANGES_SH.contains("DECREE_ROUTINE_EXIT_CODE"));
    }

    #[test]
    fn test_git_stash_changes_named_stashes() {
        assert!(GIT_STASH_CHANGES_SH.contains("decree: ${message_id} attempt ${ATTEMPT}"));
        assert!(GIT_STASH_CHANGES_SH.contains("decree-exhausted: ${message_id}"));
    }

    #[test]
    fn test_git_stash_changes_has_parameters() {
        assert!(GIT_STASH_CHANGES_SH.contains("message_file="));
        assert!(GIT_STASH_CHANGES_SH.contains("message_id="));
        assert!(GIT_STASH_CHANGES_SH.contains("message_dir="));
        assert!(GIT_STASH_CHANGES_SH.contains("chain="));
        assert!(GIT_STASH_CHANGES_SH.contains("seq="));
    }

    #[test]
    fn test_git_stash_changes_no_destructive_commands() {
        assert!(!GIT_STASH_CHANGES_SH.contains("git reset"));
        assert!(!GIT_STASH_CHANGES_SH.contains("git clean"));
        assert!(!GIT_STASH_CHANGES_SH.contains("git checkout ."));
    }

    #[test]
    fn test_git_stash_changes_restores_baseline_on_exhaustion() {
        // Should restore baseline when exit code != 0 and attempt == max_retries
        assert!(GIT_STASH_CHANGES_SH.contains("EXIT_CODE\" -ne 0"));
        assert!(GIT_STASH_CHANGES_SH.contains("ATTEMPT\" -eq \"$MAX_RETRIES\""));
        assert!(GIT_STASH_CHANGES_SH.contains("decree-baseline: ${message_id}"));
    }

    #[test]
    fn test_git_baseline_restores_on_final_retry() {
        // Final retry should stash failed changes and restore baseline
        assert!(GIT_BASELINE_SH.contains("ATTEMPT\" -eq \"$MAX_RETRIES\""));
        assert!(GIT_BASELINE_SH.contains("git stash apply"));
    }
}
