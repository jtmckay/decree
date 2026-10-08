//! `decree skill` (docs/reference/cli.md): writes decree's skill files into
//! `.claude/skills/decree/` (`claude`) or `.github/skills/decree/` (`copilot`), overwriting
//! decree's own, removing a file in `reference/` decree no longer ships, and leaving any other
//! file alone; reports `unchanged` for files already identical; without `--ai`, refreshes
//! every skill folder that exists, else writes the one of the backend `init` would pick.
//! `opencode` reads no skills: an error, exit 1. Outside a project: exit 1.

use assert_cmd::cargo::cargo_bin_cmd;
use serde_json::Value;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

mod common;
use common::write_script;
#[path = "common/schema.rs"]
mod schema;

/// The skill decree ships: path under the skill folder, and content.
const SKILL: [(&str, &str); 5] = [
    (
        "SKILL.md",
        include_str!("../src/templates/skills/decree/SKILL.md"),
    ),
    (
        "reference/machines.md",
        include_str!("../src/templates/skills/decree/reference/machines.md"),
    ),
    (
        "reference/messages.md",
        include_str!("../src/templates/skills/decree/reference/messages.md"),
    ),
    (
        "reference/runs.md",
        include_str!("../src/templates/skills/decree/reference/runs.md"),
    ),
    (
        "reference/scripts.md",
        include_str!("../src/templates/skills/decree/reference/scripts.md"),
    ),
];

const CLAUDE_DIR: &str = ".claude/skills/decree";
const COPILOT_DIR: &str = ".github/skills/decree";

/// An empty project: a bare `.decree/`.
fn project() -> TempDir {
    let tmp = TempDir::new().unwrap();
    fs::create_dir(tmp.path().join(".decree")).unwrap();
    tmp
}

/// `decree <args>` in `dir`, with `path` as `PATH` when given: (exit code, stdout, stderr).
fn run_with_path(dir: &Path, args: &[&str], path: Option<&Path>) -> (i32, String, String) {
    let mut cmd = cargo_bin_cmd!("decree");
    cmd.current_dir(dir).env("NO_COLOR", "1").args(args);
    if let Some(path) = path {
        cmd.env("PATH", path);
    }
    let out = cmd.output().unwrap();
    (
        out.status.code().unwrap(),
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

fn run(dir: &Path, args: &[&str]) -> (i32, String, String) {
    run_with_path(dir, args, None)
}

/// Each skill file's path under `skill_dir`, with `suffix` (` unchanged`, or empty), one per
/// line, as text prints them.
fn lines(skill_dir: &str, suffix: &str) -> String {
    SKILL
        .iter()
        .map(|(name, _)| format!("{skill_dir}/{name}{suffix}\n"))
        .collect()
}

/// Asserts every shipped file under `root/skill_dir` is decree's.
fn assert_installed(root: &Path, skill_dir: &str) {
    for (name, content) in SKILL {
        let path = root.join(skill_dir).join(name);
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            content,
            "{}",
            path.display()
        );
    }
}

/// A directory holding `which` and an executable stub for each of `clis`, to stand for
/// `PATH`: `init`'s detection finds exactly those.
fn path_with(clis: &[&str]) -> TempDir {
    let bin = TempDir::new().unwrap();
    let which = std::process::Command::new("sh")
        .args(["-c", "command -v which"])
        .output()
        .unwrap();
    let which = PathBuf::from(String::from_utf8(which.stdout).unwrap().trim());
    symlink(&which, bin.path().join("which")).unwrap();
    for cli in clis {
        write_script(&bin.path().join(cli), "#!/bin/sh\nexit 0\n");
    }
    bin
}

#[test]
fn fresh_write_for_each_ai() {
    for (ai, dir) in [("claude", CLAUDE_DIR), ("copilot", COPILOT_DIR)] {
        let p = project();
        let (code, stdout, stderr) = run(p.path(), &["skill", "--ai", ai]);
        assert_eq!(code, 0, "{ai}: {stderr}");
        assert_eq!(stdout, lines(dir, ""), "{ai}");
        assert_installed(p.path(), dir);
        let other = if ai == "claude" {
            COPILOT_DIR
        } else {
            CLAUDE_DIR
        };
        assert!(!p.path().join(other).exists(), "{ai}");
    }
}

#[test]
fn opencode_writes_nothing_and_fails() {
    let p = project();
    let (code, stdout, stderr) = run(p.path(), &["skill", "--ai", "opencode"]);
    assert_eq!(code, 1);
    assert_eq!(stdout, "");
    assert_eq!(
        stderr,
        "error: decree writes the skill for claude (.claude/skills/decree/) or copilot \
         (.github/skills/decree/); opencode has none\n"
    );
    assert!(!p.path().join(".claude").exists());
    assert!(!p.path().join(".github").exists());
}

/// Given an older `SKILL.md`, a user file and a reference file decree no longer ships:
/// `SKILL.md` is overwritten, the user file kept, the stale reference removed, and the
/// output lists each; a second run reports every file `unchanged`.
#[test]
fn refresh_overwrites_keeps_user_files_and_removes_stale_references() {
    let p = project();
    let dir = p.path().join(CLAUDE_DIR);
    fs::create_dir_all(dir.join("reference")).unwrap();
    for (name, content) in &SKILL[1..] {
        fs::write(dir.join(name), content).unwrap();
    }
    fs::write(dir.join("SKILL.md"), "# An older decree's skill\n").unwrap();
    fs::write(dir.join("notes.md"), "my notes\n").unwrap();
    fs::write(dir.join("reference/old.md"), "gone in this version\n").unwrap();

    let (code, stdout, stderr) = run(p.path(), &["skill"]);
    assert_eq!(code, 0, "{stderr}");
    let mut expected = format!("{CLAUDE_DIR}/SKILL.md\n");
    for (name, _) in &SKILL[1..] {
        expected.push_str(&format!("{CLAUDE_DIR}/{name} unchanged\n"));
    }
    expected.push_str(&format!("{CLAUDE_DIR}/reference/old.md removed\n"));
    assert_eq!(stdout, expected);
    assert_installed(p.path(), CLAUDE_DIR);
    assert_eq!(
        fs::read_to_string(dir.join("notes.md")).unwrap(),
        "my notes\n"
    );
    assert!(!dir.join("reference/old.md").exists());
    // Only the existing folder is refreshed.
    assert!(!p.path().join(".github").exists());

    let (code, stdout, _) = run(p.path(), &["skill"]);
    assert_eq!(code, 0);
    assert_eq!(stdout, lines(CLAUDE_DIR, " unchanged"));
    assert_eq!(
        fs::read_to_string(dir.join("notes.md")).unwrap(),
        "my notes\n"
    );
}

/// Without `--ai`, every skill folder that exists is refreshed, claude's then copilot's.
#[test]
fn refreshes_every_existing_skill_folder() {
    let p = project();
    fs::create_dir_all(p.path().join(CLAUDE_DIR)).unwrap();
    fs::create_dir_all(p.path().join(COPILOT_DIR)).unwrap();
    let (code, stdout, stderr) = run(p.path(), &["skill"]);
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(stdout, lines(CLAUDE_DIR, "") + &lines(COPILOT_DIR, ""));
    assert_installed(p.path(), CLAUDE_DIR);
    assert_installed(p.path(), COPILOT_DIR);
}

/// Without `--ai` and with no skill folder, the backend is picked as `init` picks it: the
/// first of opencode, claude, copilot on `PATH`, else opencode, which reads no skills.
#[test]
fn without_a_folder_picks_the_backend_as_init_does() {
    for (clis, written) in [
        (&["claude", "copilot"][..], Some(CLAUDE_DIR)),
        (&["copilot"][..], Some(COPILOT_DIR)),
        (&["opencode", "claude"][..], None),
        (&[][..], None),
    ] {
        let p = project();
        let bin = path_with(clis);
        let (code, stdout, stderr) = run_with_path(p.path(), &["skill"], Some(bin.path()));
        match written {
            Some(dir) => {
                assert_eq!(code, 0, "{clis:?}: {stderr}");
                assert_eq!(stdout, lines(dir, ""), "{clis:?}");
                assert_installed(p.path(), dir);
            }
            None => {
                assert_eq!(code, 1, "{clis:?}");
                assert_eq!(stdout, "", "{clis:?}");
                assert!(
                    stderr.ends_with("; opencode has none\n"),
                    "{clis:?}: {stderr}"
                );
                assert!(!p.path().join(".claude").exists(), "{clis:?}");
                assert!(!p.path().join(".github").exists(), "{clis:?}");
            }
        }
    }
}

#[test]
fn no_project_exits_1() {
    let tmp = TempDir::new().unwrap();
    let (code, stdout, stderr) = run(tmp.path(), &["skill", "--ai", "claude"]);
    assert_eq!(code, 1);
    assert_eq!(stdout, "");
    assert!(stderr.contains("not inside a decree project"), "{stderr}");
    assert!(!tmp.path().join(".claude").exists());
}

/// `--format json` validates against `skill.schema.json`, on a fresh write, a refresh with
/// a removal, and a run with nothing to do.
#[test]
fn json_output_matches_its_schema() {
    let validator = schema::validator(schema::CLI_SKILL_SCHEMA);
    let json = |root: &Path| -> Value {
        let (code, stdout, stderr) = run(root, &["skill", "--format", "json"]);
        assert_eq!(code, 0, "{stderr}");
        let doc: Value = serde_json::from_str(&stdout).unwrap();
        let errors = schema::errors(&validator, &doc);
        assert!(errors.is_empty(), "{errors:?}\n{doc:#}");
        doc
    };
    let paths = |dir: &str| -> Vec<String> {
        SKILL
            .iter()
            .map(|(name, _)| format!("{dir}/{name}"))
            .collect()
    };

    let p = project();
    fs::create_dir_all(p.path().join(COPILOT_DIR).join("reference")).unwrap();
    fs::write(p.path().join(COPILOT_DIR).join("reference/old.md"), "x\n").unwrap();
    assert_eq!(
        json(p.path()),
        serde_json::json!({
            "written": paths(COPILOT_DIR),
            "unchanged": [],
            "removed": [format!("{COPILOT_DIR}/reference/old.md")],
        })
    );
    assert_eq!(
        json(p.path()),
        serde_json::json!({
            "written": [],
            "unchanged": paths(COPILOT_DIR),
            "removed": [],
        })
    );
}
