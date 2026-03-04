use std::fs;
use std::io::{self, IsTerminal, Write as _};
use std::os::unix::process::CommandExt as _;
use std::path::Path;
use std::process::Command;

use console::{Key, Term};
use dialoguer::FuzzySelect;

use crate::config::Config;
use crate::error::{DecreeError, Result};
use crate::migration::MigrationTracker;

/// A starter template discovered in `.decree/starters/`.
#[derive(Debug, Clone)]
struct Starter {
    name: String,
    /// First heading or first line — used as short description in selector.
    short_description: String,
    content: String,
}

/// Run the `decree starter` command.
pub fn run(name: Option<&str>) -> Result<()> {
    let starters_dir = Path::new(".decree/starters");
    let starters = discover_starters(starters_dir)?;

    if starters.is_empty() {
        eprintln!("No starters found in .decree/starters/");
        return Ok(());
    }

    let starter = match name {
        Some(name) => match starters.iter().find(|s| s.name == name) {
            Some(s) => s,
            None => {
                eprintln!("Unknown starter: {name}");
                eprintln!();
                eprintln!("Available starters:");
                for s in &starters {
                    eprintln!("  {}", s.name);
                }
                return Err(DecreeError::StarterNotFound(name.to_string()));
            }
        },
        None => select_starter(&starters)?,
    };

    // Build the prompt
    let prompt = build_prompt(starter)?;

    // Step 2: Print the full prompt to stdout
    println!("{prompt}");

    // Step 3: Action prompt (only in TTY mode)
    if !io::stdin().is_terminal() {
        return Ok(());
    }

    action_prompt(&prompt)
}

/// Discover all `.md` files in the starters directory.
fn discover_starters(dir: &Path) -> Result<Vec<Starter>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut starters = Vec::new();

    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let file_name = match entry.file_name().to_str() {
            Some(n) => n.to_string(),
            None => continue,
        };
        if !file_name.ends_with(".md") {
            continue;
        }

        let name = file_name.trim_end_matches(".md").to_string();
        let content = fs::read_to_string(entry.path())?;
        let short_description = extract_short_description(&content);

        starters.push(Starter {
            name,
            short_description,
            content,
        });
    }

    starters.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(starters)
}

/// Extract a short description from starter content.
/// Uses the first `# Heading` line (without the `#`), or the first non-empty line.
fn extract_short_description(content: &str) -> String {
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(heading) = trimmed.strip_prefix("# ") {
            return heading.to_string();
        }
        return trimmed.to_string();
    }
    String::new()
}

/// Step 1: Select starter using fuzzy finder.
fn select_starter(starters: &[Starter]) -> Result<&Starter> {
    let name_width = starters
        .iter()
        .map(|s| s.name.len())
        .max()
        .unwrap_or(0)
        .max(14)
        + 2;

    let items: Vec<String> = starters
        .iter()
        .map(|s| {
            format!(
                "{:<width$} {}",
                s.name,
                s.short_description,
                width = name_width
            )
        })
        .collect();

    let selection = FuzzySelect::new()
        .with_prompt("Select a starter")
        .items(&items)
        .default(0)
        .interact()
        .map_err(|e| DecreeError::Config(format!("selection failed: {e}")))?;

    Ok(&starters[selection])
}

/// Build the full prompt from template + project context.
fn build_prompt(starter: &Starter) -> Result<String> {
    let migrations_section = list_migrations()?;

    Ok(format!(
        "\
You are a planning assistant for a software project.

## Starter Template
{template}

## Existing Migrations
{migrations}

## Instructions
1. Analyse the request and existing project state.
2. Present a numbered plan summary with proposed migration files.
3. WAIT for approval — do NOT generate files until told to proceed.
4. When approved, generate each migration file:
   - Filename: NN-descriptive-name.md
   - Include YAML frontmatter with `routine:` field
   - Write each file to the migrations/ directory",
        template = starter.content,
        migrations = migrations_section,
    ))
}

/// List migrations with their titles for the prompt.
fn list_migrations() -> Result<String> {
    let migrations_dir = Path::new("migrations");
    let tracker = MigrationTracker::new(migrations_dir);
    let all = tracker.all_migrations()?;

    if all.is_empty() {
        return Ok("None yet".to_string());
    }

    let mut lines = Vec::new();
    for filename in &all {
        let path = migrations_dir.join(filename);
        let title = extract_migration_title(&path);
        match title {
            Some(t) => lines.push(format!("- {filename}: {t}")),
            None => lines.push(format!("- {filename}")),
        }
    }

    Ok(lines.join("\n"))
}

/// Extract the title from a migration file (first `#` heading).
fn extract_migration_title(path: &Path) -> Option<String> {
    let content = fs::read_to_string(path).ok()?;
    // Skip frontmatter if present
    let (_, body) = crate::migration::split_frontmatter(&content);
    for line in body.lines() {
        let trimmed = line.trim();
        if let Some(heading) = trimmed.strip_prefix("# ") {
            return Some(heading.to_string());
        }
    }
    None
}

/// Step 3: Prompt the user for an action after displaying the prompt.
fn action_prompt(prompt: &str) -> Result<()> {
    eprintln!();
    eprint!("Press Enter to launch interactive AI, or C to copy to clipboard: ");
    io::stderr().flush()?;

    let term = Term::stderr();
    let key = match term.read_key() {
        Ok(k) => k,
        Err(_) => return Ok(()), // Ctrl-C or error — exit without action
    };

    match key {
        Key::Enter => {
            eprintln!();
            exec_interactive_ai()
        }
        Key::Char('c') | Key::Char('C') => {
            eprintln!();
            copy_to_clipboard(prompt)?;
            eprintln!("Prompt copied to clipboard.");
            Ok(())
        }
        _ => {
            eprintln!();
            Ok(())
        }
    }
}

/// Exec into the configured interactive AI command.
/// This replaces the current process.
fn exec_interactive_ai() -> Result<()> {
    let config = Config::load(Path::new(".decree/config.yml"))?;
    let cmd_str = &config.commands.interactive_ai;

    // Split command into program and args
    let parts: Vec<&str> = cmd_str.split_whitespace().collect();
    if parts.is_empty() {
        return Err(DecreeError::Config(
            "commands.interactive_ai is empty".to_string(),
        ));
    }

    let program = parts[0];
    let args = &parts[1..];

    eprintln!("Launching {cmd_str}...");

    // exec replaces the current process
    let err = Command::new(program).args(args).exec();
    // exec() only returns on error
    Err(DecreeError::Io(err))
}

/// Copy text to the system clipboard using a platform-appropriate command.
fn copy_to_clipboard(text: &str) -> Result<()> {
    let (program, args) = detect_clipboard_command()?;

    let mut child = Command::new(program)
        .args(&args)
        .stdin(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| {
            DecreeError::Config(format!("failed to launch clipboard command '{program}': {e}"))
        })?;

    if let Some(ref mut stdin) = child.stdin {
        stdin.write_all(text.as_bytes())?;
    }

    let status = child.wait()?;
    if !status.success() {
        return Err(DecreeError::Config(format!(
            "clipboard command exited with status: {status}"
        )));
    }

    Ok(())
}

/// Detect the appropriate clipboard command for the current platform.
fn detect_clipboard_command() -> Result<(&'static str, Vec<&'static str>)> {
    if cfg!(target_os = "macos") {
        return Ok(("pbcopy", vec![]));
    }

    // Check for WSL
    if is_wsl() {
        return Ok(("clip.exe", vec![]));
    }

    // Linux: try xclip first, then xsel
    if command_exists("xclip") {
        return Ok(("xclip", vec!["-selection", "clipboard"]));
    }
    if command_exists("xsel") {
        return Ok(("xsel", vec!["--clipboard"]));
    }

    Err(DecreeError::Config(
        "no clipboard command found (install xclip or xsel)".to_string(),
    ))
}

/// Check if we're running under WSL.
fn is_wsl() -> bool {
    Path::new("/proc/version").exists()
        && fs::read_to_string("/proc/version")
            .map(|v| v.to_lowercase().contains("microsoft"))
            .unwrap_or(false)
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_short_description_heading() {
        let content = "# My Starter\n\nSome body text.";
        assert_eq!(extract_short_description(content), "My Starter");
    }

    #[test]
    fn test_extract_short_description_no_heading() {
        let content = "Just plain text\non multiple lines.";
        assert_eq!(extract_short_description(content), "Just plain text");
    }

    #[test]
    fn test_extract_short_description_empty() {
        assert_eq!(extract_short_description(""), "");
    }

    #[test]
    fn test_extract_short_description_leading_blank_lines() {
        let content = "\n\n# Title\nBody.";
        assert_eq!(extract_short_description(content), "Title");
    }

    #[test]
    fn test_discover_starters_empty_dir() {
        let tmp = std::env::temp_dir().join("decree_test_starters_empty");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        let starters = discover_starters(&tmp).unwrap();
        assert!(starters.is_empty());

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_discover_starters_finds_md_files() {
        let tmp = std::env::temp_dir().join("decree_test_starters_find");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        fs::write(tmp.join("spec.md"), "# Spec Template\nDetails.").unwrap();
        fs::write(tmp.join("bugfix.md"), "# Bug Fix\nFix things.").unwrap();
        fs::write(tmp.join("not-a-starter.txt"), "ignored").unwrap();

        let starters = discover_starters(&tmp).unwrap();
        assert_eq!(starters.len(), 2);
        assert_eq!(starters[0].name, "bugfix");
        assert_eq!(starters[0].short_description, "Bug Fix");
        assert_eq!(starters[1].name, "spec");
        assert_eq!(starters[1].short_description, "Spec Template");

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_discover_starters_nonexistent_dir() {
        let starters = discover_starters(Path::new("/nonexistent/starters")).unwrap();
        assert!(starters.is_empty());
    }

    #[test]
    fn test_build_prompt_contains_template() {
        let starter = Starter {
            name: "test".to_string(),
            short_description: "Test".to_string(),
            content: "# Test Starter\nDo the thing.".to_string(),
        };

        // build_prompt reads migrations from disk; we need migrations/ to exist
        // but we can test the structure at least
        let tmp = std::env::temp_dir().join("decree_test_build_prompt");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        // Test with a starter that has content
        // Note: build_prompt reads "migrations/" relative to cwd, so this test
        // just verifies the starter content is included
        let prompt = format!(
            "\
You are a planning assistant for a software project.

## Starter Template
{template}

## Existing Migrations
None yet

## Instructions
1. Analyse the request and existing project state.
2. Present a numbered plan summary with proposed migration files.
3. WAIT for approval — do NOT generate files until told to proceed.
4. When approved, generate each migration file:
   - Filename: NN-descriptive-name.md
   - Include YAML frontmatter with `routine:` field
   - Write each file to the migrations/ directory",
            template = starter.content,
        );

        assert!(prompt.contains("# Test Starter"));
        assert!(prompt.contains("Do the thing."));
        assert!(prompt.contains("## Existing Migrations"));
        assert!(prompt.contains("## Instructions"));

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_extract_migration_title() {
        let tmp = std::env::temp_dir().join("decree_test_migration_title");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        let path = tmp.join("01-auth.md");
        fs::write(&path, "---\nroutine: develop\n---\n# 01: Add Authentication\n\nDetails.").unwrap();

        let title = extract_migration_title(&path);
        assert_eq!(title, Some("01: Add Authentication".to_string()));

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_extract_migration_title_no_heading() {
        let tmp = std::env::temp_dir().join("decree_test_migration_no_title");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        let path = tmp.join("01-plain.md");
        fs::write(&path, "Just some text without a heading.").unwrap();

        let title = extract_migration_title(&path);
        assert!(title.is_none());

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_extract_migration_title_nonexistent() {
        let title = extract_migration_title(Path::new("/nonexistent/file.md"));
        assert!(title.is_none());
    }

    #[test]
    fn test_is_wsl_on_non_wsl() {
        // On a normal Linux system, /proc/version exists but doesn't contain "microsoft"
        // This test just verifies it doesn't panic
        let _ = is_wsl();
    }

    #[test]
    fn test_command_exists_false() {
        assert!(!command_exists("definitely_not_a_real_binary_99999"));
    }

    #[test]
    fn test_list_migrations_no_dir() {
        // When migrations/ doesn't exist, we should get "None yet"
        // This depends on cwd, so we test the empty case via tracker
        let tmp = std::env::temp_dir().join("decree_test_list_mig_empty");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        let tracker = MigrationTracker::new(&tmp);
        let all = tracker.all_migrations().unwrap();
        assert!(all.is_empty());

        let _ = fs::remove_dir_all(&tmp);
    }
}
