use crate::config;
use crate::error::color;
use crate::error::DecreeError;
use crate::message;
use std::path::Path;

/// Run `decree log [ID]`.
pub fn run(project_root: &Path, id: Option<&str>) -> Result<(), DecreeError> {
    let runs = message::list_runs(project_root)?;

    match id {
        None => {
            if runs.is_empty() {
                println!("No runs found.");
                return Ok(());
            }
            run_no_id(project_root, &runs)
        }
        Some(query) => run_with_id(project_root, query),
    }
}

/// No ID provided.
fn run_no_id(project_root: &Path, runs: &[String]) -> Result<(), DecreeError> {
    if color::is_tty() {
        // TTY: arrow-key selector (most recent first)
        let mut options: Vec<String> = runs.to_vec();
        options.reverse();
        let selection = inquire::Select::new("Select run:", options)
            .prompt()
            .map_err(|e| DecreeError::Other(format!("selection cancelled: {e}")))?;
        display_run_logs(project_root, &selection)
    } else {
        // Non-TTY: print most recent
        if let Some(latest) = runs.last() {
            display_run_logs(project_root, latest)
        } else {
            Ok(())
        }
    }
}

/// ID provided (may be full, chain, or prefix).
fn run_with_id(project_root: &Path, query: &str) -> Result<(), DecreeError> {
    let matches = message::find_matching_runs(project_root, query)?;

    match matches.len() {
        0 => Err(DecreeError::MessageNotFound(query.to_string())),
        1 => display_run_logs(project_root, &matches[0]),
        _ => {
            if color::is_tty() {
                // Ambiguous + TTY: arrow-key selector
                let selection = inquire::Select::new("Multiple matches — select run:", matches)
                    .prompt()
                    .map_err(|e| DecreeError::Other(format!("selection cancelled: {e}")))?;
                display_run_logs(project_root, &selection)
            } else {
                // Ambiguous + Non-TTY: list all
                for m in &matches {
                    display_run_logs(project_root, m)?;
                    println!();
                }
                Ok(())
            }
        }
    }
}

/// Display all log files from a run directory.
fn display_run_logs(project_root: &Path, run_name: &str) -> Result<(), DecreeError> {
    let run_dir = project_root
        .join(config::DECREE_DIR)
        .join(config::RUNS_DIR)
        .join(run_name);

    if !run_dir.exists() {
        return Err(DecreeError::MessageNotFound(run_name.to_string()));
    }

    // Collect log files, sorted
    let mut logs: Vec<String> = std::fs::read_dir(&run_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .is_some_and(|ext| ext == "log")
        })
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    logs.sort();

    if logs.is_empty() {
        println!(
            "{}: no logs found",
            color::dim(run_name),
        );
        return Ok(());
    }

    let multiple = logs.len() > 1;

    for (i, log_name) in logs.iter().enumerate() {
        if multiple {
            let attempt = i + 1;
            println!(
                "{}",
                color::bold(&format!("=== {run_name} — Attempt {attempt} ({log_name}) ==="))
            );
        } else {
            println!("{}", color::bold(&format!("=== {run_name} ===")));
        }

        let log_path = run_dir.join(log_name);
        let content = std::fs::read_to_string(&log_path)?;
        print!("{content}");

        if !content.ends_with('\n') {
            println!();
        }
    }

    Ok(())
}
