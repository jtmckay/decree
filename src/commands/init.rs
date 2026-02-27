use std::fs;
use std::io::Write;
use std::path::Path;

use indicatif::{ProgressBar, ProgressStyle};
use inquire::Select;

use crate::config::{AiProvider, Config};
use crate::error::DecreeError;

const GGUF_URL: &str = "https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct-GGUF/resolve/main/qwen2.5-1.5b-instruct-q5_k_m.gguf";

// Embedded templates
pub const TEMPLATE_SOW: &str = include_str!("../templates/sow.md");
pub const TEMPLATE_SPEC: &str = include_str!("../templates/spec.md");
pub const TEMPLATE_GITIGNORE: &str = include_str!("../templates/gitignore");
pub const TEMPLATE_DEVELOP_SH: &str = include_str!("../templates/develop.sh");
pub const TEMPLATE_DEVELOP_IPYNB: &str = include_str!("../templates/develop.ipynb");
pub const TEMPLATE_RUST_DEVELOP_SH: &str = include_str!("../templates/rust-develop.sh");
pub const TEMPLATE_RUST_DEVELOP_IPYNB: &str = include_str!("../templates/rust-develop.ipynb");

/// Get the AI command and allowed tools string for a given provider.
pub fn ai_cmd_and_tools(provider: AiProvider) -> (&'static str, &'static str) {
    match provider {
        AiProvider::ClaudeCli => (
            "claude -p",
            "--allowedTools 'Edit,Write,Bash(cargo*),Bash(npm*),Bash(python*),Bash(make*)'",
        ),
        AiProvider::CopilotCli => (
            "copilot -p",
            "--allowedTools 'Edit,Write,Bash(cargo*),Bash(npm*),Bash(python*),Bash(make*)'",
        ),
        AiProvider::Embedded => ("decree ai -p", ""),
    }
}

/// Replace `{AI_CMD}` and `{ALLOWED_TOOLS}` placeholders in a template.
pub fn render_template(template: &str, ai_cmd: &str, allowed_tools: &str) -> String {
    template
        .replace("{AI_CMD}", ai_cmd)
        .replace("{ALLOWED_TOOLS}", allowed_tools)
}

/// Write a file only if it doesn't already exist. Returns true if written.
pub fn write_if_absent(path: &Path, content: &str) -> Result<bool, DecreeError> {
    if path.exists() {
        eprintln!("  exists: {}", path.display());
        return Ok(false);
    }
    fs::write(path, content)?;
    Ok(true)
}

/// Write a file and set executable permissions (Unix only).
pub fn write_executable(path: &Path, content: &str) -> Result<bool, DecreeError> {
    if path.exists() {
        eprintln!("  exists: {}", path.display());
        return Ok(false);
    }
    fs::write(path, content)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
    }
    Ok(true)
}

pub fn run(model_path_override: Option<&str>) -> Result<(), DecreeError> {
    let project_root = std::env::current_dir()?;
    let decree_dir = project_root.join(".decree");

    println!("Initializing decree project...");

    // --- Create directory structure ---
    let dirs = [
        decree_dir.clone(),
        decree_dir.join("routines"),
        decree_dir.join("plans"),
        decree_dir.join("cron"),
        decree_dir.join("inbox"),
        decree_dir.join("inbox/done"),
        decree_dir.join("inbox/dead"),
        decree_dir.join("runs"),
        decree_dir.join("sessions"),
    ];
    for dir in &dirs {
        fs::create_dir_all(dir)?;
    }

    // specs/ at project root
    let specs_dir = project_root.join("specs");
    fs::create_dir_all(&specs_dir)?;
    let processed = specs_dir.join("processed-spec.md");
    if !processed.exists() {
        fs::write(&processed, "")?;
    }

    // --- AI provider selection ---
    let planning_choices = vec![
        AiProvider::ClaudeCli,
        AiProvider::CopilotCli,
        AiProvider::Embedded,
    ];
    let planning_provider = Select::new(
        "Planning AI — which AI handles `decree plan`?",
        planning_choices,
    )
    .with_help_message("↑↓ to move, type to filter, Enter to select")
    .prompt()
    .map_err(|e| DecreeError::Config(format!("selection cancelled: {e}")))?;

    let router_choices = vec![
        AiProvider::Embedded,
        AiProvider::ClaudeCli,
        AiProvider::CopilotCli,
    ];
    let router_provider = Select::new(
        "Router AI — which AI selects routines for messages?",
        router_choices,
    )
    .with_help_message("↑↓ to move, type to filter, Enter to select")
    .prompt()
    .map_err(|e| DecreeError::Config(format!("selection cancelled: {e}")))?;

    // --- Build config ---
    let model_path = model_path_override
        .map(String::from)
        .unwrap_or_else(|| "~/.decree/models/qwen2.5-1.5b-instruct-q5_k_m.gguf".to_string());

    let mut config = Config {
        ai: crate::config::AiConfig {
            model_path,
            n_gpu_layers: 0,
        },
        commands: crate::config::CommandsConfig {
            planning: planning_provider.planning_command().to_string(),
            planning_continue: planning_provider.planning_continue_command().to_string(),
            router: router_provider.router_command().to_string(),
        },
        ..Config::default()
    };

    // --- GGUF model download ---
    let resolved_path = config.resolved_model_path();
    if !resolved_path.exists() {
        println!();
        let confirm = inquire::Confirm::new(
            "Model not found. Download Qwen 2.5 1.5B-Instruct Q5_K_M (~1.1 GB)?",
        )
        .with_default(true)
        .prompt()
        .unwrap_or(false);

        if confirm {
            download_model(&resolved_path)?;
        } else {
            println!("You can download the model manually from:");
            println!("  {GGUF_URL}");
            println!("Place it at: {}", resolved_path.display());
        }
    }

    // --- Notebook support ---
    println!();
    let notebook_support =
        inquire::Confirm::new("Enable Jupyter Notebook routine support? (requires Python 3)")
            .with_default(false)
            .prompt()
            .unwrap_or(false);
    config.notebook_support = notebook_support;

    // --- Write config ---
    config.save(&project_root)?;
    println!("  wrote: .decree/config.yml");

    // --- Write templates ---
    write_if_absent(&decree_dir.join(".gitignore"), TEMPLATE_GITIGNORE)?;
    write_if_absent(&decree_dir.join("plans/sow.md"), TEMPLATE_SOW)?;
    write_if_absent(&decree_dir.join("plans/spec.md"), TEMPLATE_SPEC)?;

    // Render routines with the selected planning AI's command
    let (ai_cmd, allowed_tools) = ai_cmd_and_tools(planning_provider);

    write_executable(
        &decree_dir.join("routines/develop.sh"),
        &render_template(TEMPLATE_DEVELOP_SH, ai_cmd, allowed_tools),
    )?;
    write_executable(
        &decree_dir.join("routines/rust-develop.sh"),
        &render_template(TEMPLATE_RUST_DEVELOP_SH, ai_cmd, allowed_tools),
    )?;

    if notebook_support {
        write_if_absent(
            &decree_dir.join("routines/develop.ipynb"),
            &render_template(TEMPLATE_DEVELOP_IPYNB, ai_cmd, allowed_tools),
        )?;
        write_if_absent(
            &decree_dir.join("routines/rust-develop.ipynb"),
            &render_template(TEMPLATE_RUST_DEVELOP_IPYNB, ai_cmd, allowed_tools),
        )?;
    }

    println!("\ndecree project initialized.");
    Ok(())
}

fn download_model(dest: &Path) -> Result<(), DecreeError> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }

    println!("Downloading model from:");
    println!("  {GGUF_URL}");

    let response = reqwest::blocking::get(GGUF_URL)
        .map_err(|e| DecreeError::Config(format!("download failed: {e}")))?;

    if !response.status().is_success() {
        return Err(DecreeError::Config(format!(
            "download failed: HTTP {}",
            response.status()
        )));
    }

    let total_size = response.content_length().unwrap_or(0);

    let pb = ProgressBar::new(total_size);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {bytes}/{total_bytes} ({eta})")
            .expect("valid template")
            .progress_chars("#>-"),
    );

    let tmp_path = dest.with_extension("part");
    let mut file = fs::File::create(&tmp_path)?;
    let mut downloaded: u64 = 0;

    let mut reader = response;
    let mut buf = [0u8; 8192];
    loop {
        use std::io::Read;
        let n = reader
            .read(&mut buf)
            .map_err(|e| DecreeError::Config(format!("download read error: {e}")))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
        downloaded += n as u64;
        pb.set_position(downloaded);
    }

    pb.finish_with_message("download complete");
    drop(file);
    fs::rename(&tmp_path, dest)?;
    println!("Model saved to: {}", dest.display());

    Ok(())
}
