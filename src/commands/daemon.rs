//! `decree daemon [--interval <duration>]` (docs/reference/cli.md): validate, mark crashed runs
//! `interrupted`, then loop: read `.decree/env` again, deliver replies and timeouts, continue `pending` runs, cron
//! tick, drain the inbox, next migration, sleep. Every step is `process`'s own
//! (`commands::process::Pipeline`); there is no second pipeline. A failed or interrupted
//! inbox run does not stop it; a failed, interrupted or waiting migration blocks later
//! migrations only. On SIGINT or SIGTERM it interrupts the current run and exits 0.

use crate::commands::check::Project;
use crate::commands::process::{context, reporting, Pipeline, Stop};
use crate::commands::report::{line, notice};
use crate::cron::{self, CronTracker};
use crate::error::DecreeError;
use crate::layout;
use crate::message;
use crate::runtime;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

/// Run the daemon polling loop. Unless `quiet`, each run's output is printed as it goes
/// (`commands::report`).
pub fn run(project_root: &Path, interval: Duration, quiet: bool) -> Result<(), DecreeError> {
    let project = Project::load(project_root)?;
    let shutdown = Arc::new(AtomicBool::new(false));
    runtime::register_signals(&shutdown)?;
    let mut pipeline = Pipeline::new(
        project_root,
        &project,
        Arc::clone(&shutdown),
        "decree daemon: ",
    )?;
    println!("decree daemon: polling every {}s", interval.as_secs());
    let observer_ctx = context(project_root, &project, Arc::clone(&shutdown));
    reporting(&mut pipeline, &observer_ctx, quiet, |pipeline| {
        poll(project_root, pipeline, &shutdown, interval)
    })
}

/// Recover, then pass and sleep until a signal.
fn poll(
    project_root: &Path,
    pipeline: &mut Pipeline,
    shutdown: &AtomicBool,
    interval: Duration,
) -> Result<(), DecreeError> {
    let mut cron_tracker = CronTracker::new();
    // The last message a blocked migration printed, so each block is reported once.
    let mut blocked = None;
    // The last `.env` error printed, likewise.
    let mut env_error = None;
    let mut result = report(pipeline.recover());
    loop {
        if result.is_ok() {
            reload_env(pipeline, &mut env_error);
            result = pass(project_root, pipeline, &mut cron_tracker, &mut blocked);
        }
        match result {
            Err(Stop::Interrupted) => {
                line("decree daemon: shutting down (signal received)".to_string());
                return Ok(());
            }
            Err(stop) => return Err(stop.into_error()),
            Ok(()) => {}
        }
        // The pipeline printed each failed run as it ended; the daemon keeps no list of them.
        pipeline.take_failed();
        // Sleep, checking for a signal every 100 ms. An interval past what `Instant` holds
        // sleeps until a signal.
        let wake = Instant::now().checked_add(interval);
        while !shutdown.load(Ordering::Relaxed) {
            let left = wake.map_or(Duration::MAX, |w| {
                w.saturating_duration_since(Instant::now())
            });
            if left.is_zero() {
                break;
            }
            thread::sleep(left.min(Duration::from_millis(100)));
        }
        if shutdown.load(Ordering::Relaxed) {
            result = Err(Stop::Interrupted);
        }
    }
}

/// Read the `.env` files again before a pass. A malformed file is printed once per change, and
/// scripts keep the variables last read until it is fixed.
fn reload_env(pipeline: &mut Pipeline, env_error: &mut Option<String>) {
    match pipeline.reload_env() {
        Ok(()) => *env_error = None,
        Err(e) => {
            let message = e.to_string();
            if env_error.as_ref() != Some(&message) {
                notice(format!(
                    "decree daemon: {message}; keeping the variables last read"
                ));
                *env_error = Some(message);
            }
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
                    notice(format!("decree daemon: {message}"));
                    *blocked = Some(message);
                }
                break;
            }
            Err(stop) => report(Err(stop))?,
        }
    }
    Ok(())
}

/// A blocked migration is printed and the daemon goes on; a signal or a fault stops it. The
/// pipeline reports a failed run itself.
fn report(result: Result<(), Stop>) -> Result<(), Stop> {
    match result {
        Err(Stop::Blocked(message)) => {
            notice(format!("decree daemon: {message}"));
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
            notice(format!("decree daemon: error scanning cron: {e}"));
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
                line(format!(
                    "decree daemon: cron fired: {} -> {id}.md",
                    cf.filename
                ));
                tracker.mark_fired(cf);
            }
            Err(e) => {
                notice(format!(
                    "decree daemon: failed to write cron message for {}: {e}",
                    cf.filename
                ));
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
            "---\ncron: \"* * * * *\"\nmachine: develop\n---\nMinutely task.\n",
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

    #[test]
    fn test_fire_due_cron_jobs_reads_crlf_files() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);
        std::fs::write(
            dir.path().join(".decree/cron/every-minute.md"),
            "---\r\ncron: \"* * * * *\"\r\nmachine: develop\r\n---\r\nMinutely task.\r\n",
        )
        .unwrap();
        fire_due_cron_jobs(dir.path(), &mut CronTracker::new());
        let inbox_files =
            crate::commands::check::md_files(&dir.path().join(".decree/inbox")).unwrap();
        assert_eq!(inbox_files.len(), 1);
        let content =
            std::fs::read_to_string(dir.path().join(".decree/inbox").join(&inbox_files[0]))
                .unwrap();
        assert!(
            content.ends_with("trigger: cron\n---\nMinutely task.\r\n"),
            "{content}"
        );
    }
}
