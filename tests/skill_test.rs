//! The official `skill` plugin (plugins/skill.md): `decree init` adds it and writes the skill
//! for its backend; its `install`, at each daemon start, copies each file that differs into
//! each skill folder the project has (`.claude/skills/decree/`, `.github/skills/decree/`),
//! removes a file in `reference/` the skill no longer ships, and leaves any other file alone.

use std::fs;
use std::path::Path;
use tempfile::TempDir;

mod common;

/// The skill the plugin ships: path under the skill folder, and content.
fn skill() -> Vec<(String, String)> {
    let text =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("plugins/skill.md")).unwrap();
    let yaml = text
        .strip_prefix("---\n")
        .and_then(|t| t.split_once("\n---\n"))
        .unwrap()
        .0;
    let fm: serde_norway::Value = serde_norway::from_str(yaml).unwrap();
    fm["scripts"]
        .as_mapping()
        .unwrap()
        .iter()
        .filter_map(|(k, v)| {
            let path = k.as_str().unwrap().strip_prefix("skill/")?;
            Some((path.to_string(), v.as_str().unwrap().to_string()))
        })
        .collect()
}

/// `decree init --ai <ai>` in a new project.
fn init(ai: &str) -> TempDir {
    let tmp = TempDir::new().unwrap();
    common::decree_at(tmp.path())
        .args(["init", "--ai", ai])
        .assert()
        .success();
    tmp
}

/// Run the installed `install.sh` in `root`, as the plugin's run does.
fn install(root: &Path) -> String {
    let out = std::process::Command::new("bash")
        .arg(".decree/machines/skill/scripts/install.sh")
        .current_dir(root)
        .env_remove("AI_CLI")
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    String::from_utf8(out.stdout).unwrap()
}

fn assert_installed(root: &Path, dir: &str) {
    for (name, content) in skill() {
        let installed = fs::read_to_string(root.join(dir).join(&name)).unwrap();
        assert_eq!(installed, content, "{dir}/{name}");
    }
}

/// `init` adds the plugin and writes the skill for Claude and Copilot; nothing for OpenCode,
/// which reads no skills.
#[test]
fn init_writes_the_skill_for_its_backend() {
    let p = init("claude");
    assert!(p.path().join(".decree/plugins/skill.md").is_file());
    assert_installed(p.path(), ".claude/skills/decree");
    let p = init("copilot");
    assert_installed(p.path(), ".github/skills/decree");
    let p = init("opencode");
    assert!(p.path().join(".decree/plugins/skill.md").is_file());
    assert!(!p.path().join(".claude").exists() && !p.path().join(".github").exists());
}

/// `install` writes only what differs, keeps the project's own files, removes a stale
/// reference file, and refreshes every skill folder that exists.
#[test]
fn install_refreshes_keeps_user_files_and_removes_stale_references() {
    let p = init("claude");
    let root = p.path();
    assert!(install(root).contains(".claude/skills/decree is current"));
    let claude = root.join(".claude/skills/decree");
    fs::write(claude.join("SKILL.md"), "old\n").unwrap();
    fs::write(claude.join("reference/gone.md"), "stale\n").unwrap();
    fs::write(claude.join("notes.md"), "mine\n").unwrap();
    fs::create_dir_all(root.join(".github/skills/decree")).unwrap();
    let out = install(root);
    assert!(
        out.contains("skill: wrote .claude/skills/decree/SKILL.md\n"),
        "{out}"
    );
    assert!(
        out.contains("skill: removed .claude/skills/decree/reference/gone.md\n"),
        "{out}"
    );
    assert!(
        !out.contains("wrote .claude/skills/decree/reference/"),
        "{out}"
    );
    assert_installed(root, ".claude/skills/decree");
    assert_installed(root, ".github/skills/decree");
    assert_eq!(
        fs::read_to_string(claude.join("notes.md")).unwrap(),
        "mine\n"
    );
    assert!(!claude.join("reference/gone.md").exists());
}

/// The plugin's run at daemon start installs the skill: an edit in the skill folder is
/// written over.
#[test]
fn the_daemon_start_installs_the_skill() {
    let p = init("claude");
    let skill_md = p.path().join(".claude/skills/decree/SKILL.md");
    fs::write(&skill_md, "edited\n").unwrap();
    common::decree_at(p.path())
        .args(["daemon", "--once", "--machine", "skill"])
        .assert()
        .success();
    assert_eq!(fs::read_to_string(&skill_md).unwrap(), skill()[0].1);
}
