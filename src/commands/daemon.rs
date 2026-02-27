use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use chrono::Local;

use crate::config::Config;
use crate::cron;
use crate::error::{find_project_root, DecreeError};
use crate::pipeline::{self, ProcessResult};

/// Execute the `decree daemon` command.
pub fn run(interval: u64) -> Result<(), DecreeError> {
    let root = find_project_root()?;
    let config = Config::load(&root)?;

    let shutdown = Arc::new(AtomicBool::new(false));

    // Register signal handlers for SIGINT and SIGTERM
    for sig in &[signal_hook::consts::SIGINT, signal_hook::consts::SIGTERM] {
        signal_hook::flag::register(*sig, Arc::clone(&shutdown)).map_err(|e| {
            DecreeError::Config(format!("failed to register signal handler: {e}"))
        })?;
    }

    println!("decree daemon started (polling every {interval}s)");

    // Track last fire time per cron file to avoid duplicates within the same minute
    let mut last_fired: HashMap<String, (u32, u32, u32, u32, u32)> = HashMap::new();

    loop {
        if shutdown.load(Ordering::Relaxed) {
            println!("shutting down...");
            break;
        }

        // Step 1-2: Evaluate cron files and create inbox messages for due jobs
        if let Err(e) = evaluate_cron_jobs(&root, &mut last_fired) {
            eprintln!("cron evaluation error: {e}");
        }

        // Step 3-4: Process inbox messages
        if let Err(e) = process_inbox(&root, &config, &shutdown) {
            eprintln!("inbox processing error: {e}");
        }

        // Step 5: Sleep for the interval, checking shutdown periodically
        for _ in 0..interval {
            if shutdown.load(Ordering::Relaxed) {
                break;
            }
            thread::sleep(Duration::from_secs(1));
        }
    }

    println!("daemon stopped");
    Ok(())
}

/// Evaluate cron files and create inbox messages for due jobs.
///
/// `last_fired` tracks (year, month, day, hour, minute) of last fire per cron file name
/// to prevent duplicate firings within the same minute.
fn evaluate_cron_jobs(
    project_root: &Path,
    last_fired: &mut HashMap<String, (u32, u32, u32, u32, u32)>,
) -> Result<(), DecreeError> {
    let cron_dir = project_root.join(".decree/cron");
    let cron_files = cron::scan_cron_files(&cron_dir)?;

    if cron_files.is_empty() {
        return Ok(());
    }

    let now = Local::now().naive_local();
    let current_key = (
        now.year() as u32,
        now.month(),
        now.day(),
        now.hour(),
        now.minute(),
    );

    for cron_file in &cron_files {
        if !cron::matches_time(&cron_file.cron_expr, &now) {
            continue;
        }

        // Check if already fired this minute
        if let Some(last) = last_fired.get(&cron_file.name) {
            if *last == current_key {
                continue;
            }
        }

        // Fire the cron job
        println!("cron: firing {}", cron_file.name);
        let path = cron::create_inbox_from_cron(project_root, cron_file)?;
        println!(
            "cron: created inbox message {}",
            path.file_name()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_default()
        );

        last_fired.insert(cron_file.name.clone(), current_key);
    }

    Ok(())
}

use chrono::{Datelike, Timelike};

/// Process all pending inbox messages, depth-first within chains.
///
/// Collects all `.md` files in the inbox, groups by chain, sorts by seq,
/// and processes chains in order.
fn process_inbox(
    project_root: &Path,
    config: &Config,
    shutdown: &AtomicBool,
) -> Result<(), DecreeError> {
    let inbox_dir = project_root.join(".decree/inbox");
    if !inbox_dir.is_dir() {
        return Ok(());
    }

    // Collect pending messages
    let mut messages = collect_inbox_messages(&inbox_dir)?;
    if messages.is_empty() {
        return Ok(());
    }

    // Sort by filename for deterministic ordering (chain ID is timestamp-based)
    messages.sort();

    // Track which chains we've already processed (depth-first handles follow-ups)
    let mut processed_chains: Vec<String> = Vec::new();

    for msg_path in &messages {
        if shutdown.load(Ordering::Relaxed) {
            println!("shutdown requested, finishing current cycle...");
            break;
        }

        let full_path = inbox_dir.join(msg_path);
        if !full_path.exists() {
            // May have been consumed as a follow-up by a prior chain
            continue;
        }

        // Extract chain from filename or frontmatter
        let chain = extract_chain(&full_path, msg_path);
        if let Some(ref chain_id) = chain {
            if processed_chains.contains(chain_id) {
                continue; // Already handled by depth-first processing
            }
        }

        println!("processing: {msg_path}");

        let result = pipeline::process_chain(project_root, config, &full_path, None)?;

        match result {
            ProcessResult::Success => {
                println!("  done: {msg_path}");
            }
            ProcessResult::DeadLettered(reason) => {
                eprintln!("  dead-lettered: {msg_path} ({reason})");
            }
        }

        if let Some(chain_id) = chain {
            processed_chains.push(chain_id);
        }
    }

    Ok(())
}

/// Collect all `.md` files in the inbox directory (not subdirectories).
fn collect_inbox_messages(inbox_dir: &Path) -> Result<Vec<String>, DecreeError> {
    let mut files = Vec::new();

    for entry in fs::read_dir(inbox_dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let filename = entry.file_name().to_string_lossy().to_string();
        if filename.ends_with(".md") {
            files.push(filename);
        }
    }

    Ok(files)
}

/// Extract the chain ID from a message file.
fn extract_chain(full_path: &Path, filename: &str) -> Option<String> {
    // Try filename first (faster)
    if let Some((chain, _)) = crate::message::chain_seq_from_filename(filename) {
        return Some(chain);
    }

    // Fall back to reading frontmatter
    if let Ok(content) = fs::read_to_string(full_path) {
        let (fm, _) = crate::message::parse_message_file(&content);
        return fm.chain;
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_collect_inbox_messages_empty() {
        let dir = tempfile::tempdir().unwrap();
        let files = collect_inbox_messages(dir.path()).unwrap();
        assert!(files.is_empty());
    }

    #[test]
    fn test_collect_inbox_messages_filters_md() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("msg1.md"), "test").unwrap();
        fs::write(dir.path().join("msg2.md"), "test").unwrap();
        fs::write(dir.path().join("not-a-message.txt"), "test").unwrap();
        fs::create_dir(dir.path().join("done")).unwrap();

        let files = collect_inbox_messages(dir.path()).unwrap();
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn test_collect_inbox_messages_ignores_subdirs() {
        let dir = tempfile::tempdir().unwrap();
        let done = dir.path().join("done");
        fs::create_dir_all(&done).unwrap();
        fs::write(done.join("old-msg.md"), "test").unwrap();
        fs::write(dir.path().join("new-msg.md"), "test").unwrap();

        let files = collect_inbox_messages(dir.path()).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0], "new-msg.md");
    }

    #[test]
    fn test_evaluate_cron_jobs_no_cron_dir() {
        let dir = tempfile::tempdir().unwrap();
        let mut last_fired = HashMap::new();
        // Should not error even if .decree/cron doesn't exist
        evaluate_cron_jobs(dir.path(), &mut last_fired).unwrap();
    }

    #[test]
    fn test_evaluate_cron_jobs_fires_matching() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let cron_dir = root.join(".decree/cron");
        let inbox_dir = root.join(".decree/inbox");
        fs::create_dir_all(&cron_dir).unwrap();
        fs::create_dir_all(&inbox_dir).unwrap();

        // Create a cron file that matches every minute
        let content = "---\ncron: \"* * * * *\"\nroutine: develop\n---\nEvery minute task.\n";
        fs::write(cron_dir.join("every-minute.md"), content).unwrap();

        let mut last_fired = HashMap::new();
        evaluate_cron_jobs(root, &mut last_fired).unwrap();

        // Should have created one inbox message
        let inbox_files: Vec<_> = fs::read_dir(&inbox_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_file())
            .collect();
        assert_eq!(inbox_files.len(), 1);

        // Should have recorded the fire time
        assert!(last_fired.contains_key("every-minute"));
    }

    #[test]
    fn test_evaluate_cron_jobs_no_duplicate_within_minute() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let cron_dir = root.join(".decree/cron");
        let inbox_dir = root.join(".decree/inbox");
        fs::create_dir_all(&cron_dir).unwrap();
        fs::create_dir_all(&inbox_dir).unwrap();

        let content = "---\ncron: \"* * * * *\"\nroutine: develop\n---\nTask.\n";
        fs::write(cron_dir.join("task.md"), content).unwrap();

        let mut last_fired = HashMap::new();

        // First evaluation fires
        evaluate_cron_jobs(root, &mut last_fired).unwrap();
        let count1 = fs::read_dir(&inbox_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_file())
            .count();
        assert_eq!(count1, 1);

        // Second evaluation within the same minute should NOT fire again
        evaluate_cron_jobs(root, &mut last_fired).unwrap();
        let count2 = fs::read_dir(&inbox_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_file())
            .count();
        assert_eq!(count2, 1);
    }

    #[test]
    fn test_cron_inbox_message_content() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let cron_dir = root.join(".decree/cron");
        let inbox_dir = root.join(".decree/inbox");
        fs::create_dir_all(&cron_dir).unwrap();
        fs::create_dir_all(&inbox_dir).unwrap();

        let content =
            "---\ncron: \"* * * * *\"\nroutine: deploy\ntarget: staging\n---\nDeploy now.\n";
        fs::write(cron_dir.join("deploy.md"), content).unwrap();

        let mut last_fired = HashMap::new();
        evaluate_cron_jobs(root, &mut last_fired).unwrap();

        // Read the created inbox message
        let inbox_files: Vec<_> = fs::read_dir(&inbox_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_file())
            .collect();
        assert_eq!(inbox_files.len(), 1);

        let msg_content = fs::read_to_string(inbox_files[0].path()).unwrap();
        assert!(msg_content.contains("seq: 0"));
        assert!(msg_content.contains("type: task"));
        assert!(msg_content.contains("routine: deploy"));
        assert!(msg_content.contains("target: staging"));
        assert!(msg_content.contains("Deploy now."));
        assert!(!msg_content.contains("cron:"));
    }
}
