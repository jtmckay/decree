use crate::config::{self, AppConfig};
use crate::error::{color, DecreeError};
use crate::message;
use crate::routine;
use std::io::{self, Read as _};
use std::os::unix::process::CommandExt;
use std::path::Path;

/// Run `decree prompt [name]`.
pub fn run(project_root: &Path, name: Option<&str>) -> Result<(), DecreeError> {
    let config = AppConfig::load_from_project(project_root)?;
    let prompts = list_prompts(project_root, &config)?;

    if prompts.is_empty() {
        println!("No prompts found in .decree/prompts/");
        return Ok(());
    }

    match name {
        Some(name) => run_named(project_root, &prompts, name),
        None => run_select(project_root, &prompts),
    }
}

/// Prompt template info.
#[derive(Debug, Clone)]
struct PromptInfo {
    name: String,
    description: String,
}

/// List all `.md` prompt templates, checking project-local first, then shared.
/// Prompts do NOT require config registration.
fn list_prompts(project_root: &Path, config: &AppConfig) -> Result<Vec<PromptInfo>, DecreeError> {
    let prompts_dir = project_root
        .join(config::DECREE_DIR)
        .join(config::PROMPTS_DIR);

    let mut seen = std::collections::HashSet::new();
    let mut prompts = Vec::new();

    // Project-local prompts first
    if prompts_dir.exists() {
        for info in scan_prompts_dir(&prompts_dir)? {
            seen.insert(info.name.clone());
            prompts.push(info);
        }
    }

    // Shared prompts (fallback)
    if let Some(shared_prompts_dir) = config.resolved_shared_prompts_dir() {
        if shared_prompts_dir.exists() {
            for info in scan_prompts_dir(&shared_prompts_dir)? {
                if !seen.contains(&info.name) {
                    prompts.push(info);
                }
            }
        }
    }

    prompts.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(prompts)
}

/// Scan a directory for `.md` prompt templates.
fn scan_prompts_dir(dir: &Path) -> Result<Vec<PromptInfo>, DecreeError> {
    let mut entries: Vec<String> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_file() && e.path().extension().is_some_and(|ext| ext == "md"))
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();

    entries.sort();

    let mut prompts = Vec::new();
    for filename in entries {
        let name = filename.strip_suffix(".md").unwrap_or(&filename).to_string();
        let path = dir.join(&filename);
        let content = std::fs::read_to_string(&path)?;
        let description = extract_description(&content);
        prompts.push(PromptInfo { name, description });
    }

    Ok(prompts)
}

/// Extract description: first non-blank line, truncated to 60 chars.
fn extract_description(content: &str) -> String {
    let desc = content
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .to_string();

    if desc.len() > 60 {
        format!("{}...", &desc[..57])
    } else {
        desc
    }
}

/// Run with a specific prompt name.
fn run_named(
    project_root: &Path,
    prompts: &[PromptInfo],
    name: &str,
) -> Result<(), DecreeError> {
    let info = match prompts.iter().find(|p| p.name == name) {
        Some(p) => p,
        None => return prompt_not_found(name, prompts),
    };

    let config = AppConfig::load_from_project(project_root)?;
    let prompt_text = build_prompt(project_root, &config, &info.name)?;

    if !color::is_tty() {
        // Non-TTY: print substituted prompt and exit
        print!("{prompt_text}");
        return Ok(());
    }

    guided_flow(project_root, &prompt_text)
}

/// Run with interactive selection.
fn run_select(
    project_root: &Path,
    prompts: &[PromptInfo],
) -> Result<(), DecreeError> {
    if !color::is_tty() {
        // Non-TTY: print list and exit
        for p in prompts {
            if p.description.is_empty() {
                println!("  {}", p.name);
            } else {
                println!("  {:<16} {}", p.name, p.description);
            }
        }
        return Ok(());
    }

    let options: Vec<String> = prompts
        .iter()
        .map(|p| {
            if p.description.is_empty() {
                p.name.clone()
            } else {
                format!("{:<16} {}", p.name, p.description)
            }
        })
        .collect();

    let selection = inquire::Select::new("Select a prompt:", options)
        .prompt()
        .map_err(|e| DecreeError::Other(format!("selection cancelled: {e}")))?;

    let selected_name = selection.split_whitespace().next().unwrap_or(&selection);

    let config = AppConfig::load_from_project(project_root)?;
    let prompt_text = build_prompt(project_root, &config, selected_name)?;
    guided_flow(project_root, &prompt_text)
}

/// Build the prompt by reading the template and substituting variables.
/// Checks project-local first, then shared prompts.
fn build_prompt(project_root: &Path, config: &AppConfig, name: &str) -> Result<String, DecreeError> {
    let prompts_dir = project_root
        .join(config::DECREE_DIR)
        .join(config::PROMPTS_DIR);
    let path = prompts_dir.join(format!("{name}.md"));

    // Check project-local first, then shared
    let template_path = if path.is_file() {
        path
    } else if let Some(shared_dir) = config.resolved_shared_prompts_dir() {
        let shared_path = shared_dir.join(format!("{name}.md"));
        if shared_path.is_file() {
            shared_path
        } else {
            return Err(DecreeError::Other(format!("prompt template not found: {name}")));
        }
    } else {
        return Err(DecreeError::Other(format!("prompt template not found: {name}")));
    };

    let template = std::fs::read_to_string(&template_path)?;

    // Substitute known variables
    let prompt = substitute_variables(project_root, &template)?;

    Ok(prompt)
}

/// Substitute `{variable}` placeholders with project context.
fn substitute_variables(project_root: &Path, template: &str) -> Result<String, DecreeError> {
    let mut result = template.to_string();

    // Only substitute known variables, leave unknown ones as-is
    if result.contains("{migrations}") {
        let migrations = build_migrations_text(project_root)?;
        result = result.replace("{migrations}", &migrations);
    }

    if result.contains("{routines}") {
        let routines = build_routines_text(project_root)?;
        result = result.replace("{routines}", &routines);
    }

    if result.contains("{processed}") {
        let processed = build_processed_text(project_root)?;
        result = result.replace("{processed}", &processed);
    }

    if result.contains("{config}") {
        let config = build_config_text(project_root)?;
        result = result.replace("{config}", &config);
    }

    Ok(result)
}

/// Build the `{migrations}` substitution text.
fn build_migrations_text(project_root: &Path) -> Result<String, DecreeError> {
    let files = message::list_migration_files(project_root)?;

    if files.is_empty() {
        return Ok("None yet".to_string());
    }

    let mut lines = Vec::new();
    for filename in &files {
        let path = project_root
            .join(config::DECREE_DIR)
            .join(config::MIGRATIONS_DIR)
            .join(filename);
        let content = std::fs::read_to_string(&path)?;
        let title = content
            .lines()
            .find(|l| !l.trim().is_empty() && !l.starts_with("---"))
            .unwrap_or("")
            .trim_start_matches("# ")
            .to_string();

        if title.is_empty() {
            lines.push(format!("- {filename}"));
        } else {
            lines.push(format!("- {filename}: {title}"));
        }
    }

    Ok(lines.join("\n"))
}

/// Build the `{routines}` substitution text.
fn build_routines_text(project_root: &Path) -> Result<String, DecreeError> {
    let config = AppConfig::load_from_project(project_root)?;
    let routines = message::list_routines(project_root, &config)?;

    if routines.is_empty() {
        return Ok("None yet".to_string());
    }

    let lines: Vec<String> = routines
        .iter()
        .map(|r| {
            if r.description.is_empty() {
                format!("- {}", r.name)
            } else {
                format!("- {}: {}", r.name, r.description)
            }
        })
        .collect();

    Ok(lines.join("\n"))
}

/// Build the `{processed}` substitution text.
fn build_processed_text(project_root: &Path) -> Result<String, DecreeError> {
    let path = project_root
        .join(config::DECREE_DIR)
        .join(config::PROCESSED_FILE);

    if !path.exists() {
        return Ok("None yet".to_string());
    }

    let content = std::fs::read_to_string(&path)?;
    if content.trim().is_empty() {
        return Ok("None yet".to_string());
    }

    Ok(content)
}

/// Build the `{config}` substitution text.
fn build_config_text(project_root: &Path) -> Result<String, DecreeError> {
    let path = project_root
        .join(config::DECREE_DIR)
        .join(config::CONFIG_FILE);

    let content = std::fs::read_to_string(&path)?;
    Ok(content)
}

/// Interactive guided flow: preview → action prompt.
fn guided_flow(project_root: &Path, prompt_text: &str) -> Result<(), DecreeError> {
    // Step 2: Preview — print the full prompt
    println!("{prompt_text}");

    // Step 3: Action prompt
    println!();
    println!("Press C to copy to clipboard, or Enter to launch AI:");

    // Read a single keypress
    let key = read_single_key()?;

    match key {
        b'c' | b'C' => {
            copy_to_clipboard(prompt_text)?;
            println!("Prompt copied to clipboard.");
            Ok(())
        }
        b'\n' | b'\r' => {
            launch_ai(project_root, prompt_text)
        }
        _ => {
            // Any other key or Ctrl-C: exit without action
            Ok(())
        }
    }
}

/// Read a single key from stdin (raw mode).
fn read_single_key() -> Result<u8, DecreeError> {
    // Put terminal in raw mode to read single key
    let mut buf = [0u8; 1];
    set_raw_mode(true)?;
    let result = io::stdin().read(&mut buf);
    set_raw_mode(false)?;
    result.map_err(DecreeError::Io)?;
    Ok(buf[0])
}

/// Set terminal raw mode on/off.
fn set_raw_mode(raw: bool) -> Result<(), DecreeError> {
    use std::process::Command;
    if raw {
        Command::new("stty")
            .arg("-echo")
            .arg("-icanon")
            .arg("min")
            .arg("1")
            .stdin(std::process::Stdio::inherit())
            .status()
            .map_err(|e| DecreeError::Other(format!("stty failed: {e}")))?;
    } else {
        Command::new("stty")
            .arg("echo")
            .arg("icanon")
            .stdin(std::process::Stdio::inherit())
            .status()
            .map_err(|e| DecreeError::Other(format!("stty failed: {e}")))?;
    }
    Ok(())
}

/// Copy text to system clipboard.
fn copy_to_clipboard(text: &str) -> Result<(), DecreeError> {
    // Try platform-appropriate clipboard commands
    let commands: &[&[&str]] = &[
        &["xclip", "-selection", "clipboard"],
        &["xsel", "--clipboard", "--input"],
        &["pbcopy"],
        &["clip.exe"],
    ];

    for cmd_args in commands {
        let cmd = cmd_args[0];
        let args = &cmd_args[1..];

        if let Ok(mut child) = std::process::Command::new(cmd)
            .args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            if let Some(ref mut stdin) = child.stdin {
                use std::io::Write;
                let _ = stdin.write_all(text.as_bytes());
            }
            if let Ok(status) = child.wait() {
                if status.success() {
                    return Ok(());
                }
            }
        }
    }

    Err(DecreeError::Other(
        "no clipboard command available (tried xclip, xsel, pbcopy, clip.exe)".into(),
    ))
}

/// Launch interactive AI via exec.
fn launch_ai(project_root: &Path, _prompt_text: &str) -> Result<(), DecreeError> {
    let config = AppConfig::load_from_project(project_root)?;
    let ai_interactive = config.commands.ai_interactive.clone();

    let parts: Vec<&str> = ai_interactive.split_whitespace().collect();
    if parts.is_empty() {
        return Err(DecreeError::Other("ai_interactive command is empty".into()));
    }

    let cmd = parts[0];
    let args = &parts[1..];

    // exec replaces the current process
    let err = std::process::Command::new(cmd)
        .args(args)
        .current_dir(project_root)
        .exec();

    // exec() only returns on error
    Err(DecreeError::Other(format!("failed to exec {cmd}: {err}")))
}

/// Handle unknown prompt: fuzzy match or list available.
fn prompt_not_found(name: &str, prompts: &[PromptInfo]) -> Result<(), DecreeError> {
    // Build RoutineInfo-compatible list for fuzzy matching
    let routine_infos: Vec<message::RoutineInfo> = prompts
        .iter()
        .map(|p| message::RoutineInfo {
            name: p.name.clone(),
            description: p.description.clone(),
        })
        .collect();

    if let Some(suggestion) = routine::find_closest_routine(name, &routine_infos, 3) {
        Err(DecreeError::Other(format!(
            "unknown prompt '{name}'\n\nDid you mean '{suggestion}'?"
        )))
    } else {
        let mut msg = format!("unknown prompt '{name}'\n\nAvailable prompts:");
        for p in prompts {
            if p.description.is_empty() {
                msg.push_str(&format!("\n  {}", p.name));
            } else {
                msg.push_str(&format!("\n  {:<16} {}", p.name, p.description));
            }
        }
        Err(DecreeError::Other(msg))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup_decree_dir(dir: &TempDir) {
        let decree = dir.path().join(".decree");
        std::fs::create_dir_all(decree.join("prompts")).unwrap();
        std::fs::create_dir_all(decree.join("routines")).unwrap();
        std::fs::create_dir_all(decree.join("migrations")).unwrap();
        std::fs::create_dir_all(decree.join("runs")).unwrap();
        std::fs::write(decree.join("processed.md"), "").unwrap();
        std::fs::write(
            decree.join("config.yml"),
            "commands:\n  ai_router: echo\n  ai_interactive: echo\n",
        )
        .unwrap();
    }

    #[test]
    fn test_extract_description_basic() {
        assert_eq!(
            extract_description("# Migration Template\n\nContent here."),
            "# Migration Template"
        );
    }

    #[test]
    fn test_extract_description_truncated() {
        let long_line = "x".repeat(80);
        let desc = extract_description(&long_line);
        assert_eq!(desc.len(), 60); // 57 + "..."
        assert!(desc.ends_with("..."));
    }

    #[test]
    fn test_extract_description_empty() {
        assert_eq!(extract_description(""), "");
    }

    #[test]
    fn test_extract_description_blank_lines_first() {
        assert_eq!(
            extract_description("\n\n# Title\nBody"),
            "# Title"
        );
    }

    #[test]
    fn test_list_prompts() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);

        std::fs::write(
            dir.path().join(".decree/prompts/migration.md"),
            "# Migration\n\nTemplate.",
        )
        .unwrap();
        std::fs::write(
            dir.path().join(".decree/prompts/sow.md"),
            "# Statement of Work\n\nSOW.",
        )
        .unwrap();

        let config = AppConfig::load_from_project(dir.path()).unwrap();
        let prompts = list_prompts(dir.path(), &config).unwrap();
        assert_eq!(prompts.len(), 2);
        assert_eq!(prompts[0].name, "migration");
        assert_eq!(prompts[1].name, "sow");
    }

    #[test]
    fn test_substitute_variables_none() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);

        let result = substitute_variables(dir.path(), "No variables here.").unwrap();
        assert_eq!(result, "No variables here.");
    }

    #[test]
    fn test_substitute_variables_processed() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);

        let result = substitute_variables(dir.path(), "Processed: {processed}").unwrap();
        assert_eq!(result, "Processed: None yet");
    }

    #[test]
    fn test_substitute_variables_unknown_left_as_is() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);

        let result = substitute_variables(dir.path(), "Unknown: {foobar}").unwrap();
        assert_eq!(result, "Unknown: {foobar}");
    }

    #[test]
    fn test_build_migrations_text_empty() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);

        let text = build_migrations_text(dir.path()).unwrap();
        assert_eq!(text, "None yet");
    }

    #[test]
    fn test_build_migrations_text_with_files() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);

        std::fs::write(
            dir.path().join(".decree/migrations/01-auth.md"),
            "# Add Authentication\n\nDetails.",
        )
        .unwrap();

        let text = build_migrations_text(dir.path()).unwrap();
        assert!(text.contains("01-auth.md"));
        assert!(text.contains("Add Authentication"));
    }

    #[test]
    fn test_build_routines_text_empty() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);

        let text = build_routines_text(dir.path()).unwrap();
        assert_eq!(text, "None yet");
    }

    #[test]
    fn test_prompt_not_found_fuzzy() {
        let prompts = vec![PromptInfo {
            name: "migration".to_string(),
            description: "Template".to_string(),
        }];

        let err = prompt_not_found("migraton", &prompts).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("Did you mean 'migration'?"));
    }

    #[test]
    fn test_prompt_not_found_no_match() {
        let prompts = vec![PromptInfo {
            name: "migration".to_string(),
            description: "Template".to_string(),
        }];

        let err = prompt_not_found("xyz", &prompts).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("Available prompts:"));
        assert!(msg.contains("migration"));
    }
}
