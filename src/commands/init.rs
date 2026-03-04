use std::fs;
use std::path::Path;
use std::process::Command;

use crate::config::{AiTool, Config, AI_TOOLS};
use crate::error::{DecreeError, Result};

const TEMPLATE_SPEC: &str = include_str!("../templates/spec.md");
const TEMPLATE_DEVELOP: &str = include_str!("../templates/develop.sh");
const TEMPLATE_RUST_DEVELOP: &str = include_str!("../templates/rust-develop.sh");
const TEMPLATE_GITIGNORE: &str = include_str!("../templates/gitignore");
const TEMPLATE_GIT_BASELINE: &str = include_str!("../templates/git-baseline.sh");
const TEMPLATE_GIT_STASH_CHANGES: &str = include_str!("../templates/git-stash-changes.sh");
const TEMPLATE_ROUTINES_DOC: &str = include_str!("../templates/routines.md");

pub fn run() -> Result<()> {
    let decree_dir = Path::new(".decree");

    if decree_dir.exists() {
        return Err(DecreeError::AlreadyInitialized(decree_dir.to_path_buf()));
    }

    // Detect AI backends
    let ai_tool = detect_ai_backend()?;
    let ai_cmd = ai_tool.to_string();

    // Build config
    let mut config = Config::default().with_ai_command(&ai_tool);

    // Git detection and hook setup
    let git_available = check_git_available();
    if git_available {
        let enable_hooks = prompt_git_hooks()?;
        if enable_hooks {
            config = config.with_git_hooks();
        }
    } else {
        eprintln!("git not found — skipping lifecycle hook setup");
    }

    // Create directory structure
    create_directories(decree_dir)?;

    // Write templates with AI_CMD replaced
    write_templates(decree_dir, &ai_cmd, git_available && config.hooks.before_each == "git-baseline")?;

    // Write ROUTINES.md in project root
    fs::write("ROUTINES.md", TEMPLATE_ROUTINES_DOC)?;

    // Write config
    config.save(&decree_dir.join("config.yml"))?;

    // Create migrations directory at project root
    let migrations_dir = Path::new("migrations");
    if !migrations_dir.exists() {
        fs::create_dir_all(migrations_dir)?;
    }
    let processed_file = migrations_dir.join("processed.md");
    if !processed_file.exists() {
        fs::write(&processed_file, "")?;
    }

    eprintln!("Initialized decree project");
    Ok(())
}

fn create_directories(base: &Path) -> Result<()> {
    let dirs = [
        base.to_path_buf(),
        base.join("routines"),
        base.join("starters"),
        base.join("cron"),
        base.join("inbox"),
        base.join("inbox/done"),
        base.join("inbox/dead"),
        base.join("runs"),
    ];
    for dir in &dirs {
        fs::create_dir_all(dir)?;
    }
    Ok(())
}

fn write_templates(base: &Path, ai_cmd: &str, write_git_hooks: bool) -> Result<()> {
    // .gitignore
    fs::write(base.join(".gitignore"), TEMPLATE_GITIGNORE)?;

    // Starter template
    fs::write(base.join("starters/spec.md"), TEMPLATE_SPEC)?;

    // Routine templates with {AI_CMD} replaced
    let develop = TEMPLATE_DEVELOP.replace("{AI_CMD}", ai_cmd);
    fs::write(base.join("routines/develop.sh"), &develop)?;
    set_executable(base.join("routines/develop.sh"))?;

    let rust_develop = TEMPLATE_RUST_DEVELOP.replace("{AI_CMD}", ai_cmd);
    fs::write(base.join("routines/rust-develop.sh"), &rust_develop)?;
    set_executable(base.join("routines/rust-develop.sh"))?;

    // Git hook routines (only if user accepted)
    if write_git_hooks {
        fs::write(base.join("routines/git-baseline.sh"), TEMPLATE_GIT_BASELINE)?;
        set_executable(base.join("routines/git-baseline.sh"))?;

        fs::write(
            base.join("routines/git-stash-changes.sh"),
            TEMPLATE_GIT_STASH_CHANGES,
        )?;
        set_executable(base.join("routines/git-stash-changes.sh"))?;
    }

    Ok(())
}

fn set_executable<P: AsRef<Path>>(path: P) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let metadata = fs::metadata(path.as_ref())?;
    let mut perms = metadata.permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path.as_ref(), perms)?;
    Ok(())
}

fn detect_ai_backend() -> Result<AiTool> {
    let mut found: Vec<AiTool> = Vec::new();

    for (tool, binary) in AI_TOOLS {
        if which_exists(binary) {
            found.push(*tool);
        }
    }

    match found.len() {
        0 => {
            eprintln!(
                "No AI backend found. Install one — we recommend opencode: https://opencode.ai/"
            );
            Ok(AiTool::Opencode)
        }
        1 => {
            eprintln!("Detected AI backend: {}", found[0]);
            Ok(found[0])
        }
        _ => select_ai_backend(&found),
    }
}

fn select_ai_backend(tools: &[AiTool]) -> Result<AiTool> {
    let items: Vec<String> = tools.iter().map(|t| t.to_string()).collect();

    let selection = dialoguer::Select::new()
        .with_prompt("Multiple AI backends found. Select one")
        .items(&items)
        .default(0)
        .interact()
        .map_err(|e| DecreeError::Config(format!("selection failed: {e}")))?;

    Ok(tools[selection])
}

fn which_exists(binary: &str) -> bool {
    Command::new("which")
        .arg(binary)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn check_git_available() -> bool {
    let git_found = which_exists("git");
    if !git_found {
        return false;
    }

    Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn prompt_git_hooks() -> Result<bool> {
    let result = dialoguer::Confirm::new()
        .with_prompt("Enable git stash hooks for change tracking?")
        .default(true)
        .interact()
        .map_err(|e| DecreeError::Config(format!("prompt failed: {e}")))?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_which_exists_false_for_nonexistent() {
        assert!(!which_exists("definitely_not_a_real_binary_12345"));
    }

    #[test]
    fn test_template_ai_cmd_replacement() {
        let result = TEMPLATE_DEVELOP.replace("{AI_CMD}", "claude");
        assert!(result.contains("claude"));
        assert!(!result.contains("{AI_CMD}"));
    }

    #[test]
    fn test_rust_develop_ai_cmd_replacement() {
        let result = TEMPLATE_RUST_DEVELOP.replace("{AI_CMD}", "copilot");
        assert!(result.contains("copilot"));
        assert!(!result.contains("{AI_CMD}"));
    }

    #[test]
    fn test_develop_has_precheck() {
        assert!(TEMPLATE_DEVELOP.contains("DECREE_PRE_CHECK"));
        assert!(TEMPLATE_DEVELOP.contains("command -v {AI_CMD}"));
    }

    #[test]
    fn test_rust_develop_has_precheck() {
        assert!(TEMPLATE_RUST_DEVELOP.contains("DECREE_PRE_CHECK"));
        assert!(TEMPLATE_RUST_DEVELOP.contains("command -v {AI_CMD}"));
        assert!(TEMPLATE_RUST_DEVELOP.contains("command -v cargo"));
    }

    #[test]
    fn test_routine_description_headers() {
        // Routines must have a description comment for `decree routine` extraction
        assert!(TEMPLATE_DEVELOP.starts_with("#!/usr/bin/env bash\n# Develop\n"));
        assert!(TEMPLATE_RUST_DEVELOP.starts_with("#!/usr/bin/env bash\n# Rust Develop\n"));
    }

    #[test]
    fn test_gitignore_excludes() {
        assert!(TEMPLATE_GITIGNORE.contains("inbox/"));
        assert!(TEMPLATE_GITIGNORE.contains("runs/"));
    }

    #[test]
    fn test_spec_template_content() {
        assert!(TEMPLATE_SPEC.contains("# Spec Template"));
        assert!(TEMPLATE_SPEC.contains("Naming"));
        assert!(TEMPLATE_SPEC.contains("Frontmatter"));
        assert!(TEMPLATE_SPEC.contains("Immutability"));
    }

    #[test]
    fn test_routines_doc_standard_parameter_table() {
        assert!(TEMPLATE_ROUTINES_DOC.contains("## Standard Parameter Mapping"));
        assert!(TEMPLATE_ROUTINES_DOC.contains("| `input_file` | `input_file`"));
        assert!(TEMPLATE_ROUTINES_DOC.contains("| *(auto)* | `message_file`"));
        assert!(TEMPLATE_ROUTINES_DOC.contains("| *(auto)* | `message_id`"));
        assert!(TEMPLATE_ROUTINES_DOC.contains("| *(auto)* | `message_dir`"));
        assert!(TEMPLATE_ROUTINES_DOC.contains("| `chain` | `chain`"));
        assert!(TEMPLATE_ROUTINES_DOC.contains("| `seq` | `seq`"));
    }

    #[test]
    fn test_routines_doc_custom_parameter_discovery() {
        assert!(TEMPLATE_ROUTINES_DOC.contains("## Custom Parameter Discovery"));
        assert!(TEMPLATE_ROUTINES_DOC.contains(r#"var_name="${var_name:-default_value}""#));
        assert!(TEMPLATE_ROUTINES_DOC.contains("${var:-}"));
    }

    #[test]
    fn test_routines_doc_precheck_section() {
        assert!(TEMPLATE_ROUTINES_DOC.contains("## Pre-Check Section"));
        assert!(TEMPLATE_ROUTINES_DOC.contains("DECREE_PRE_CHECK"));
        assert!(TEMPLATE_ROUTINES_DOC.contains("exit 0"));
    }

    #[test]
    fn test_routines_doc_minimal_example() {
        assert!(TEMPLATE_ROUTINES_DOC.contains("## Minimal Routine Example"));
        // Standard params present
        assert!(TEMPLATE_ROUTINES_DOC.contains(r#"message_file="${message_file:-}""#));
        assert!(TEMPLATE_ROUTINES_DOC.contains(r#"message_id="${message_id:-}""#));
        // Pre-check present
        assert!(TEMPLATE_ROUTINES_DOC.contains(r#"if [ "${DECREE_PRE_CHECK:-}" = "true" ]"#));
        // Custom params present
        assert!(TEMPLATE_ROUTINES_DOC.contains(r#"my_option="${my_option:-default_value}""#));
    }

    #[test]
    fn test_routines_doc_frontmatter_example() {
        assert!(TEMPLATE_ROUTINES_DOC.contains("## Corresponding Message Frontmatter"));
        assert!(TEMPLATE_ROUTINES_DOC.contains("routine: my-routine"));
        assert!(TEMPLATE_ROUTINES_DOC.contains("my_option: custom_value"));
    }

    #[test]
    fn test_routines_doc_comment_header_format() {
        assert!(TEMPLATE_ROUTINES_DOC.contains("## Comment Header Format"));
        assert!(TEMPLATE_ROUTINES_DOC.contains("# Title Here"));
        assert!(TEMPLATE_ROUTINES_DOC.contains("# Short description"));
    }

    #[test]
    fn test_routines_doc_nested_routines() {
        assert!(TEMPLATE_ROUTINES_DOC.contains("## Nested Routines"));
        assert!(TEMPLATE_ROUTINES_DOC.contains("routine: deploy/staging"));
        assert!(TEMPLATE_ROUTINES_DOC.contains("routine: review/pr"));
    }

    #[test]
    fn test_routines_doc_tips_section() {
        assert!(TEMPLATE_ROUTINES_DOC.contains("## Tips and Gotchas"));
        assert!(TEMPLATE_ROUTINES_DOC.contains("# --- Parameters ---"));
        assert!(TEMPLATE_ROUTINES_DOC.contains("set -euo pipefail"));
        assert!(TEMPLATE_ROUTINES_DOC.contains("AI-specific"));
    }

    #[test]
    fn test_git_baseline_template() {
        assert!(TEMPLATE_GIT_BASELINE.starts_with("#!/usr/bin/env bash\n# Git Baseline\n"));
        assert!(TEMPLATE_GIT_BASELINE.contains("DECREE_PRE_CHECK"));
        assert!(TEMPLATE_GIT_BASELINE.contains("git rev-parse --is-inside-work-tree"));
        assert!(TEMPLATE_GIT_BASELINE.contains("git add -A"));
        assert!(TEMPLATE_GIT_BASELINE.contains("git commit --allow-empty --no-verify"));
        assert!(TEMPLATE_GIT_BASELINE.contains("decree-baseline: ${message_id}"));
    }

    #[test]
    fn test_git_baseline_has_standard_params() {
        assert!(TEMPLATE_GIT_BASELINE.contains(r#"message_file="${message_file:-}""#));
        assert!(TEMPLATE_GIT_BASELINE.contains(r#"message_id="${message_id:-}""#));
        assert!(TEMPLATE_GIT_BASELINE.contains(r#"message_dir="${message_dir:-}""#));
        assert!(TEMPLATE_GIT_BASELINE.contains(r#"chain="${chain:-}""#));
        assert!(TEMPLATE_GIT_BASELINE.contains(r#"seq="${seq:-}""#));
    }

    #[test]
    fn test_git_stash_changes_template() {
        assert!(
            TEMPLATE_GIT_STASH_CHANGES
                .starts_with("#!/usr/bin/env bash\n# Git Stash Changes\n")
        );
        assert!(TEMPLATE_GIT_STASH_CHANGES.contains("DECREE_PRE_CHECK"));
        assert!(TEMPLATE_GIT_STASH_CHANGES.contains("git add -A"));
        assert!(TEMPLATE_GIT_STASH_CHANGES.contains("git stash create"));
        assert!(TEMPLATE_GIT_STASH_CHANGES.contains(r#"git stash store -m "decree: ${message_id}""#));
        assert!(TEMPLATE_GIT_STASH_CHANGES.contains("git reset --soft HEAD~1"));
        assert!(TEMPLATE_GIT_STASH_CHANGES.contains("git reset HEAD ."));
    }

    #[test]
    fn test_git_stash_changes_has_standard_params() {
        assert!(TEMPLATE_GIT_STASH_CHANGES.contains(r#"message_file="${message_file:-}""#));
        assert!(TEMPLATE_GIT_STASH_CHANGES.contains(r#"message_id="${message_id:-}""#));
        assert!(TEMPLATE_GIT_STASH_CHANGES.contains(r#"message_dir="${message_dir:-}""#));
        assert!(TEMPLATE_GIT_STASH_CHANGES.contains(r#"chain="${chain:-}""#));
        assert!(TEMPLATE_GIT_STASH_CHANGES.contains(r#"seq="${seq:-}""#));
    }

    #[test]
    fn test_git_stash_names_use_decree_prefix() {
        // Stash entries must be named "decree: <message_id>" for easy identification
        assert!(TEMPLATE_GIT_STASH_CHANGES.contains(r#""decree: ${message_id}""#));
    }

    #[test]
    fn test_git_baseline_precheck_verifies_git_repo() {
        // Pre-check must verify both git binary and that we're in a repo
        assert!(TEMPLATE_GIT_BASELINE.contains("command -v git"));
        assert!(TEMPLATE_GIT_BASELINE.contains("git rev-parse --is-inside-work-tree"));
    }

    #[test]
    fn test_git_stash_changes_precheck_verifies_git() {
        // Pre-check must verify git binary is available
        assert!(TEMPLATE_GIT_STASH_CHANGES.contains("command -v git"));
    }
}
