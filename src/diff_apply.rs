use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::error::DecreeError;
use crate::message;

// ---------------------------------------------------------------------------
// Diff parsing types
// ---------------------------------------------------------------------------

/// A single hunk from a unified diff.
#[derive(Debug, Clone)]
pub struct Hunk {
    /// Starting line in the old file (1-based, 0 for new files).
    pub old_start: usize,
    pub old_count: usize,
    /// Starting line in the new file (1-based, 0 for deleted files).
    pub new_start: usize,
    pub new_count: usize,
    /// The raw lines of the hunk (context, +, -).
    pub lines: Vec<DiffLine>,
}

/// A single line within a hunk.
#[derive(Debug, Clone)]
pub enum DiffLine {
    Context(String),
    Add(String),
    Remove(String),
}

/// The type of change for a file in the diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileChangeKind {
    /// New file (from /dev/null).
    Add,
    /// Deleted file (to /dev/null).
    Delete,
    /// Modified file.
    Modify,
    /// Binary file change.
    Binary,
}

/// A parsed file entry from a unified diff.
#[derive(Debug, Clone)]
pub struct FileDiff {
    /// Relative path of the file.
    pub path: String,
    pub kind: FileChangeKind,
    pub hunks: Vec<Hunk>,
    /// For binary files, optional base64 content.
    pub binary_content: Option<String>,
}

/// Diff statistics for display.
#[derive(Debug, Clone, Default)]
pub struct DiffStats {
    pub additions: usize,
    pub deletions: usize,
    pub files: usize,
}

// ---------------------------------------------------------------------------
// Diff parsing
// ---------------------------------------------------------------------------

/// Parse a unified diff string into a list of file diffs.
pub fn parse_diff(diff_text: &str) -> Result<Vec<FileDiff>, DecreeError> {
    let lines: Vec<&str> = diff_text.lines().collect();
    let mut files = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        // Look for a --- line that starts a file diff, or a "diff --decree" line
        // for binary files.
        if lines[i].starts_with("diff --decree ") {
            // Binary file header
            let (file_diff, next) = parse_binary_block(&lines, i)?;
            files.push(file_diff);
            i = next;
            continue;
        }

        if lines[i].starts_with("--- ") {
            let (file_diff, next) = parse_file_diff(&lines, i)?;
            files.push(file_diff);
            i = next;
            continue;
        }

        i += 1;
    }

    Ok(files)
}

/// Parse a binary file block starting at the "diff --decree" line.
fn parse_binary_block(lines: &[&str], start: usize) -> Result<(FileDiff, usize), DecreeError> {
    // diff --decree a/path b/path
    let header = lines[start];
    let path = header
        .strip_prefix("diff --decree a/")
        .and_then(|rest| rest.split_once(" b/"))
        .map(|(p, _)| p.to_string())
        .ok_or_else(|| DecreeError::DiffParse(format!("invalid binary header: {header}")))?;

    let mut i = start + 1;
    let mut binary_content = None;
    let mut kind = FileChangeKind::Binary;

    while i < lines.len() {
        if lines[i].starts_with("Binary files ") {
            // Determine add/delete/modify from the binary header
            if lines[i].contains("/dev/null and b/") {
                kind = FileChangeKind::Add;
            } else if lines[i].contains("a/") && lines[i].contains("and /dev/null") {
                kind = FileChangeKind::Delete;
            }
            i += 1;
        } else if let Some(content) = lines[i].strip_prefix("Base64-Content: ") {
            binary_content = Some(content.to_string());
            i += 1;
        } else {
            break;
        }
    }

    Ok((
        FileDiff {
            path,
            kind,
            hunks: Vec::new(),
            binary_content,
        },
        i,
    ))
}

/// Parse a text file diff starting at the "--- " line.
fn parse_file_diff(lines: &[&str], start: usize) -> Result<(FileDiff, usize), DecreeError> {
    let old_line = lines[start];
    let old_path = parse_diff_path(old_line.strip_prefix("--- ").unwrap_or(old_line));

    let i = start + 1;
    if i >= lines.len() || !lines[i].starts_with("+++ ") {
        return Err(DecreeError::DiffParse(format!(
            "expected +++ line after --- at line {start}"
        )));
    }
    let new_line = lines[i];
    let new_path = parse_diff_path(new_line.strip_prefix("+++ ").unwrap_or(new_line));

    let kind = if old_path == "/dev/null" {
        FileChangeKind::Add
    } else if new_path == "/dev/null" {
        FileChangeKind::Delete
    } else {
        FileChangeKind::Modify
    };

    let path = if kind == FileChangeKind::Add {
        new_path.clone()
    } else {
        old_path.clone()
    };

    let mut hunks = Vec::new();
    let mut i = i + 1;

    while i < lines.len() {
        if lines[i].starts_with("@@ ") {
            let (hunk, next) = parse_hunk(lines, i)?;
            hunks.push(hunk);
            i = next;
        } else if lines[i].starts_with("--- ")
            || lines[i].starts_with("diff --decree ")
        {
            // Next file starts
            break;
        } else {
            i += 1;
        }
    }

    Ok((
        FileDiff {
            path,
            kind,
            hunks,
            binary_content: None,
        },
        i,
    ))
}

/// Strip the a/ or b/ prefix from a diff path, or return as-is for /dev/null.
fn parse_diff_path(s: &str) -> String {
    if s == "/dev/null" {
        return s.to_string();
    }
    s.strip_prefix("a/")
        .or_else(|| s.strip_prefix("b/"))
        .unwrap_or(s)
        .to_string()
}

/// Parse a single hunk starting at the @@ line.
fn parse_hunk(lines: &[&str], start: usize) -> Result<(Hunk, usize), DecreeError> {
    let header = lines[start];
    let (old_start, old_count, new_start, new_count) = parse_hunk_header(header)?;

    let mut hunk_lines = Vec::new();
    let mut i = start + 1;

    while i < lines.len() {
        let line = lines[i];
        if line.starts_with("@@ ")
            || line.starts_with("--- ")
            || line.starts_with("diff --decree ")
        {
            break;
        }

        if let Some(content) = line.strip_prefix('+') {
            hunk_lines.push(DiffLine::Add(content.to_string()));
        } else if let Some(content) = line.strip_prefix('-') {
            hunk_lines.push(DiffLine::Remove(content.to_string()));
        } else if let Some(content) = line.strip_prefix(' ') {
            hunk_lines.push(DiffLine::Context(content.to_string()));
        } else if line == "\\ No newline at end of file" {
            // Skip this marker
        } else {
            // Treat as context (empty context line)
            hunk_lines.push(DiffLine::Context(line.to_string()));
        }

        i += 1;
    }

    Ok((
        Hunk {
            old_start,
            old_count,
            new_start,
            new_count,
            lines: hunk_lines,
        },
        i,
    ))
}

/// Parse "@@ -old_start,old_count +new_start,new_count @@" header.
fn parse_hunk_header(header: &str) -> Result<(usize, usize, usize, usize), DecreeError> {
    // Format: @@ -A,B +C,D @@  (or @@ -A +C @@ for single-line)
    let stripped = header
        .strip_prefix("@@ ")
        .and_then(|s| s.split_once(" @@"))
        .map(|(s, _)| s)
        .ok_or_else(|| DecreeError::DiffParse(format!("invalid hunk header: {header}")))?;

    let parts: Vec<&str> = stripped.split_whitespace().collect();
    if parts.len() < 2 {
        return Err(DecreeError::DiffParse(format!(
            "invalid hunk header: {header}"
        )));
    }

    let (old_start, old_count) = parse_range(parts[0].strip_prefix('-').unwrap_or(parts[0]))?;
    let (new_start, new_count) = parse_range(parts[1].strip_prefix('+').unwrap_or(parts[1]))?;

    Ok((old_start, old_count, new_start, new_count))
}

fn parse_range(s: &str) -> Result<(usize, usize), DecreeError> {
    if let Some((start, count)) = s.split_once(',') {
        Ok((
            start
                .parse()
                .map_err(|_| DecreeError::DiffParse(format!("invalid range: {s}")))?,
            count
                .parse()
                .map_err(|_| DecreeError::DiffParse(format!("invalid range: {s}")))?,
        ))
    } else {
        let start: usize = s
            .parse()
            .map_err(|_| DecreeError::DiffParse(format!("invalid range: {s}")))?;
        Ok((start, 1))
    }
}

// ---------------------------------------------------------------------------
// Diff statistics
// ---------------------------------------------------------------------------

/// Compute diff stats from raw diff text (fast, no full parse needed).
pub fn compute_stats(diff_text: &str) -> DiffStats {
    let mut stats = DiffStats::default();
    let mut files = std::collections::HashSet::new();

    for line in diff_text.lines() {
        if line.starts_with("+++ ") {
            let path = line.strip_prefix("+++ ").unwrap_or(line);
            if path != "/dev/null" {
                files.insert(parse_diff_path(path));
            }
        } else if line.starts_with("--- ") {
            let path = line.strip_prefix("--- ").unwrap_or(line);
            if path != "/dev/null" {
                files.insert(parse_diff_path(path));
            }
        } else if line.starts_with('+') && !line.starts_with("+++ ") {
            stats.additions += 1;
        } else if line.starts_with('-') && !line.starts_with("--- ") {
            stats.deletions += 1;
        }
    }

    stats.files = files.len();
    stats
}

// ---------------------------------------------------------------------------
// Conflict checking
// ---------------------------------------------------------------------------

/// A single conflict found during dry-run.
#[derive(Debug)]
pub struct Conflict {
    pub file: String,
    pub detail: String,
}

/// Check whether a list of file diffs can be applied cleanly to the project.
pub fn check_conflicts(
    project_root: &Path,
    file_diffs: &[FileDiff],
) -> Vec<Conflict> {
    let mut conflicts = Vec::new();

    for fd in file_diffs {
        let abs = project_root.join(&fd.path);

        match fd.kind {
            FileChangeKind::Add => {
                if abs.exists() {
                    conflicts.push(Conflict {
                        file: fd.path.clone(),
                        detail: "file already exists (would be overwritten)".to_string(),
                    });
                }
            }
            FileChangeKind::Delete => {
                if !abs.exists() {
                    conflicts.push(Conflict {
                        file: fd.path.clone(),
                        detail: "file does not exist (expected for deletion)".to_string(),
                    });
                } else {
                    // Verify content matches pre-image
                    check_hunk_preimage(project_root, fd, &mut conflicts);
                }
            }
            FileChangeKind::Modify => {
                if !abs.exists() {
                    conflicts.push(Conflict {
                        file: fd.path.clone(),
                        detail: "file does not exist (expected for modification)".to_string(),
                    });
                } else {
                    check_hunk_preimage(project_root, fd, &mut conflicts);
                }
            }
            FileChangeKind::Binary => {
                // Binary files: just check existence for modifications
                if !abs.exists() && fd.binary_content.is_none() {
                    conflicts.push(Conflict {
                        file: fd.path.clone(),
                        detail: "binary file does not exist".to_string(),
                    });
                }
            }
        }
    }

    conflicts
}

/// Check that a file's current content matches the pre-image lines in the hunks.
fn check_hunk_preimage(
    project_root: &Path,
    fd: &FileDiff,
    conflicts: &mut Vec<Conflict>,
) {
    let abs = project_root.join(&fd.path);
    let content = match fs::read_to_string(&abs) {
        Ok(c) => c,
        Err(_) => {
            conflicts.push(Conflict {
                file: fd.path.clone(),
                detail: "cannot read file".to_string(),
            });
            return;
        }
    };

    let file_lines: Vec<&str> = content.lines().collect();

    for hunk in &fd.hunks {
        // old_start is 1-based; for new files old_start=0 so skip
        if hunk.old_start == 0 {
            continue;
        }

        let start_idx = hunk.old_start - 1; // Convert to 0-based
        let mut file_idx = start_idx;

        for diff_line in &hunk.lines {
            match diff_line {
                DiffLine::Context(expected) | DiffLine::Remove(expected) => {
                    if file_idx >= file_lines.len() {
                        conflicts.push(Conflict {
                            file: fd.path.clone(),
                            detail: format!(
                                "hunk at line {}: file is shorter than expected",
                                hunk.old_start
                            ),
                        });
                        return;
                    }
                    if file_lines[file_idx] != expected.as_str() {
                        conflicts.push(Conflict {
                            file: fd.path.clone(),
                            detail: format!(
                                "hunk at line {}: expected {:?} but found {:?}",
                                file_idx + 1,
                                truncate(expected, 40),
                                truncate(file_lines[file_idx], 40),
                            ),
                        });
                        return;
                    }
                    file_idx += 1;
                }
                DiffLine::Add(_) => {
                    // Added lines don't consume old file lines
                }
            }
        }
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max])
    }
}

// ---------------------------------------------------------------------------
// Apply logic
// ---------------------------------------------------------------------------

/// Apply parsed file diffs to the project tree.
///
/// This assumes conflict checking has already passed (or --force is used).
pub fn apply_diffs(
    project_root: &Path,
    file_diffs: &[FileDiff],
) -> Result<(), DecreeError> {
    for fd in file_diffs {
        match fd.kind {
            FileChangeKind::Add => {
                apply_text_patch(project_root, fd)?;
            }
            FileChangeKind::Delete => {
                let abs = project_root.join(&fd.path);
                if abs.exists() {
                    fs::remove_file(&abs)?;
                }
            }
            FileChangeKind::Modify => {
                apply_text_patch(project_root, fd)?;
            }
            FileChangeKind::Binary => {
                apply_binary(project_root, fd)?;
            }
        }
    }
    Ok(())
}

/// Apply a text diff to a single file.
fn apply_text_patch(project_root: &Path, fd: &FileDiff) -> Result<(), DecreeError> {
    let abs = project_root.join(&fd.path);

    if fd.kind == FileChangeKind::Add && fd.hunks.is_empty() {
        // Empty new file
        if let Some(parent) = abs.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&abs, "")?;
        return Ok(());
    }

    // Build the new file content by applying hunks
    let old_content = if abs.exists() {
        fs::read_to_string(&abs)?
    } else {
        String::new()
    };

    let old_lines: Vec<&str> = if old_content.is_empty() {
        Vec::new()
    } else {
        old_content.lines().collect()
    };

    let new_lines = apply_hunks_to_lines(&old_lines, &fd.hunks)?;

    if let Some(parent) = abs.parent() {
        fs::create_dir_all(parent)?;
    }

    // Reconstruct the file: join with newlines, preserve trailing newline
    // if the last hunk adds content.
    let mut result = new_lines.join("\n");
    // If the original had a trailing newline or we're creating a new file with content,
    // add trailing newline.
    if !result.is_empty()
        && (old_content.ends_with('\n') || fd.kind == FileChangeKind::Add)
    {
        result.push('\n');
    }

    fs::write(&abs, result)?;
    Ok(())
}

/// Apply hunks to old file lines to produce new file lines.
fn apply_hunks_to_lines(
    old_lines: &[&str],
    hunks: &[Hunk],
) -> Result<Vec<String>, DecreeError> {
    let mut result: Vec<String> = Vec::new();
    let mut old_idx: usize = 0; // Current position in old_lines (0-based)

    for hunk in hunks {
        let hunk_start = if hunk.old_start == 0 {
            0
        } else {
            hunk.old_start - 1
        };

        // Copy lines before this hunk
        while old_idx < hunk_start && old_idx < old_lines.len() {
            result.push(old_lines[old_idx].to_string());
            old_idx += 1;
        }

        // Apply the hunk
        for diff_line in &hunk.lines {
            match diff_line {
                DiffLine::Context(text) => {
                    result.push(text.clone());
                    old_idx += 1;
                }
                DiffLine::Remove(_) => {
                    old_idx += 1;
                }
                DiffLine::Add(text) => {
                    result.push(text.clone());
                }
            }
        }
    }

    // Copy remaining lines after the last hunk
    while old_idx < old_lines.len() {
        result.push(old_lines[old_idx].to_string());
        old_idx += 1;
    }

    Ok(result)
}

/// Apply a binary file diff.
fn apply_binary(project_root: &Path, fd: &FileDiff) -> Result<(), DecreeError> {
    let abs = project_root.join(&fd.path);

    if fd.kind == FileChangeKind::Delete {
        if abs.exists() {
            fs::remove_file(&abs)?;
        }
        return Ok(());
    }

    // Write base64-decoded content
    if let Some(ref b64) = fd.binary_content {
        let data = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            b64,
        )
        .map_err(|e| DecreeError::DiffParse(format!("invalid base64: {e}")))?;

        if let Some(parent) = abs.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&abs, data)?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Message discovery and listing
// ---------------------------------------------------------------------------

/// Info about a single message for listing purposes.
#[derive(Debug)]
pub struct MessageInfo {
    pub id: String,
    pub chain: String,
    pub seq: String,
    pub stats: DiffStats,
    pub description: String,
}

/// Gather info about all messages grouped by chain.
pub fn list_messages(
    runs_dir: &Path,
) -> Result<BTreeMap<String, Vec<MessageInfo>>, DecreeError> {
    let mut chains: BTreeMap<String, Vec<MessageInfo>> = BTreeMap::new();

    if !runs_dir.is_dir() {
        return Ok(chains);
    }

    let mut dirs: Vec<String> = Vec::new();
    for entry in fs::read_dir(runs_dir)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            dirs.push(entry.file_name().to_string_lossy().to_string());
        }
    }
    dirs.sort();

    for dir_name in &dirs {
        let mid = match message::MessageId::parse(dir_name) {
            Some(id) => id,
            None => continue,
        };

        let msg_dir = runs_dir.join(dir_name);
        let diff_path = msg_dir.join("changes.diff");

        let stats = if diff_path.exists() {
            let content = fs::read_to_string(&diff_path).unwrap_or_default();
            compute_stats(&content)
        } else {
            DiffStats::default()
        };

        // Extract description from message.md frontmatter
        let description = extract_description(&msg_dir);

        let info = MessageInfo {
            id: dir_name.clone(),
            chain: mid.chain.clone(),
            seq: format!("-{}", mid.seq),
            stats,
            description,
        };

        chains
            .entry(mid.chain.clone())
            .or_default()
            .push(info);
    }

    Ok(chains)
}

/// Extract a short description from a message's frontmatter.
fn extract_description(msg_dir: &Path) -> String {
    let msg_path = msg_dir.join("message.md");
    if !msg_path.exists() {
        return String::new();
    }

    let content = match fs::read_to_string(&msg_path) {
        Ok(c) => c,
        Err(_) => return String::new(),
    };

    let (fm, body) = message::parse_message_file(&content);

    // Use input_file for spec type, or first line of body for tasks
    if let Some(ref input_file) = fm.input_file {
        return input_file.clone();
    }

    // Check for a "task" custom field
    if let Some(task) = fm.custom_fields.get("task") {
        if let Some(s) = task.as_str() {
            return format!("task: {s}");
        }
    }

    // Fall back to first line of body, truncated
    body.lines()
        .find(|l| !l.trim().is_empty())
        .map(|l| {
            if l.len() > 50 {
                format!("{}...", &l[..47])
            } else {
                l.to_string()
            }
        })
        .unwrap_or_default()
}

/// Resolve a target specification into an ordered list of message directory names.
///
/// Handles:
/// - Single message ID (chain-seq)
/// - Chain ID (all messages in chain)
/// - Prefix matching
pub fn resolve_targets(
    runs_dir: &Path,
    id: &str,
) -> Result<Vec<String>, DecreeError> {
    let mut matches = message::resolve_id(runs_dir, id)?;
    matches.sort();
    Ok(matches)
}

/// Get all message directories in chronological order.
pub fn all_messages(runs_dir: &Path) -> Result<Vec<String>, DecreeError> {
    if !runs_dir.is_dir() {
        return Err(DecreeError::MessageNotFound("(no runs)".to_string()));
    }

    let mut dirs: Vec<String> = Vec::new();
    for entry in fs::read_dir(runs_dir)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            dirs.push(entry.file_name().to_string_lossy().to_string());
        }
    }
    dirs.sort();

    if dirs.is_empty() {
        return Err(DecreeError::MessageNotFound("(no runs)".to_string()));
    }

    Ok(dirs)
}

/// Get messages from `since_id` through the most recent.
pub fn messages_since(
    runs_dir: &Path,
    since_id: &str,
) -> Result<Vec<String>, DecreeError> {
    let all = all_messages(runs_dir)?;

    // Resolve the since_id to a specific message
    let since_matches = message::resolve_id(runs_dir, since_id)?;
    let since = if since_matches.len() == 1 {
        since_matches[0].clone()
    } else {
        // If prefix matches multiple, take the first (earliest)
        let first_chain = since_matches[0].rsplit_once('-').map(|(c, _)| c);
        let all_same_chain = since_matches
            .iter()
            .all(|m| m.rsplit_once('-').map(|(c, _)| c) == first_chain);
        if all_same_chain {
            since_matches[0].clone()
        } else {
            return Err(DecreeError::AmbiguousId {
                prefix: since_id.to_string(),
                candidates: since_matches,
            });
        }
    };

    let result: Vec<String> = all.into_iter().filter(|m| m.as_str() >= since.as_str()).collect();

    if result.is_empty() {
        return Err(DecreeError::MessageNotFound(since_id.to_string()));
    }

    Ok(result)
}

/// Get messages from the oldest through `through_id` (inclusive).
pub fn messages_through(
    runs_dir: &Path,
    through_id: &str,
) -> Result<Vec<String>, DecreeError> {
    let all = all_messages(runs_dir)?;

    // Resolve the through_id to a specific message
    let through_matches = message::resolve_id(runs_dir, through_id)?;
    let through = if through_matches.len() == 1 {
        through_matches[0].clone()
    } else {
        let first_chain = through_matches[0].rsplit_once('-').map(|(c, _)| c);
        let all_same_chain = through_matches
            .iter()
            .all(|m| m.rsplit_once('-').map(|(c, _)| c) == first_chain);
        if all_same_chain {
            through_matches.last().cloned().unwrap_or_default()
        } else {
            return Err(DecreeError::AmbiguousId {
                prefix: through_id.to_string(),
                candidates: through_matches,
            });
        }
    };

    let result: Vec<String> = all
        .into_iter()
        .filter(|m| m.as_str() <= through.as_str())
        .collect();

    if result.is_empty() {
        return Err(DecreeError::MessageNotFound(through_id.to_string()));
    }

    Ok(result)
}

/// Read the changes.diff for a given message directory, returning the content
/// or None if no diff exists.
pub fn read_diff(runs_dir: &Path, msg_id: &str) -> Result<Option<String>, DecreeError> {
    let diff_path = runs_dir.join(msg_id).join("changes.diff");
    if !diff_path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(&diff_path)?;
    if content.trim().is_empty() {
        return Ok(None);
    }
    Ok(Some(content))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_hunk_header_basic() {
        let (os, oc, ns, nc) = parse_hunk_header("@@ -1,5 +1,7 @@").unwrap();
        assert_eq!((os, oc, ns, nc), (1, 5, 1, 7));
    }

    #[test]
    fn test_parse_hunk_header_new_file() {
        let (os, oc, ns, nc) = parse_hunk_header("@@ -0,0 +1,3 @@").unwrap();
        assert_eq!((os, oc, ns, nc), (0, 0, 1, 3));
    }

    #[test]
    fn test_parse_hunk_header_single_line() {
        let (os, oc, ns, nc) = parse_hunk_header("@@ -1 +1 @@").unwrap();
        assert_eq!((os, oc, ns, nc), (1, 1, 1, 1));
    }

    #[test]
    fn test_parse_diff_new_file() {
        let diff = "\
--- /dev/null
+++ b/hello.txt
@@ -0,0 +1,2 @@
+hello
+world
";
        let files = parse_diff(diff).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "hello.txt");
        assert_eq!(files[0].kind, FileChangeKind::Add);
        assert_eq!(files[0].hunks.len(), 1);
        assert_eq!(files[0].hunks[0].lines.len(), 2);
    }

    #[test]
    fn test_parse_diff_delete_file() {
        let diff = "\
--- a/old.txt
+++ /dev/null
@@ -1,2 +0,0 @@
-goodbye
-world
";
        let files = parse_diff(diff).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "old.txt");
        assert_eq!(files[0].kind, FileChangeKind::Delete);
    }

    #[test]
    fn test_parse_diff_modify() {
        let diff = "\
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,3 +1,3 @@
 fn main() {
-    println!(\"hello\");
+    println!(\"world\");
 }
";
        let files = parse_diff(diff).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "src/main.rs");
        assert_eq!(files[0].kind, FileChangeKind::Modify);
        assert_eq!(files[0].hunks.len(), 1);
    }

    #[test]
    fn test_parse_diff_multiple_files() {
        let diff = "\
--- /dev/null
+++ b/new.txt
@@ -0,0 +1 @@
+new file
--- a/existing.txt
+++ b/existing.txt
@@ -1,2 +1,3 @@
 line 1
+inserted
 line 2
";
        let files = parse_diff(diff).unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, "new.txt");
        assert_eq!(files[1].path, "existing.txt");
    }

    #[test]
    fn test_compute_stats() {
        let diff = "\
--- /dev/null
+++ b/new.txt
@@ -0,0 +1,3 @@
+line 1
+line 2
+line 3
--- a/mod.txt
+++ b/mod.txt
@@ -1,2 +1,2 @@
-old
+new
 same
";
        let stats = compute_stats(diff);
        assert_eq!(stats.additions, 4);
        assert_eq!(stats.deletions, 1);
        assert_eq!(stats.files, 2);
    }

    #[test]
    fn test_apply_hunks_new_file() {
        let hunks = vec![Hunk {
            old_start: 0,
            old_count: 0,
            new_start: 1,
            new_count: 2,
            lines: vec![
                DiffLine::Add("hello".to_string()),
                DiffLine::Add("world".to_string()),
            ],
        }];
        let result = apply_hunks_to_lines(&[], &hunks).unwrap();
        assert_eq!(result, vec!["hello", "world"]);
    }

    #[test]
    fn test_apply_hunks_modify() {
        let old = vec!["fn main() {", "    println!(\"hello\");", "}"];
        let hunks = vec![Hunk {
            old_start: 1,
            old_count: 3,
            new_start: 1,
            new_count: 3,
            lines: vec![
                DiffLine::Context("fn main() {".to_string()),
                DiffLine::Remove("    println!(\"hello\");".to_string()),
                DiffLine::Add("    println!(\"world\");".to_string()),
                DiffLine::Context("}".to_string()),
            ],
        }];
        let result = apply_hunks_to_lines(&old, &hunks).unwrap();
        assert_eq!(
            result,
            vec!["fn main() {", "    println!(\"world\");", "}"]
        );
    }

    #[test]
    fn test_conflict_new_file_exists() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        fs::write(root.join("existing.txt"), "content").unwrap();

        let diffs = vec![FileDiff {
            path: "existing.txt".to_string(),
            kind: FileChangeKind::Add,
            hunks: vec![],
            binary_content: None,
        }];

        let conflicts = check_conflicts(root, &diffs);
        assert_eq!(conflicts.len(), 1);
        assert!(conflicts[0].detail.contains("already exists"));
    }

    #[test]
    fn test_conflict_modify_missing() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();

        let diffs = vec![FileDiff {
            path: "missing.txt".to_string(),
            kind: FileChangeKind::Modify,
            hunks: vec![],
            binary_content: None,
        }];

        let conflicts = check_conflicts(root, &diffs);
        assert_eq!(conflicts.len(), 1);
        assert!(conflicts[0].detail.contains("does not exist"));
    }

    #[test]
    fn test_apply_new_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();

        let diffs = vec![FileDiff {
            path: "new.txt".to_string(),
            kind: FileChangeKind::Add,
            hunks: vec![Hunk {
                old_start: 0,
                old_count: 0,
                new_start: 1,
                new_count: 2,
                lines: vec![
                    DiffLine::Add("hello".to_string()),
                    DiffLine::Add("world".to_string()),
                ],
            }],
            binary_content: None,
        }];

        apply_diffs(root, &diffs).unwrap();
        let content = fs::read_to_string(root.join("new.txt")).unwrap();
        assert_eq!(content, "hello\nworld\n");
    }

    #[test]
    fn test_apply_delete_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        fs::write(root.join("delete_me.txt"), "content").unwrap();

        let diffs = vec![FileDiff {
            path: "delete_me.txt".to_string(),
            kind: FileChangeKind::Delete,
            hunks: vec![],
            binary_content: None,
        }];

        apply_diffs(root, &diffs).unwrap();
        assert!(!root.join("delete_me.txt").exists());
    }

    #[test]
    fn test_apply_modify_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        fs::write(root.join("mod.txt"), "line1\nline2\nline3\n").unwrap();

        let diffs = vec![FileDiff {
            path: "mod.txt".to_string(),
            kind: FileChangeKind::Modify,
            hunks: vec![Hunk {
                old_start: 1,
                old_count: 3,
                new_start: 1,
                new_count: 3,
                lines: vec![
                    DiffLine::Context("line1".to_string()),
                    DiffLine::Remove("line2".to_string()),
                    DiffLine::Add("replaced".to_string()),
                    DiffLine::Context("line3".to_string()),
                ],
            }],
            binary_content: None,
        }];

        apply_diffs(root, &diffs).unwrap();
        let content = fs::read_to_string(root.join("mod.txt")).unwrap();
        assert_eq!(content, "line1\nreplaced\nline3\n");
    }

    #[test]
    fn test_conflict_preimage_mismatch() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        fs::write(root.join("file.txt"), "different\ncontent\n").unwrap();

        let diffs = vec![FileDiff {
            path: "file.txt".to_string(),
            kind: FileChangeKind::Modify,
            hunks: vec![Hunk {
                old_start: 1,
                old_count: 2,
                new_start: 1,
                new_count: 2,
                lines: vec![
                    DiffLine::Remove("expected".to_string()),
                    DiffLine::Add("new".to_string()),
                    DiffLine::Context("content".to_string()),
                ],
            }],
            binary_content: None,
        }];

        let conflicts = check_conflicts(root, &diffs);
        assert_eq!(conflicts.len(), 1);
        assert!(conflicts[0].detail.contains("expected"));
    }

    #[test]
    fn test_parse_binary_diff() {
        let diff = "\
diff --decree a/image.png b/image.png
Binary files /dev/null and b/image.png differ
Base64-Content: aGVsbG8=
";
        let files = parse_diff(diff).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "image.png");
        assert_eq!(files[0].binary_content, Some("aGVsbG8=".to_string()));
    }
}
