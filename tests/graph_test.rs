//! `decree graph` (spec section 9) against the fixtures in `tests/fixtures/graph/` and the
//! documents in `mock/graph/`. Each test copies its `.decree/` into a temp directory.

use assert_cmd::cargo::cargo_bin_cmd;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const CONFIG: &str = "commands:\n  ai_router: \"true {prompt}\"\n";

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

/// Run `decree graph [args]` in `tmp`: (exit code, stdout, stderr).
fn graph(tmp: &TempDir, args: &[&str]) -> (i32, String, String) {
    let out = cargo_bin_cmd!("decree")
        .current_dir(tmp.path())
        .env("NO_COLOR", "1")
        .arg("graph")
        .args(args)
        .output()
        .unwrap();
    (
        out.status.code().unwrap(),
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

#[test]
fn graph_feature_matches_fixture() {
    let tmp = machines_project(&["feature.yml", "hello.yml"]);
    let (code, stdout, stderr) = graph(&tmp, &["feature"]);
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(stdout, read("tests/fixtures/graph/feature.md"));
}

#[test]
fn graph_system_matches_fixture() {
    let tmp = project(&repo().join("tests/fixtures/graph/system"));
    let (code, stdout, stderr) = graph(&tmp, &[]);
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(stdout, read("tests/fixtures/graph/system.md"));
}

#[test]
fn graph_mock_matches_mock_graph() {
    let tmp = project(&repo().join("mock/.decree"));
    let machines: Vec<String> = fs::read_dir(repo().join("mock/.decree/machines"))
        .unwrap()
        .map(|e| {
            let path = e.unwrap().path();
            path.file_stem().unwrap().to_str().unwrap().to_string()
        })
        .collect();
    assert_eq!(machines.len(), 5);
    for m in &machines {
        let (code, stdout, stderr) = graph(&tmp, &[m]);
        assert_eq!(code, 0, "{m}: {stderr}");
        assert_eq!(stdout, read(&format!("mock/graph/{m}.md")), "{m}");
    }
    let (code, stdout, stderr) = graph(&tmp, &[]);
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(stdout, read("mock/graph/system.md"));
}

#[test]
fn graph_unknown_machine_exits_1() {
    let tmp = machines_project(&["hello.yml"]);
    let (code, stdout, stderr) = graph(&tmp, &["nope"]);
    assert_eq!(code, 1);
    assert!(stdout.is_empty(), "{stdout}");
    assert!(stderr.contains("unknown machine `nope`"), "{stderr}");
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
    let (code, stdout, stderr) = graph(&tmp, &[]);
    assert_eq!(code, 0, "{stderr}");
    assert!(
        stdout.contains("    cron___ourly[/\"cron: Hourly\"/]\n    cron___ourly -->|cron| hello\n"),
        "{stdout}"
    );
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
        "decree graph feature > feature.md",
        "decree graph > machines.md",
        "Ctrl+Shift+V",
        "Cmd+Shift+V",
        "VS Code 1.121",
        "GitHub, GitLab and Obsidian",
        "https://mermaid.live",
    ] {
        assert!(help.contains(needle), "missing {needle:?} in:\n{help}");
    }
}
