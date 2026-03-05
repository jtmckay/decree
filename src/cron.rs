use crate::config;
use crate::error::DecreeError;
use crate::message::{build_chain_id, next_day_counter, parse_frontmatter, InboxMessage};
use chrono::{Local, Utc};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::str::FromStr;

/// A parsed cron file from `.decree/cron/`.
#[derive(Debug, Clone)]
pub struct CronFile {
    /// Filename (e.g., `hourly-maintenance.md`).
    pub filename: String,
    /// The stem used for chain ID naming (e.g., `hourly-maintenance`).
    pub name_stem: String,
    /// Parsed cron schedule.
    pub schedule: cron::Schedule,
    /// Optional routine override.
    pub routine: Option<String>,
    /// Custom frontmatter fields (cron field stripped).
    pub custom_fields: BTreeMap<String, serde_yaml::Value>,
    /// Markdown body.
    pub body: String,
}

/// Scan `.decree/cron/` for valid cron files.
pub fn scan_cron_files(project_root: &Path) -> Result<Vec<CronFile>, DecreeError> {
    let cron_dir = project_root
        .join(config::DECREE_DIR)
        .join(config::CRON_DIR);

    if !cron_dir.exists() {
        return Ok(Vec::new());
    }

    let mut entries: Vec<String> = std::fs::read_dir(&cron_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_file() && e.path().extension().is_some_and(|ext| ext == "md"))
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();

    entries.sort();

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

/// Parse a single cron file from its filename and content.
fn parse_cron_file(filename: &str, content: &str) -> Result<CronFile, DecreeError> {
    let (fields, body) = parse_frontmatter(content)?;

    let cron_expr = fields
        .get("cron")
        .and_then(|v| match v {
            serde_yaml::Value::String(s) => Some(s.clone()),
            _ => None,
        })
        .ok_or_else(|| DecreeError::Other(format!("no cron field in {filename}")))?;

    // The cron crate expects 6 or 7 fields (seconds included).
    // Standard 5-field cron needs a "0" seconds prefix.
    let fields_count = cron_expr.split_whitespace().count();
    let schedule_expr = if fields_count == 5 {
        format!("0 {cron_expr}")
    } else {
        cron_expr.clone()
    };

    let schedule = cron::Schedule::from_str(&schedule_expr)
        .map_err(|e| DecreeError::Other(format!("invalid cron expression in {filename}: {e}")))?;

    let routine = fields.get("routine").and_then(|v| match v {
        serde_yaml::Value::String(s) => Some(s.clone()),
        _ => None,
    });

    // Collect custom fields, stripping "cron" and known message fields
    let strip_fields: &[&str] = &["cron", "routine"];
    let custom_fields: BTreeMap<String, serde_yaml::Value> = fields
        .into_iter()
        .filter(|(k, _)| !strip_fields.contains(&k.as_str()))
        .collect();

    let name_stem = filename
        .strip_suffix(".md")
        .unwrap_or(filename)
        .to_string();

    Ok(CronFile {
        filename: filename.to_string(),
        name_stem,
        schedule,
        routine,
        custom_fields,
        body,
    })
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
    /// Returns true if the job should fire.
    pub fn is_due(&self, cron_file: &CronFile) -> bool {
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
        if let Some(next) = cron_file.schedule.upcoming(Utc).next() {
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

/// Create an inbox message from a fired cron job.
pub fn cron_to_inbox_message(
    project_root: &Path,
    cron_file: &CronFile,
) -> Result<InboxMessage, DecreeError> {
    let now = Local::now();
    let hhmm = now.format("%H%M").to_string();
    let day = next_day_counter(project_root, &hhmm)?;
    let chain = build_chain_id(&day, &hhmm, &cron_file.name_stem);
    let filename = format!("{chain}-0.md");
    let id = format!("{chain}-0");

    Ok(InboxMessage {
        id: Some(id),
        chain: Some(chain),
        seq: Some(0),
        routine: cron_file.routine.clone(),
        migration: None,
        body: cron_file.body.clone(),
        custom_fields: cron_file.custom_fields.clone(),
        filename,
    })
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
        let content = "---\ncron: \"0 * * * *\"\nroutine: develop\n---\nRun hourly task.\n";
        let cf = parse_cron_file("hourly-task.md", content).unwrap();
        assert_eq!(cf.filename, "hourly-task.md");
        assert_eq!(cf.name_stem, "hourly-task");
        assert_eq!(cf.routine, Some("develop".to_string()));
        assert_eq!(cf.body, "Run hourly task.\n");
        assert!(cf.custom_fields.is_empty());
    }

    #[test]
    fn test_parse_cron_file_no_routine() {
        let content = "---\ncron: \"*/15 * * * *\"\n---\nEvery 15 minutes.\n";
        let cf = parse_cron_file("frequent.md", content).unwrap();
        assert!(cf.routine.is_none());
    }

    #[test]
    fn test_parse_cron_file_custom_fields() {
        let content =
            "---\ncron: \"0 9 * * *\"\npriority: high\ntags: daily\n---\nDaily task.\n";
        let cf = parse_cron_file("daily.md", content).unwrap();
        assert_eq!(cf.custom_fields.len(), 2);
        assert_eq!(
            cf.custom_fields.get("priority"),
            Some(&serde_yaml::Value::String("high".into()))
        );
    }

    #[test]
    fn test_parse_cron_file_strips_cron_field() {
        let content = "---\ncron: \"0 * * * *\"\nroutine: develop\n---\nBody.\n";
        let cf = parse_cron_file("test.md", content).unwrap();
        // "cron" should NOT be in custom_fields
        assert!(!cf.custom_fields.contains_key("cron"));
        // "routine" is extracted separately, also not in custom_fields
        assert!(!cf.custom_fields.contains_key("routine"));
    }

    #[test]
    fn test_parse_cron_file_no_cron_field() {
        let content = "---\nroutine: develop\n---\nBody.\n";
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
            "---\nroutine: develop\n---\nNo cron.\n",
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
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);

        let content = "---\ncron: \"0 * * * *\"\nroutine: develop\npriority: high\n---\nHourly maintenance.\n";
        let cf = parse_cron_file("hourly-maintenance.md", content).unwrap();

        let msg = cron_to_inbox_message(dir.path(), &cf).unwrap();

        // Check chain contains the cron file stem
        let chain = msg.chain.as_ref().unwrap();
        assert!(chain.contains("hourly-maintenance"));

        // Check seq is 0
        assert_eq!(msg.seq, Some(0));

        // Check routine preserved
        assert_eq!(msg.routine.as_deref(), Some("develop"));

        // Check custom fields preserved (cron stripped)
        assert!(!msg.custom_fields.contains_key("cron"));
        assert_eq!(
            msg.custom_fields.get("priority"),
            Some(&serde_yaml::Value::String("high".into()))
        );

        // Check body preserved
        assert_eq!(msg.body, "Hourly maintenance.\n");

        // Check filename format
        assert!(msg.filename.ends_with("-0.md"));
    }

    #[test]
    fn test_cron_to_inbox_message_no_routine() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);

        let content = "---\ncron: \"0 * * * *\"\n---\nTask.\n";
        let cf = parse_cron_file("task.md", content).unwrap();

        let msg = cron_to_inbox_message(dir.path(), &cf).unwrap();
        assert!(msg.routine.is_none());
    }

    #[test]
    fn test_various_cron_expressions() {
        // Every minute
        let cf = parse_cron_file("t.md", "---\ncron: \"* * * * *\"\n---\n").unwrap();
        assert!(cf.schedule.upcoming(Utc).next().is_some());

        // Every hour
        let cf = parse_cron_file("t.md", "---\ncron: \"0 * * * *\"\n---\n").unwrap();
        assert!(cf.schedule.upcoming(Utc).next().is_some());

        // Daily at 9am
        let cf = parse_cron_file("t.md", "---\ncron: \"0 9 * * *\"\n---\n").unwrap();
        assert!(cf.schedule.upcoming(Utc).next().is_some());

        // Weekdays at 9am
        let cf = parse_cron_file("t.md", "---\ncron: \"0 9 * * 1-5\"\n---\n").unwrap();
        assert!(cf.schedule.upcoming(Utc).next().is_some());

        // Every 15 minutes
        let cf = parse_cron_file("t.md", "---\ncron: \"*/15 * * * *\"\n---\n").unwrap();
        assert!(cf.schedule.upcoming(Utc).next().is_some());
    }
}
