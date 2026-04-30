use crate::cli::{SkillScope, SkillTarget};
use crate::config::expand_tilde;
use crate::error::{color, DecreeError};
use std::path::PathBuf;

const DECREE_SKILL_MD: &str =
    include_str!("../templates/skills/decree/SKILL.md");
const DECREE_REF_HOOKS_MD: &str =
    include_str!("../templates/skills/decree/reference/hooks-and-cron.md");
const DECREE_REF_MIGRATIONS_MD: &str =
    include_str!("../templates/skills/decree/reference/migrations.md");
const DECREE_REF_PIPELINE_MD: &str =
    include_str!("../templates/skills/decree/reference/pipeline-and-vars.md");
const DECREE_REF_ROUTINES_MD: &str =
    include_str!("../templates/skills/decree/reference/routines.md");
const SOW_SKILL_MD: &str =
    include_str!("../templates/skills/sow/SKILL.md");

struct SkillFile {
    path: &'static str,
    content: &'static str,
}

struct SkillEntry {
    name: &'static str,
    description: &'static str,
    files: &'static [SkillFile],
    claude_project_dir: &'static str,
    claude_user_dir: &'static str,
    copilot_project_dir: &'static str,
}

const DECREE_FILES: &[SkillFile] = &[
    SkillFile { path: "SKILL.md", content: DECREE_SKILL_MD },
    SkillFile { path: "reference/hooks-and-cron.md", content: DECREE_REF_HOOKS_MD },
    SkillFile { path: "reference/migrations.md", content: DECREE_REF_MIGRATIONS_MD },
    SkillFile { path: "reference/pipeline-and-vars.md", content: DECREE_REF_PIPELINE_MD },
    SkillFile { path: "reference/routines.md", content: DECREE_REF_ROUTINES_MD },
];

const SOW_FILES: &[SkillFile] = &[
    SkillFile { path: "SKILL.md", content: SOW_SKILL_MD },
];

const SKILLS: &[SkillEntry] = &[
    SkillEntry {
        name: "decree",
        description: "Work within the Decree migration ecosystem safely and idiomatically",
        files: DECREE_FILES,
        claude_project_dir: ".claude/skills/decree",
        claude_user_dir: "~/.claude/skills/decree",
        copilot_project_dir: ".github/skills/decree",
    },
    SkillEntry {
        name: "sow",
        description: "Guide for writing a Statement of Work document",
        files: SOW_FILES,
        claude_project_dir: ".claude/skills/sow",
        claude_user_dir: "~/.claude/skills/sow",
        copilot_project_dir: ".github/skills/sow",
    },
];

fn resolve_scope(scope: Option<SkillScope>) -> Result<SkillScope, DecreeError> {
    match scope {
        Some(s) => Ok(s),
        None => {
            if !color::is_tty() {
                return Err(DecreeError::Other(
                    "scope not specified; use --scope project|user".to_string(),
                ));
            }
            let options = vec!["project".to_string(), "user".to_string()];
            let selection = inquire::Select::new("Install at which scope?", options)
                .prompt()
                .map_err(|e| DecreeError::Other(format!("selection cancelled: {e}")))?;
            match selection.as_str() {
                "project" => Ok(SkillScope::Project),
                _ => Ok(SkillScope::User),
            }
        }
    }
}

fn resolve_target(target: Option<SkillTarget>) -> Result<SkillTarget, DecreeError> {
    match target {
        Some(t) => Ok(t),
        None => {
            if !color::is_tty() {
                return Err(DecreeError::Other(
                    "target not specified; use --target claude|copilot".to_string(),
                ));
            }
            let options = vec!["claude".to_string(), "copilot".to_string()];
            let selection = inquire::Select::new("Which AI assistant?", options)
                .prompt()
                .map_err(|e| DecreeError::Other(format!("selection cancelled: {e}")))?;
            match selection.as_str() {
                "claude" => Ok(SkillTarget::Claude),
                _ => Ok(SkillTarget::Copilot),
            }
        }
    }
}

fn skill_dest_dir(entry: &SkillEntry, scope: &SkillScope, target: &SkillTarget) -> Result<PathBuf, DecreeError> {
    match (target, scope) {
        (SkillTarget::Claude, SkillScope::Project) => Ok(std::env::current_dir()?.join(entry.claude_project_dir)),
        (SkillTarget::Claude, SkillScope::User) => Ok(expand_tilde(entry.claude_user_dir)),
        (SkillTarget::Copilot, SkillScope::Project) => Ok(std::env::current_dir()?.join(entry.copilot_project_dir)),
        (SkillTarget::Copilot, SkillScope::User) => Err(DecreeError::Other(
            "user-scope Copilot installation is not supported; use --scope project".to_string(),
        )),
    }
}

fn scope_label(scope: &SkillScope) -> &'static str {
    match scope {
        SkillScope::Project => "project",
        SkillScope::User => "user",
    }
}

fn install_one(dest: &PathBuf, content: &str, force: bool) -> InstallResult {
    let content = content.trim_end_matches('\n').to_string() + "\n";

    if dest.exists() {
        let existing = match std::fs::read_to_string(dest) {
            Ok(s) => s,
            Err(e) => return InstallResult::Error(e.to_string()),
        };
        if existing == content {
            return InstallResult::UpToDate;
        }
        if !force {
            return InstallResult::Conflict;
        }
    }

    if let Some(parent) = dest.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return InstallResult::Error(e.to_string());
        }
    }

    let was_existing = dest.exists();
    if let Err(e) = std::fs::write(dest, &content) {
        return InstallResult::Error(e.to_string());
    }

    if was_existing && force {
        InstallResult::Overwrote
    } else {
        InstallResult::Installed
    }
}

enum InstallResult {
    Installed,
    Overwrote,
    UpToDate,
    Conflict,
    Error(String),
}

fn install_entry(
    entry: &SkillEntry,
    dest_dir: &PathBuf,
    force: bool,
    installed: &mut usize,
    up_to_date: &mut usize,
    conflicts: &mut usize,
) -> Result<(), DecreeError> {
    let total = entry.files.len();
    for file in entry.files {
        let dest = dest_dir.join(file.path);
        let result = install_one(&dest, file.content, force);
        let is_root = file.path == "SKILL.md";
        match result {
            InstallResult::UpToDate => {
                if is_root {
                    let suffix = if total > 1 { format!(" ({total} files)") } else { String::new() };
                    println!(
                        "Already up to date: {}{}",
                        color::dim(&dest.display().to_string()),
                        suffix
                    );
                }
                *up_to_date += 1;
            }
            InstallResult::Installed => {
                if is_root {
                    let suffix = if total > 1 { format!(" ({total} files)") } else { String::new() };
                    println!("{}: {}{}", color::success("Installed"), dest.display(), suffix);
                }
                *installed += 1;
            }
            InstallResult::Overwrote => {
                if is_root {
                    let suffix = if total > 1 { format!(" ({total} files)") } else { String::new() };
                    println!("{}: {}{}", color::success("Overwrote"), dest.display(), suffix);
                }
                *installed += 1;
            }
            InstallResult::Conflict => {
                eprintln!(
                    "{}: {} already exists with different content. Use --force to overwrite.",
                    color::error("conflict"),
                    dest.display()
                );
                *conflicts += 1;
            }
            InstallResult::Error(e) => return Err(DecreeError::Other(e)),
        }
    }
    Ok(())
}

fn select_skills(
    scope: &SkillScope,
    skill_names: &[String],
    all: bool,
) -> Result<Vec<usize>, DecreeError> {
    if all {
        return Ok((0..SKILLS.len()).collect());
    }

    if !skill_names.is_empty() {
        let mut indices = Vec::new();
        for name in skill_names {
            match SKILLS.iter().position(|e| e.name == name.as_str()) {
                Some(i) => indices.push(i),
                None => {
                    return Err(DecreeError::Other(format!(
                        "unknown skill '{}'; available: {}",
                        name,
                        SKILLS
                            .iter()
                            .map(|e| e.name)
                            .collect::<Vec<_>>()
                            .join(", ")
                    )))
                }
            }
        }
        return Ok(indices);
    }

    // Non-TTY without --skill or --all: error
    if !color::is_tty() {
        return Err(DecreeError::Other(
            "non-TTY: specify skills with --skill <name> (repeatable) or --all".to_string(),
        ));
    }

    // TTY: interactive MultiSelect
    let options: Vec<String> = SKILLS
        .iter()
        .map(|e| format!("{:<18} {}", e.name, e.description))
        .collect();

    let prompt = format!("Which skills would you like to install? (scope: {})", scope_label(scope));
    let result = inquire::MultiSelect::new(&prompt, options)
        .with_default(&[0])
        .prompt();

    match result {
        Ok(selected) => {
            if selected.is_empty() {
                // Empty selection: install only the first (decree)
                Ok(vec![0])
            } else {
                // Map selected display strings back to indices
                let indices = selected
                    .iter()
                    .filter_map(|s| {
                        SKILLS
                            .iter()
                            .position(|e| s.starts_with(e.name))
                    })
                    .collect();
                Ok(indices)
            }
        }
        Err(_) => {
            // Cancelled (Ctrl-C / Esc)
            println!("selection cancelled");
            std::process::exit(0);
        }
    }
}

/// Install the decree skill at project scope for the given AI backend.
/// Called automatically by `decree init`. Silently skips unsupported backends.
pub fn install_for_init(ai_name: &str) -> Result<(), DecreeError> {
    let target = match ai_name {
        "claude" => SkillTarget::Claude,
        "copilot" => SkillTarget::Copilot,
        _ => return Ok(()),
    };
    let entry = &SKILLS[0]; // decree skill
    let dest_dir = skill_dest_dir(entry, &SkillScope::Project, &target)?;
    let mut installed = 0usize;
    let mut up_to_date = 0usize;
    let mut conflicts = 0usize;
    install_entry(entry, &dest_dir, false, &mut installed, &mut up_to_date, &mut conflicts)?;
    if installed > 0 {
        println!("Installed decree skill ({} file(s)).", installed);
    } else if up_to_date > 0 && conflicts == 0 {
        println!("Decree skill already up to date.");
    }
    Ok(())
}

pub fn run(
    scope: Option<SkillScope>,
    target: Option<SkillTarget>,
    force: bool,
    skill_names: Vec<String>,
    all: bool,
) -> Result<(), DecreeError> {
    let scope = resolve_scope(scope)?;
    let target = resolve_target(target)?;

    let indices = select_skills(&scope, &skill_names, all)?;

    let mut installed = 0usize;
    let mut up_to_date = 0usize;
    let mut conflicts = 0usize;

    for idx in &indices {
        let entry = &SKILLS[*idx];
        let dest_dir = skill_dest_dir(entry, &scope, &target)?;
        install_entry(entry, &dest_dir, force, &mut installed, &mut up_to_date, &mut conflicts)?;
    }

    println!(
        "{} skill file(s) installed, {} already up to date, {} conflict(s).",
        installed, up_to_date, conflicts
    );
    println!("Next: reopen your session in this repository to load the new skill.");

    if conflicts > 0 {
        return Err(DecreeError::Other(format!(
            "{conflict}(s) prevented installation; re-run with --force to overwrite",
            conflict = conflicts
        )));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bundled_decree_skill_not_empty() {
        assert!(!DECREE_SKILL_MD.is_empty());
    }

    #[test]
    fn test_bundled_sow_skill_not_empty() {
        assert!(!SOW_SKILL_MD.is_empty());
    }

    #[test]
    fn test_bundled_reference_files_not_empty() {
        assert!(!DECREE_REF_HOOKS_MD.is_empty());
        assert!(!DECREE_REF_MIGRATIONS_MD.is_empty());
        assert!(!DECREE_REF_PIPELINE_MD.is_empty());
        assert!(!DECREE_REF_ROUTINES_MD.is_empty());
    }

    #[test]
    fn test_decree_skill_has_five_files() {
        assert_eq!(DECREE_FILES.len(), 5);
        assert_eq!(DECREE_FILES[0].path, "SKILL.md");
        assert_eq!(DECREE_FILES[1].path, "reference/hooks-and-cron.md");
        assert_eq!(DECREE_FILES[2].path, "reference/migrations.md");
        assert_eq!(DECREE_FILES[3].path, "reference/pipeline-and-vars.md");
        assert_eq!(DECREE_FILES[4].path, "reference/routines.md");
    }

    #[test]
    fn test_sow_skill_has_one_file() {
        assert_eq!(SOW_FILES.len(), 1);
        assert_eq!(SOW_FILES[0].path, "SKILL.md");
    }

    #[test]
    fn test_claude_skill_covers_immutability() {
        assert!(
            DECREE_SKILL_MD.contains("mmutab"),
            "SKILL.md must cover migration immutability"
        );
    }

    #[test]
    fn test_claude_skill_covers_given_when_then() {
        assert!(DECREE_SKILL_MD.contains("Given"));
        assert!(DECREE_SKILL_MD.contains("When"));
        assert!(DECREE_SKILL_MD.contains("Then"));
    }

    #[test]
    fn test_claude_skill_covers_day_sized() {
        assert!(
            DECREE_SKILL_MD.to_lowercase().contains("day-sized")
                || DECREE_SKILL_MD.to_lowercase().contains("day sized"),
            "SKILL.md must mention day-sized migrations"
        );
    }

    fn decree_all_content() -> String {
        DECREE_FILES.iter().map(|f| f.content).collect::<Vec<_>>().join("\n")
    }

    #[test]
    fn test_claude_skill_covers_splitting() {
        assert!(
            decree_all_content().contains("smallest feasible"),
            "decree skill must instruct splitting into smallest feasible chunks"
        );
    }

    #[test]
    fn test_claude_skill_covers_migrations_location() {
        assert!(
            decree_all_content().contains(".decree/migrations"),
            "decree skill must specify the correct migration directory"
        );
    }

    #[test]
    fn test_decree_skill_has_routine_authoring_content() {
        let all = decree_all_content();
        assert!(all.contains("Pre-Check"));
        assert!(all.contains("Custom Parameter Discovery"));
        assert!(all.contains("DECREE_PRE_CHECK"));
        assert!(all.contains("set -euo pipefail"));
        assert!(all.contains("Nested Routines"));
    }

    #[test]
    fn test_sow_skill_has_required_content() {
        assert!(SOW_SKILL_MD.contains("# Statement of Work Template"));
        assert!(SOW_SKILL_MD.contains("Jobs to Be Done"));
        assert!(SOW_SKILL_MD.contains("Acceptance Criteria"));
    }

    #[test]
    fn test_claude_skills_registry_has_two_entries() {
        assert_eq!(SKILLS.len(), 2);
        assert_eq!(SKILLS[0].name, "decree");
        assert_eq!(SKILLS[1].name, "sow");
    }
}
