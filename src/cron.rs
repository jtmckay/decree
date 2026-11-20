use crate::error::DecreeError;
use crate::layout;
use crate::message::Message;
use chrono::Utc;
use serde_norway::{Mapping, Value};
use std::collections::HashMap;
use std::path::Path;
use std::str::FromStr;

/// A parsed cron file from `.decree/cron/`.
#[derive(Debug, Clone)]
pub struct CronFile {
    /// Filename (e.g., `hourly-maintenance.md`).
    pub filename: String,
    /// Raw cron expression as written in the frontmatter (e.g., `*/15 * * * *`).
    pub cron_expr: String,
    /// When it fires.
    pub when: When,
    /// The file as a message, parsed as every other message is (docs/reference/messages.md,
    /// Parsing and writing).
    pub message: Message,
}

impl CronFile {
    /// The machine its messages name: frontmatter `machine`.
    pub fn machine(&self) -> Option<&str> {
        self.message.machine()
    }
}

/// Vixie cron's `@reboot`: fire once when the cron daemon starts (`decree daemon`).
pub const REBOOT: &str = "@reboot";

/// When a cron file fires: on a schedule, or once each time `decree daemon` starts.
#[derive(Debug, Clone)]
pub enum When {
    Schedule(Box<cron::Schedule>),
    DaemonStart,
}

/// Parse a `cron:` value: `@reboot`, or a schedule ([`parse_schedule`]).
pub fn parse_when(cron_expr: &str) -> Result<When, cron::error::Error> {
    if cron_expr.trim() == REBOOT {
        return Ok(When::DaemonStart);
    }
    parse_schedule(cron_expr).map(|s| When::Schedule(Box::new(s)))
}

/// Scan `.decree/cron/` for valid cron files.
pub fn scan_cron_files(project_root: &Path) -> Result<Vec<CronFile>, DecreeError> {
    let cron_dir = project_root.join(layout::DECREE_DIR).join(layout::CRON_DIR);

    if !cron_dir.exists() {
        return Ok(Vec::new());
    }

    let entries = file_names(&cron_dir)?;

    let mut cron_files = Vec::new();
    for filename in entries {
        let path = cron_dir.join(&filename);
        let content = std::fs::read_to_string(&path)?;
        match parse_cron_file(&filename, &content) {
            Ok(cf) => cron_files.push(cf),
            Err(_) => {
                // Skip files with invalid cron expressions or no cron field
                continue;
            }
        }
    }

    Ok(cron_files)
}

/// Every cron file in `cron_dir`, relative to it, sorted: each `*.md` there, and each `*.md`
/// in a folder there, as `<folder>/<file>.md`: a plugin installs its cron files in
/// `cron/<plugin>/` (docs/reference/plugins.md). Deeper folders are not read.
pub fn file_names(cron_dir: &Path) -> std::io::Result<Vec<String>> {
    let is_md = |p: &Path| p.is_file() && p.extension().is_some_and(|ext| ext == "md");
    let mut out = Vec::new();
    if !cron_dir.is_dir() {
        return Ok(out);
    }
    for entry in std::fs::read_dir(cron_dir)? {
        let entry = entry?;
        let path = entry.path();
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        if name.starts_with('.') {
            continue;
        }
        if is_md(&path) {
            out.push(name);
        } else if path.is_dir() {
            for inner in std::fs::read_dir(&path)? {
                let inner = inner?;
                if let Ok(file) = inner.file_name().into_string() {
                    if !file.starts_with('.') && is_md(&inner.path()) {
                        out.push(format!("{name}/{file}"));
                    }
                }
            }
        }
    }
    out.sort();
    Ok(out)
}

/// Parse a single cron file from its filename and content.
fn parse_cron_file(filename: &str, content: &str) -> Result<CronFile, DecreeError> {
    let message = Message::parse(content)
        .map_err(|(line, e)| DecreeError::Other(format!("{filename}: line {line}: {e}")))?;
    let cron_expr = message
        .text("cron")
        .ok_or_else(|| DecreeError::Other(format!("no cron field in {filename}")))?
        .to_string();
    let when = parse_when(&cron_expr)
        .map_err(|e| DecreeError::Other(format!("invalid cron expression in {filename}: {e}")))?;
    Ok(CronFile {
        filename: filename.to_string(),
        cron_expr,
        when,
        message,
    })
}

/// Parse a cron expression. Standard 5-field cron is accepted: the cron crate expects 6 or 7
/// fields (seconds included), so a 5-field expression gets a "0" seconds prefix.
fn parse_schedule(cron_expr: &str) -> Result<cron::Schedule, cron::error::Error> {
    let fields_count = cron_expr.split_whitespace().count();
    let schedule_expr = if fields_count == 5 {
        format!("0 {cron_expr}")
    } else {
        cron_expr.to_string()
    };
    cron::Schedule::from_str(&schedule_expr)
}

/// Tracker for preventing duplicate firings within the same minute.
#[derive(Debug, Default)]
pub struct CronTracker {
    /// Maps cron filename to the last minute string it fired (e.g., "202603041530").
    last_fire: HashMap<String, String>,
}

impl CronTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Check if a cron file is due and hasn't already fired this minute.
    /// Returns true if the job should fire. A `@reboot` file is never due: it fires only
    /// when the daemon starts.
    pub fn is_due(&self, cron_file: &CronFile) -> bool {
        let When::Schedule(schedule) = &cron_file.when else {
            return false;
        };
        let now = Utc::now();
        let minute_key = now.format("%Y%m%d%H%M").to_string();

        // Check if already fired this minute
        if let Some(last) = self.last_fire.get(&cron_file.filename) {
            if last == &minute_key {
                return false;
            }
        }

        // Check if the cron expression matches the current time
        // Get the next upcoming occurrence and see if it falls within the current minute
        if let Some(next) = schedule.upcoming(Utc).next() {
            let diff = next.signed_duration_since(now);
            // If the next occurrence is within 60 seconds, the current minute matches
            diff.num_seconds() < 60
        } else {
            false
        }
    }

    /// Record that a cron file has fired.
    pub fn mark_fired(&mut self, cron_file: &CronFile) {
        let minute_key = Utc::now().format("%Y%m%d%H%M").to_string();
        self.last_fire
            .insert(cron_file.filename.clone(), minute_key);
    }
}

/// The message a fired cron job queues: the cron file's keys in the order written, without
/// `cron` (and any `trigger`), then `trigger: cron`; and its body.
/// `message::queue` writes it to `inbox/` and gives it its `id`.
pub fn cron_to_inbox_message(cron_file: &CronFile) -> Message {
    let source = &cron_file.message;
    let mut frontmatter = Mapping::new();
    for (key, value) in &source.frontmatter {
        match key.as_str() {
            Some("cron" | "trigger") => {}
            _ => {
                frontmatter.insert(key.clone(), value.clone());
            }
        }
    }
    frontmatter.insert("trigger".into(), Value::from("cron"));
    let mut message = Message::new(source.body.as_str());
    message.frontmatter = frontmatter;
    message
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
    }

    #[test]
    fn test_parse_cron_file_basic() {
        let content = "---\ncron: \"0 * * * *\"\nmachine: develop\n---\nRun hourly task.\n";
        let cf = parse_cron_file("hourly-task.md", content).unwrap();
        assert_eq!(cf.filename, "hourly-task.md");
        assert_eq!(cf.machine(), Some("develop"));
        assert_eq!(cf.message.body, "Run hourly task.\n");
    }

    #[test]
    fn test_parse_cron_file_no_machine() {
        let content = "---\ncron: \"*/15 * * * *\"\n---\nEvery 15 minutes.\n";
        let cf = parse_cron_file("frequent.md", content).unwrap();
        assert!(cf.machine().is_none());
    }

    #[test]
    fn test_parse_cron_file_crlf() {
        let content = "---\r\ncron: \"0 * * * *\"\r\nmachine: develop\r\n---\r\nBody.\r\n";
        let cf = parse_cron_file("crlf.md", content).unwrap();
        assert_eq!(cf.cron_expr, "0 * * * *");
        assert_eq!(cf.machine(), Some("develop"));
        assert_eq!(cf.message.body, "Body.\r\n");
    }

    #[test]
    fn test_inbox_message_keeps_key_order_and_drops_cron() {
        let content = "---\nzeta: 1\ncron: \"0 9 * * *\"\nmachine: develop\ntrigger: manual\nalpha: [a, b]\n---\nDaily task.\n";
        let cf = parse_cron_file("daily.md", content).unwrap();
        let msg = cron_to_inbox_message(&cf);
        assert_eq!(
            String::from_utf8(msg.to_bytes()).unwrap(),
            "---\nzeta: 1\nmachine: develop\nalpha:\n- a\n- b\ntrigger: cron\n---\nDaily task.\n"
        );
    }

    #[test]
    fn test_parse_cron_file_no_cron_field() {
        let content = "---\nmachine: develop\n---\nBody.\n";
        let result = parse_cron_file("test.md", content);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_cron_file_invalid_expression() {
        let content = "---\ncron: \"invalid cron\"\n---\nBody.\n";
        let result = parse_cron_file("test.md", content);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_cron_file_no_frontmatter() {
        let content = "Just plain text.\n";
        let result = parse_cron_file("test.md", content);
        assert!(result.is_err());
    }

    #[test]
    fn test_scan_cron_files_empty() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);
        let files = scan_cron_files(dir.path()).unwrap();
        assert!(files.is_empty());
    }

    #[test]
    fn test_scan_cron_files_no_dir() {
        let dir = TempDir::new().unwrap();
        let files = scan_cron_files(dir.path()).unwrap();
        assert!(files.is_empty());
    }

    #[test]
    fn test_scan_cron_files_sorted() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);
        let cron_dir = dir.path().join(".decree/cron");

        std::fs::write(
            cron_dir.join("beta.md"),
            "---\ncron: \"0 * * * *\"\n---\nBeta.\n",
        )
        .unwrap();
        std::fs::write(
            cron_dir.join("alpha.md"),
            "---\ncron: \"0 * * * *\"\n---\nAlpha.\n",
        )
        .unwrap();
        // Non-md files should be excluded
        std::fs::write(cron_dir.join("notes.txt"), "not cron").unwrap();

        let files = scan_cron_files(dir.path()).unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].filename, "alpha.md");
        assert_eq!(files[1].filename, "beta.md");
    }

    #[test]
    fn test_scan_cron_files_skips_invalid() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);
        let cron_dir = dir.path().join(".decree/cron");

        std::fs::write(
            cron_dir.join("valid.md"),
            "---\ncron: \"0 * * * *\"\n---\nValid.\n",
        )
        .unwrap();
        std::fs::write(
            cron_dir.join("invalid.md"),
            "---\ncron: \"bad expression\"\n---\nInvalid.\n",
        )
        .unwrap();
        std::fs::write(
            cron_dir.join("no-cron.md"),
            "---\nmachine: develop\n---\nNo cron.\n",
        )
        .unwrap();

        let files = scan_cron_files(dir.path()).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].filename, "valid.md");
    }

    #[test]
    fn test_cron_tracker_prevents_duplicate() {
        let content = "---\ncron: \"* * * * *\"\n---\nBody.\n";
        let cf = parse_cron_file("test.md", content).unwrap();

        let mut tracker = CronTracker::new();

        // First check: is_due should return true (every minute matches)
        assert!(tracker.is_due(&cf));

        // Mark as fired
        tracker.mark_fired(&cf);

        // Second check within same minute: should return false
        assert!(!tracker.is_due(&cf));
    }

    #[test]
    fn test_cron_to_inbox_message() {
        let content = "---\ncron: \"0 * * * *\"\nmachine: develop\npriority: high\ntrigger: x\n---\nHourly maintenance.\n";
        let cf = parse_cron_file("hourly-maintenance.md", content).unwrap();
        let msg = cron_to_inbox_message(&cf);
        assert_eq!(
            String::from_utf8(msg.to_bytes()).unwrap(),
            "---\nmachine: develop\npriority: high\ntrigger: cron\n---\nHourly maintenance.\n"
        );
    }

    #[test]
    fn test_cron_to_inbox_message_without_machine() {
        let cf = parse_cron_file("task.md", "---\ncron: \"0 * * * *\"\n---\nTask.\n").unwrap();
        let msg = cron_to_inbox_message(&cf);
        assert_eq!(msg.text("machine"), None);
        assert_eq!(msg.text("trigger"), Some("cron"));
    }

    fn next(cf: &CronFile) -> bool {
        match &cf.when {
            When::Schedule(s) => s.upcoming(Utc).next().is_some(),
            When::DaemonStart => false,
        }
    }

    #[test]
    fn test_reboot_fires_at_daemon_start_only() {
        let cf = parse_cron_file(
            "boot.md",
            "---\ncron: \"@reboot\"\nmachine: boot\n---\nWarm up.\n",
        )
        .unwrap();
        assert!(matches!(cf.when, When::DaemonStart));
        assert_eq!(cf.cron_expr, "@reboot");
        assert!(!CronTracker::new().is_due(&cf));
        let msg = cron_to_inbox_message(&cf);
        assert_eq!(
            String::from_utf8(msg.to_bytes()).unwrap(),
            "---\nmachine: boot\ntrigger: cron\n---\nWarm up.\n"
        );
    }

    #[test]
    fn test_parse_when_only_reboot_is_new() {
        assert!(matches!(parse_when("@reboot"), Ok(When::DaemonStart)));
        assert!(matches!(parse_when("0 * * * *"), Ok(When::Schedule(_))));
        assert!(parse_when("@daemon").is_err());
        assert!(parse_when("@reboot 0").is_err());
    }

    #[test]
    fn test_various_cron_expressions() {
        // Every minute
        let cf = parse_cron_file("t.md", "---\ncron: \"* * * * *\"\n---\n").unwrap();
        assert!(next(&cf));

        // Every hour
        let cf = parse_cron_file("t.md", "---\ncron: \"0 * * * *\"\n---\n").unwrap();
        assert!(next(&cf));

        // Daily at 9am
        let cf = parse_cron_file("t.md", "---\ncron: \"0 9 * * *\"\n---\n").unwrap();
        assert!(next(&cf));

        // Weekdays at 9am
        let cf = parse_cron_file("t.md", "---\ncron: \"0 9 * * 1-5\"\n---\n").unwrap();
        assert!(next(&cf));

        // Every 15 minutes
        let cf = parse_cron_file("t.md", "---\ncron: \"*/15 * * * *\"\n---\n").unwrap();
        assert!(next(&cf));
    }
}
