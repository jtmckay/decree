use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use crate::error::DecreeError;

/// Parsed spec file frontmatter.
#[derive(Debug, Clone, Default)]
pub struct SpecFrontmatter {
    pub routine: Option<String>,
}

/// Parse optional YAML frontmatter from a spec file.
/// Returns the routine if specified in frontmatter.
pub fn parse_spec_frontmatter(content: &str) -> SpecFrontmatter {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return SpecFrontmatter::default();
    }

    // Find closing ---
    let after_open = &trimmed[3..];
    let close = match after_open.find("\n---") {
        Some(pos) => pos,
        None => return SpecFrontmatter::default(),
    };

    let yaml_block = &after_open[..close];
    let map: serde_yaml::Value = match serde_yaml::from_str(yaml_block) {
        Ok(v) => v,
        Err(_) => return SpecFrontmatter::default(),
    };

    let routine = map
        .get("routine")
        .and_then(|v| v.as_str())
        .map(String::from);

    SpecFrontmatter { routine }
}

/// Read the set of already-processed spec filenames from `specs/processed-spec.md`.
/// Creates the file if it doesn't exist.
pub fn read_processed(project_root: &Path) -> Result<BTreeSet<String>, DecreeError> {
    let path = project_root.join("specs/processed-spec.md");
    if !path.exists() {
        fs::write(&path, "")?;
        return Ok(BTreeSet::new());
    }
    let content = fs::read_to_string(&path)?;
    let set = content
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect();
    Ok(set)
}

/// List all `*.spec.md` files in `specs/` alphabetically.
pub fn list_specs(project_root: &Path) -> Result<Vec<String>, DecreeError> {
    let specs_dir = project_root.join("specs");
    if !specs_dir.is_dir() {
        return Ok(Vec::new());
    }

    let mut specs: Vec<String> = Vec::new();
    for entry in fs::read_dir(&specs_dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if name.ends_with(".spec.md") {
            specs.push(name);
        }
    }
    specs.sort();
    Ok(specs)
}

/// Return the next unprocessed spec filename, or `None` if all are processed.
pub fn next_unprocessed(project_root: &Path) -> Result<Option<String>, DecreeError> {
    let processed = read_processed(project_root)?;
    let all = list_specs(project_root)?;
    Ok(all.into_iter().find(|s| !processed.contains(s)))
}

/// Append a spec filename to `specs/processed-spec.md`.
pub fn mark_processed(project_root: &Path, spec_name: &str) -> Result<(), DecreeError> {
    let path = project_root.join("specs/processed-spec.md");
    let mut content = if path.exists() {
        fs::read_to_string(&path)?
    } else {
        String::new()
    };
    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    content.push_str(spec_name);
    content.push('\n');
    fs::write(&path, content)?;
    Ok(())
}
