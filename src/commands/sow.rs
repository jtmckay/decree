use std::process::Command;

use crate::config::Config;
use crate::error::{find_project_root, DecreeError};

pub fn run() -> Result<(), DecreeError> {
    let root = find_project_root()?;
    let config = Config::load(&root)?;

    let sow_template_path = root.join(".decree/plans/sow.md");
    if !sow_template_path.exists() {
        return Err(DecreeError::Config(
            ".decree/plans/sow.md not found — run `decree init` first".into(),
        ));
    }

    let specs_dir = root.join("specs");
    if !specs_dir.is_dir() {
        return Err(DecreeError::NoSpecs);
    }

    // Check that at least one spec file exists
    let has_specs = std::fs::read_dir(&specs_dir)?.any(|entry| {
        entry
            .ok()
            .and_then(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                if name.ends_with(".spec.md") {
                    Some(())
                } else {
                    None
                }
            })
            .is_some()
    });
    if !has_specs {
        return Err(DecreeError::NoSpecs);
    }

    let sow_template = std::fs::read_to_string(&sow_template_path)?;

    let prompt = format!(
        "You are generating a Statement of Work (SOW) for a project.\n\n\
         Use the following SOW template as structural guidance:\n\n\
         {sow_template}\n\n\
         Read all spec files in the `{specs_dir}` directory to understand what \
         has been built or is planned. Then synthesize a coherent SOW that captures \
         the business intent behind the full body of work.\n\n\
         Output only the SOW content in markdown format.",
        specs_dir = specs_dir.display()
    );

    let planning_cmd = &config.commands.planning;

    if planning_cmd.contains("{prompt}") {
        // External CLI command with prompt injection
        let full_cmd = planning_cmd.replace("{prompt}", &shell_escape(&prompt));
        println!("Running planning AI...");

        let output = Command::new("bash")
            .arg("-c")
            .arg(&full_cmd)
            .output()
            .map_err(|e| DecreeError::Config(format!("failed to run planning command: {e}")))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        if !output.status.success() {
            eprintln!("{stderr}");
            return Err(DecreeError::Config(format!(
                "planning command failed with exit code: {}",
                output.status
            )));
        }

        let sow_output = root.join("sow.md");
        std::fs::write(&sow_output, stdout.as_ref())?;
        println!("SOW written to: {}", sow_output.display());
    } else if planning_cmd == "decree ai" {
        // Embedded AI — placeholder for spec 03
        println!("Embedded AI (decree ai) for planning is not yet implemented.");
        println!("Use `claude -p` or `copilot -p` as the planning AI.");
        return Err(DecreeError::Config(
            "embedded planning AI not yet available".into(),
        ));
    } else {
        return Err(DecreeError::Config(format!(
            "unrecognized planning command: {planning_cmd}"
        )));
    }

    Ok(())
}

/// Escape a string for safe embedding in a shell command.
fn shell_escape(s: &str) -> String {
    // Use single quotes, escaping any embedded single quotes
    format!("'{}'", s.replace('\'', "'\\''"))
}
