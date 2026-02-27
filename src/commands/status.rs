use std::fs;

use crate::error::{find_project_root, DecreeError};

pub fn run() -> Result<(), DecreeError> {
    let root = find_project_root()?;

    // --- Processed specs ---
    let processed_path = root.join("specs/processed-spec.md");
    let processed_specs: Vec<String> = if processed_path.exists() {
        fs::read_to_string(&processed_path)?
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(String::from)
            .collect()
    } else {
        Vec::new()
    };

    let specs_dir = root.join("specs");
    let mut all_specs: Vec<String> = Vec::new();
    if specs_dir.is_dir() {
        for entry in fs::read_dir(&specs_dir)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().to_string();
            if name.ends_with(".spec.md") {
                all_specs.push(name);
            }
        }
    }
    all_specs.sort();

    println!("=== Specs ===");
    println!(
        "  {}/{} processed",
        processed_specs.len(),
        all_specs.len()
    );
    for spec in &all_specs {
        let status = if processed_specs.iter().any(|p| p.contains(spec)) {
            "✓"
        } else {
            " "
        };
        println!("  [{status}] {spec}");
    }

    // --- Pending inbox messages ---
    let inbox_dir = root.join(".decree/inbox");
    let mut pending: Vec<String> = Vec::new();
    if inbox_dir.is_dir() {
        for entry in fs::read_dir(&inbox_dir)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().to_string();
            if name.ends_with(".md") && entry.file_type()?.is_file() {
                pending.push(name);
            }
        }
    }
    pending.sort();

    println!("\n=== Inbox ===");
    if pending.is_empty() {
        println!("  (empty)");
    } else {
        for msg in &pending {
            println!("  {msg}");
        }
    }

    // --- Recent message history ---
    let runs_dir = root.join(".decree/runs");
    let mut runs: Vec<String> = Vec::new();
    if runs_dir.is_dir() {
        for entry in fs::read_dir(&runs_dir)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                runs.push(entry.file_name().to_string_lossy().to_string());
            }
        }
    }
    runs.sort();

    println!("\n=== Recent Messages ===");
    if runs.is_empty() {
        println!("  (none)");
    } else {
        // Show last 10
        let start = if runs.len() > 10 { runs.len() - 10 } else { 0 };
        for run_id in &runs[start..] {
            let msg_path = runs_dir.join(run_id).join("message.md");
            let summary = if msg_path.exists() {
                let content = fs::read_to_string(&msg_path).unwrap_or_default();
                // First non-empty line as summary
                content
                    .lines()
                    .find(|l| !l.trim().is_empty())
                    .unwrap_or("(no content)")
                    .chars()
                    .take(60)
                    .collect::<String>()
            } else {
                "(no message.md)".to_string()
            };
            println!("  {run_id}  {summary}");
        }
    }

    Ok(())
}
