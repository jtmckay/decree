use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use similar::TextDiff;

use crate::error::DecreeError;

// ---------------------------------------------------------------------------
// Manifest types
// ---------------------------------------------------------------------------

/// Metadata for a single file in the project tree.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileEntry {
    pub sha256: String,
    pub size: u64,
    pub mode: String,
}

/// A snapshot of every tracked file in the project tree at a point in time.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Manifest {
    pub files: BTreeMap<String, FileEntry>,
}

// ---------------------------------------------------------------------------
// Tree walking
// ---------------------------------------------------------------------------

/// Walk the project tree starting at `project_root`, respecting ignore rules.
///
/// Always excludes `.decree/` and `.git/`. Reads `.gitignore` and
/// `.decreeignore` if present (using the `ignore` crate).
pub fn walk_tree(project_root: &Path) -> Result<Vec<PathBuf>, DecreeError> {
    let mut builder = ignore::WalkBuilder::new(project_root);
    builder
        .hidden(false) // don't skip dotfiles by default
        .git_ignore(true) // honour .gitignore if .git/ exists
        .git_global(false)
        .git_exclude(false)
        // Also treat .gitignore as a custom ignore filename so it works
        // even without a .git/ directory (the ignore crate requires .git/
        // for its git_ignore support).
        .add_custom_ignore_filename(".gitignore")
        .add_custom_ignore_filename(".decreeignore");

    let mut paths = Vec::new();

    for entry in builder.build() {
        let entry = entry.map_err(|e| DecreeError::Io(std::io::Error::new(
            std::io::ErrorKind::Other,
            e.to_string(),
        )))?;

        let path = entry.path();

        // Skip directories themselves — we only record files.
        if !path.is_file() {
            continue;
        }

        // Strip the project root to get the relative path.
        let rel = path
            .strip_prefix(project_root)
            .map_err(|e| DecreeError::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                e.to_string(),
            )))?;

        let rel_str = rel.to_string_lossy();

        // Always skip .decree/ and .git/ subtrees.
        if rel_str.starts_with(".decree/")
            || rel_str.starts_with(".decree")
                && rel.components().count() == 1
            || rel_str.starts_with(".git/")
            || rel_str.starts_with(".git")
                && rel.components().count() == 1
        {
            continue;
        }

        paths.push(path.to_path_buf());
    }

    paths.sort();
    Ok(paths)
}

// ---------------------------------------------------------------------------
// Hashing helpers
// ---------------------------------------------------------------------------

/// Compute the SHA-256 hex digest of `data`.
pub fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

/// Get the unix permission mode string for a file (e.g. "644", "755").
#[cfg(unix)]
fn file_mode(path: &Path) -> Result<String, DecreeError> {
    use std::os::unix::fs::PermissionsExt;
    let meta = fs::metadata(path)?;
    Ok(format!("{:o}", meta.permissions().mode() & 0o777))
}

#[cfg(not(unix))]
fn file_mode(_path: &Path) -> Result<String, DecreeError> {
    Ok("644".to_string())
}

/// Returns true if `data` contains a null byte (heuristic for binary).
fn is_binary(data: &[u8]) -> bool {
    data.contains(&0)
}

// ---------------------------------------------------------------------------
// Manifest creation
// ---------------------------------------------------------------------------

/// Create a manifest of the project tree rooted at `project_root`.
pub fn create_manifest(project_root: &Path) -> Result<Manifest, DecreeError> {
    let paths = walk_tree(project_root)?;
    let mut files = BTreeMap::new();

    for path in &paths {
        let rel = path
            .strip_prefix(project_root)
            .map_err(|e| DecreeError::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                e.to_string(),
            )))?;
        let rel_str = rel.to_string_lossy().to_string();
        // Normalize to forward slashes for cross-platform consistency.
        let rel_str = rel_str.replace('\\', "/");

        let contents = fs::read(path)?;
        let hash = sha256_hex(&contents);
        let size = contents.len() as u64;
        let mode = file_mode(path)?;

        files.insert(
            rel_str,
            FileEntry {
                sha256: hash,
                size,
                mode,
            },
        );
    }

    Ok(Manifest { files })
}

/// Save the manifest as JSON to `dest`.
pub fn save_manifest(manifest: &Manifest, dest: &Path) -> Result<(), DecreeError> {
    let json = serde_json::to_string_pretty(manifest)
        .map_err(|e| DecreeError::Io(std::io::Error::new(
            std::io::ErrorKind::Other,
            e.to_string(),
        )))?;
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(dest, json)?;
    Ok(())
}

/// Load a manifest from a JSON file.
pub fn load_manifest(path: &Path) -> Result<Manifest, DecreeError> {
    let data = fs::read_to_string(path)?;
    let manifest: Manifest = serde_json::from_str(&data)
        .map_err(|e| DecreeError::Io(std::io::Error::new(
            std::io::ErrorKind::Other,
            e.to_string(),
        )))?;
    Ok(manifest)
}

// ---------------------------------------------------------------------------
// Checkpoint: save pre-execution manifest
// ---------------------------------------------------------------------------

/// Save a pre-execution checkpoint (manifest.json) for the given message dir.
///
/// Returns the manifest for later use.
pub fn save_checkpoint(
    project_root: &Path,
    msg_dir: &Path,
) -> Result<Manifest, DecreeError> {
    let manifest = create_manifest(project_root)?;
    fs::create_dir_all(msg_dir)?;
    save_manifest(&manifest, &msg_dir.join("manifest.json"))?;
    Ok(manifest)
}

// ---------------------------------------------------------------------------
// Diff generation
// ---------------------------------------------------------------------------

/// Categorised changes between two snapshots.
#[derive(Debug)]
struct Changes {
    /// Files added (in post but not pre).
    new_files: Vec<String>,
    /// Files deleted (in pre but not post).
    deleted_files: Vec<String>,
    /// Files with different hashes.
    modified_files: Vec<String>,
}

/// Compare the pre-execution manifest against the current tree and return the
/// categorised changes.
fn detect_changes(
    pre: &Manifest,
    post: &Manifest,
) -> Changes {
    let mut new_files = Vec::new();
    let mut deleted_files = Vec::new();
    let mut modified_files = Vec::new();

    for key in post.files.keys() {
        match pre.files.get(key) {
            None => new_files.push(key.clone()),
            Some(pre_entry) => {
                let post_entry = &post.files[key];
                if pre_entry.sha256 != post_entry.sha256 {
                    modified_files.push(key.clone());
                }
            }
        }
    }

    for key in pre.files.keys() {
        if !post.files.contains_key(key) {
            deleted_files.push(key.clone());
        }
    }

    Changes {
        new_files,
        deleted_files,
        modified_files,
    }
}

/// Generate a unified diff for a single file change.
fn diff_for_file(
    _project_root: &Path,
    rel_path: &str,
    old_content: Option<&[u8]>,
    new_content: Option<&[u8]>,
) -> String {
    let empty = Vec::new();
    let old = old_content.unwrap_or(&empty);
    let new = new_content.unwrap_or(&empty);

    let old_label = if old_content.is_some() {
        format!("a/{rel_path}")
    } else {
        "/dev/null".to_string()
    };
    let new_label = if new_content.is_some() {
        format!("b/{rel_path}")
    } else {
        "/dev/null".to_string()
    };

    // Binary file handling.
    if is_binary(old) || is_binary(new) {
        let mut out = format!("diff --decree a/{rel_path} b/{rel_path}\n");
        out.push_str(&format!(
            "Binary files {old_label} and {new_label} differ\n"
        ));
        if let Some(data) = new_content {
            let encoded = base64::engine::general_purpose::STANDARD.encode(data);
            out.push_str(&format!("Base64-Content: {encoded}\n"));
        }
        return out;
    }

    // Text diff — use similar's unified diff.
    let old_text = String::from_utf8_lossy(old);
    let new_text = String::from_utf8_lossy(new);

    let diff = TextDiff::from_lines(old_text.as_ref(), new_text.as_ref());

    let mut out = String::new();
    out.push_str(&format!("--- {old_label}\n"));
    out.push_str(&format!("+++ {new_label}\n"));

    // Use the built-in unified diff formatter.
    let mut udiff = diff.unified_diff();
    let formatted = udiff
        .context_radius(3)
        .header(&old_label, &new_label)
        .to_string();

    // The header from similar includes --- and +++ lines already; replace our
    // stub with the full output.
    let _ = out; // discard partial
    out = formatted;

    // If similar produced an empty diff (identical content), return empty.
    // This shouldn't happen since we only diff files with different hashes.
    out
}

/// A cache of file contents captured before execution, keyed by relative path.
pub type ContentCache = BTreeMap<String, Vec<u8>>;

/// Read all file contents from the project tree and return a cache.
/// Call this before routine execution to preserve content for deleted files.
pub fn capture_content_cache(project_root: &Path) -> Result<ContentCache, DecreeError> {
    let paths = walk_tree(project_root)?;
    let mut cache = BTreeMap::new();

    for path in &paths {
        let rel = path
            .strip_prefix(project_root)
            .map_err(|e| DecreeError::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                e.to_string(),
            )))?;
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        let contents = fs::read(path)?;
        cache.insert(rel_str, contents);
    }

    Ok(cache)
}

/// Generate the full `changes.diff` using a pre-execution content cache.
///
/// This is the preferred API: it correctly handles deleted and modified files
/// by having the old content available from the cache.
pub fn generate_diff_with_cache(
    project_root: &Path,
    pre_manifest: &Manifest,
    pre_cache: &ContentCache,
    msg_dir: &Path,
) -> Result<String, DecreeError> {
    let post_manifest = create_manifest(project_root)?;
    let changes = detect_changes(pre_manifest, &post_manifest);

    let mut full_diff = String::new();

    // New files: diff from /dev/null to new content.
    for rel_path in &changes.new_files {
        let abs = project_root.join(rel_path);
        let content = fs::read(&abs)?;
        let piece = diff_for_file(project_root, rel_path, None, Some(&content));
        if !full_diff.is_empty() && !full_diff.ends_with('\n') {
            full_diff.push('\n');
        }
        full_diff.push_str(&piece);
    }

    // Deleted files: diff from old content to /dev/null.
    for rel_path in &changes.deleted_files {
        let old_content = pre_cache.get(rel_path).map(|v| v.as_slice());
        let piece = diff_for_file(project_root, rel_path, old_content, None);
        if !full_diff.is_empty() && !full_diff.ends_with('\n') {
            full_diff.push('\n');
        }
        full_diff.push_str(&piece);
    }

    // Modified files: diff from old to new.
    for rel_path in &changes.modified_files {
        let abs = project_root.join(rel_path);
        let new_content = fs::read(&abs)?;
        let old_content = pre_cache.get(rel_path).map(|v| v.as_slice());
        let piece = diff_for_file(project_root, rel_path, old_content, Some(&new_content));
        if !full_diff.is_empty() && !full_diff.ends_with('\n') {
            full_diff.push('\n');
        }
        full_diff.push_str(&piece);
    }

    // Write to msg_dir/changes.diff
    fs::create_dir_all(msg_dir)?;
    fs::write(msg_dir.join("changes.diff"), &full_diff)?;

    Ok(full_diff)
}

// ---------------------------------------------------------------------------
// Revert
// ---------------------------------------------------------------------------

/// Revert the project tree to the state described by `manifest`, using the
/// content from `changes.diff` and the manifest to guide restoration.
///
/// Strategy:
/// - Modified files: apply reverse patch (swap old/new in the diff)
/// - New files (added by routine): delete them
/// - Deleted files (removed by routine): restore from diff content
///
/// After revert, verifies all affected files match the manifest hashes.
pub fn revert(
    project_root: &Path,
    pre_manifest: &Manifest,
    pre_cache: &ContentCache,
) -> Result<(), DecreeError> {
    let post_manifest = create_manifest(project_root)?;
    let changes = detect_changes(pre_manifest, &post_manifest);

    // Delete new files (files added by the routine).
    for rel_path in &changes.new_files {
        let abs = project_root.join(rel_path);
        if abs.exists() {
            fs::remove_file(&abs)?;
            // Remove empty parent directories up to project_root.
            remove_empty_parents(&abs, project_root);
        }
    }

    // Restore deleted files from the pre-execution cache.
    for rel_path in &changes.deleted_files {
        let abs = project_root.join(rel_path);
        if let Some(parent) = abs.parent() {
            fs::create_dir_all(parent)?;
        }
        match pre_cache.get(rel_path) {
            Some(content) => {
                fs::write(&abs, content)?;
                // Restore mode from manifest.
                if let Some(entry) = pre_manifest.files.get(rel_path) {
                    restore_mode(&abs, &entry.mode)?;
                }
            }
            None => {
                return Err(DecreeError::RevertFailed(format!(
                    "cannot restore deleted file {rel_path}: content not in cache"
                )));
            }
        }
    }

    // Restore modified files from the pre-execution cache.
    for rel_path in &changes.modified_files {
        let abs = project_root.join(rel_path);
        match pre_cache.get(rel_path) {
            Some(content) => {
                fs::write(&abs, content)?;
                if let Some(entry) = pre_manifest.files.get(rel_path) {
                    restore_mode(&abs, &entry.mode)?;
                }
            }
            None => {
                return Err(DecreeError::RevertFailed(format!(
                    "cannot restore modified file {rel_path}: content not in cache"
                )));
            }
        }
    }

    // Verify integrity of all affected files.
    let affected: Vec<&str> = changes
        .new_files
        .iter()
        .chain(changes.deleted_files.iter())
        .chain(changes.modified_files.iter())
        .map(|s| s.as_str())
        .collect();

    verify_integrity(project_root, pre_manifest, &affected)?;

    Ok(())
}

/// Remove empty parent directories between `path` and `stop_at` (exclusive).
fn remove_empty_parents(path: &Path, stop_at: &Path) {
    let mut dir = path.parent();
    while let Some(d) = dir {
        if d == stop_at {
            break;
        }
        // Only remove if empty.
        if fs::read_dir(d).map(|mut rd| rd.next().is_none()).unwrap_or(false) {
            let _ = fs::remove_dir(d);
        } else {
            break;
        }
        dir = d.parent();
    }
}

/// Restore the unix file mode from the manifest.
#[cfg(unix)]
fn restore_mode(path: &Path, mode_str: &str) -> Result<(), DecreeError> {
    use std::os::unix::fs::PermissionsExt;
    let mode = u32::from_str_radix(mode_str, 8).map_err(|e| {
        DecreeError::RevertFailed(format!("invalid mode {mode_str}: {e}"))
    })?;
    let perms = std::fs::Permissions::from_mode(mode);
    fs::set_permissions(path, perms)?;
    Ok(())
}

#[cfg(not(unix))]
fn restore_mode(_path: &Path, _mode_str: &str) -> Result<(), DecreeError> {
    Ok(())
}

// ---------------------------------------------------------------------------
// Integrity verification
// ---------------------------------------------------------------------------

/// Verify that all `affected` files match their entries in the manifest.
///
/// - Files that should exist (in manifest) must have matching SHA-256 hashes.
/// - Files that should not exist (not in manifest, i.e. new files that were
///   deleted during revert) must not be on disk.
///
/// Returns `Ok(())` on success, or `Err(CheckpointIntegrity)` listing all
/// mismatched paths.
pub fn verify_integrity(
    project_root: &Path,
    manifest: &Manifest,
    affected: &[&str],
) -> Result<(), DecreeError> {
    let mut mismatches = Vec::new();

    for &rel_path in affected {
        let abs = project_root.join(rel_path);
        match manifest.files.get(rel_path) {
            Some(entry) => {
                // File should exist and match.
                if !abs.is_file() {
                    mismatches.push(format!("{rel_path}: expected to exist but missing"));
                    continue;
                }
                let content = fs::read(&abs).map_err(|e| {
                    DecreeError::RevertFailed(format!("failed to read {rel_path}: {e}"))
                })?;
                let hash = sha256_hex(&content);
                if hash != entry.sha256 {
                    mismatches.push(format!(
                        "{rel_path}: expected hash {} but got {hash}",
                        entry.sha256
                    ));
                }
            }
            None => {
                // File should NOT exist (was a new file, now deleted).
                if abs.is_file() {
                    mismatches.push(format!(
                        "{rel_path}: should not exist after revert but still present"
                    ));
                }
            }
        }
    }

    if mismatches.is_empty() {
        Ok(())
    } else {
        Err(DecreeError::CheckpointIntegrity(mismatches))
    }
}

// ---------------------------------------------------------------------------
// High-level API for the execution flow
// ---------------------------------------------------------------------------

/// Everything needed to revert a checkpoint: the manifest and content cache.
pub struct Checkpoint {
    pub manifest: Manifest,
    pub content_cache: ContentCache,
}

/// Create a checkpoint before routine execution.
///
/// 1. Walks the tree, builds the manifest, saves `manifest.json`.
/// 2. Captures all file contents for later revert/diff.
pub fn create_checkpoint(
    project_root: &Path,
    msg_dir: &Path,
) -> Result<Checkpoint, DecreeError> {
    let manifest = save_checkpoint(project_root, msg_dir)?;
    let content_cache = capture_content_cache(project_root)?;
    Ok(Checkpoint {
        manifest,
        content_cache,
    })
}

/// Generate the changes.diff after routine execution.
pub fn finalize_diff(
    project_root: &Path,
    checkpoint: &Checkpoint,
    msg_dir: &Path,
) -> Result<String, DecreeError> {
    generate_diff_with_cache(
        project_root,
        &checkpoint.manifest,
        &checkpoint.content_cache,
        msg_dir,
    )
}

/// Revert the project tree to the checkpoint state and verify integrity.
pub fn revert_to_checkpoint(
    project_root: &Path,
    checkpoint: &Checkpoint,
) -> Result<(), DecreeError> {
    revert(project_root, &checkpoint.manifest, &checkpoint.content_cache)
}
