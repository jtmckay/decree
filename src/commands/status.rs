use std::fs;
use std::path::Path;

use crate::error::Result;

pub fn run() -> Result<()> {
    // Processed migrations
    let processed_path = Path::new("migrations/processed.md");
    let processed_count = if processed_path.exists() {
        let content = fs::read_to_string(processed_path)?;
        content.lines().filter(|l| !l.trim().is_empty()).count()
    } else {
        0
    };

    // Pending migrations
    let migrations_dir = Path::new("migrations");
    let pending_migrations = if migrations_dir.exists() {
        count_files_with_ext(migrations_dir, "md")?
            .saturating_sub(if processed_path.exists() { 1 } else { 0 }) // exclude processed.md
    } else {
        0
    };

    // Pending inbox messages
    let inbox_dir = Path::new(".decree/inbox");
    let pending_inbox = if inbox_dir.exists() {
        count_md_files_toplevel(inbox_dir)?
    } else {
        0
    };

    // Done inbox messages
    let done_dir = Path::new(".decree/inbox/done");
    let done_count = if done_dir.exists() {
        count_md_files_toplevel(done_dir)?
    } else {
        0
    };

    // Dead inbox messages
    let dead_dir = Path::new(".decree/inbox/dead");
    let dead_count = if dead_dir.exists() {
        count_md_files_toplevel(dead_dir)?
    } else {
        0
    };

    // Recent runs
    let runs_dir = Path::new(".decree/runs");
    let recent_runs = if runs_dir.exists() {
        list_recent_runs(runs_dir, 5)?
    } else {
        Vec::new()
    };

    // Print summary
    println!("Migrations: {} processed, {} pending", processed_count, pending_migrations);
    println!(
        "Inbox: {} pending, {} done, {} dead",
        pending_inbox, done_count, dead_count
    );

    if recent_runs.is_empty() {
        println!("\nNo recent runs.");
    } else {
        println!("\nRecent runs:");
        for run in &recent_runs {
            println!("  {run}");
        }
    }

    Ok(())
}

fn count_files_with_ext(dir: &Path, ext: &str) -> Result<usize> {
    let mut count = 0;
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if entry.file_type()?.is_file() {
            if let Some(e) = entry.path().extension() {
                if e == ext {
                    count += 1;
                }
            }
        }
    }
    Ok(count)
}

fn count_md_files_toplevel(dir: &Path) -> Result<usize> {
    count_files_with_ext(dir, "md")
}

fn list_recent_runs(runs_dir: &Path, limit: usize) -> Result<Vec<String>> {
    let mut entries: Vec<String> = Vec::new();

    for entry in fs::read_dir(runs_dir)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            if let Some(name) = entry.file_name().to_str() {
                entries.push(name.to_string());
            }
        }
    }

    // Sort descending (most recent first — lexicographic works for timestamp IDs)
    entries.sort();
    entries.reverse();
    entries.truncate(limit);

    Ok(entries)
}
