//! `decree daemon [--interval <s>]` (docs/reference/cli.md): validate, mark crashed runs
//! `interrupted`, then loop: deliver replies and timeouts, continue `pending` runs, cron
//! tick, drain the inbox, next migration, sleep. Every step is `process`'s own
//! (`commands::process::Pipeline`); there is no second pipeline. A failed or interrupted
//! inbox run does not stop it; a failed, interrupted or waiting migration blocks later
//! migrations only. On SIGINT or SIGTERM it interrupts the current run and exits 0.

use crate::commands::check::Project;
use crate::commands::process::{Pipeline, Stop};
use crate::cron::{self, CronTracker};
use crate::error::DecreeError;
use crate::layout;
use crate::message;
use crate::runtime;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

/// Run the daemon polling loop.
pub fn run(project_root: &Path, interval: u64) -> Result<(), DecreeError> {
    let project = Project::load(project_root)?;
    let shutdown = Arc::new(AtomicBool::new(false));
    runtime::register_signals(&shutdown)?;
    let mut pipeline = Pipeline::new(project_root, &project, Arc::clone(&shutdown))?;
    println!("decree daemon: polling every {interval}s");

    let mut cron_tracker = CronTracker::new();
    // The last message a blocked migration printed, so each block is reported once.
    let mut blocked = None;
    let mut result = report(pipeline.recover());
    loop {
        if result.is_ok() {
            result = pass(project_root, &mut pipeline, &mut cron_tracker, &mut blocked);
        }
        match result {
            Err(Stop::Interrupted) => {
                println!("decree daemon: shutting down (signal received)");
                return Ok(());
            }
            Err(stop) => return Err(stop.into_error()),
            Ok(()) => {}
        }
        // Sleep, checking for a signal every 100 ms.
        for _ in 0..interval.saturating_mul(10) {
            if shutdown.load(Ordering::Relaxed) {
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }
        if shutdown.load(Ordering::Relaxed) {
            result = Err(Stop::Interrupted);
        }
    }
}

/// One daemon pass. Returns an error only for a signal or a fault that stops the daemon.
fn pass(
    project_root: &Path,
    pipeline: &mut Pipeline,
    cron_tracker: &mut CronTracker,
    blocked: &mut Option<String>,
) -> Result<(), Stop> {
    report(pipeline.deliver_timeouts())?;
    for id in pipeline.pending()? {
        report(pipeline.continue_one(&id))?;
    }
    fire_due_cron_jobs(project_root, cron_tracker);
    loop {
        match pipeline.next_inbox() {
            Ok(true) => {}
            Ok(false) => break,
            Err(stop) => report(Err(stop))?,
        }
    }
    loop {
        match pipeline.next_migration() {
            Ok(true) => *blocked = None,
            Ok(false) => break,
            Err(Stop::Blocked(message)) => {
                if blocked.as_ref() != Some(&message) {
                    eprintln!("decree daemon: {message}");
                    *blocked = Some(message);
                }
                break;
            }
            Err(stop) => report(Err(stop))?,
        }
    }
    Ok(())
}

/// A failed run, or a blocked migration, is printed and the daemon goes on; a signal or a
/// fault stops it.
fn report(result: Result<(), Stop>) -> Result<(), Stop> {
    match result {
        Err(Stop::Failed(message) | Stop::Blocked(message)) => {
            eprintln!("decree daemon: {message}");
            Ok(())
        }
        other => other,
    }
}

/// Check cron directory and fire due jobs into inbox.
fn fire_due_cron_jobs(project_root: &Path, tracker: &mut CronTracker) {
    let cron_files = match cron::scan_cron_files(project_root) {
        Ok(files) => files,
        Err(e) => {
            eprintln!("decree daemon: error scanning cron: {e}");
            return;
        }
    };

    for cf in &cron_files {
        if !tracker.is_due(cf) {
            continue;
        }

        let mut msg = cron::cron_to_inbox_message(cf);
        let decree_dir = project_root.join(layout::DECREE_DIR);
        match message::queue(&decree_dir, &mut msg) {
            Ok(id) => {
                println!("decree daemon: cron fired: {} -> {id}.md", cf.filename);
                tracker.mark_fired(cf);
            }
            Err(e) => {
                eprintln!(
                    "decree daemon: failed to write cron message for {}: {e}",
                    cf.filename
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup_decree_dir(dir: &TempDir) {
        let decree = dir.path().join(".decree");
        std::fs::create_dir_all(decree.join("inbox")).unwrap();
        std::fs::create_dir_all(decree.join("cron")).unwrap();
    }

    #[test]
    fn test_fire_due_cron_jobs() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);

        // Write a cron file that fires every minute
        std::fs::write(
            dir.path().join(".decree/cron/every-minute.md"),
            "---\ncron: \"* * * * *\"\nroutine: develop\n---\nMinutely task.\n",
        )
        .unwrap();

        let mut tracker = CronTracker::new();
        fire_due_cron_jobs(dir.path(), &mut tracker);

        // Should have created an inbox message
        let inbox_files =
            crate::commands::check::md_files(&dir.path().join(".decree/inbox")).unwrap();
        assert_eq!(inbox_files.len(), 1);

        // Verify the inbox message content
        let content =
            std::fs::read_to_string(dir.path().join(".decree/inbox").join(&inbox_files[0]))
                .unwrap();
        let id = inbox_files[0].strip_suffix(".md").unwrap();
        assert_eq!(
            content,
            format!("---\nid: {id}\nmachine: develop\ntrigger: cron\n---\nMinutely task.\n")
        );
        // cron field should NOT be present as a standalone YAML key
        assert!(!content.lines().any(|l| l.starts_with("cron:")));

        // Second fire within same minute should not create duplicate
        fire_due_cron_jobs(dir.path(), &mut tracker);
        let inbox_files2 =
            crate::commands::check::md_files(&dir.path().join(".decree/inbox")).unwrap();
        assert_eq!(inbox_files2.len(), 1); // Still just one
    }
}
