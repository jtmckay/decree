use crate::config;
use crate::error::color;
use crate::error::DecreeError;
use crate::message;
use std::path::Path;

/// Run `decree status`.
pub fn run(project_root: &Path) -> Result<(), DecreeError> {
    let decree_dir = project_root.join(config::DECREE_DIR);

    // --- Migrations ---
    println!("{}", color::bold("Migrations:"));
    let migrations_dir = decree_dir.join(config::MIGRATIONS_DIR);
    let processed_path = decree_dir.join(config::PROCESSED_FILE);

    let all_migrations = list_migrations(&migrations_dir)?;
    let processed = read_processed(&processed_path)?;

    let processed_count = all_migrations
        .iter()
        .filter(|m| processed.contains(&m.to_string()))
        .count();
    let total = all_migrations.len();

    println!("  Processed: {} of {}", processed_count, total);

    if let Some(next) = all_migrations
        .iter()
        .find(|m| !processed.contains(&m.to_string()))
    {
        println!("  Next: {}", next);
    }

    println!();

    // --- Inbox ---
    println!("{}", color::bold("Inbox:"));
    let inbox_dir = decree_dir.join(config::INBOX_DIR);
    let inbox_dead_dir = inbox_dir.join(config::DEAD_DIR);

    let pending = count_files(&inbox_dir)?;
    let dead = count_files(&inbox_dead_dir)?;

    println!(
        "  Pending: {} message{}",
        pending,
        if pending == 1 { "" } else { "s" }
    );
    println!(
        "  Dead-lettered: {} message{}",
        dead,
        if dead == 1 { "" } else { "s" }
    );

    println!();

    // --- Recent Activity ---
    println!("{}", color::bold("Recent Activity (last 5):"));
    let runs = message::list_runs(project_root)?;

    if runs.is_empty() {
        println!("  No activity yet.");
    } else {
        // Dead-lettered message IDs (files in inbox/dead/)
        let dead_ids = list_dead_ids(&inbox_dead_dir)?;

        let recent: Vec<&String> = runs.iter().rev().take(5).collect();
        for run_name in recent.iter().rev() {
            let run_dir = decree_dir.join(config::RUNS_DIR).join(run_name);
            let routine = detect_routine(&run_dir);
            let disposition = if dead_ids.iter().any(|d| run_name.starts_with(d)) {
                color::error("dead")
            } else {
                color::success("done")
            };

            // Parse to check if follow-up
            let description = match message::MessageId::parse(run_name) {
                Ok(id) if id.seq > 0 => color::dim("(follow-up)"),
                Ok(_) => detect_migration_name(run_name),
                Err(_) => run_name.to_string(),
            };

            println!(
                "  {}  {}  {}  {}",
                color::dim(run_name),
                routine,
                disposition,
                description,
            );
        }
    }

    Ok(())
}

/// List migration files sorted alphabetically.
fn list_migrations(migrations_dir: &Path) -> Result<Vec<String>, DecreeError> {
    if !migrations_dir.exists() {
        return Ok(Vec::new());
    }
    let mut files: Vec<String> = std::fs::read_dir(migrations_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .is_some_and(|ext| ext == "md")
        })
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    files.sort();
    Ok(files)
}

/// Read processed migration list from processed.md.
fn read_processed(path: &Path) -> Result<Vec<String>, DecreeError> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = std::fs::read_to_string(path)?;
    Ok(content
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect())
}

/// Count regular files in a directory (non-recursive).
fn count_files(dir: &Path) -> Result<usize, DecreeError> {
    if !dir.exists() {
        return Ok(0);
    }
    let count = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_file())
        .count();
    Ok(count)
}

/// List message IDs from dead letter directory.
fn list_dead_ids(dead_dir: &Path) -> Result<Vec<String>, DecreeError> {
    if !dead_dir.exists() {
        return Ok(Vec::new());
    }
    let ids: Vec<String> = std::fs::read_dir(dead_dir)?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            e.file_name()
                .into_string()
                .ok()
                .map(|name| name.trim_end_matches(".md").to_string())
        })
        .collect();
    Ok(ids)
}

/// Try to detect which routine was used from the run directory.
fn detect_routine(run_dir: &Path) -> String {
    // Look for routine.log or <name>.log files
    if let Ok(entries) = std::fs::read_dir(run_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.ends_with(".log") && name != "routine.log" {
                return name.trim_end_matches(".log").to_string();
            }
        }
        // Fallback to routine.log
        if run_dir.join("routine.log").exists() {
            return "routine".to_string();
        }
    }
    "unknown".to_string()
}

/// Extract migration name from a chain ID.
/// Chain format: `D<NNNN>-HHmm-<name>`
fn detect_migration_name(run_name: &str) -> String {
    // Skip D<NNNN>-HHmm- prefix (11 chars) to get the name part
    if run_name.len() > 11 {
        let name_part = &run_name[11..];
        // Remove trailing -<seq> if present
        if let Some(last_dash) = name_part.rfind('-') {
            let potential_name = &name_part[..last_dash];
            if !potential_name.is_empty() {
                return format!("{potential_name}.md");
            }
        }
    }
    run_name.to_string()
}
