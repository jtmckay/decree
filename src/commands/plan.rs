use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::process::Command;

use inquire::Select;

use crate::config::Config;
use crate::error::{find_project_root, DecreeError};
use crate::llm::{self, ChatMessage, DEFAULT_CTX_SIZE};
use crate::session::Session;
use crate::spec;

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Run the `decree plan` command.
///
/// If `plan_name` is provided, use that plan template directly.
/// Otherwise, present a fuzzy selector of available plans.
pub fn run(plan_name: Option<&str>) -> Result<(), DecreeError> {
    let root = find_project_root()?;
    let config = Config::load(&root)?;

    let plans_dir = root.join(".decree/plans");
    let available = list_plans(&plans_dir)?;

    if available.is_empty() {
        return Err(DecreeError::Config(
            "no plan templates found in .decree/plans/".into(),
        ));
    }

    // Resolve plan name
    let selected = match plan_name {
        Some(name) => {
            if !available.contains(&name.to_string()) {
                let list = available.join(", ");
                return Err(DecreeError::Config(format!(
                    "plan '{name}' not found. available plans: {list}"
                )));
            }
            name.to_string()
        }
        None => {
            let choice = Select::new("Select a plan template:", available.clone())
                .with_help_message("↑↓ to move, type to filter, Enter to select")
                .prompt()
                .map_err(|e| DecreeError::Config(format!("selection cancelled: {e}")))?;
            choice
        }
    };

    // Load plan template
    let template_path = plans_dir.join(format!("{selected}.md"));
    let template_content = fs::read_to_string(&template_path).map_err(|e| {
        DecreeError::Config(format!(
            "failed to read plan template {}: {e}",
            template_path.display()
        ))
    })?;

    // Build planning prompt
    let prompt = build_planning_prompt(&root, &template_content)?;

    // Dispatch: embedded vs external
    let planning_cmd = &config.commands.planning;

    if planning_cmd == "decree ai" || planning_cmd.starts_with("decree ai ") {
        run_embedded_plan(&root, &config, &prompt)
    } else {
        run_external_plan(&config, &prompt)
    }
}

// ---------------------------------------------------------------------------
// Plan listing
// ---------------------------------------------------------------------------

/// List available plan names from `.decree/plans/` (filenames without `.md`).
pub fn list_plans(plans_dir: &Path) -> Result<Vec<String>, DecreeError> {
    if !plans_dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut names = Vec::new();
    for entry in fs::read_dir(plans_dir)? {
        let entry = entry?;
        let filename = entry.file_name().to_string_lossy().to_string();
        if let Some(stem) = filename.strip_suffix(".md") {
            names.push(stem.to_string());
        }
    }
    names.sort();
    Ok(names)
}

// ---------------------------------------------------------------------------
// Prompt construction
// ---------------------------------------------------------------------------

/// Build the planning prompt from the selected template and project state.
fn build_planning_prompt(
    project_root: &Path,
    plan_template: &str,
) -> Result<String, DecreeError> {
    let specs_summary = build_specs_summary(project_root)?;

    Ok(format!(
        "You are a planning assistant for a software project.\n\
         \n\
         ## Plan Template\n\
         {plan_template}\n\
         \n\
         ## Existing Specs\n\
         {specs_summary}\n\
         \n\
         ## User Request\n\
         The user will describe their goals interactively.\n\
         \n\
         ## Instructions\n\
         1. Analyse the request and existing project state.\n\
         2. Present a numbered plan summary with proposed spec files.\n\
         3. WAIT for the user to approve or request changes -- do NOT generate\n\
            spec files until explicitly told to proceed.\n\
         4. When approved, generate each spec file using the template format:\n\
            - Filename: NN-descriptive-name.spec.md\n\
            - Include YAML frontmatter with `routine:` field\n\
            - Write each file to the specs/ directory"
    ))
}

/// Build a summary of existing spec files (filename + first heading).
fn build_specs_summary(project_root: &Path) -> Result<String, DecreeError> {
    let specs = spec::list_specs(project_root)?;
    if specs.is_empty() {
        return Ok("None yet".to_string());
    }

    let mut lines = Vec::new();
    let specs_dir = project_root.join("specs");
    for name in &specs {
        let path = specs_dir.join(name);
        let title = extract_spec_title(&path);
        lines.push(format!("- {name}: {title}"));
    }
    Ok(lines.join("\n"))
}

/// Extract the first markdown heading from a spec file, or fall back to the filename.
fn extract_spec_title(path: &Path) -> String {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return "(unreadable)".to_string(),
    };

    // Skip YAML frontmatter
    let body = skip_frontmatter(&content);

    // Find first # heading
    for line in body.lines() {
        let trimmed = line.trim();
        if let Some(heading) = trimmed.strip_prefix('#') {
            let heading = heading.trim_start_matches('#').trim();
            if !heading.is_empty() {
                return heading.to_string();
            }
        }
    }

    "(no title)".to_string()
}

/// Skip YAML frontmatter (--- ... ---) and return the remaining content.
fn skip_frontmatter(content: &str) -> &str {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return content;
    }
    let after_open = &trimmed[3..];
    match after_open.find("\n---") {
        Some(pos) => {
            let rest = &after_open[pos + 4..];
            // Skip the newline after closing ---
            rest.strip_prefix('\n').unwrap_or(rest)
        }
        None => content,
    }
}

// ---------------------------------------------------------------------------
// External AI path
// ---------------------------------------------------------------------------

/// Run the planning session using an external AI command.
///
/// 1. Inject the prompt via `commands.planning`
/// 2. Exec into `commands.planning_continue` for interactive continuation
fn run_external_plan(config: &Config, prompt: &str) -> Result<(), DecreeError> {
    let planning_cmd = &config.commands.planning;

    // Step 1: Seed the conversation with the planning prompt
    if planning_cmd.contains("{prompt}") {
        let escaped = shell_escape(prompt);
        let full_cmd = planning_cmd.replace("{prompt}", &escaped);

        let status = Command::new("bash")
            .arg("-c")
            .arg(&full_cmd)
            .status()
            .map_err(|e| DecreeError::Config(format!("planning command failed: {e}")))?;

        if !status.success() {
            eprintln!(
                "warning: planning command exited with {}",
                status.code().unwrap_or(-1)
            );
        }
    } else {
        return Err(DecreeError::Config(format!(
            "planning command missing {{prompt}} placeholder: {planning_cmd}"
        )));
    }

    // Step 2: Continue the session interactively
    let continue_cmd = &config.commands.planning_continue;

    if continue_cmd.is_empty() {
        // No continue command — fallback: we already ran the prompt above
        return Ok(());
    }

    // Exec into the continue command (process replacement on Unix)
    exec_into(continue_cmd)
}

/// Replace the current process with the given shell command (Unix exec).
/// On non-Unix platforms, spawns the command and waits.
fn exec_into(cmd: &str) -> Result<(), DecreeError> {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // This replaces the current process — does not return on success
        let err = Command::new("bash").arg("-c").arg(cmd).exec();
        // If we get here, exec failed
        Err(DecreeError::Config(format!(
            "exec into continue command failed: {err}"
        )))
    }

    #[cfg(not(unix))]
    {
        let status = Command::new("cmd")
            .args(["/C", cmd])
            .status()
            .map_err(|e| DecreeError::Config(format!("continue command failed: {e}")))?;

        if !status.success() {
            eprintln!(
                "warning: continue command exited with {}",
                status.code().unwrap_or(-1)
            );
        }
        Ok(())
    }
}

/// Escape a string for safe embedding in a single-quoted shell argument.
fn shell_escape(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

// ---------------------------------------------------------------------------
// Embedded AI path
// ---------------------------------------------------------------------------

/// Run the planning session using the embedded AI (llama.cpp).
///
/// 1. Load model, send the planning prompt, display response
/// 2. Enter interactive REPL (reusing context functions from ai module)
fn run_embedded_plan(
    root: &Path,
    config: &Config,
    planning_prompt: &str,
) -> Result<(), DecreeError> {
    let model_path = llm::ensure_model(config)?;
    let backend = llm::init_backend(true)?;
    let model = llm::load_model(&backend, &model_path, config.ai.n_gpu_layers)?;
    let ctx_size = DEFAULT_CTX_SIZE;
    let max_gen = DEFAULT_CTX_SIZE;
    let mut ctx = llm::create_context(&model, &backend, ctx_size)?;

    let mut session = Session::new();
    println!("decree plan — interactive planning session (type 'exit' or Ctrl-D to quit)");
    println!("session: {}", session.id);

    // Build initial messages: system = planning prompt, user = initial instruction
    let system_msg = ChatMessage {
        role: "system".to_string(),
        content: planning_prompt.to_string(),
    };

    let initial_user_msg = ChatMessage {
        role: "user".to_string(),
        content: "I'm ready to plan. Please acknowledge that you've loaded the project context \
                  and are ready for my goals."
            .to_string(),
    };

    session.history.push(initial_user_msg.clone());
    let mut working_history: Vec<ChatMessage> = vec![initial_user_msg];

    // Generate initial response with the planning context
    {
        let mut messages = vec![system_msg.clone()];
        messages.extend(working_history.iter().cloned());

        let prompt_text = llm::build_chatml(&messages, true);
        let tokens = llm::tokenize(&model, &prompt_text, false)?;
        ctx.clear_kv_cache();

        let (output, _stats) = llm::generate(&mut ctx, &model, &tokens, max_gen, |piece| {
            print!("{piece}");
            let _ = io::stdout().flush();
        })?;

        if !output.ends_with('\n') {
            println!();
        }
        println!();

        let assistant_msg = ChatMessage {
            role: "assistant".to_string(),
            content: output,
        };
        session.history.push(assistant_msg.clone());
        working_history.push(assistant_msg);
        session.save(root)?;
    }

    // Interactive REPL loop
    let stdin = io::stdin();
    let mut reader = io::BufRead::lines(stdin.lock());

    loop {
        let pct = super::ai::context_usage_pct(&model, &working_history, ctx_size)?;
        print!("[{pct}%] plan> ");
        io::stdout().flush().map_err(DecreeError::Io)?;

        let line = match reader.next() {
            Some(Ok(line)) => line,
            Some(Err(e)) => return Err(DecreeError::Io(e)),
            None => {
                // EOF (Ctrl-D)
                println!();
                break;
            }
        };

        let input = line.trim().to_string();
        if input.is_empty() {
            continue;
        }
        if input == "exit" || input == "quit" {
            break;
        }

        let user_msg = ChatMessage {
            role: "user".to_string(),
            content: input,
        };
        session.history.push(user_msg.clone());
        working_history.push(user_msg);

        // Truncate working history if needed
        let dropped =
            super::ai::truncate_history(&model, &mut working_history, ctx_size, max_gen)?;
        if dropped > 0 {
            println!(
                "~ context: dropped {} earliest messages (history exceeded context window)",
                dropped
            );
        }

        // Build full prompt
        let mut messages = vec![system_msg.clone()];
        messages.extend(working_history.iter().cloned());

        let prompt_text = llm::build_chatml(&messages, true);
        let tokens = llm::tokenize(&model, &prompt_text, false)?;
        ctx.clear_kv_cache();

        let (output, _stats) = llm::generate(&mut ctx, &model, &tokens, max_gen, |piece| {
            print!("{piece}");
            let _ = io::stdout().flush();
        })?;

        if !output.ends_with('\n') {
            println!();
        }
        println!();

        let assistant_msg = ChatMessage {
            role: "assistant".to_string(),
            content: output,
        };
        session.history.push(assistant_msg.clone());
        working_history.push(assistant_msg);
        session.save(root)?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup_project(tmp: &TempDir) {
        let root = tmp.path();
        fs::create_dir_all(root.join(".decree/plans")).unwrap();
        fs::create_dir_all(root.join(".decree/sessions")).unwrap();
        fs::create_dir_all(root.join("specs")).unwrap();

        // Write a minimal config
        let config = Config::default();
        config.save(root).unwrap();
    }

    #[test]
    fn test_list_plans_empty() {
        let tmp = TempDir::new().unwrap();
        let plans_dir = tmp.path().join("plans");
        fs::create_dir_all(&plans_dir).unwrap();
        let plans = list_plans(&plans_dir).unwrap();
        assert!(plans.is_empty());
    }

    #[test]
    fn test_list_plans_finds_md_files() {
        let tmp = TempDir::new().unwrap();
        let plans_dir = tmp.path().join("plans");
        fs::create_dir_all(&plans_dir).unwrap();

        fs::write(plans_dir.join("sow.md"), "# SOW").unwrap();
        fs::write(plans_dir.join("spec.md"), "# Spec").unwrap();
        fs::write(plans_dir.join("notes.txt"), "not a plan").unwrap();

        let plans = list_plans(&plans_dir).unwrap();
        assert_eq!(plans, vec!["sow", "spec"]);
    }

    #[test]
    fn test_list_plans_sorted_alphabetically() {
        let tmp = TempDir::new().unwrap();
        let plans_dir = tmp.path().join("plans");
        fs::create_dir_all(&plans_dir).unwrap();

        fs::write(plans_dir.join("z-plan.md"), "").unwrap();
        fs::write(plans_dir.join("a-plan.md"), "").unwrap();
        fs::write(plans_dir.join("m-plan.md"), "").unwrap();

        let plans = list_plans(&plans_dir).unwrap();
        assert_eq!(plans, vec!["a-plan", "m-plan", "z-plan"]);
    }

    #[test]
    fn test_list_plans_nonexistent_dir() {
        let tmp = TempDir::new().unwrap();
        let plans_dir = tmp.path().join("no-such-dir");
        let plans = list_plans(&plans_dir).unwrap();
        assert!(plans.is_empty());
    }

    #[test]
    fn test_build_specs_summary_no_specs() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("specs")).unwrap();

        let summary = build_specs_summary(root).unwrap();
        assert_eq!(summary, "None yet");
    }

    #[test]
    fn test_build_specs_summary_with_specs() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("specs")).unwrap();

        fs::write(
            root.join("specs/01-init.spec.md"),
            "---\nroutine: develop\n---\n# 01: Initialize Project\n\nDetails here.",
        )
        .unwrap();
        fs::write(
            root.join("specs/02-auth.spec.md"),
            "# 02: Add Authentication\n\nAuth stuff.",
        )
        .unwrap();

        let summary = build_specs_summary(root).unwrap();
        assert!(summary.contains("01-init.spec.md: 01: Initialize Project"));
        assert!(summary.contains("02-auth.spec.md: 02: Add Authentication"));
    }

    #[test]
    fn test_extract_spec_title_with_frontmatter() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("test.spec.md");
        fs::write(
            &path,
            "---\nroutine: develop\n---\n# 01: My Title\n\nBody.",
        )
        .unwrap();

        assert_eq!(extract_spec_title(&path), "01: My Title");
    }

    #[test]
    fn test_extract_spec_title_no_frontmatter() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("test.spec.md");
        fs::write(&path, "# Simple Title\n\nBody text.").unwrap();

        assert_eq!(extract_spec_title(&path), "Simple Title");
    }

    #[test]
    fn test_extract_spec_title_no_heading() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("test.spec.md");
        fs::write(&path, "Just some text without headings.").unwrap();

        assert_eq!(extract_spec_title(&path), "(no title)");
    }

    #[test]
    fn test_extract_spec_title_multi_hash_heading() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("test.spec.md");
        fs::write(&path, "## Sub Heading\n\nBody.").unwrap();

        assert_eq!(extract_spec_title(&path), "Sub Heading");
    }

    #[test]
    fn test_skip_frontmatter_with_fm() {
        let content = "---\nroutine: develop\n---\n# Title\nBody";
        let body = skip_frontmatter(content);
        assert!(body.starts_with("# Title"));
    }

    #[test]
    fn test_skip_frontmatter_without_fm() {
        let content = "# Title\nBody";
        let body = skip_frontmatter(content);
        assert_eq!(body, content);
    }

    #[test]
    fn test_skip_frontmatter_unclosed() {
        let content = "---\nroutine: develop\n# Title";
        let body = skip_frontmatter(content);
        // Unclosed frontmatter returns original content
        assert_eq!(body, content);
    }

    #[test]
    fn test_build_planning_prompt_includes_template() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("specs")).unwrap();

        let prompt =
            build_planning_prompt(root, "# My Custom Plan\n\nDo things.").unwrap();

        assert!(prompt.contains("You are a planning assistant"));
        assert!(prompt.contains("# My Custom Plan"));
        assert!(prompt.contains("Do things."));
        assert!(prompt.contains("None yet"));
        assert!(prompt.contains("WAIT for the user to approve"));
    }

    #[test]
    fn test_build_planning_prompt_includes_existing_specs() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("specs")).unwrap();
        fs::write(
            root.join("specs/01-setup.spec.md"),
            "# 01: Setup\n\nInitial setup.",
        )
        .unwrap();

        let prompt = build_planning_prompt(root, "# Plan").unwrap();

        assert!(prompt.contains("01-setup.spec.md: 01: Setup"));
        assert!(!prompt.contains("None yet"));
    }

    #[test]
    fn test_shell_escape_simple() {
        assert_eq!(shell_escape("hello world"), "'hello world'");
    }

    #[test]
    fn test_shell_escape_quotes() {
        assert_eq!(shell_escape("it's"), "'it'\\''s'");
    }

    #[test]
    fn test_plan_not_found_error() {
        let tmp = TempDir::new().unwrap();
        setup_project(&tmp);

        fs::write(tmp.path().join(".decree/plans/sow.md"), "# SOW").unwrap();
        fs::write(tmp.path().join(".decree/plans/spec.md"), "# Spec").unwrap();

        let plans_dir = tmp.path().join(".decree/plans");
        let available = list_plans(&plans_dir).unwrap();

        assert!(available.contains(&"sow".to_string()));
        assert!(available.contains(&"spec".to_string()));
        assert!(!available.contains(&"nonexistent".to_string()));
    }

    #[test]
    fn test_no_plans_available() {
        let tmp = TempDir::new().unwrap();
        let plans_dir = tmp.path().join(".decree/plans");
        fs::create_dir_all(&plans_dir).unwrap();

        let plans = list_plans(&plans_dir).unwrap();
        assert!(plans.is_empty());
    }
}
