use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use crate::config::Config;
use crate::cron::CronTracker;
use crate::error::{DecreeError, Result};

use super::process::{
    drain_inbox, install_signal_handlers, run_hook, was_interrupted, HookOutcome, HookType,
};

pub fn run(interval_secs: u64) -> Result<()> {
    install_signal_handlers();

    let config = Config::load(Path::new(".decree/config.yml"))?;
    let inbox_dir = PathBuf::from(".decree/inbox");
    let cron_dir = PathBuf::from(".decree/cron");
    let runs_dir = PathBuf::from(".decree/runs");
    let routines_dir = PathBuf::from(".decree/routines");

    // Ensure directories exist
    fs::create_dir_all(inbox_dir.join("done"))?;
    fs::create_dir_all(inbox_dir.join("dead"))?;
    fs::create_dir_all(&runs_dir)?;
    fs::create_dir_all(&cron_dir)?;

    // beforeAll hook — failure aborts daemon entirely
    if let HookOutcome::Failed(code) =
        run_hook(&config.hooks.before_all, HookType::BeforeAll, &routines_dir, None)?
    {
        return Err(DecreeError::HookFailed {
            hook: "beforeAll".to_string(),
            code,
        });
    }

    eprintln!("[decree] daemon started (polling every {interval_secs}s)");

    let mut cron_tracker = CronTracker::new();

    loop {
        // Check for graceful shutdown
        if was_interrupted() {
            eprintln!("[decree] daemon received signal, shutting down");
            break;
        }

        // Step 1-2: Check cron and copy due jobs into inbox
        match cron_tracker.check_and_fire(&cron_dir, &inbox_dir) {
            Ok(created) => {
                for path in &created {
                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                        eprintln!("[decree] cron fired: {name}");
                    }
                }
            }
            Err(e) => {
                eprintln!("[decree] cron error: {e}");
            }
        }

        // Check for graceful shutdown between cron and inbox
        if was_interrupted() {
            eprintln!("[decree] daemon received signal, shutting down");
            break;
        }

        // Steps 3-4: Process inbox messages
        if let Err(e) = drain_inbox(&config, &inbox_dir, &runs_dir, &routines_dir) {
            // Dead-lettered messages don't halt the daemon — only log I/O or
            // config errors as warnings and continue.
            eprintln!("[decree] inbox processing error: {e}");
        }

        // Step 5: Sleep (check for interrupt periodically within the sleep)
        if !sleep_interruptible(interval_secs) {
            eprintln!("[decree] daemon received signal, shutting down");
            break;
        }
    }

    // afterAll hook — failure logs warning, exits with hook's code
    if let HookOutcome::Failed(code) =
        run_hook(&config.hooks.after_all, HookType::AfterAll, &routines_dir, None)?
    {
        eprintln!(
            "[decree] warning: afterAll hook failed (exit {})",
            code
        );
        return Err(DecreeError::HookFailed {
            hook: "afterAll".to_string(),
            code,
        });
    }

    eprintln!("[decree] daemon stopped");
    Ok(())
}

/// Sleep for the given number of seconds, checking for interrupt every 500ms.
/// Returns `true` if the full sleep completed, `false` if interrupted.
fn sleep_interruptible(secs: u64) -> bool {
    let total_ms = secs * 1000;
    let check_interval = Duration::from_millis(500);
    let mut elapsed = 0u64;

    while elapsed < total_ms {
        if was_interrupted() {
            return false;
        }
        thread::sleep(check_interval);
        elapsed += 500;
    }

    !was_interrupted()
}
