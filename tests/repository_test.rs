//! This repository as git holds it (A10): no installed plugin is tracked, every machine the
//! examples used to hold is an official plugin, and `decree check` passes in a fresh clone,
//! in the root and in every example with a `.decree/`, before any other command has run.
//! Copies the files git would clone into temp directories; writes nothing here.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The files a fresh clone has, relative to the repository: tracked, plus untracked ones not
/// ignored (what the next commit adds). `None` without git (a source tarball).
fn clone_files() -> Option<Vec<String>> {
    let out = Command::new("git")
        .args([
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ])
        .current_dir(repo())
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    let mut files: Vec<String> = String::from_utf8(out.stdout)
        .unwrap()
        .split('\0')
        .filter(|p| !p.is_empty())
        .filter(|p| repo().join(p).is_file())
        .map(String::from)
        .collect();
    files.sort();
    Some(files)
}

/// AC (A10, D86): of any `.decree/machines/` outside `tests/fixtures/`, git holds only what a
/// project commits (each `graph.md`, `order/` and `*.git.*` file), never an installed copy;
/// and every machine `examples/` held is an official plugin in `plugins/`.
#[test]
fn no_installed_copy_is_tracked_and_the_examples_machines_are_official_plugins() {
    let Some(files) = clone_files() else {
        eprintln!("not a git work tree: skipping");
        return;
    };
    let tracked: Vec<&String> = files
        .iter()
        .filter(|f| !f.starts_with("tests/fixtures/"))
        .filter(|f| f.contains(".decree/machines/") || f.contains(".decree/scripts/"))
        .filter(|f| !(f.ends_with("/graph.md") || f.contains("/order/") || f.contains(".git.")))
        .collect();
    assert!(tracked.is_empty(), "installed copies in git: {tracked:?}");
    for name in [
        "decree_newsletter",
        "decree_comfy",
        "decree_develop_by_size",
        "decree_gliner_router",
        "decree_llm_router",
        "decree_lease",
        "decree_develop",
        "decree_infra",
        "decree_qa",
        "decree_router",
    ] {
        let file = format!("plugins/{name}.md");
        assert!(files.contains(&file), "{file} is not in git");
    }
    for plugins in [".decree/plugins/", "examples/project/.decree/plugins/"] {
        assert!(
            files.iter().any(|f| f.starts_with(plugins)),
            "{plugins} is not in git"
        );
    }
}

/// AC (A10, D85): in a fresh clone's root and in every directory under `examples/` that has a
/// `.decree/`, `decree install` installs the plugins, and `decree check` then passes.
#[test]
fn check_passes_in_a_fresh_clone() {
    let Some(files) = clone_files() else {
        eprintln!("not a git work tree: skipping");
        return;
    };
    let clone = tempfile::tempdir().unwrap();
    for file in &files {
        let to = clone.path().join(file);
        fs::create_dir_all(to.parent().unwrap()).unwrap();
        fs::copy(repo().join(file), &to).unwrap();
    }
    let mut projects = vec![clone.path().to_path_buf()];
    for entry in fs::read_dir(clone.path().join("examples")).unwrap() {
        let dir = entry.unwrap().path();
        if dir.join(".decree").is_dir() {
            projects.push(dir);
        }
    }
    assert!(projects.len() >= 2, "{projects:?}");
    for project in &projects {
        // A fresh clone has no machine installed: only what a project commits.
        let installed: Vec<PathBuf> = fs::read_dir(project.join(".decree/machines"))
            .into_iter()
            .flatten()
            .map(|e| e.unwrap().path().join("machine.yml"))
            .filter(|p| p.exists())
            .collect();
        assert!(installed.is_empty(), "{installed:?}");
        let install = Command::new(env!("CARGO_BIN_EXE_decree"))
            .args(["install"])
            .current_dir(project)
            .output()
            .unwrap();
        assert!(
            install.status.success(),
            "decree install in {}:\n{}",
            project.display(),
            String::from_utf8_lossy(&install.stderr)
        );
        let out = Command::new(env!("CARGO_BIN_EXE_decree"))
            .arg("check")
            .current_dir(project)
            .env("NO_COLOR", "1")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "decree check in {} ({}):\n{}{}",
            project
                .strip_prefix(clone.path())
                .unwrap_or(Path::new("."))
                .display(),
            out.status,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            project.join(".decree/machines").is_dir(),
            "{}",
            project.display()
        );
    }
    // AC (A14): this repository's own `develop` is its own plugin, extending `decree_develop`,
    // and its order files and `run-queue.sh` are where they were.
    let root = clone.path();
    let out = Command::new(env!("CARGO_BIN_EXE_decree"))
        .arg("list")
        .current_dir(root)
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    let list = String::from_utf8_lossy(&out.stdout);
    let develop = list
        .lines()
        .find(|l| l.trim_start().starts_with("develop "))
        .unwrap_or_else(|| panic!("{list}"));
    assert!(develop.ends_with("[own, extends decree_develop]"), "{list}");
    assert!(root
        .join(".decree/machines/develop/order/A14-extends.md")
        .is_file());
    assert!(root
        .join(".decree/machines/develop/scripts/run-queue.sh")
        .is_file());
    assert!(root
        .join(".decree/machines/develop/state/order.git.txt")
        .is_file());
}
