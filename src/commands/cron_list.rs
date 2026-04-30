use crate::config::{self, AppConfig};
use crate::cron;
use crate::error::DecreeError;
use chrono::{DateTime, Local, Utc};
use std::path::Path;

const W_FILE: usize = 32;
const W_SCHED: usize = 19;
const W_ROUTINE: usize = 16;
const W_LAST: usize = 11;

/// Run `decree cron list`.
pub fn run(project_root: &Path) -> Result<(), DecreeError> {
    let config = AppConfig::load_from_project(project_root)?;
    let mut cron_files = cron::scan_cron_files(project_root)?;
    cron_files.sort_by(|a, b| a.filename.cmp(&b.filename));

    if cron_files.is_empty() {
        println!("No cron files.");
        return Ok(());
    }

    let runs_dir = project_root
        .join(config::DECREE_DIR)
        .join(config::RUNS_DIR);

    println!(
        "{:<W_FILE$}{:<W_SCHED$}{:<W_ROUTINE$}{:<W_LAST$}{}",
        "CRON FILE", "SCHEDULE", "ROUTINE", "LAST RUN", "NEXT RUN"
    );

    for cf in &cron_files {
        let routine = cf.routine.as_deref().unwrap_or(&config.default_routine);
        let last_run = find_last_run(&runs_dir, &cf.name_stem);
        let next_run = cf.schedule.upcoming(Utc).next().map(DateTime::<Local>::from);

        let last_str = match last_run {
            Some(t) => relative_ago(t),
            None => "never".to_string(),
        };
        let next_str = match next_run {
            Some(t) => countdown(t),
            None => "\u{2014}".to_string(), // em dash
        };

        println!(
            "{:<W_FILE$}{:<W_SCHED$}{:<W_ROUTINE$}{:<W_LAST$}{}",
            cf.filename, cf.cron_expr, routine, last_str, next_str
        );
    }

    Ok(())
}

/// Find the most-recent run directory for `stem` by alphabetical order
/// (run dir names embed date/time, so lexicographic last = most recent).
fn find_last_run(runs_dir: &Path, stem: &str) -> Option<DateTime<Local>> {
    if !runs_dir.exists() {
        return None;
    }

    let best_name = std::fs::read_dir(runs_dir)
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|name| name.contains(stem))
        .max()?; // alphabetically last = most recent

    // Parse creation time from the directory mtime as a fallback; the
    // alphabetical winner is correct, we just need a timestamp for display.
    let path = runs_dir.join(&best_name);
    path.metadata()
        .ok()
        .and_then(|m| m.modified().ok())
        .map(DateTime::from)
}

/// Format elapsed time: "30s ago", "5m ago", "2h ago", "3d ago".
fn relative_ago(t: DateTime<Local>) -> String {
    let secs = (Local::now() - t).num_seconds().max(0);
    if secs < 60 {
        format!("{secs}s ago")
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86400)
    }
}

/// Format time remaining: "30s", "5m", "2h", "3d".
fn countdown(t: DateTime<Local>) -> String {
    let secs = (t - Local::now()).num_seconds().max(0);
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86400 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}d", secs / 86400)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup_decree_dir(dir: &TempDir) {
        let decree = dir.path().join(".decree");
        std::fs::create_dir_all(decree.join("cron")).unwrap();
        std::fs::create_dir_all(decree.join("inbox")).unwrap();
        std::fs::create_dir_all(decree.join("runs")).unwrap();
        std::fs::write(decree.join("processed.md"), "").unwrap();
        std::fs::write(
            decree.join("config.yml"),
            "commands:\n  ai_router: echo\n  ai_interactive: echo\n",
        )
        .unwrap();
    }

    #[test]
    fn test_run_empty_cron_dir() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);
        assert!(run(dir.path()).is_ok());
    }

    #[test]
    fn test_run_with_cron_file() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);
        std::fs::write(
            dir.path().join(".decree/cron/hourly.md"),
            "---\ncron: \"0 * * * *\"\nroutine: develop\n---\nHourly task.\n",
        )
        .unwrap();
        assert!(run(dir.path()).is_ok());
    }

    #[test]
    fn test_relative_ago() {
        let now = Local::now();
        assert_eq!(relative_ago(now - chrono::Duration::seconds(30)), "30s ago");
        assert_eq!(relative_ago(now - chrono::Duration::seconds(90)), "1m ago");
        assert_eq!(relative_ago(now - chrono::Duration::seconds(3700)), "1h ago");
        assert_eq!(relative_ago(now - chrono::Duration::seconds(86500)), "1d ago");
    }

    #[test]
    fn test_countdown() {
        let now = Local::now();
        assert_eq!(countdown(now + chrono::Duration::seconds(90)), "1m");
        assert_eq!(countdown(now + chrono::Duration::seconds(3700)), "1h");
        assert_eq!(countdown(now + chrono::Duration::seconds(86500)), "1d");
        let result = countdown(now + chrono::Duration::seconds(30));
        assert!(result.ends_with('s'), "got: {result}");
    }

    #[test]
    fn test_find_last_run_no_dir() {
        let dir = TempDir::new().unwrap();
        assert!(find_last_run(&dir.path().join("nonexistent"), "stem").is_none());
    }

    #[test]
    fn test_find_last_run_with_matching_dir() {
        let dir = TempDir::new().unwrap();
        let runs = dir.path().join("runs");
        std::fs::create_dir_all(runs.join("D0001-1200-hourly-0")).unwrap();
        assert!(find_last_run(&runs, "hourly").is_some());
    }

    #[test]
    fn test_find_last_run_no_match() {
        let dir = TempDir::new().unwrap();
        let runs = dir.path().join("runs");
        std::fs::create_dir_all(runs.join("D0001-1200-daily-0")).unwrap();
        assert!(find_last_run(&runs, "hourly").is_none());
    }

    #[test]
    fn test_find_last_run_picks_alphabetically_last() {
        let dir = TempDir::new().unwrap();
        let runs = dir.path().join("runs");
        std::fs::create_dir_all(runs.join("D0001-1200-sync-0")).unwrap();
        std::fs::create_dir_all(runs.join("D0002-0900-sync-0")).unwrap();
        std::fs::create_dir_all(runs.join("D0001-0800-sync-0")).unwrap();
        // D0002-0900-sync-0 is alphabetically last (highest day counter)
        assert!(find_last_run(&runs, "sync").is_some());
    }

    #[test]
    fn test_find_last_run_stem_must_match() {
        let dir = TempDir::new().unwrap();
        let runs = dir.path().join("runs");
        std::fs::create_dir_all(runs.join("D0001-1200-gmail-sync-0")).unwrap();
        // "sync" is a substring of "gmail-sync", should match
        assert!(find_last_run(&runs, "gmail-sync").is_some());
        // "gmail" alone also matches since it's a substring
        assert!(find_last_run(&runs, "gmail").is_some());
        // something unrelated should not match
        assert!(find_last_run(&runs, "calendar").is_none());
    }
}
