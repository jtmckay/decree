use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{Datelike, NaiveDateTime, Timelike};

use crate::error::DecreeError;
use crate::message::{self, MessageId};

// ---------------------------------------------------------------------------
// Cron expression parsing
// ---------------------------------------------------------------------------

/// A parsed 5-field cron expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronExpr {
    pub minute: CronField,
    pub hour: CronField,
    pub dom: CronField,
    pub month: CronField,
    pub dow: CronField,
}

/// A single cron field: a set of matching values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronField {
    /// The set of values this field matches (e.g. {0, 15, 30, 45} for */15).
    pub values: Vec<u32>,
    /// True if this field matches any value (wildcard).
    pub wildcard: bool,
}

impl CronField {
    fn matches(&self, value: u32) -> bool {
        self.wildcard || self.values.contains(&value)
    }
}

/// Parse a standard 5-field cron expression.
///
/// Fields: minute hour day-of-month month day-of-week
pub fn parse_cron_expr(s: &str) -> Result<CronExpr, DecreeError> {
    let fields: Vec<&str> = s.split_whitespace().collect();
    if fields.len() != 5 {
        return Err(DecreeError::Config(format!(
            "invalid cron expression '{s}': expected 5 fields, got {}",
            fields.len()
        )));
    }

    Ok(CronExpr {
        minute: parse_field(fields[0], 0, 59)?,
        hour: parse_field(fields[1], 0, 23)?,
        dom: parse_field(fields[2], 1, 31)?,
        month: parse_field(fields[3], 1, 12)?,
        dow: parse_field(fields[4], 0, 7)?,
    })
}

/// Parse a single cron field. Supports:
/// - `*` (wildcard)
/// - `N` (single value)
/// - `N-M` (range)
/// - `*/N` (step from min)
/// - `N-M/S` (range with step)
/// - `A,B,C` (list of any of the above)
fn parse_field(s: &str, min: u32, max: u32) -> Result<CronField, DecreeError> {
    if s == "*" {
        return Ok(CronField {
            values: Vec::new(),
            wildcard: true,
        });
    }

    let mut values = Vec::new();

    for part in s.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }

        if let Some(rest) = part.strip_prefix("*/") {
            // Step from min
            let step: u32 = rest
                .parse()
                .map_err(|_| DecreeError::Config(format!("invalid cron step: {part}")))?;
            if step == 0 {
                return Err(DecreeError::Config(format!("cron step cannot be 0: {part}")));
            }
            let mut v = min;
            while v <= max {
                values.push(v);
                v += step;
            }
        } else if part.contains('/') {
            // Range with step: N-M/S
            let (range_part, step_str) = part
                .split_once('/')
                .ok_or_else(|| DecreeError::Config(format!("invalid cron field: {part}")))?;
            let step: u32 = step_str
                .parse()
                .map_err(|_| DecreeError::Config(format!("invalid cron step: {part}")))?;
            if step == 0 {
                return Err(DecreeError::Config(format!("cron step cannot be 0: {part}")));
            }
            let (start, end) = parse_range(range_part, min, max)?;
            let mut v = start;
            while v <= end {
                values.push(v);
                v += step;
            }
        } else if part.contains('-') {
            // Range: N-M
            let (start, end) = parse_range(part, min, max)?;
            for v in start..=end {
                values.push(v);
            }
        } else {
            // Single value
            let v: u32 = part
                .parse()
                .map_err(|_| DecreeError::Config(format!("invalid cron value: {part}")))?;
            if v < min || v > max {
                return Err(DecreeError::Config(format!(
                    "cron value {v} out of range {min}-{max}"
                )));
            }
            values.push(v);
        }
    }

    values.sort();
    values.dedup();
    Ok(CronField {
        values,
        wildcard: false,
    })
}

fn parse_range(s: &str, min: u32, max: u32) -> Result<(u32, u32), DecreeError> {
    let (start_str, end_str) = s
        .split_once('-')
        .ok_or_else(|| DecreeError::Config(format!("invalid cron range: {s}")))?;
    let start: u32 = start_str
        .parse()
        .map_err(|_| DecreeError::Config(format!("invalid cron range start: {s}")))?;
    let end: u32 = end_str
        .parse()
        .map_err(|_| DecreeError::Config(format!("invalid cron range end: {s}")))?;
    if start < min || end > max || start > end {
        return Err(DecreeError::Config(format!(
            "cron range {s} out of bounds {min}-{max}"
        )));
    }
    Ok((start, end))
}

/// Check if a cron expression matches the given time.
pub fn matches_time(expr: &CronExpr, time: &NaiveDateTime) -> bool {
    let minute = time.minute();
    let hour = time.hour();
    let dom = time.day();
    let month = time.month();
    // chrono: Monday=1 .. Sunday=7; cron: Sunday=0, Monday=1 .. Saturday=6, Sunday=7
    let dow_chrono = time.weekday().num_days_from_sunday(); // Sunday=0, Monday=1, ..., Saturday=6

    expr.minute.matches(minute)
        && expr.hour.matches(hour)
        && expr.dom.matches(dom)
        && expr.month.matches(month)
        && (expr.dow.matches(dow_chrono) || (dow_chrono == 0 && expr.dow.matches(7)))
}

// ---------------------------------------------------------------------------
// Cron file scanning
// ---------------------------------------------------------------------------

/// A parsed cron file from `.decree/cron/`.
#[derive(Debug, Clone)]
pub struct CronFile {
    pub path: PathBuf,
    pub name: String,
    pub cron_expr: CronExpr,
    pub routine: Option<String>,
    pub custom_fields: BTreeMap<String, serde_yaml::Value>,
    pub body: String,
}

/// Scan `.decree/cron/` for valid cron files.
pub fn scan_cron_files(cron_dir: &Path) -> Result<Vec<CronFile>, DecreeError> {
    if !cron_dir.is_dir() {
        return Ok(Vec::new());
    }

    let mut files = Vec::new();

    for entry in fs::read_dir(cron_dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let filename = entry.file_name().to_string_lossy().to_string();
        if !filename.ends_with(".md") {
            continue;
        }

        let content = fs::read_to_string(&path)?;
        let (fm, body) = message::parse_message_file(&content);

        // Check for `cron` field in custom_fields
        let cron_value = fm.custom_fields.get("cron");
        let cron_str = match cron_value {
            Some(serde_yaml::Value::String(s)) => s.clone(),
            _ => continue, // No cron field — skip
        };

        let cron_expr = match parse_cron_expr(&cron_str) {
            Ok(expr) => expr,
            Err(e) => {
                eprintln!("warning: skipping cron file {filename}: {e}");
                continue;
            }
        };

        // Collect custom fields, stripping `cron`
        let mut custom_fields = fm.custom_fields;
        custom_fields.remove("cron");

        let name = filename
            .strip_suffix(".md")
            .unwrap_or(&filename)
            .to_string();

        files.push(CronFile {
            path,
            name,
            cron_expr,
            routine: fm.routine,
            custom_fields,
            body,
        });
    }

    Ok(files)
}

/// Create an inbox message from a fired cron file.
///
/// Returns the path to the newly created inbox message.
pub fn create_inbox_from_cron(
    project_root: &Path,
    cron_file: &CronFile,
) -> Result<PathBuf, DecreeError> {
    let inbox_dir = project_root.join(".decree/inbox");
    fs::create_dir_all(&inbox_dir)?;

    let chain = MessageId::new_chain(0);
    let filename = format!("{chain}-0.md");
    let file_path = inbox_dir.join(&filename);

    let mut fm = String::from("---\n");
    fm.push_str("seq: 0\n");
    fm.push_str("type: task\n");

    if let Some(ref routine) = cron_file.routine {
        fm.push_str(&format!("routine: {routine}\n"));
    }

    for (key, value) in &cron_file.custom_fields {
        let val_str = match value {
            serde_yaml::Value::String(s) => s.clone(),
            other => serde_yaml::to_string(other)
                .unwrap_or_default()
                .trim()
                .to_string(),
        };
        fm.push_str(&format!("{key}: {val_str}\n"));
    }

    fm.push_str("---\n");
    if !cron_file.body.is_empty() {
        fm.push_str(&cron_file.body);
        if !cron_file.body.ends_with('\n') {
            fm.push('\n');
        }
    }

    fs::write(&file_path, &fm)?;
    Ok(file_path)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    #[test]
    fn test_parse_wildcard() {
        let expr = parse_cron_expr("* * * * *").unwrap();
        assert!(expr.minute.wildcard);
        assert!(expr.hour.wildcard);
        assert!(expr.dom.wildcard);
        assert!(expr.month.wildcard);
        assert!(expr.dow.wildcard);
    }

    #[test]
    fn test_parse_specific_values() {
        let expr = parse_cron_expr("0 12 1 6 3").unwrap();
        assert_eq!(expr.minute.values, vec![0]);
        assert_eq!(expr.hour.values, vec![12]);
        assert_eq!(expr.dom.values, vec![1]);
        assert_eq!(expr.month.values, vec![6]);
        assert_eq!(expr.dow.values, vec![3]);
    }

    #[test]
    fn test_parse_step() {
        let expr = parse_cron_expr("*/15 * * * *").unwrap();
        assert_eq!(expr.minute.values, vec![0, 15, 30, 45]);
    }

    #[test]
    fn test_parse_range() {
        let expr = parse_cron_expr("1-5 * * * *").unwrap();
        assert_eq!(expr.minute.values, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn test_parse_list() {
        let expr = parse_cron_expr("0,30 * * * *").unwrap();
        assert_eq!(expr.minute.values, vec![0, 30]);
    }

    #[test]
    fn test_parse_range_with_step() {
        let expr = parse_cron_expr("0-30/10 * * * *").unwrap();
        assert_eq!(expr.minute.values, vec![0, 10, 20, 30]);
    }

    #[test]
    fn test_parse_invalid_field_count() {
        assert!(parse_cron_expr("* *").is_err());
    }

    #[test]
    fn test_parse_invalid_value() {
        assert!(parse_cron_expr("60 * * * *").is_err());
    }

    #[test]
    fn test_parse_zero_step() {
        assert!(parse_cron_expr("*/0 * * * *").is_err());
    }

    #[test]
    fn test_matches_every_minute() {
        let expr = parse_cron_expr("* * * * *").unwrap();
        let time = NaiveDate::from_ymd_opt(2026, 2, 26)
            .unwrap()
            .and_hms_opt(14, 30, 0)
            .unwrap();
        assert!(matches_time(&expr, &time));
    }

    #[test]
    fn test_matches_specific_time() {
        let expr = parse_cron_expr("30 14 * * *").unwrap();
        let time = NaiveDate::from_ymd_opt(2026, 2, 26)
            .unwrap()
            .and_hms_opt(14, 30, 0)
            .unwrap();
        assert!(matches_time(&expr, &time));

        let time2 = NaiveDate::from_ymd_opt(2026, 2, 26)
            .unwrap()
            .and_hms_opt(14, 31, 0)
            .unwrap();
        assert!(!matches_time(&expr, &time2));
    }

    #[test]
    fn test_matches_hourly() {
        let expr = parse_cron_expr("0 * * * *").unwrap();
        let time_match = NaiveDate::from_ymd_opt(2026, 2, 26)
            .unwrap()
            .and_hms_opt(10, 0, 0)
            .unwrap();
        assert!(matches_time(&expr, &time_match));

        let time_no_match = NaiveDate::from_ymd_opt(2026, 2, 26)
            .unwrap()
            .and_hms_opt(10, 1, 0)
            .unwrap();
        assert!(!matches_time(&expr, &time_no_match));
    }

    #[test]
    fn test_matches_day_of_week() {
        // 2026-02-26 is a Thursday (dow=4 in cron)
        let expr = parse_cron_expr("0 0 * * 4").unwrap();
        let time = NaiveDate::from_ymd_opt(2026, 2, 26)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap();
        assert!(matches_time(&expr, &time));

        // Sunday=0 should also match Sunday=7
        let expr_sun = parse_cron_expr("0 0 * * 0").unwrap();
        // 2026-03-01 is a Sunday
        let sunday = NaiveDate::from_ymd_opt(2026, 3, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap();
        assert!(matches_time(&expr_sun, &sunday));
    }

    #[test]
    fn test_scan_cron_files_empty() {
        let dir = tempfile::tempdir().unwrap();
        let files = scan_cron_files(dir.path()).unwrap();
        assert!(files.is_empty());
    }

    #[test]
    fn test_scan_cron_files_with_valid_file() {
        let dir = tempfile::tempdir().unwrap();
        let content = "---\ncron: \"0 * * * *\"\nroutine: develop\n---\nDo the thing.\n";
        fs::write(dir.path().join("hourly-task.md"), content).unwrap();

        let files = scan_cron_files(dir.path()).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].name, "hourly-task");
        assert_eq!(files[0].routine.as_deref(), Some("develop"));
        assert_eq!(files[0].body.trim(), "Do the thing.");
        assert!(!files[0].custom_fields.contains_key("cron"));
    }

    #[test]
    fn test_scan_cron_files_skips_no_cron() {
        let dir = tempfile::tempdir().unwrap();
        let content = "---\nroutine: develop\n---\nNo cron field.\n";
        fs::write(dir.path().join("no-cron.md"), content).unwrap();

        let files = scan_cron_files(dir.path()).unwrap();
        assert!(files.is_empty());
    }

    #[test]
    fn test_scan_cron_files_custom_fields_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let content =
            "---\ncron: \"* * * * *\"\nroutine: deploy\ntarget: production\n---\nDeploy it.\n";
        fs::write(dir.path().join("deploy.md"), content).unwrap();

        let files = scan_cron_files(dir.path()).unwrap();
        assert_eq!(files.len(), 1);
        assert!(files[0].custom_fields.contains_key("target"));
        assert!(!files[0].custom_fields.contains_key("cron"));
    }

    #[test]
    fn test_create_inbox_from_cron() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join(".decree/inbox")).unwrap();

        let cron_file = CronFile {
            path: PathBuf::from("test.md"),
            name: "test".into(),
            cron_expr: parse_cron_expr("* * * * *").unwrap(),
            routine: Some("develop".into()),
            custom_fields: BTreeMap::new(),
            body: "Run the task.\n".into(),
        };

        let path = create_inbox_from_cron(root, &cron_file).unwrap();
        assert!(path.exists());

        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("seq: 0"));
        assert!(content.contains("type: task"));
        assert!(content.contains("routine: develop"));
        assert!(content.contains("Run the task."));
        assert!(!content.contains("cron:"));
    }

    #[test]
    fn test_create_inbox_from_cron_with_custom_fields() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join(".decree/inbox")).unwrap();

        let mut custom = BTreeMap::new();
        custom.insert(
            "target".into(),
            serde_yaml::Value::String("production".into()),
        );

        let cron_file = CronFile {
            path: PathBuf::from("deploy.md"),
            name: "deploy".into(),
            cron_expr: parse_cron_expr("0 0 * * *").unwrap(),
            routine: Some("deploy".into()),
            custom_fields: custom,
            body: "Deploy to prod.\n".into(),
        };

        let path = create_inbox_from_cron(root, &cron_file).unwrap();
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("routine: deploy"));
        assert!(content.contains("target: production"));
        assert!(content.contains("Deploy to prod."));
        assert!(!content.contains("cron:"));
    }

    #[test]
    fn test_create_inbox_from_cron_no_routine() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join(".decree/inbox")).unwrap();

        let cron_file = CronFile {
            path: PathBuf::from("generic.md"),
            name: "generic".into(),
            cron_expr: parse_cron_expr("* * * * *").unwrap(),
            routine: None,
            custom_fields: BTreeMap::new(),
            body: "Generic task.\n".into(),
        };

        let path = create_inbox_from_cron(root, &cron_file).unwrap();
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("seq: 0"));
        assert!(content.contains("type: task"));
        assert!(!content.contains("routine:"));
    }
}
