//! `decree graph` (docs/reference/graph.md) against the `system` fixture in `tests/fixtures/graph/`
//! and the machines and documents of every project in `examples/`, of the escalation ladder in
//! `tests/fixtures/escalation/` and of the `feature` fixture in `tests/fixtures/feature/`. Each test copies its `.decree/` into a temp directory.

use assert_cmd::cargo::cargo_bin_cmd;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

mod common;

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

/// A temp project whose `.decree/` is a copy of `tree`.
fn project(tree: &Path) -> TempDir {
    let tmp = TempDir::new().unwrap();
    copy_dir(tree, &tmp.path().join(".decree"));
    tmp
}

/// A temp project holding only the given machine files, relative to the repository.
fn machines_project(files: &[&str]) -> TempDir {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join(".decree/machines");
    fs::create_dir_all(&dir).unwrap();
    for file in files {
        let from = repo().join(file);
        match file.strip_suffix(".md") {
            // A plugin file: the machine it installs.
            Some(plugin) => {
                let name = Path::new(plugin).file_name().unwrap().to_str().unwrap();
                common::write_file(
                    &dir.join(name).join("machine.yml"),
                    common::plugin_machine(file),
                );
            }
            // `machines/<name>/machine.yml`: the machine's folder names it.
            None => {
                let name = from.parent().unwrap().file_name().unwrap();
                fs::create_dir_all(dir.join(name)).unwrap();
                fs::copy(&from, dir.join(name).join("machine.yml")).unwrap();
            }
        }
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

/// The file `decree graph` wrote, `.decree/<rel>`: `machines/<name>/graph.md` or `system.md`.
fn written(tmp: &TempDir, rel: &str) -> String {
    fs::read_to_string(tmp.path().join(".decree").join(rel)).unwrap()
}

/// Every graph file in `decree` (a `.decree/`), relative to it, sorted: each
/// `machines/<name>/graph.md`, and `system.md`.
fn graph_files(decree: &Path) -> Vec<String> {
    let mut names = Vec::new();
    if let Ok(entries) = fs::read_dir(decree.join("machines")) {
        for e in entries {
            let name = e.unwrap().file_name().into_string().unwrap();
            if decree
                .join("machines")
                .join(&name)
                .join("graph.md")
                .is_file()
            {
                names.push(format!("machines/{name}/graph.md"));
            }
        }
    }
    if decree.join("system.md").is_file() {
        names.push("system.md".to_string());
    }
    names.sort();
    names
}

#[test]
fn graph_writes_one_file_per_machine_and_system_md() {
    // `feature` has a compound state, a check, a model and a person.
    let tmp = machines_project(&[
        "tests/fixtures/feature/.decree/machines/feature/machine.yml",
        "examples/project/.decree/plugins/hello.md",
        "tests/fixtures/feature/.decree/machines/router/machine.yml",
    ]);
    let (code, stdout, stderr) = graph(&tmp);
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(
        stdout,
        ".decree/machines/feature/graph.md\n.decree/machines/hello/graph.md\n.decree/machines/router/graph.md\n.decree/system.md\n"
    );
    assert_eq!(
        written(&tmp, "machines/feature/graph.md"),
        read("tests/fixtures/feature/.decree/machines/feature/graph.md")
    );
    assert!(written(&tmp, "machines/hello/graph.md")
        .contains("Machine: [machines/hello/machine.yml](machine.yml)\n"));
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

/// The example projects: each directory of `examples/` with a `.decree/`, the escalation
/// ladder in `tests/fixtures/escalation/` and the `feature` fixture, in name order.
fn example_projects() -> Vec<PathBuf> {
    let mut projects: Vec<PathBuf> = fs::read_dir(repo().join("examples"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.join(".decree").is_dir())
        .collect();
    projects.push(repo().join("tests/fixtures/escalation"));
    projects.push(repo().join("tests/fixtures/feature"));
    projects.sort();
    projects
}

#[test]
fn graph_rewrites_every_example_graph_unchanged() {
    let projects = example_projects();
    assert!(projects.len() >= 2, "{projects:?}");
    for project_dir in projects {
        let tmp = project(&project_dir.join(".decree"));
        // A project of plugins draws what `decree install` installs.
        if tmp.path().join(".decree/plugins").is_dir() {
            common::decree_at(tmp.path())
                .args(["install"])
                .assert()
                .success();
        }
        let (code, _, stderr) = graph(&tmp);
        assert_eq!(code, 0, "{}: {stderr}", project_dir.display());
        let committed = project_dir.join(".decree");
        let names = graph_files(&committed);
        assert!(names.contains(&"system.md".to_string()), "{names:?}");
        assert_eq!(graph_files(&tmp.path().join(".decree")), names);
        for name in &names {
            assert_eq!(
                written(&tmp, name),
                fs::read_to_string(committed.join(name)).unwrap(),
                "{}: {name}",
                project_dir.display()
            );
        }
    }
}

#[test]
fn graph_removes_stale_graph_files_only() {
    let tmp = machines_project(&["examples/project/.decree/plugins/hello.md"]);
    let decree = tmp.path().join(".decree");
    common::write_file(&decree.join("machines/gone/graph.md"), "# gone\n");
    fs::write(decree.join("machines/hello/notes.md"), "kept\n").unwrap();
    let (code, _, stderr) = graph(&tmp);
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(
        graph_files(&decree),
        ["machines/hello/graph.md", "system.md"]
    );
    assert!(decree.join("machines/hello/notes.md").exists());
}

/// A cron file that names no machine runs the machine whose folder holds it.
#[test]
fn graph_cron_without_machine_runs_its_folders_machine() {
    let tmp = machines_project(&["examples/project/.decree/plugins/hello.md"]);
    let decree = tmp.path().join(".decree");
    common::write_file(
        &decree.join("machines/hello/cron/Hourly.md"),
        "---\ncron: \"0 * * * *\"\n---\nTick.\n",
    );
    let (code, _, stderr) = graph(&tmp);
    assert_eq!(code, 0, "{stderr}");
    let system = written(&tmp, "system.md");
    assert!(system.contains("|cron| hello"), "{system}");
}

#[test]
fn graph_takes_no_machine_argument() {
    let tmp = machines_project(&["examples/project/.decree/plugins/hello.md"]);
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
        "Run `decree graph`, then open `.decree/machines/<machine>/graph.md` (or `.decree/system.md`)",
        "Ctrl+Shift+V",
        "Cmd+Shift+V",
        "VS Code 1.121",
        "GitHub, GitLab and Obsidian",
        "https://mermaid.live",
    ] {
        assert!(help.contains(needle), "missing {needle:?} in:\n{help}");
    }
}
