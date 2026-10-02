//! `decree graph` (spec section 9) against the fixtures in `tests/fixtures/graph/` and the
//! documents in `mock/.decree/graph/`. Each test copies its `.decree/` into a temp directory.

use assert_cmd::cargo::cargo_bin_cmd;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const CONFIG: &str = "max_attempts: 3\n";

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(rel: &str) -> String {
    fs::read_to_string(repo().join(rel)).unwrap()
}

/// Copy `src` to `dst` recursively.
fn copy_dir(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).unwrap();
    for entry in fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let to = dst.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &to);
        } else {
            fs::copy(entry.path(), &to).unwrap();
        }
    }
}

/// A temp project whose `.decree/` is a copy of `tree`, with a default `config.yml` unless
/// the tree has its own.
fn project(tree: &Path) -> TempDir {
    let tmp = TempDir::new().unwrap();
    let decree = tmp.path().join(".decree");
    copy_dir(tree, &decree);
    if !decree.join("config.yml").exists() {
        fs::write(decree.join("config.yml"), CONFIG).unwrap();
    }
    tmp
}

/// A temp project holding only the given machine files.
fn machines_project(files: &[&str]) -> TempDir {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join(".decree/machines");
    fs::create_dir_all(&dir).unwrap();
    fs::write(tmp.path().join(".decree/config.yml"), CONFIG).unwrap();
    for file in files {
        fs::copy(
            repo().join("tests/fixtures/machines").join(file),
            dir.join(file),
        )
        .unwrap();
    }
    tmp
}

/// Run `decree graph` in `tmp`: (exit code, stdout, stderr).
fn graph(tmp: &TempDir) -> (i32, String, String) {
    let out = cargo_bin_cmd!("decree")
        .current_dir(tmp.path())
        .env("NO_COLOR", "1")
        .arg("graph")
        .output()
        .unwrap();
    (
        out.status.code().unwrap(),
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

/// The file `decree graph` wrote, `.decree/graph/<name>`.
fn written(tmp: &TempDir, name: &str) -> String {
    fs::read_to_string(tmp.path().join(".decree/graph").join(name)).unwrap()
}

/// The `.md` filenames in `dir`, sorted.
fn md_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|n| n.ends_with(".md"))
        .collect();
    names.sort();
    names
}

#[test]
fn graph_writes_one_file_per_machine_and_system_md() {
    let tmp = machines_project(&["feature.yml", "hello.yml", "claude_router.yml"]);
    fs::write(
        tmp.path().join(".decree/config.yml"),
        format!("{CONFIG}default_router: claude_router\n"),
    )
    .unwrap();
    let (code, stdout, stderr) = graph(&tmp);
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(
        stdout,
        ".decree/graph/claude_router.md\n.decree/graph/feature.md\n.decree/graph/hello.md\n.decree/graph/system.md\n"
    );
    assert_eq!(
        written(&tmp, "feature.md"),
        read("tests/fixtures/graph/feature.md")
    );
    assert!(written(&tmp, "hello.md")
        .contains("Machine: [machines/hello.yml](../machines/hello.yml)\n"));
}

#[test]
fn graph_system_matches_fixture() {
    let tmp = project(&repo().join("tests/fixtures/graph/system"));
    let (code, _, stderr) = graph(&tmp);
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(
        written(&tmp, "system.md"),
        read("tests/fixtures/graph/system.md")
    );
}

#[test]
fn graph_mock_rewrites_mock_graph_unchanged() {
    let tmp = project(&repo().join("mock/.decree"));
    let (code, _, stderr) = graph(&tmp);
    assert_eq!(code, 0, "{stderr}");
    let mock = repo().join("mock/.decree/graph");
    let names = md_names(&mock);
    assert_eq!(names.len(), 8, "{names:?}");
    assert_eq!(md_names(&tmp.path().join(".decree/graph")), names);
    for name in &names {
        assert_eq!(
            written(&tmp, name),
            fs::read_to_string(mock.join(name)).unwrap(),
            "{name}"
        );
    }
}

#[test]
fn graph_removes_stale_md_files_only() {
    let tmp = machines_project(&["hello.yml"]);
    let dir = tmp.path().join(".decree/graph");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("gone.md"), "# gone\n").unwrap();
    fs::write(dir.join("notes.txt"), "kept\n").unwrap();
    let (code, _, stderr) = graph(&tmp);
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(md_names(&dir), ["hello.md", "system.md"]);
    assert!(dir.join("notes.txt").exists());
}

#[test]
fn graph_cron_without_machine_points_at_default_machine() {
    let tmp = machines_project(&["hello.yml"]);
    let decree = tmp.path().join(".decree");
    fs::write(
        decree.join("config.yml"),
        format!("{CONFIG}default_routine: hello\n"),
    )
    .unwrap();
    fs::create_dir_all(decree.join("cron")).unwrap();
    fs::write(
        decree.join("cron/Hourly.md"),
        "---\ncron: \"0 * * * *\"\n---\nTick.\n",
    )
    .unwrap();
    let (code, _, stderr) = graph(&tmp);
    assert_eq!(code, 0, "{stderr}");
    let system = written(&tmp, "system.md");
    assert!(
        system.contains("    cron___ourly[/\"cron: Hourly\"/]\n    cron___ourly -->|cron| hello\n"),
        "{system}"
    );
}

#[test]
fn graph_takes_no_machine_argument() {
    let tmp = machines_project(&["hello.yml"]);
    let out = cargo_bin_cmd!("decree")
        .current_dir(tmp.path())
        .args(["graph", "hello"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(!tmp.path().join(".decree/graph").exists());
}

#[test]
fn graph_help_has_viewing_instructions() {
    let out = cargo_bin_cmd!("decree")
        .args(["graph", "--help"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let help = String::from_utf8(out.stdout).unwrap();
    for needle in [
        "Run `decree graph`, then open `.decree/graph/<machine>.md` (or `system.md`)",
        "Ctrl+Shift+V",
        "Cmd+Shift+V",
        "VS Code 1.121",
        "GitHub, GitLab and Obsidian",
        "https://mermaid.live",
    ] {
        assert!(help.contains(needle), "missing {needle:?} in:\n{help}");
    }
}
