use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{DecreeError, Result};

pub fn run(id: Option<&str>) -> Result<()> {
    let runs_dir = Path::new(".decree/runs");

    if !runs_dir.exists() {
        return Err(DecreeError::MessageNotFound(
            "no runs directory".to_string(),
        ));
    }

    match id {
        None => show_most_recent_log(runs_dir),
        Some(id) => show_log_by_id(runs_dir, id),
    }
}

fn show_most_recent_log(runs_dir: &Path) -> Result<()> {
    let most_recent = find_most_recent_run(runs_dir)?;
    print_run_logs(&most_recent)
}

fn show_log_by_id(runs_dir: &Path, id: &str) -> Result<()> {
    // Try exact match first (full message ID like "2025022514320000-0")
    let exact = runs_dir.join(id);
    if exact.is_dir() {
        return print_run_logs(&exact);
    }

    // Try as chain ID — find all matching run dirs
    let matching = find_runs_by_prefix(runs_dir, id)?;

    if matching.is_empty() {
        return Err(DecreeError::MessageNotFound(id.to_string()));
    }

    // Show all matching runs (chain view)
    for (i, run_dir) in matching.iter().enumerate() {
        if i > 0 {
            println!();
        }
        let dir_name = run_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");
        println!("=== {} ===", dir_name);
        print_run_logs(run_dir)?;
    }

    Ok(())
}

fn find_most_recent_run(runs_dir: &Path) -> Result<PathBuf> {
    let mut entries: Vec<PathBuf> = Vec::new();

    for entry in fs::read_dir(runs_dir)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            entries.push(entry.path());
        }
    }

    entries.sort();
    entries
        .pop()
        .ok_or_else(|| DecreeError::MessageNotFound("no runs found".to_string()))
}

fn find_runs_by_prefix(runs_dir: &Path, prefix: &str) -> Result<Vec<PathBuf>> {
    let mut matching: Vec<PathBuf> = Vec::new();

    for entry in fs::read_dir(runs_dir)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            if let Some(name) = entry.file_name().to_str() {
                if name.starts_with(prefix) {
                    matching.push(entry.path());
                }
            }
        }
    }

    matching.sort();
    Ok(matching)
}

fn print_run_logs(run_dir: &Path) -> Result<()> {
    // Collect all log files: routine.log, routine-2.log, routine-3.log, etc.
    let mut log_files: Vec<PathBuf> = Vec::new();

    let primary = run_dir.join("routine.log");
    if primary.exists() {
        log_files.push(primary);
    }

    // Check for retry logs
    for i in 2..=20 {
        let retry_log = run_dir.join(format!("routine-{i}.log"));
        if retry_log.exists() {
            log_files.push(retry_log);
        } else {
            break;
        }
    }

    if log_files.is_empty() {
        let dir_name = run_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");
        println!("No logs found for {dir_name}");
        return Ok(());
    }

    for (i, log_path) in log_files.iter().enumerate() {
        if log_files.len() > 1 {
            let label = if i == 0 {
                "Attempt 1".to_string()
            } else {
                format!("Attempt {}", i + 1)
            };
            println!("--- {label} ---");
        }

        let content = fs::read_to_string(log_path)?;
        print!("{content}");

        // Ensure trailing newline
        if !content.ends_with('\n') {
            println!();
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_runs_by_prefix_empty() {
        let dir = std::env::temp_dir().join("decree_test_empty_runs");
        let _ = fs::create_dir_all(&dir);
        let result = find_runs_by_prefix(&dir, "nonexistent").unwrap();
        assert!(result.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }
}
