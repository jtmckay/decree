use crate::cli::{SkillScope, SkillTarget};
use crate::config::expand_tilde;
use crate::error::{color, DecreeError};
use std::path::PathBuf;

const CLAUDE_DECREE_SKILL_MD: &str =
    include_str!("../templates/skills/decree/SKILL.md");
const CLAUDE_SOW_SKILL_MD: &str =
    include_str!("../templates/skills/sow/SKILL.md");

struct SkillEntry {
    name: &'static str,
    description: &'static str,
    content: &'static str,
    claude_project_path: &'static str,
    claude_user_path: &'static str,
    copilot_project_path: &'static str,
}

const SKILLS: &[SkillEntry] = &[
    SkillEntry {
        name: "decree",
        description: "Work within the Decree migration ecosystem safely and idiomatically",
        content: CLAUDE_DECREE_SKILL_MD,
        claude_project_path: ".claude/skills/decree/SKILL.md",
        claude_user_path: "~/.claude/skills/decree/SKILL.md",
        copilot_project_path: ".github/skills/decree/SKILL.md",
    },
    SkillEntry {
        name: "sow",
        description: "Guide for writing a Statement of Work document",
        content: CLAUDE_SOW_SKILL_MD,
        claude_project_path: ".claude/skills/sow/SKILL.md",
        claude_user_path: "~/.claude/skills/sow/SKILL.md",
        copilot_project_path: ".github/skills/sow/SKILL.md",
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

fn skill_dest(entry: &SkillEntry, scope: &SkillScope, target: &SkillTarget) -> Result<PathBuf, DecreeError> {
    match (target, scope) {
        (SkillTarget::Claude, SkillScope::Project) => Ok(std::env::current_dir()?.join(entry.claude_project_path)),
        (SkillTarget::Claude, SkillScope::User) => Ok(expand_tilde(entry.claude_user_path)),
        (SkillTarget::Copilot, SkillScope::Project) => Ok(std::env::current_dir()?.join(entry.copilot_project_path)),
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
        let dest = skill_dest(entry, &scope, &target)?;
        let result = install_one(&dest, entry.content, force);
        match result {
            InstallResult::UpToDate => {
                println!(
                    "Already up to date: {}",
                    color::dim(&dest.display().to_string())
                );
                up_to_date += 1;
            }
            InstallResult::Installed => {
                println!("{}: {}", color::success("Installed"), dest.display());
                installed += 1;
            }
            InstallResult::Overwrote => {
                println!("{}: {}", color::success("Overwrote"), dest.display());
                installed += 1;
            }
            InstallResult::Conflict => {
                eprintln!(
                    "{}: {} already exists with different content. Use --force to overwrite.",
                    color::error("conflict"),
                    dest.display()
                );
                conflicts += 1;
            }
            InstallResult::Error(e) => return Err(DecreeError::Other(e)),
        }
    }

    println!(
        "{} skill(s) installed, {} already up to date, {} conflict(s).",
        installed, up_to_date, conflicts
    );
    println!("Next: reopen your session in this repository to load the new skill.");

    if conflicts > 0 && installed == 0 {
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
    fn test_bundled_claude_skill_not_empty() {
        assert!(!CLAUDE_DECREE_SKILL_MD.is_empty());
    }

    #[test]
    fn test_bundled_sow_skill_not_empty() {
        assert!(!CLAUDE_SOW_SKILL_MD.is_empty());
    }

    #[test]
    fn test_claude_skill_covers_immutability() {
        assert!(
            CLAUDE_DECREE_SKILL_MD.contains("mmutab"),
            "SKILL.md must cover migration immutability"
        );
    }

    #[test]
    fn test_claude_skill_covers_given_when_then() {
        assert!(CLAUDE_DECREE_SKILL_MD.contains("Given"));
        assert!(CLAUDE_DECREE_SKILL_MD.contains("When"));
        assert!(CLAUDE_DECREE_SKILL_MD.contains("Then"));
    }

    #[test]
    fn test_claude_skill_covers_day_sized() {
        assert!(
            CLAUDE_DECREE_SKILL_MD.to_lowercase().contains("day-sized")
                || CLAUDE_DECREE_SKILL_MD.to_lowercase().contains("day sized"),
            "SKILL.md must mention day-sized migrations"
        );
    }

    #[test]
    fn test_claude_skill_covers_splitting() {
        assert!(
            CLAUDE_DECREE_SKILL_MD.contains("smallest feasible"),
            "SKILL.md must instruct splitting into smallest feasible chunks"
        );
    }

    #[test]
    fn test_claude_skill_covers_migrations_location() {
        assert!(
            CLAUDE_DECREE_SKILL_MD.contains(".decree/migrations"),
            "SKILL.md must specify the correct migration directory"
        );
    }

    #[test]
    fn test_decree_skill_has_routine_authoring_content() {
        assert!(CLAUDE_DECREE_SKILL_MD.contains("Pre-Check Section"));
        assert!(CLAUDE_DECREE_SKILL_MD.contains("Custom Parameter Discovery"));
        assert!(CLAUDE_DECREE_SKILL_MD.contains("DECREE_PRE_CHECK"));
        assert!(CLAUDE_DECREE_SKILL_MD.contains("set -euo pipefail"));
        assert!(CLAUDE_DECREE_SKILL_MD.contains("Nested Routines"));
    }

    #[test]
    fn test_sow_skill_has_required_content() {
        assert!(CLAUDE_SOW_SKILL_MD.contains("# Statement of Work Template"));
        assert!(CLAUDE_SOW_SKILL_MD.contains("Jobs to Be Done"));
        assert!(CLAUDE_SOW_SKILL_MD.contains("Acceptance Criteria"));
    }

    #[test]
    fn test_claude_skills_registry_has_two_entries() {
        assert_eq!(SKILLS.len(), 2);
        assert_eq!(SKILLS[0].name, "decree");
        assert_eq!(SKILLS[1].name, "sow");
    }
}
