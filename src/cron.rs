use std::collections::BTreeMap;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{Datelike, Local, NaiveDateTime, Timelike};
use serde_yaml::Value;

use crate::error::{DecreeError, Result};
use crate::message::{self, Message, MessageType};
use crate::migration::split_frontmatter;

// ---------------------------------------------------------------------------
// Cron expression parsing
// ---------------------------------------------------------------------------

/// A single field in a cron expression (e.g., minute, hour).
#[derive(Debug, Clone, PartialEq, Eq)]
enum CronField {
    /// Matches all values (`*`).
    Any,
    /// Matches specific values.
    Values(Vec<u32>),
}

impl CronField {
    fn matches(&self, value: u32) -> bool {
        match self {
            CronField::Any => true,
            CronField::Values(vals) => vals.contains(&value),
        }
    }
}

/// A parsed 5-field cron expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronExpr {
    minute: CronField,
    hour: CronField,
    dom: CronField,
    month: CronField,
    dow: CronField,
}

impl CronExpr {
    /// Parse a standard 5-field cron expression.
    ///
    /// Fields: minute hour day-of-month month day-of-week
    /// Supports: `*`, numbers, ranges (`1-5`), lists (`1,3,5`), steps (`*/5`, `1-10/2`).
    pub fn parse(expr: &str) -> Result<Self> {
        let fields: Vec<&str> = expr.split_whitespace().collect();
        if fields.len() != 5 {
            return Err(DecreeError::Config(format!(
                "cron expression must have 5 fields, got {}: {expr}",
                fields.len()
            )));
        }

        Ok(Self {
            minute: parse_field(fields[0], 0, 59)?,
            hour: parse_field(fields[1], 0, 23)?,
            dom: parse_field(fields[2], 1, 31)?,
            month: parse_field(fields[3], 1, 12)?,
            dow: parse_field(fields[4], 0, 6)?,
        })
    }

    /// Check if this expression matches the given time.
    pub fn matches_time(&self, time: &NaiveDateTime) -> bool {
        self.minute.matches(time.minute())
            && self.hour.matches(time.hour())
            && self.dom.matches(time.day())
            && self.month.matches(time.month())
            && self.dow.matches(time.weekday().num_days_from_sunday())
    }
}

/// Parse a single cron field (e.g., `*/5`, `1-3`, `1,2,3`, `*`).
fn parse_field(field: &str, min: u32, max: u32) -> Result<CronField> {
    // Handle step notation: `<range>/<step>`
    if let Some((range_part, step_part)) = field.split_once('/') {
        let step: u32 = step_part
            .parse()
            .map_err(|_| DecreeError::Config(format!("invalid cron step: {field}")))?;
        if step == 0 {
            return Err(DecreeError::Config(format!("cron step cannot be 0: {field}")));
        }

        let (start, end) = if range_part == "*" {
            (min, max)
        } else {
            parse_range(range_part, min, max)?
        };

        let values: Vec<u32> = (start..=end).step_by(step as usize).collect();
        return Ok(CronField::Values(values));
    }

    // `*` matches all
    if field == "*" {
        return Ok(CronField::Any);
    }

    // List: `1,3,5`
    if field.contains(',') {
        let mut values = Vec::new();
        for part in field.split(',') {
            if part.contains('-') {
                let (start, end) = parse_range(part, min, max)?;
                values.extend(start..=end);
            } else {
                let v: u32 = part.parse().map_err(|_| {
                    DecreeError::Config(format!("invalid cron value: {part}"))
                })?;
                validate_range(v, min, max, field)?;
                values.push(v);
            }
        }
        values.sort();
        values.dedup();
        return Ok(CronField::Values(values));
    }

    // Range: `1-5`
    if field.contains('-') {
        let (start, end) = parse_range(field, min, max)?;
        let values: Vec<u32> = (start..=end).collect();
        return Ok(CronField::Values(values));
    }

    // Single value
    let v: u32 = field
        .parse()
        .map_err(|_| DecreeError::Config(format!("invalid cron value: {field}")))?;
    validate_range(v, min, max, field)?;
    Ok(CronField::Values(vec![v]))
}

fn parse_range(s: &str, min: u32, max: u32) -> Result<(u32, u32)> {
    let parts: Vec<&str> = s.splitn(2, '-').collect();
    if parts.len() != 2 {
        return Err(DecreeError::Config(format!("invalid cron range: {s}")));
    }
    let start: u32 = parts[0]
        .parse()
        .map_err(|_| DecreeError::Config(format!("invalid cron range start: {s}")))?;
    let end: u32 = parts[1]
        .parse()
        .map_err(|_| DecreeError::Config(format!("invalid cron range end: {s}")))?;
    validate_range(start, min, max, s)?;
    validate_range(end, min, max, s)?;
    Ok((start, end))
}

fn validate_range(value: u32, min: u32, max: u32, context: &str) -> Result<()> {
    if value < min || value > max {
        return Err(DecreeError::Config(format!(
            "cron value {value} out of range {min}-{max}: {context}"
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Cron file scanning
// ---------------------------------------------------------------------------

/// A parsed cron file from `.decree/cron/`.
#[derive(Debug)]
pub struct CronFile {
    pub path: PathBuf,
    pub cron_expr: CronExpr,
    pub routine: Option<String>,
    pub custom_fields: BTreeMap<String, Value>,
    pub body: String,
}

/// Known frontmatter fields that get special handling.
const CRON_KNOWN_FIELDS: &[&str] = &["cron", "routine"];

impl CronFile {
    /// Load and parse a cron file. Returns `None` if the file has no `cron` field.
    pub fn load(path: &Path) -> Result<Option<Self>> {
        let content = fs::read_to_string(path)?;
        let (frontmatter, body) = split_frontmatter(&content);

        let yaml_str = match frontmatter {
            Some(s) if !s.trim().is_empty() => s,
            _ => return Ok(None),
        };

        let mapping: BTreeMap<String, Value> = serde_yaml::from_str(yaml_str)
            .map_err(|e| DecreeError::Config(format!("invalid cron YAML in {}: {e}", path.display())))?;

        // Extract `cron` field — required
        let cron_value = match mapping.get("cron") {
            Some(Value::String(s)) => s.clone(),
            Some(_) => {
                return Err(DecreeError::Config(format!(
                    "cron field must be a string in {}",
                    path.display()
                )));
            }
            None => return Ok(None),
        };

        let cron_expr = CronExpr::parse(&cron_value)?;

        // Extract `routine` field — optional
        let routine = mapping
            .get("routine")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        // Collect custom fields (everything except cron and routine)
        let mut custom_fields = BTreeMap::new();
        for (key, value) in &mapping {
            if !CRON_KNOWN_FIELDS.contains(&key.as_str()) {
                custom_fields.insert(key.clone(), value.clone());
            }
        }

        Ok(Some(Self {
            path: path.to_path_buf(),
            cron_expr,
            routine,
            custom_fields,
            body: body.to_string(),
        }))
    }
}

/// Scan `.decree/cron/` for all valid cron files.
pub fn scan_cron_dir(cron_dir: &Path) -> Result<Vec<CronFile>> {
    if !cron_dir.exists() {
        return Ok(Vec::new());
    }

    let mut cron_files = Vec::new();
    for entry in fs::read_dir(cron_dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        match CronFile::load(&path)? {
            Some(cf) => cron_files.push(cf),
            None => {} // no cron field, skip
        }
    }

    Ok(cron_files)
}

// ---------------------------------------------------------------------------
// Cron tracker — prevents duplicate firings within the same minute
// ---------------------------------------------------------------------------

/// Tracks the last fire minute for each cron file to prevent duplicates.
pub struct CronTracker {
    last_fired: HashMap<PathBuf, NaiveDateTime>,
}

impl CronTracker {
    pub fn new() -> Self {
        Self {
            last_fired: HashMap::new(),
        }
    }

    /// Check and fire due cron jobs. Returns the list of inbox messages created.
    pub fn check_and_fire(
        &mut self,
        cron_dir: &Path,
        inbox_dir: &Path,
    ) -> Result<Vec<PathBuf>> {
        let now = Local::now().naive_local();
        // Truncate to the current minute
        let current_minute = now
            .with_second(0)
            .and_then(|t| t.with_nanosecond(0))
            .unwrap_or(now);

        let cron_files = scan_cron_dir(cron_dir)?;
        let mut created = Vec::new();

        for cf in cron_files {
            // Check if expression matches current time
            if !cf.cron_expr.matches_time(&current_minute) {
                continue;
            }

            // Check if already fired this minute
            if let Some(last) = self.last_fired.get(&cf.path) {
                if *last == current_minute {
                    continue;
                }
            }

            // Fire: create inbox message
            let path = create_inbox_message(&cf, inbox_dir)?;
            self.last_fired.insert(cf.path.clone(), current_minute);
            created.push(path);
        }

        Ok(created)
    }
}

/// Create an inbox message from a cron file.
fn create_inbox_message(cf: &CronFile, inbox_dir: &Path) -> Result<PathBuf> {
    let chain = message::generate_chain_id();
    let id = format!("{chain}-0");
    let filename = format!("{chain}-0.md");
    let path = inbox_dir.join(&filename);

    let msg = Message {
        id,
        chain,
        seq: 0,
        message_type: MessageType::Task,
        input_file: None,
        routine: cf.routine.clone().unwrap_or_else(|| String::new()),
        custom_fields: cf.custom_fields.clone(),
        body: cf.body.clone(),
        path: path.clone(),
    };

    // Write the message — the routine field will be empty string if not set,
    // which means normalization will apply the usual fallback chain.
    msg.write()?;

    Ok(path)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- CronExpr parsing --

    #[test]
    fn test_parse_all_stars() {
        let expr = CronExpr::parse("* * * * *").unwrap();
        assert_eq!(expr.minute, CronField::Any);
        assert_eq!(expr.hour, CronField::Any);
        assert_eq!(expr.dom, CronField::Any);
        assert_eq!(expr.month, CronField::Any);
        assert_eq!(expr.dow, CronField::Any);
    }

    #[test]
    fn test_parse_specific_values() {
        let expr = CronExpr::parse("30 14 1 6 3").unwrap();
        assert_eq!(expr.minute, CronField::Values(vec![30]));
        assert_eq!(expr.hour, CronField::Values(vec![14]));
        assert_eq!(expr.dom, CronField::Values(vec![1]));
        assert_eq!(expr.month, CronField::Values(vec![6]));
        assert_eq!(expr.dow, CronField::Values(vec![3]));
    }

    #[test]
    fn test_parse_range() {
        let expr = CronExpr::parse("1-5 * * * *").unwrap();
        assert_eq!(expr.minute, CronField::Values(vec![1, 2, 3, 4, 5]));
    }

    #[test]
    fn test_parse_list() {
        let expr = CronExpr::parse("0,15,30,45 * * * *").unwrap();
        assert_eq!(expr.minute, CronField::Values(vec![0, 15, 30, 45]));
    }

    #[test]
    fn test_parse_step() {
        let expr = CronExpr::parse("*/15 * * * *").unwrap();
        assert_eq!(expr.minute, CronField::Values(vec![0, 15, 30, 45]));
    }

    #[test]
    fn test_parse_range_step() {
        let expr = CronExpr::parse("1-10/3 * * * *").unwrap();
        assert_eq!(expr.minute, CronField::Values(vec![1, 4, 7, 10]));
    }

    #[test]
    fn test_parse_list_with_range() {
        let expr = CronExpr::parse("1-3,7,10-12 * * * *").unwrap();
        assert_eq!(expr.minute, CronField::Values(vec![1, 2, 3, 7, 10, 11, 12]));
    }

    #[test]
    fn test_parse_too_few_fields() {
        assert!(CronExpr::parse("* * *").is_err());
    }

    #[test]
    fn test_parse_too_many_fields() {
        assert!(CronExpr::parse("* * * * * *").is_err());
    }

    #[test]
    fn test_parse_out_of_range() {
        assert!(CronExpr::parse("60 * * * *").is_err());
        assert!(CronExpr::parse("* 24 * * *").is_err());
        assert!(CronExpr::parse("* * 0 * *").is_err());
        assert!(CronExpr::parse("* * * 13 *").is_err());
        assert!(CronExpr::parse("* * * * 7").is_err());
    }

    #[test]
    fn test_parse_zero_step() {
        assert!(CronExpr::parse("*/0 * * * *").is_err());
    }

    // -- CronExpr matching --

    #[test]
    fn test_matches_all_stars() {
        let expr = CronExpr::parse("* * * * *").unwrap();
        // Any time should match
        let time = NaiveDateTime::parse_from_str("2025-06-15 14:30:00", "%Y-%m-%d %H:%M:%S")
            .unwrap();
        assert!(expr.matches_time(&time));
    }

    #[test]
    fn test_matches_specific_minute() {
        let expr = CronExpr::parse("0 * * * *").unwrap();
        let match_time =
            NaiveDateTime::parse_from_str("2025-06-15 14:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
        let no_match =
            NaiveDateTime::parse_from_str("2025-06-15 14:30:00", "%Y-%m-%d %H:%M:%S").unwrap();
        assert!(expr.matches_time(&match_time));
        assert!(!expr.matches_time(&no_match));
    }

    #[test]
    fn test_matches_specific_time() {
        let expr = CronExpr::parse("30 14 15 6 *").unwrap();
        let match_time =
            NaiveDateTime::parse_from_str("2025-06-15 14:30:00", "%Y-%m-%d %H:%M:%S").unwrap();
        assert!(expr.matches_time(&match_time));

        let no_match =
            NaiveDateTime::parse_from_str("2025-06-15 14:31:00", "%Y-%m-%d %H:%M:%S").unwrap();
        assert_eq!(no_match.minute(), 31); // sanity
        assert!(!expr.matches_time(&no_match));
    }

    #[test]
    fn test_matches_dow() {
        // 2025-06-15 is a Sunday (dow=0)
        let expr = CronExpr::parse("* * * * 0").unwrap();
        let sunday =
            NaiveDateTime::parse_from_str("2025-06-15 12:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
        let monday =
            NaiveDateTime::parse_from_str("2025-06-16 12:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
        assert!(expr.matches_time(&sunday));
        assert!(!expr.matches_time(&monday));
    }

    #[test]
    fn test_matches_step() {
        let expr = CronExpr::parse("*/15 * * * *").unwrap();
        let m0 =
            NaiveDateTime::parse_from_str("2025-06-15 12:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
        let m15 =
            NaiveDateTime::parse_from_str("2025-06-15 12:15:00", "%Y-%m-%d %H:%M:%S").unwrap();
        let m7 =
            NaiveDateTime::parse_from_str("2025-06-15 12:07:00", "%Y-%m-%d %H:%M:%S").unwrap();
        assert!(expr.matches_time(&m0));
        assert!(expr.matches_time(&m15));
        assert!(!expr.matches_time(&m7));
    }

    // -- CronFile parsing --

    #[test]
    fn test_cron_file_load() {
        let tmp = std::env::temp_dir().join("decree_test_cron_load");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        let path = tmp.join("hourly.md");
        fs::write(
            &path,
            "---\ncron: \"0 * * * *\"\nroutine: develop\n---\nRun hourly task.\n",
        )
        .unwrap();

        let cf = CronFile::load(&path).unwrap().unwrap();
        assert_eq!(cf.routine, Some("develop".to_string()));
        assert_eq!(cf.body.trim(), "Run hourly task.");
        assert!(cf.custom_fields.is_empty());

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_cron_file_no_cron_field() {
        let tmp = std::env::temp_dir().join("decree_test_cron_no_field");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        let path = tmp.join("no-cron.md");
        fs::write(&path, "---\nroutine: develop\n---\nJust a file.\n").unwrap();

        let result = CronFile::load(&path).unwrap();
        assert!(result.is_none());

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_cron_file_custom_fields() {
        let tmp = std::env::temp_dir().join("decree_test_cron_custom");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        let path = tmp.join("custom.md");
        fs::write(
            &path,
            "---\ncron: \"*/5 * * * *\"\nroutine: develop\npriority: 5\ntag: maintenance\n---\nBody.\n",
        )
        .unwrap();

        let cf = CronFile::load(&path).unwrap().unwrap();
        assert_eq!(cf.routine, Some("develop".to_string()));
        assert_eq!(
            cf.custom_fields.get("priority"),
            Some(&Value::Number(serde_yaml::Number::from(5)))
        );
        assert_eq!(
            cf.custom_fields.get("tag"),
            Some(&Value::String("maintenance".to_string()))
        );

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_cron_file_no_routine() {
        let tmp = std::env::temp_dir().join("decree_test_cron_no_routine");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        let path = tmp.join("no-routine.md");
        fs::write(&path, "---\ncron: \"0 * * * *\"\n---\nTask.\n").unwrap();

        let cf = CronFile::load(&path).unwrap().unwrap();
        assert!(cf.routine.is_none());

        let _ = fs::remove_dir_all(&tmp);
    }

    // -- scan_cron_dir --

    #[test]
    fn test_scan_cron_dir() {
        let tmp = std::env::temp_dir().join("decree_test_scan_cron");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        fs::write(
            tmp.join("hourly.md"),
            "---\ncron: \"0 * * * *\"\n---\nHourly.\n",
        )
        .unwrap();
        fs::write(
            tmp.join("daily.md"),
            "---\ncron: \"0 9 * * *\"\n---\nDaily.\n",
        )
        .unwrap();
        fs::write(tmp.join("not-cron.md"), "Just text, no frontmatter.\n").unwrap();
        fs::write(tmp.join("readme.txt"), "Not an md file.").unwrap();

        let files = scan_cron_dir(&tmp).unwrap();
        assert_eq!(files.len(), 2);

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_scan_nonexistent_dir() {
        let files = scan_cron_dir(Path::new("/nonexistent/path")).unwrap();
        assert!(files.is_empty());
    }

    // -- CronTracker --

    #[test]
    fn test_cron_tracker_fires_and_deduplicates() {
        let tmp = std::env::temp_dir().join("decree_test_cron_tracker");
        let _ = fs::remove_dir_all(&tmp);
        let cron_dir = tmp.join("cron");
        let inbox_dir = tmp.join("inbox");
        fs::create_dir_all(&cron_dir).unwrap();
        fs::create_dir_all(&inbox_dir).unwrap();

        // Write a cron file that matches every minute
        fs::write(
            cron_dir.join("every-minute.md"),
            "---\ncron: \"* * * * *\"\nroutine: develop\n---\nEvery minute task.\n",
        )
        .unwrap();

        let mut tracker = CronTracker::new();

        // First check should fire
        let created1 = tracker.check_and_fire(&cron_dir, &inbox_dir).unwrap();
        assert_eq!(created1.len(), 1);

        // Second check within same minute should not fire
        let created2 = tracker.check_and_fire(&cron_dir, &inbox_dir).unwrap();
        assert_eq!(created2.len(), 0);

        // Verify the created message
        let content = fs::read_to_string(&created1[0]).unwrap();
        assert!(content.contains("type: task"));
        assert!(content.contains("seq: 0"));
        assert!(content.contains("routine: develop"));
        assert!(!content.contains("cron:")); // cron field should be stripped
        assert!(content.contains("Every minute task."));

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_cron_tracker_preserves_custom_fields() {
        let tmp = std::env::temp_dir().join("decree_test_cron_custom_fields");
        let _ = fs::remove_dir_all(&tmp);
        let cron_dir = tmp.join("cron");
        let inbox_dir = tmp.join("inbox");
        fs::create_dir_all(&cron_dir).unwrap();
        fs::create_dir_all(&inbox_dir).unwrap();

        fs::write(
            cron_dir.join("with-fields.md"),
            "---\ncron: \"* * * * *\"\nroutine: develop\npriority: 3\nenv: staging\n---\nBody.\n",
        )
        .unwrap();

        let mut tracker = CronTracker::new();
        let created = tracker.check_and_fire(&cron_dir, &inbox_dir).unwrap();
        assert_eq!(created.len(), 1);

        let content = fs::read_to_string(&created[0]).unwrap();
        assert!(content.contains("priority: 3"));
        assert!(content.contains("env: staging"));
        assert!(!content.contains("cron:"));

        let _ = fs::remove_dir_all(&tmp);
    }

    // -- create_inbox_message --

    #[test]
    fn test_create_inbox_message_format() {
        let tmp = std::env::temp_dir().join("decree_test_create_inbox_msg");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        let cf = CronFile {
            path: PathBuf::from("test.md"),
            cron_expr: CronExpr::parse("0 * * * *").unwrap(),
            routine: Some("develop".to_string()),
            custom_fields: BTreeMap::new(),
            body: "Hourly task.".to_string(),
        };

        let path = create_inbox_message(&cf, &tmp).unwrap();
        assert!(path.exists());

        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("seq: 0"));
        assert!(content.contains("type: task"));
        assert!(content.contains("routine: develop"));
        assert!(content.contains("Hourly task."));

        // Filename should be <chain>-0.md
        let filename = path.file_name().unwrap().to_str().unwrap();
        assert!(filename.ends_with("-0.md"));

        let _ = fs::remove_dir_all(&tmp);
    }
}
