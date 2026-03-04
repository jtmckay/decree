use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{DecreeError, Result};

/// A migration file discovered in the `migrations/` directory.
#[derive(Debug, Clone)]
pub struct Migration {
    pub filename: String,
    pub path: PathBuf,
    pub frontmatter: Option<String>,
    pub body: String,
}

impl Migration {
    /// Parse a migration file, splitting optional YAML frontmatter from body.
    pub fn load(path: &Path) -> Result<Self> {
        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| DecreeError::Config(format!("invalid migration path: {}", path.display())))?
            .to_string();

        let content = fs::read_to_string(path)?;
        let (frontmatter, body) = split_frontmatter(&content);

        Ok(Self {
            filename,
            path: path.to_path_buf(),
            frontmatter: frontmatter.map(|s| s.to_string()),
            body: body.to_string(),
        })
    }

    /// Extract the `routine` field from frontmatter, if present.
    pub fn routine(&self) -> Option<String> {
        let fm = self.frontmatter.as_ref()?;
        let mapping: serde_yaml::Value = serde_yaml::from_str(fm).ok()?;
        mapping
            .get("routine")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }
}

/// Tracks which migrations have been processed.
pub struct MigrationTracker {
    migrations_dir: PathBuf,
    processed_path: PathBuf,
}

impl MigrationTracker {
    pub fn new(migrations_dir: &Path) -> Self {
        let processed_path = migrations_dir.join("processed.md");
        Self {
            migrations_dir: migrations_dir.to_path_buf(),
            processed_path,
        }
    }

    /// Read the set of already-processed migration filenames.
    pub fn processed_set(&self) -> Result<Vec<String>> {
        if !self.processed_path.exists() {
            return Ok(Vec::new());
        }
        let content = fs::read_to_string(&self.processed_path)?;
        Ok(content
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .map(|l| l.to_string())
            .collect())
    }

    /// List all migration .md files alphabetically, excluding `processed.md`.
    pub fn all_migrations(&self) -> Result<Vec<String>> {
        if !self.migrations_dir.exists() {
            return Ok(Vec::new());
        }

        let mut filenames: Vec<String> = Vec::new();

        for entry in fs::read_dir(&self.migrations_dir)? {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            let name = match entry.file_name().to_str() {
                Some(n) => n.to_string(),
                None => continue,
            };
            if name == "processed.md" {
                continue;
            }
            if name.ends_with(".md") {
                filenames.push(name);
            }
        }

        filenames.sort();
        Ok(filenames)
    }

    /// Return the next unprocessed migration, if any.
    pub fn next_unprocessed(&self) -> Result<Option<Migration>> {
        let processed = self.processed_set()?;
        let all = self.all_migrations()?;

        for filename in &all {
            if !processed.contains(filename) {
                let path = self.migrations_dir.join(filename);
                return Ok(Some(Migration::load(&path)?));
            }
        }

        Ok(None)
    }

    /// Append a filename to `processed.md` after successful processing.
    pub fn mark_processed(&self, filename: &str) -> Result<()> {
        // Ensure parent directory exists
        if let Some(parent) = self.processed_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let mut content = if self.processed_path.exists() {
            fs::read_to_string(&self.processed_path)?
        } else {
            String::new()
        };

        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        content.push_str(filename);
        content.push('\n');

        fs::write(&self.processed_path, content)?;
        Ok(())
    }
}

/// Split content into optional YAML frontmatter and body.
/// Frontmatter is delimited by `---` on its own line at the start.
pub fn split_frontmatter(content: &str) -> (Option<&str>, &str) {
    let trimmed = content.trim_start();

    if !trimmed.starts_with("---") {
        return (None, content);
    }

    // Find the opening delimiter
    let after_open = match trimmed.strip_prefix("---") {
        Some(rest) => rest,
        None => return (None, content),
    };

    // The rest after the opening `---` should start with a newline (or be empty for just `---\n`)
    let after_open = if after_open.starts_with('\n') {
        &after_open[1..]
    } else if after_open.starts_with("\r\n") {
        &after_open[2..]
    } else if after_open.is_empty() {
        after_open
    } else {
        // `---` followed by non-newline text — not frontmatter
        return (None, content);
    };

    // Find the closing `---`
    if let Some(end_pos) = find_closing_delimiter(after_open) {
        let yaml = &after_open[..end_pos];
        let after_close = &after_open[end_pos..];
        // Skip the closing `---` and the newline after it
        let body = after_close
            .strip_prefix("---")
            .unwrap_or(after_close);
        let body = if body.starts_with('\n') {
            &body[1..]
        } else if body.starts_with("\r\n") {
            &body[2..]
        } else {
            body
        };
        (Some(yaml.trim()), body)
    } else {
        (None, content)
    }
}

/// Find the position of a line that is exactly `---`.
fn find_closing_delimiter(s: &str) -> Option<usize> {
    let mut pos = 0;
    for line in s.lines() {
        if line.trim() == "---" {
            return Some(pos);
        }
        // Advance past this line + its newline
        pos += line.len();
        if pos < s.len() {
            // skip \n or \r\n
            if s.as_bytes().get(pos) == Some(&b'\r') {
                pos += 1;
            }
            if s.as_bytes().get(pos) == Some(&b'\n') {
                pos += 1;
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_split_frontmatter_with_yaml() {
        let content = "---\nroutine: develop\n---\nHello world";
        let (fm, body) = split_frontmatter(content);
        assert_eq!(fm, Some("routine: develop"));
        assert_eq!(body, "Hello world");
    }

    #[test]
    fn test_split_frontmatter_bare_message() {
        let content = "Just a bare message with no frontmatter.";
        let (fm, body) = split_frontmatter(content);
        assert!(fm.is_none());
        assert_eq!(body, content);
    }

    #[test]
    fn test_split_frontmatter_empty_body() {
        let content = "---\nroutine: develop\n---\n";
        let (fm, body) = split_frontmatter(content);
        assert_eq!(fm, Some("routine: develop"));
        assert_eq!(body, "");
    }

    #[test]
    fn test_split_frontmatter_no_trailing_newline() {
        let content = "---\nroutine: develop\n---";
        let (fm, body) = split_frontmatter(content);
        assert_eq!(fm, Some("routine: develop"));
        assert_eq!(body, "");
    }

    #[test]
    fn test_split_frontmatter_multiline_yaml() {
        let content = "---\nid: 2025022514320000-0\nchain: 2025022514320000\nseq: 0\ntype: spec\n---\nBody text here.";
        let (fm, body) = split_frontmatter(content);
        assert!(fm.is_some());
        assert!(fm.as_ref().map_or(false, |f| f.contains("chain: 2025022514320000")));
        assert_eq!(body, "Body text here.");
    }

    #[test]
    fn test_migration_routine_extraction() {
        let tmp = std::env::temp_dir().join("decree_test_migration_routine");
        let _ = fs::create_dir_all(&tmp);
        let path = tmp.join("01-test.md");
        fs::write(&path, "---\nroutine: develop\n---\nDo something.").unwrap();

        let migration = Migration::load(&path).unwrap();
        assert_eq!(migration.routine(), Some("develop".to_string()));
        assert_eq!(migration.body, "Do something.");

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_migration_no_frontmatter() {
        let tmp = std::env::temp_dir().join("decree_test_migration_no_fm");
        let _ = fs::create_dir_all(&tmp);
        let path = tmp.join("01-test.md");
        fs::write(&path, "Just a description.").unwrap();

        let migration = Migration::load(&path).unwrap();
        assert!(migration.routine().is_none());
        assert_eq!(migration.body, "Just a description.");

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_tracker_all_migrations_sorted() {
        let tmp = std::env::temp_dir().join("decree_test_tracker_sorted");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        fs::write(tmp.join("03-third.md"), "").unwrap();
        fs::write(tmp.join("01-first.md"), "").unwrap();
        fs::write(tmp.join("02-second.md"), "").unwrap();
        fs::write(tmp.join("processed.md"), "").unwrap();

        let tracker = MigrationTracker::new(&tmp);
        let all = tracker.all_migrations().unwrap();
        assert_eq!(all, vec!["01-first.md", "02-second.md", "03-third.md"]);

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_tracker_next_unprocessed() {
        let tmp = std::env::temp_dir().join("decree_test_tracker_next");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        fs::write(tmp.join("01-first.md"), "First").unwrap();
        fs::write(tmp.join("02-second.md"), "Second").unwrap();
        fs::write(tmp.join("processed.md"), "01-first.md\n").unwrap();

        let tracker = MigrationTracker::new(&tmp);
        let next = tracker.next_unprocessed().unwrap();
        assert!(next.is_some());
        assert_eq!(next.as_ref().map(|m| m.filename.as_str()), Some("02-second.md"));

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_tracker_all_processed() {
        let tmp = std::env::temp_dir().join("decree_test_tracker_all_done");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        fs::write(tmp.join("01-first.md"), "First").unwrap();
        fs::write(tmp.join("processed.md"), "01-first.md\n").unwrap();

        let tracker = MigrationTracker::new(&tmp);
        let next = tracker.next_unprocessed().unwrap();
        assert!(next.is_none());

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_tracker_mark_processed() {
        let tmp = std::env::temp_dir().join("decree_test_tracker_mark");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        fs::write(tmp.join("01-first.md"), "First").unwrap();
        fs::write(tmp.join("02-second.md"), "Second").unwrap();

        let tracker = MigrationTracker::new(&tmp);
        tracker.mark_processed("01-first.md").unwrap();

        let processed = tracker.processed_set().unwrap();
        assert_eq!(processed, vec!["01-first.md"]);

        let next = tracker.next_unprocessed().unwrap();
        assert_eq!(next.as_ref().map(|m| m.filename.as_str()), Some("02-second.md"));

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_tracker_empty_dir() {
        let tmp = std::env::temp_dir().join("decree_test_tracker_empty");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        let tracker = MigrationTracker::new(&tmp);
        let all = tracker.all_migrations().unwrap();
        assert!(all.is_empty());

        let next = tracker.next_unprocessed().unwrap();
        assert!(next.is_none());

        let _ = fs::remove_dir_all(&tmp);
    }
}
