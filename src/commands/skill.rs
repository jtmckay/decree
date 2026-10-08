//! `decree skill`: write the decree skill into the project, overwriting decree's own files
//! (`SKILL.md`, `reference/*.md`), each through a temp file and a rename, and remove a file
//! in `reference/` that decree no longer ships; any other file in the skill folder stays.
//! `init` writes the skill once and never overwrites; this refreshes it after an upgrade
//! (docs/reference/cli.md, Upgrading decree; docs/decisions.md, D54).

use std::path::Path;

use crate::cli::{AiBackend, Format};
use crate::commands::{init, print_json};
use crate::error::DecreeError;
use crate::message::write_replace;

/// The decree skill: path under the skill directory, and content.
pub(crate) const DECREE_SKILL: &[(&str, &str)] = &[
    (
        "SKILL.md",
        include_str!("../templates/skills/decree/SKILL.md"),
    ),
    (
        "reference/machines.md",
        include_str!("../templates/skills/decree/reference/machines.md"),
    ),
    (
        "reference/messages.md",
        include_str!("../templates/skills/decree/reference/messages.md"),
    ),
    (
        "reference/runs.md",
        include_str!("../templates/skills/decree/reference/runs.md"),
    ),
    (
        "reference/scripts.md",
        include_str!("../templates/skills/decree/reference/scripts.md"),
    ),
];

/// The folder under the skill directory whose files are all decree's.
const REFERENCE_DIR: &str = "reference";

/// What `write` did to a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Written,
    Unchanged,
    Removed,
}

impl Outcome {
    /// The word text prints after the path (none for `Written`) and the key in JSON.
    fn key(self) -> &'static str {
        match self {
            Outcome::Written => "written",
            Outcome::Unchanged => "unchanged",
            Outcome::Removed => "removed",
        }
    }
}

pub fn run(project_root: &Path, ai: Option<AiBackend>, format: Format) -> Result<(), DecreeError> {
    let mut report = Vec::new();
    for dir in skill_dirs(project_root, ai)? {
        report.extend(write(project_root, dir)?);
    }
    match format {
        Format::Text => {
            for (path, outcome) in &report {
                match outcome {
                    Outcome::Written => println!("{path}"),
                    _ => println!("{path} {}", outcome.key()),
                }
            }
            Ok(())
        }
        Format::Json => {
            let paths = |want: Outcome| -> Vec<&String> {
                report
                    .iter()
                    .filter(|(_, outcome)| *outcome == want)
                    .map(|(path, _)| path)
                    .collect()
            };
            print_json(&serde_json::json!({
                "written": paths(Outcome::Written),
                "unchanged": paths(Outcome::Unchanged),
                "removed": paths(Outcome::Removed),
            }))
        }
    }
}

/// The skill folders to write: `--ai`'s; without it, every one that exists, else the one of
/// the backend `init` would pick. An error for a backend that reads no skills.
fn skill_dirs(
    project_root: &Path,
    ai: Option<AiBackend>,
) -> Result<Vec<&'static str>, DecreeError> {
    if ai.is_none() {
        let existing: Vec<&'static str> = init::skill_dirs()
            .filter(|dir| project_root.join(dir).is_dir())
            .collect();
        if !existing.is_empty() {
            return Ok(existing);
        }
    }
    match init::skill_dir(ai) {
        (_, Some(dir)) => Ok(vec![dir]),
        (name, None) => Err(DecreeError::Other(format!(
            "decree writes the skill for claude (.claude/skills/decree/) or copilot \
             (.github/skills/decree/); {name} has none"
        ))),
    }
}

/// Write the skill into `skill_dir` (relative to `project_root`): every file in
/// `DECREE_SKILL` that differs from it, then remove each file in `reference/` that is not
/// one. Returns each file's path relative to `project_root`, and what was done to it, in order.
pub fn write(project_root: &Path, skill_dir: &str) -> Result<Vec<(String, Outcome)>, DecreeError> {
    let dir = project_root.join(skill_dir);
    let mut report = Vec::new();
    for (name, content) in DECREE_SKILL {
        let path = dir.join(name);
        let rel = format!("{skill_dir}/{name}");
        match std::fs::read(&path) {
            Ok(on_disk) if on_disk == content.as_bytes() => {
                report.push((rel, Outcome::Unchanged));
                continue;
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(DecreeError::Other(format!(
                    "cannot read {}: {e}",
                    path.display()
                )))
            }
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        write_replace(&path, content.as_bytes()).map_err(|(path, e)| {
            DecreeError::Other(format!("cannot write {}: {e}", path.display()))
        })?;
        report.push((rel, Outcome::Written));
    }
    for name in stale_references(&dir)? {
        let rel = format!("{REFERENCE_DIR}/{name}");
        std::fs::remove_file(dir.join(&rel)).map_err(|e| {
            DecreeError::Other(format!("cannot remove {}/{rel}: {e}", dir.display()))
        })?;
        report.push((format!("{skill_dir}/{rel}"), Outcome::Removed));
    }
    Ok(report)
}

/// The names of the files directly in `<dir>/reference/` that decree does not ship, sorted.
fn stale_references(dir: &Path) -> Result<Vec<String>, DecreeError> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir.join(REFERENCE_DIR))? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let shipped = format!("{REFERENCE_DIR}/{name}");
        if !DECREE_SKILL.iter().any(|(known, _)| *known == shipped) {
            out.push(name);
        }
    }
    out.sort();
    Ok(out)
}
